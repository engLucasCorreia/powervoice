//! Attack/release smoother on a section gain in dB (SPEC-016 §4.6): a branching one-pole,
//! `G ← G_t + (G − G_t)·α`, with `τ` the 63.2 % time of a step.

use super::SNAP_DB;

/// Which way the gain moves in response to a **rising** level (the attack direction).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttackDirection {
    /// Compressor, limiter: the gain falls (`G_t < G` is attack).
    Down,
    /// Expander: the gain rises (`G_t > G` is attack).
    Up,
}

/// Branching one-pole smoother in the dB gain domain.
#[derive(Clone, Debug)]
pub struct Ballistics {
    gain_db: f64,
    alpha_attack: f64,
    alpha_release: f64,
    direction: AttackDirection,
}

impl Ballistics {
    /// A smoother at 0 dB with the given one-pole coefficients
    /// (see [`time_constant_alpha`](super::time_constant_alpha)).
    pub fn new(direction: AttackDirection, alpha_attack: f64, alpha_release: f64) -> Self {
        Self {
            gain_db: 0.0,
            alpha_attack,
            alpha_release,
            direction,
        }
    }

    /// New attack coefficient; the gain stays continuous.
    pub fn set_attack(&mut self, alpha: f64) {
        self.alpha_attack = alpha;
    }

    /// New release coefficient; the gain stays continuous.
    pub fn set_release(&mut self, alpha: f64) {
        self.alpha_release = alpha;
    }

    /// Back to 0 dB.
    pub fn reset(&mut self) {
        self.gain_db = 0.0;
    }

    /// Current gain in dB.
    pub fn gain_db(&self) -> f64 {
        self.gain_db
    }

    /// One sample toward `target_db`; returns the new gain.
    pub fn process(&mut self, target_db: f64) -> f64 {
        let attack = match self.direction {
            AttackDirection::Down => target_db < self.gain_db,
            AttackDirection::Up => target_db > self.gain_db,
        };
        let alpha = if attack {
            self.alpha_attack
        } else {
            self.alpha_release
        };
        self.gain_db = target_db + (self.gain_db - target_db) * alpha;
        if (self.gain_db - target_db).abs() <= SNAP_DB {
            self.gain_db = target_db;
        }
        self.gain_db
    }
}

#[cfg(test)]
mod tests {
    use super::super::time_constant_alpha;
    use super::*;

    /// Samples until the gain first covers 63.2 % of a step from 0 to `target`.
    fn tau_samples(b: &mut Ballistics, from: f64, target: f64) -> usize {
        let goal = from + (target - from) * (1.0 - (-1.0f64).exp());
        (1..)
            .find(|_| (b.process(target) - goal) * (target - from).signum() >= 0.0)
            .unwrap()
    }

    #[test]
    fn time_constants_are_63_percent_times() {
        let fs = 48_000.0;
        for tau_ms in [0.5, 10.0, 100.0] {
            let mut b = Ballistics::new(
                AttackDirection::Down,
                time_constant_alpha(tau_ms, fs),
                time_constant_alpha(1000.0, fs),
            );
            let n = tau_samples(&mut b, 0.0, -7.5);
            let want = tau_ms / 1000.0 * fs;
            assert!(
                (n as f64 - want).abs() <= 1.0,
                "attack {tau_ms} ms: {n} vs {want}"
            );
            // Release back to 0 with the release constant.
            for _ in 0..(want as usize * 20) {
                b.process(-7.5);
            }
            assert_eq!(b.gain_db().to_bits(), (-7.5f64).to_bits(), "snapped");
            let n = tau_samples(&mut b, -7.5, 0.0);
            assert!((n as f64 - 48_000.0).abs() <= 1.0, "release: {n}");
        }
    }

    #[test]
    fn upward_direction_uses_attack_when_rising() {
        let fast = time_constant_alpha(1.0, 48_000.0);
        let slow = time_constant_alpha(1000.0, 48_000.0);
        let mut b = Ballistics::new(AttackDirection::Up, fast, slow);
        b.process(-20.0); // falling target: release (slow)
        assert!(b.gain_db() > -0.1);
        let mut b = Ballistics::new(AttackDirection::Down, fast, slow);
        b.process(-20.0); // falling target: attack (fast)
        assert!(b.gain_db() < -0.3);
    }
}
