//! Smoothed EQ bands (SPEC-015 §4.4, §4.6), evolving sample by sample so the output never
//! depends on the block partition.
//!
//! - Continuous values ramp linearly over `round(0.020·fs)` samples in log2(Hz), dB and ln(Q):
//!   the value at the event sample already moves one step and the last ramp sample is the target
//!   exactly ([`LinearRamp`]). While any ramp of a band runs, its coefficients are recomputed
//!   every sample; the sample that ends the ramp uses the plain target values, so static
//!   coefficients are bit-identical to what `ResponseCurve` evaluates.
//! - Sections always run on their own input, even while the band is off. The band's output is
//!   `x + w·(F(x) − x)` with `w` ramping 0 ↔ 1 over 20 ms; with no fade running it is exactly
//!   `x` (w = 0) or `F(x)` (w = 1). A shelf/peak at exactly 0 dB with no ramp running outputs `x`.
//! - HP/LP slope changes crossfade from the old cascade to a zero-state new one over 20 ms; a
//!   slope change during that fade is applied when it ends (latest value wins).

use super::coeffs::{Biquad, GainShape, PassKind, Sections, butterworth};
use super::{DENORMAL_FLUSH, MAX_ORDER, MAX_SECTIONS, ramp_samples};
use crate::dynamics::ramp::LinearRamp;

/// Direct Form I state of one section.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Df1 {
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Df1 {
    /// One sample through `c`.
    #[inline]
    pub fn tick(&mut self, c: &Biquad, x: f64) -> f64 {
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    /// Zero state.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Sets state values with |v| < [`DENORMAL_FLUSH`] to 0.
    pub fn flush_denormals(&mut self) {
        for v in [&mut self.x1, &mut self.x2, &mut self.y1, &mut self.y2] {
            if v.abs() < DENORMAL_FLUSH {
                *v = 0.0;
            }
        }
    }

    /// The state values `[x1, x2, y1, y2]` (test hook).
    pub fn values(&self) -> [f64; 4] {
        [self.x1, self.x2, self.y1, self.y2]
    }
}

/// Advances a log-domain ramp; returns the plain value (`plain` exactly once the ramp is done).
fn advance_log(r: &mut LinearRamp, plain: f64, exp: fn(f64) -> f64) -> f64 {
    let v = r.advance();
    if r.is_ramping() { exp(v) } else { plain }
}

/// On/off mix: `dry` or `wet` exactly unless a fade is running.
fn mix(on: &mut LinearRamp, dry: f64, wet: f64) -> f64 {
    let w = on.advance();
    if on.is_ramping() {
        dry + w * (wet - dry)
    } else if w >= 0.5 {
        wet
    } else {
        dry
    }
}

fn weight(on: bool) -> f64 {
    if on { 1.0 } else { 0.0 }
}

/// A low shelf, peak or high shelf band.
#[derive(Clone, Debug)]
pub struct GainBand {
    shape: GainShape,
    sample_rate: f64,
    freq_hz: f64,
    gain_db: f64,
    q: f64,
    log2_freq: LinearRamp,
    gain: LinearRamp,
    ln_q: LinearRamp,
    on: LinearRamp,
    coeffs: Biquad,
    state: Df1,
}

impl GainBand {
    /// A settled band at 48 kHz (call [`set_sample_rate`](Self::set_sample_rate) at activate).
    pub fn new(shape: GainShape, freq_hz: f64, gain_db: f64, q: f64, on: bool) -> Self {
        let mut b = Self {
            shape,
            sample_rate: 48_000.0,
            freq_hz,
            gain_db,
            q,
            log2_freq: LinearRamp::new(freq_hz.log2()),
            gain: LinearRamp::new(gain_db),
            ln_q: LinearRamp::new(q.ln()),
            on: LinearRamp::new(weight(on)),
            coeffs: Biquad::IDENTITY,
            state: Df1::default(),
        };
        b.snap();
        b
    }

    /// Sets the rate and the ramp lengths; snaps every ramp (state kept).
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        let n = ramp_samples(sample_rate);
        for r in [
            &mut self.log2_freq,
            &mut self.gain,
            &mut self.ln_q,
            &mut self.on,
        ] {
            r.set_len(n);
        }
        self.snap();
    }

    fn static_coeffs(&self) -> Biquad {
        self.shape
            .coeffs(self.freq_hz, self.q, self.gain_db, self.sample_rate)
    }

    fn shape_ramping(&self) -> bool {
        self.log2_freq.is_ramping() || self.gain.is_ramping() || self.ln_q.is_ramping()
    }

    /// After a new target: static coefficients at once if no ramp runs (length 0).
    fn retarget(&mut self) {
        if !self.shape_ramping() {
            self.coeffs = self.static_coeffs();
        }
    }

    /// Glides the frequency to `freq_hz` (log2 domain).
    pub fn set_freq_hz(&mut self, freq_hz: f64) {
        self.freq_hz = freq_hz;
        self.log2_freq.set_target(freq_hz.log2());
        self.retarget();
    }

    /// Glides the gain to `gain_db` (dB domain).
    pub fn set_gain_db(&mut self, gain_db: f64) {
        self.gain_db = gain_db;
        self.gain.set_target(gain_db);
        self.retarget();
    }

    /// Glides Q to `q` (ln domain).
    pub fn set_q(&mut self, q: f64) {
        self.q = q;
        self.ln_q.set_target(q.ln());
        self.retarget();
    }

    /// Fades the band in or out.
    pub fn set_on(&mut self, on: bool) {
        self.on.set_target(weight(on));
    }

    /// Every ramp and fade at its target, static coefficients; state kept.
    pub fn snap(&mut self) {
        for r in [
            &mut self.log2_freq,
            &mut self.gain,
            &mut self.ln_q,
            &mut self.on,
        ] {
            r.snap();
        }
        self.coeffs = self.static_coeffs();
    }

    /// [`snap`](Self::snap) and zero state.
    pub fn reset(&mut self) {
        self.snap();
        self.state.clear();
    }

    /// True while a frequency, gain, Q or on/off ramp runs.
    pub fn is_ramping(&self) -> bool {
        self.shape_ramping() || self.on.is_ramping()
    }

    /// Current coefficients.
    pub fn coeffs(&self) -> Biquad {
        self.coeffs
    }

    /// One sample.
    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        if self.shape_ramping() {
            let f = advance_log(&mut self.log2_freq, self.freq_hz, f64::exp2);
            let g = self.gain.advance();
            let g = if self.gain.is_ramping() {
                g
            } else {
                self.gain_db
            };
            let q = advance_log(&mut self.ln_q, self.q, f64::exp);
            self.coeffs = self.shape.coeffs(f, q, g, self.sample_rate);
        }
        let wet = self.state.tick(&self.coeffs, x);
        // Identity rule: exactly 0 dB (current and target) with no ramp running.
        let neutral = !self.shape_ramping() && self.gain_db.abs() <= 0.0;
        mix(&mut self.on, x, if neutral { x } else { wet })
    }

    /// Block-end denormal flush.
    pub fn flush_denormals(&mut self) {
        self.state.flush_denormals();
    }

    /// State values (test hook).
    pub fn state_values(&self) -> [f64; 4] {
        self.state.values()
    }
}

/// One Butterworth cascade with its state.
#[derive(Clone, Copy, Debug)]
struct Cascade {
    order: usize,
    sections: Sections,
    states: [Df1; MAX_SECTIONS],
}

impl Cascade {
    fn new(kind: PassKind, order: usize, freq_hz: f64, sample_rate: f64) -> Self {
        Self {
            order,
            sections: butterworth(kind, order, freq_hz, sample_rate),
            states: [Df1::default(); MAX_SECTIONS],
        }
    }

    fn tune(&mut self, kind: PassKind, freq_hz: f64, sample_rate: f64) {
        self.sections = butterworth(kind, self.order, freq_hz, sample_rate);
    }

    #[inline]
    fn tick(&mut self, x: f64) -> f64 {
        let mut y = x;
        for (s, c) in self
            .states
            .iter_mut()
            .zip(&self.sections.coeffs)
            .take(self.sections.len)
        {
            y = s.tick(c, y);
        }
        y
    }

    fn clear(&mut self) {
        self.states = [Df1::default(); MAX_SECTIONS];
    }

    fn flush_denormals(&mut self) {
        for s in &mut self.states {
            s.flush_denormals();
        }
    }
}

/// A Butterworth high-pass or low-pass band, 6–48 dB/oct.
#[derive(Clone, Debug)]
pub struct PassBand {
    kind: PassKind,
    sample_rate: f64,
    freq_hz: f64,
    /// Latest requested order (1…8).
    order: usize,
    log2_freq: LinearRamp,
    on: LinearRamp,
    cascades: [Cascade; 2],
    /// Cascade in use (the outgoing one during a slope fade).
    current: usize,
    /// Weight of the incoming cascade `1 − current` while `fading`.
    fade: LinearRamp,
    fading: bool,
}

impl PassBand {
    /// A settled band at 48 kHz of order `order` (1…8).
    pub fn new(kind: PassKind, freq_hz: f64, order: usize, on: bool) -> Self {
        let order = order.clamp(1, MAX_ORDER);
        let c = Cascade::new(kind, order, freq_hz, 48_000.0);
        let mut b = Self {
            kind,
            sample_rate: 48_000.0,
            freq_hz,
            order,
            log2_freq: LinearRamp::new(freq_hz.log2()),
            on: LinearRamp::new(weight(on)),
            cascades: [c, c],
            current: 0,
            fade: LinearRamp::new(0.0),
            fading: false,
        };
        b.snap();
        b
    }

    /// Sets the rate and the ramp lengths; snaps every ramp and fade (state kept).
    pub fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate;
        let n = ramp_samples(sample_rate);
        for r in [&mut self.log2_freq, &mut self.on, &mut self.fade] {
            r.set_len(n);
        }
        self.snap();
    }

    /// Frequency of the last rendered sample.
    fn current_freq(&self) -> f64 {
        if self.log2_freq.is_ramping() {
            self.log2_freq.current().exp2()
        } else {
            self.freq_hz
        }
    }

    fn tune(&mut self, freq_hz: f64) {
        let (kind, fs) = (self.kind, self.sample_rate);
        self.cascades[self.current].tune(kind, freq_hz, fs);
        if self.fading {
            self.cascades[1 - self.current].tune(kind, freq_hz, fs);
        }
    }

    /// Starts a slope fade towards `self.order` if the cascade in use has another order.
    fn start_slope_fade(&mut self) {
        if self.cascades[self.current].order == self.order {
            return;
        }
        let next = 1 - self.current;
        self.cascades[next] =
            Cascade::new(self.kind, self.order, self.current_freq(), self.sample_rate);
        self.fade.set_immediate(0.0);
        self.fade.set_target(1.0);
        if self.fade.is_ramping() {
            self.fading = true;
        } else {
            self.current = next;
        }
    }

    /// Glides the frequency to `freq_hz` (log2 domain).
    pub fn set_freq_hz(&mut self, freq_hz: f64) {
        self.freq_hz = freq_hz;
        self.log2_freq.set_target(freq_hz.log2());
        if !self.log2_freq.is_ramping() {
            self.tune(freq_hz);
        }
    }

    /// Crossfades to order `order` (1…8); during a running slope fade it is applied when that
    /// fade ends.
    pub fn set_order(&mut self, order: usize) {
        self.order = order.clamp(1, MAX_ORDER);
        if !self.fading {
            self.start_slope_fade();
        }
    }

    /// Fades the band in or out.
    pub fn set_on(&mut self, on: bool) {
        self.on.set_target(weight(on));
    }

    /// Every ramp and fade at its target (latest order), static coefficients; state kept.
    pub fn snap(&mut self) {
        self.log2_freq.snap();
        self.on.snap();
        self.fade.snap();
        if self.fading {
            self.current = 1 - self.current;
            self.fading = false;
        }
        let c = &mut self.cascades[self.current];
        c.order = self.order;
        c.tune(self.kind, self.freq_hz, self.sample_rate);
    }

    /// [`snap`](Self::snap) and zero state.
    pub fn reset(&mut self) {
        self.snap();
        for c in &mut self.cascades {
            c.clear();
        }
    }

    /// True while a frequency, on/off or slope fade runs.
    pub fn is_ramping(&self) -> bool {
        self.log2_freq.is_ramping() || self.on.is_ramping() || self.fading
    }

    /// Order of the cascade in use (the outgoing one during a slope fade).
    pub fn active_order(&self) -> usize {
        self.cascades[self.current].order
    }

    /// Sections of the cascade in use.
    pub fn sections(&self) -> Sections {
        self.cascades[self.current].sections
    }

    /// One sample.
    #[inline]
    pub fn tick(&mut self, x: f64) -> f64 {
        if self.log2_freq.is_ramping() {
            let f = advance_log(&mut self.log2_freq, self.freq_hz, f64::exp2);
            self.tune(f);
        }
        let old = self.cascades[self.current].tick(x);
        let wet = if self.fading {
            let v = self.fade.advance();
            let new = self.cascades[1 - self.current].tick(x);
            if self.fade.is_ramping() {
                old + v * (new - old)
            } else {
                self.current = 1 - self.current;
                self.fading = false;
                self.start_slope_fade();
                new
            }
        } else {
            old
        };
        mix(&mut self.on, x, wet)
    }

    /// Block-end denormal flush (both cascades).
    pub fn flush_denormals(&mut self) {
        for c in &mut self.cascades {
            c.flush_denormals();
        }
    }

    /// State values of every section of both cascades (test hook).
    pub fn state_values(&self) -> impl Iterator<Item = f64> + '_ {
        self.cascades
            .iter()
            .flat_map(|c| c.states.iter().flat_map(Df1::values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eq::coeffs::Biquad;
    use std::f64::consts::TAU;

    const FS: f64 = 48_000.0;
    const N: usize = 960;

    fn active_gain_band() -> GainBand {
        let mut b = GainBand::new(GainShape::Peak, 1000.0, 6.0, 1.0, true);
        b.set_sample_rate(FS);
        b
    }

    /// AC-7: the ramp's last sample (k + n − 1) uses exactly the target coefficients.
    #[test]
    fn ramp_ends_exactly_on_the_target_coefficients() {
        let mut b = active_gain_band();
        for _ in 0..10 {
            b.tick(0.1);
        }
        b.set_freq_hz(3000.0);
        b.set_gain_db(-12.0);
        b.set_q(4.0);
        let target = Biquad::peaking(3000.0, 4.0, -12.0, FS);
        let first = {
            b.tick(0.1);
            b.coeffs()
        };
        assert_ne!(first, target, "moves gradually");
        assert_ne!(first, Biquad::peaking(1000.0, 1.0, 6.0, FS), "moves at k");
        for _ in 1..N - 1 {
            b.tick(0.1);
            assert!(b.is_ramping());
        }
        b.tick(0.1); // sample k + n − 1
        assert!(!b.is_ramping());
        assert_eq!(b.coeffs(), target, "bit-identical target coefficients");
    }

    #[test]
    fn log_domain_ramp_midpoint() {
        let mut b = active_gain_band();
        b.set_freq_hz(4000.0);
        for _ in 0..N / 2 {
            b.tick(0.0);
        }
        // Halfway in log2(Hz): 2 kHz.
        let mid = Biquad::peaking(2000.0, 1.0, 6.0, FS);
        let c = b.coeffs();
        assert!((c.a1 - mid.a1).abs() < 1e-12 && (c.b1 - mid.b1).abs() < 1e-12);
    }

    #[test]
    fn off_and_zero_db_bands_are_exact_identity() {
        let mut off = GainBand::new(GainShape::Peak, 700.0, 12.0, 3.0, false);
        off.set_sample_rate(FS);
        let mut flat = GainBand::new(GainShape::LowShelf, 700.0, 0.0, 0.5, true);
        flat.set_sample_rate(FS);
        let mut hp = PassBand::new(PassKind::HighPass, 500.0, 8, false);
        hp.set_sample_rate(FS);
        for i in 0..5000 {
            let x = (f64::from(i) * 0.37).sin() * 0.8 + 1e-3;
            assert_eq!(off.tick(x).to_bits(), x.to_bits());
            assert_eq!(flat.tick(x).to_bits(), x.to_bits());
            assert_eq!(hp.tick(x).to_bits(), x.to_bits());
        }
        // Sections kept running on their input while off.
        assert!(off.state_values().iter().any(|v| v.abs() > 0.0));
        assert!(hp.state_values().any(|v| v.abs() > 0.0));
    }

    #[test]
    fn on_off_fades_over_20_ms() {
        let mut b = active_gain_band();
        b.set_on(false);
        let mut last = 0.0;
        for i in 0..N {
            last = b.tick(1.0);
            assert_eq!(b.is_ramping(), i < N - 1);
        }
        assert_eq!(
            last.to_bits(),
            1f64.to_bits(),
            "dry exactly at the end of the fade"
        );
    }

    #[test]
    fn slope_change_crossfades_and_latest_wins() {
        let mut b = PassBand::new(PassKind::LowPass, 2000.0, 3, true);
        b.set_sample_rate(FS);
        b.set_order(6);
        assert_eq!(b.active_order(), 3, "old cascade until the fade ends");
        b.tick(0.5);
        b.set_order(8); // during the fade: pending
        b.set_order(2); // latest wins
        for _ in 1..N {
            b.tick(0.5);
        }
        assert_eq!(b.active_order(), 6, "first fade done");
        assert!(b.is_ramping(), "pending fade to 2 started");
        for _ in 0..N {
            b.tick(0.5);
        }
        assert_eq!(b.active_order(), 2);
        assert!(!b.is_ramping());
        assert_eq!(b.sections(), butterworth(PassKind::LowPass, 2, 2000.0, FS));
        // reset during a fade snaps to the latest order.
        b.set_order(5);
        b.tick(0.5);
        b.reset();
        assert_eq!(b.active_order(), 5);
        assert!(!b.is_ramping());
    }

    fn impulse_db(band: &mut PassBand, freqs: &[f64], len: usize) -> Vec<f64> {
        let h: Vec<f64> = (0..len)
            .map(|i| band.tick(if i == 0 { 1.0 } else { 0.0 }))
            .collect();
        freqs
            .iter()
            .map(|&f| {
                let w = TAU * f / FS;
                let (re, im) = h.iter().enumerate().fold((0.0, 0.0), |(re, im), (n, &v)| {
                    let p = w * n as f64;
                    (re + v * p.cos(), im - v * p.sin())
                });
                10.0 * (re * re + im * im).log10()
            })
            .collect()
    }

    /// AC-5 slopes, measured on the f64 cascade's impulse response (process() output is f32 and
    /// can't resolve the −190 dB of N = 8 at fc/16).
    #[test]
    fn butterworth_asymptotic_slopes() {
        for order in 1..=MAX_ORDER {
            let mut hp = PassBand::new(PassKind::HighPass, 1000.0, order, true);
            hp.set_sample_rate(FS);
            let a = impulse_db(&mut hp, &[62.5, 125.0], 4000);
            let slope = a[1] - a[0];
            let want = 6.020_6 * order as f64;
            assert!(
                (slope - want).abs() < 0.1,
                "HP N {order}: {slope} vs {want}"
            );

            let mut lp = PassBand::new(PassKind::LowPass, 100.0, order, true);
            lp.set_sample_rate(FS);
            let a = impulse_db(&mut lp, &[800.0, 1600.0], 40_000);
            let slope = a[0] - a[1];
            assert!(
                (slope - want).abs() < 0.3,
                "LP N {order}: {slope} vs {want}"
            );
        }
    }

    #[test]
    fn denormal_flush_zeroes_tiny_state() {
        let mut b = active_gain_band();
        b.tick(1e-35);
        b.flush_denormals();
        assert!(b.state_values().iter().all(|v| *v == 0.0));
        let mut p = PassBand::new(PassKind::HighPass, 100.0, 7, true);
        p.set_sample_rate(FS);
        p.tick(1e-35);
        p.flush_denormals();
        assert!(p.state_values().all(|v| v == 0.0));
    }
}
