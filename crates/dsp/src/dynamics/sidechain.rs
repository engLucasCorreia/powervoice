//! Sidechain high-pass (SPEC-013 §4.1): 4th-order Butterworth as two cascaded RBJ-cookbook
//! high-pass biquads (Q 0.5412 and 1.3066) in transposed direct form II, `f64` coefficients and
//! state, states flushed to 0 below 1e-30.

/// Q values of a 4th-order Butterworth cascade (`docs/references.md`).
pub const BUTTERWORTH4_Q: [f64; 2] = [0.541_196_100_146_197, 1.306_562_964_876_376_7];

const FLUSH: f64 = 1e-30;

/// One high-pass biquad, TDF-II.
#[derive(Clone, Copy, Debug, Default)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}

impl Biquad {
    fn set_highpass(&mut self, freq_hz: f64, q: f64, sample_rate: f64) {
        let w0 = std::f64::consts::TAU * freq_hz / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = (1.0 + cos) / 2.0 / a0;
        self.b1 = -(1.0 + cos) / a0;
        self.b2 = self.b0;
        self.a1 = -2.0 * cos / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        if self.z1.abs() < FLUSH {
            self.z1 = 0.0;
        }
        if self.z2.abs() < FLUSH {
            self.z2 = 0.0;
        }
        y
    }
}

/// 24 dB/oct Butterworth high-pass.
#[derive(Clone, Debug)]
pub struct ButterworthHighpass4 {
    stages: [Biquad; 2],
}

impl ButterworthHighpass4 {
    /// A filter at `freq_hz` (limited to 0.45·fs).
    pub fn new(freq_hz: f64, sample_rate: f64) -> Self {
        let mut f = Self {
            stages: [Biquad::default(); 2],
        };
        f.set_freq(freq_hz, sample_rate);
        f
    }

    /// New corner frequency (limited to 0.45·fs); the state is kept. RT-safe.
    pub fn set_freq(&mut self, freq_hz: f64, sample_rate: f64) {
        let f = freq_hz.clamp(1.0, 0.45 * sample_rate);
        for (stage, q) in self.stages.iter_mut().zip(BUTTERWORTH4_Q) {
            stage.set_highpass(f, q, sample_rate);
        }
    }

    /// Clears the state.
    pub fn reset(&mut self) {
        for s in &mut self.stages {
            s.z1 = 0.0;
            s.z2 = 0.0;
        }
    }

    /// Filters one sample.
    pub fn process(&mut self, x: f64) -> f64 {
        let y = self.stages[0].process(x);
        self.stages[1].process(y)
    }

    /// The four state variables (tests: exact zero after silence, no subnormals).
    pub fn state(&self) -> [f64; 4] {
        [
            self.stages[0].z1,
            self.stages[0].z2,
            self.stages[1].z1,
            self.stages[1].z2,
        ]
    }
}

/// Analog 4th-order Butterworth high-pass magnitude in dB at `freq_hz` for corner `corner_hz`.
pub fn butterworth4_highpass_db(freq_hz: f64, corner_hz: f64) -> f64 {
    let r4 = (freq_hz / corner_hz).powi(4);
    20.0 * (r4 / (1.0 + r4 * r4).sqrt()).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measured_db(freq: f64, corner: f64) -> f64 {
        let fs = 48_000.0;
        let mut f = ButterworthHighpass4::new(corner, fs);
        let w = std::f64::consts::TAU * freq / fs;
        let n = 48_000 * 2;
        let mut peak: f64 = 0.0;
        for i in 0..n {
            let y = f.process((w * f64::from(i)).sin());
            if i > n / 2 {
                peak = peak.max(y.abs());
            }
        }
        20.0 * peak.log10()
    }

    #[test]
    fn magnitude_matches_the_analog_prototype() {
        for (freq, corner) in [(40.0, 100.0), (25.0, 100.0), (100.0, 100.0), (997.0, 100.0)] {
            let got = measured_db(freq, corner);
            let want = butterworth4_highpass_db(freq, corner);
            assert!(
                (got - want).abs() < 0.05,
                "{freq} Hz @ {corner}: {got} vs {want}"
            );
        }
        assert!((butterworth4_highpass_db(40.0, 100.0) + 31.84).abs() < 0.01);
        assert!((butterworth4_highpass_db(100.0, 100.0) + 3.0103).abs() < 1e-3);
    }

    #[test]
    fn state_flushes_to_exact_zero_on_silence() {
        let mut f = ButterworthHighpass4::new(100.0, 48_000.0);
        for i in 0..4800 {
            f.process(if i % 7 == 0 { 1.0 } else { -0.5 });
        }
        for _ in 0..48_000 * 10 {
            f.process(0.0);
        }
        assert!(
            f.state().iter().all(|v| v.to_bits() == 0),
            "{:?}",
            f.state()
        );
    }
}
