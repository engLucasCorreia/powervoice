//! Gate core (SPEC-016 §4.7): hysteresis (open threshold for a closed gate, close threshold for
//! an open one), hold, a raised-cosine opening of exactly `attack` samples and an exponential
//! release toward the floor.

use super::SNAP_LIN;

/// Gate state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GatePhase {
    /// At the floor.
    Closed,
    /// Raised-cosine ramp up.
    Opening,
    /// Unity gain.
    Open,
    /// Exponential approach to the floor.
    Releasing,
}

/// The gate state machine. Timing values are in samples; all per-sample work is O(1).
#[derive(Clone, Debug)]
pub struct GateCore {
    phase: GatePhase,
    hold_samples: u32,
    hold_left: u32,
    attack_samples: u32,
    alpha_release: f64,
    gain: f64,
    ramp_from: f64,
    ramp_pos: u32,
}

impl GateCore {
    /// A closed gate at `floor`.
    pub fn new(attack_samples: u32, hold_samples: u32, alpha_release: f64, floor: f64) -> Self {
        Self {
            phase: GatePhase::Closed,
            hold_samples,
            hold_left: 0,
            attack_samples: attack_samples.max(1),
            alpha_release,
            gain: floor,
            ramp_from: floor,
            ramp_pos: 0,
        }
    }

    /// Back to `Closed` at `floor` (reset / activate).
    pub fn reset(&mut self, floor: f64) {
        self.phase = GatePhase::Closed;
        self.hold_left = 0;
        self.gain = floor;
        self.ramp_from = floor;
        self.ramp_pos = 0;
    }

    /// New attack length (`>= 1`). An opening in progress restarts from the current gain, so the
    /// gain stays continuous.
    pub fn set_attack(&mut self, samples: u32) {
        self.attack_samples = samples.max(1);
        if self.phase == GatePhase::Opening {
            self.ramp_from = self.gain;
            self.ramp_pos = 0;
        }
    }

    /// New hold length; a running hold keeps counting from its remaining value, capped.
    pub fn set_hold(&mut self, samples: u32) {
        self.hold_samples = samples;
        self.hold_left = self.hold_left.min(samples);
    }

    /// New release coefficient.
    pub fn set_release(&mut self, alpha: f64) {
        self.alpha_release = alpha;
    }

    /// Current phase.
    pub fn phase(&self) -> GatePhase {
        self.phase
    }

    /// True while `Opening` or `Open` (the "gate open" indicator).
    pub fn is_open(&self) -> bool {
        matches!(self.phase, GatePhase::Opening | GatePhase::Open)
    }

    /// Current linear gain.
    pub fn gain(&self) -> f64 {
        self.gain
    }

    /// One sample: `level_db` against `open_db` (closed gate) or `close_db` (open gate), toward
    /// the linear `floor`. Returns the gain.
    pub fn process(&mut self, level_db: f64, open_db: f64, close_db: f64, floor: f64) -> f64 {
        match self.phase {
            GatePhase::Opening | GatePhase::Open => {
                if level_db >= close_db {
                    self.hold_left = self.hold_samples;
                } else if self.hold_left > 0 {
                    self.hold_left -= 1;
                } else {
                    self.phase = GatePhase::Releasing;
                }
            }
            GatePhase::Closed | GatePhase::Releasing => {
                if level_db >= open_db {
                    self.phase = GatePhase::Opening;
                    self.ramp_from = self.gain;
                    self.ramp_pos = 0;
                    self.hold_left = self.hold_samples;
                }
            }
        }
        match self.phase {
            GatePhase::Opening => {
                self.ramp_pos += 1;
                if self.ramp_pos >= self.attack_samples {
                    self.gain = 1.0;
                    self.phase = GatePhase::Open;
                } else {
                    let p = f64::from(self.ramp_pos) / f64::from(self.attack_samples);
                    let shape = (1.0 - (std::f64::consts::PI * p).cos()) / 2.0;
                    self.gain = self.ramp_from + (1.0 - self.ramp_from) * shape;
                }
            }
            GatePhase::Open => self.gain = 1.0,
            GatePhase::Releasing => {
                self.gain = floor + (self.gain - floor) * self.alpha_release;
                if (self.gain - floor).abs() <= SNAP_LIN {
                    self.gain = floor;
                    self.phase = GatePhase::Closed;
                }
            }
            GatePhase::Closed => self.gain = floor,
        }
        self.gain
    }
}

#[cfg(test)]
mod tests {
    use super::super::time_constant_alpha;
    use super::*;

    const OPEN: f64 = -40.0;
    const CLOSE: f64 = -43.0;

    #[test]
    fn opens_with_raised_cosine_of_exact_length() {
        let mut g = GateCore::new(96, 10, time_constant_alpha(100.0, 48_000.0), 0.0);
        assert_eq!(g.process(-50.0, OPEN, CLOSE, 0.0).to_bits(), 0f64.to_bits());
        for p in 1..=96u32 {
            let got = g.process(-20.0, OPEN, CLOSE, 0.0);
            let want = (1.0 - (std::f64::consts::PI * f64::from(p) / 96.0).cos()) / 2.0;
            assert!((got - want).abs() < 1e-12, "p {p}: {got} vs {want}");
        }
        assert_eq!(g.phase(), GatePhase::Open);
        assert_eq!(g.gain().to_bits(), 1f64.to_bits());
    }

    #[test]
    fn hysteresis_hold_and_release() {
        let alpha = time_constant_alpha(1.0, 48_000.0);
        let mut g = GateCore::new(1, 5, alpha, 0.0);
        g.process(-39.0, OPEN, CLOSE, 0.0);
        assert!(g.is_open());
        // Between the thresholds an open gate stays open indefinitely.
        for _ in 0..1000 {
            assert_eq!(g.process(-42.9, OPEN, CLOSE, 0.0).to_bits(), 1f64.to_bits());
        }
        // Below the close threshold: 5 hold samples at unity, then the release.
        for _ in 0..5 {
            assert_eq!(g.process(-50.0, OPEN, CLOSE, 0.0).to_bits(), 1f64.to_bits());
        }
        let first = g.process(-50.0, OPEN, CLOSE, 0.0);
        assert!((first - alpha).abs() < 1e-15);
        assert_eq!(g.phase(), GatePhase::Releasing);
        // A closed / releasing gate ignores levels between the thresholds.
        assert!(g.process(-41.0, OPEN, CLOSE, 0.0) < first);
        let n = (0..10_000)
            .position(|_| g.process(-50.0, OPEN, CLOSE, 0.0) == 0.0)
            .unwrap();
        assert!(n < 14 * 48 + 1, "exact silence after {n} samples");
        assert_eq!(g.phase(), GatePhase::Closed);
    }

    #[test]
    fn reopen_mid_release_is_continuous_and_attack_change_restarts() {
        let mut g = GateCore::new(48, 0, time_constant_alpha(10.0, 48_000.0), 0.1);
        g.process(0.0, OPEN, CLOSE, 0.1);
        for _ in 0..60 {
            g.process(0.0, OPEN, CLOSE, 0.1);
        }
        for _ in 0..200 {
            g.process(-90.0, OPEN, CLOSE, 0.1);
        }
        let g0 = g.gain();
        assert!(g0 > 0.1 && g0 < 1.0);
        let next = g.process(0.0, OPEN, CLOSE, 0.1);
        assert!(next > g0 && next - g0 < 0.01, "{g0} → {next}");
        let before = g.gain();
        g.set_attack(4);
        let after = g.process(0.0, OPEN, CLOSE, 0.1);
        assert!(after >= before && after - before < 0.5);
        for _ in 0..3 {
            g.process(0.0, OPEN, CLOSE, 0.1);
        }
        assert_eq!(g.gain().to_bits(), 1f64.to_bits());
        g.reset(0.1);
        assert_eq!(g.phase(), GatePhase::Closed);
        assert_eq!(g.gain().to_bits(), 0.1f64.to_bits());
    }
}
