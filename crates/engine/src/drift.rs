//! The monitoring drift servo (T-107, ADR-002 §6, SPEC-002 §2.7): a PI controller on the monitor
//! ring's fill level that steers the output-side resampler's ratio.
//!
//! Plant: with the fill `e` expressed in seconds of input, `de/dt = (δ_in − δ_out) + u`, where
//! `δ` are the true clock deviations of the two devices and `u` is the relative ratio correction
//! (`ratio = out/in · (1 + u)`; `u > 0` consumes the input more slowly, so the ring fills).
//! Controller: `u = −(KP·ē + KI·∫ē dt)` on the low-passed error `ē` (τ = 0.5 s), clamped to
//! ±1000 ppm with anti-windup. Closed loop `τs³ + s² + KP·s + KI` = `0.5s³ + s² + 2s + 0.5`: poles
//! ≈ −0.29 and −0.86 ± 1.66j — a ±200 ppm drift leaves a transient error of ≈ 0.1 ms, removed by
//! the integral within ~10 s. Typical corrections are well under 100 ppm (inaudible).
//!
//! Pure arithmetic: RT-safe (no allocation, no branches on unbounded loops).

/// Largest relative correction (±1000 ppm ≈ ±1.7 cents, ADR-002 §6).
pub(crate) const MAX_CORRECTION: f64 = 1e-3;
/// Low-pass time constant of the fill error (ADR-002 §6: τ ≈ 0.5 s).
const TAU_S: f64 = 0.5;
/// Proportional gain (1/s): 1 ms of error → 2000 ppm (clamped).
const KP: f64 = 2.0;
/// Integral gain (1/s²).
const KI: f64 = 0.5;

/// PI controller state (see the module docs).
#[derive(Clone, Debug, Default)]
pub(crate) struct DriftServo {
    /// Low-passed fill error, seconds.
    lp_s: f64,
    /// Integral of `lp_s`, seconds².
    integral: f64,
    /// The last correction `u`.
    u: f64,
    started: bool,
}

impl DriftServo {
    /// Forgets everything (after priming: the fill starts at the target).
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    /// One update: `err_s` = fill − target in seconds of input, `dt_s` = time since the last
    /// update. Returns the relative ratio `1 + u`, `|u| ≤` [`MAX_CORRECTION`].
    pub(crate) fn update(&mut self, err_s: f64, dt_s: f64) -> f64 {
        if !err_s.is_finite() || !dt_s.is_finite() || dt_s <= 0.0 {
            return 1.0 + self.u;
        }
        if self.started {
            self.lp_s += dt_s / (TAU_S + dt_s) * (err_s - self.lp_s);
        } else {
            // Start from the first measurement: no low-pass start-up lag.
            self.lp_s = err_s;
            self.started = true;
        }
        let limit = MAX_CORRECTION / KI;
        self.integral = (self.integral + self.lp_s * dt_s).clamp(-limit, limit);
        self.u = (-(KP * self.lp_s + KI * self.integral)).clamp(-MAX_CORRECTION, MAX_CORRECTION);
        1.0 + self.u
    }

    /// The last correction `u` (relative, 0 = none).
    #[cfg(test)]
    pub(crate) fn correction(&self) -> f64 {
        self.u
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Simulates the plant `de/dt = drift + u` for `secs` with 5.33 ms updates (256 frames at
    /// 48 kHz); returns the error trace (seconds) per update.
    fn simulate(drift: f64, e0: f64, secs: f64) -> (Vec<f64>, DriftServo) {
        let dt = 256.0 / 48_000.0;
        let mut s = DriftServo::default();
        let mut e = e0;
        let mut trace = Vec::new();
        let steps = (secs / dt) as usize;
        for _ in 0..steps {
            let rel = s.update(e, dt);
            assert!((rel - 1.0).abs() <= MAX_CORRECTION + 1e-15);
            e += (drift + (rel - 1.0)) * dt;
            trace.push(e);
        }
        (trace, s)
    }

    /// ADR-002 §6 / T-107: at ±200 ppm the fill stays within ±1 ms of the target from the start
    /// and has converged (±20 µs) within 10 s; the correction then cancels the drift.
    #[test]
    fn converges_on_a_200_ppm_drift_within_10_s() {
        for drift in [200e-6, -200e-6] {
            let (trace, s) = simulate(drift, 0.0, 60.0);
            let worst = trace.iter().fold(0.0f64, |m, e| m.max(e.abs()));
            assert!(worst < 0.25e-3, "{drift}: worst error {worst} s");
            let after_10s = &trace[(10.0 / (256.0 / 48_000.0)) as usize..];
            let late = after_10s.iter().fold(0.0f64, |m, e| m.max(e.abs()));
            assert!(late < 20e-6, "{drift}: error after 10 s {late} s");
            assert!(
                (s.correction() + drift).abs() < 1e-6,
                "u = {}",
                s.correction()
            );
        }
    }

    /// PI step response: a 2 ms fill error (e.g. after a target change) is removed without
    /// runaway; the correction saturates at ±1000 ppm, then settles.
    #[test]
    fn step_response_saturates_then_settles() {
        let (trace, _) = simulate(0.0, 2e-3, 30.0);
        assert!(trace.iter().all(|e| e.abs() <= 2e-3 + 1e-9));
        assert!(
            trace.last().unwrap().abs() < 20e-6,
            "{}",
            trace.last().unwrap()
        );
        // Saturated at first: after 0.5 s the error dropped by ≈ 1000 ppm · 0.5 s.
        let i = (0.5 / (256.0 / 48_000.0)) as usize;
        assert!((trace[i] - 1.5e-3).abs() < 0.05e-3, "{}", trace[i]);
    }

    /// Non-finite input or a zero time step keeps the last correction.
    #[test]
    fn ignores_bad_input() {
        let mut s = DriftServo::default();
        let r = s.update(1e-3, 0.01).to_bits();
        assert_eq!(s.update(f64::NAN, 0.01).to_bits(), r);
        assert_eq!(s.update(1e-3, 0.0).to_bits(), r);
    }
}
