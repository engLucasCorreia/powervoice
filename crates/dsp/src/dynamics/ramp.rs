//! Linear parameter ramps (SPEC-016 §4.8): the first ramped value applies at the event sample,
//! the last one is exactly the target; a new target mid-ramp restarts from the current value.

/// A linear ramp of a fixed length in samples.
#[derive(Clone, Debug)]
pub struct LinearRamp {
    start: f64,
    target: f64,
    len: u32,
    pos: u32,
}

impl LinearRamp {
    /// A settled ramp at `value` with length 0 (set it with [`set_len`](Self::set_len)).
    pub fn new(value: f64) -> Self {
        Self {
            start: value,
            target: value,
            len: 0,
            pos: 0,
        }
    }

    /// Ramp length in samples (`0` = changes are immediate). Snaps the ramp.
    pub fn set_len(&mut self, len: u32) {
        self.len = len;
        self.snap();
    }

    /// Starts a ramp from the current value to `target` (applies from the next [`advance`](Self::advance)).
    pub fn set_target(&mut self, target: f64) {
        self.start = self.current();
        self.target = target;
        self.pos = 0;
    }

    /// Jumps to the target (reset).
    pub fn snap(&mut self) {
        self.start = self.target;
        self.pos = self.len;
    }

    /// Sets the value immediately, without a ramp.
    pub fn set_immediate(&mut self, value: f64) {
        self.target = value;
        self.snap();
    }

    /// Target value.
    pub fn target(&self) -> f64 {
        self.target
    }

    /// True while a ramp is running.
    pub fn is_ramping(&self) -> bool {
        self.pos < self.len
    }

    /// Value of the last rendered sample.
    pub fn current(&self) -> f64 {
        if self.pos >= self.len {
            self.target
        } else {
            self.start + (self.target - self.start) * f64::from(self.pos) / f64::from(self.len)
        }
    }

    /// Advances one sample and returns its value.
    pub fn advance(&mut self) -> f64 {
        if self.pos < self.len {
            self.pos += 1;
        }
        self.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_and_exact_at_the_end() {
        let mut r = LinearRamp::new(0.0);
        r.set_len(4);
        r.set_target(1.0);
        let v: Vec<f64> = (0..6).map(|_| r.advance()).collect();
        assert_eq!(v, [0.25, 0.5, 0.75, 1.0, 1.0, 1.0]);
        assert!(!r.is_ramping());
        // Mid-ramp retarget starts from the current value.
        r.set_target(0.0);
        r.advance();
        r.set_target(2.0);
        assert!(r.is_ramping());
        let first = r.advance();
        assert!((first - (0.75 + 1.25 * 0.25)).abs() < 1e-12, "{first}");
        r.snap();
        assert_eq!(r.current().to_bits(), 2f64.to_bits());
        let mut z = LinearRamp::new(3.0);
        z.set_target(5.0);
        assert_eq!(
            z.advance().to_bits(),
            5f64.to_bits(),
            "length 0 is immediate"
        );
    }
}
