//! Linear, equal-gain crossfades between a slot's signal sources (ADR-005 §9).
//!
//! A [`Mixer`] holds one gain per source. A fade moves every gain linearly from its current
//! value to a one-hot target over `len` samples: sample `k` of the fade (0-based) uses
//! `a = (k + 1) / len`, and sample `len − 1` is exactly the target. The gains always sum to 1
//! (equal-gain), and a new fade started mid-fade continues from the current gains, so there is
//! never a jump. A fade may start after a **hold**: the gains stay at their start values for
//! `hold` samples first (a new instance with latency L produces valid output only after L
//! samples, so its fade-in waits for it).

/// Number of sources.
pub(crate) const SOURCES: usize = 5;
/// The module output.
pub(crate) const WET: usize = 0;
/// The outgoing (replaced) instance's output.
pub(crate) const AUX: usize = 1;
/// The slot input, undelayed (what the chain had before an insert / after a removal).
pub(crate) const DRY0: usize = 2;
/// The slot input delayed by the module's latency (bypass).
pub(crate) const DRY_LAT: usize = 3;
/// The slot input delayed by the outgoing instance's latency (bypassed replacement).
pub(crate) const DRY_AUX: usize = 4;

/// Crossfade state. RT-safe, `Copy`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Mixer {
    start: [f32; SOURCES],
    target: [f32; SOURCES],
    /// Samples of the current fade already rendered.
    pos: u32,
    /// Fade length (0: steady).
    len: u32,
    /// The source `target` is one-hot on.
    target_src: usize,
    /// Samples the start gains are held before the fade begins.
    hold: u32,
}

fn one_hot(src: usize) -> [f32; SOURCES] {
    let mut g = [0.0; SOURCES];
    g[src] = 1.0;
    g
}

impl Mixer {
    /// Steady on `src`.
    pub(crate) fn steady(src: usize) -> Self {
        Self {
            start: one_hot(src),
            target: one_hot(src),
            pos: 0,
            len: 0,
            target_src: src,
            hold: 0,
        }
    }

    /// Holding `from` (arbitrary gains summing to 1) for `hold` samples, then fading to `to`
    /// over `len` samples.
    pub(crate) fn fading(from: [f32; SOURCES], to: usize, len: u32, hold: u32) -> Self {
        let mut m = Self {
            start: from,
            target: from,
            pos: 0,
            len: 0,
            // Not a one-hot state yet: force the fade below.
            target_src: usize::MAX,
            hold: 0,
        };
        m.fade_to_after(to, len, hold);
        m
    }

    /// True while a hold or fade is in progress.
    pub(crate) fn is_fading(&self) -> bool {
        self.pos < self.hold + self.len
    }

    /// The source a steady mixer plays.
    pub(crate) fn steady_source(&self) -> Option<usize> {
        (!self.is_fading()).then_some(self.target_src)
    }

    /// The source the mixer is heading to.
    pub(crate) fn target_source(&self) -> usize {
        self.target_src
    }

    /// Gain of `src` right now (after the last rendered sample).
    pub(crate) fn current(&self) -> [f32; SOURCES] {
        if !self.is_fading() {
            return self.target;
        }
        if self.pos <= self.hold {
            return self.start;
        }
        let a = (self.pos - self.hold) as f32 / self.len as f32;
        let mut g = [0.0; SOURCES];
        for (s, g) in g.iter_mut().enumerate() {
            *g = self.start[s] + (self.target[s] - self.start[s]) * a;
        }
        g
    }

    /// Starts a fade to `src` from the current gains (no-op if already steady there).
    pub(crate) fn fade_to(&mut self, src: usize, len: u32) {
        self.fade_to_after(src, len, 0);
    }

    /// Holds the current gains for `hold` samples, then fades to `src` over `len` samples.
    pub(crate) fn fade_to_after(&mut self, src: usize, len: u32, hold: u32) {
        if !self.is_fading() && self.target_src == src {
            return;
        }
        if len <= 1 && hold == 0 {
            *self = Self::steady(src);
            return;
        }
        self.start = self.current();
        self.target = one_hot(src);
        self.target_src = src;
        self.pos = 0;
        self.len = len.max(1);
        self.hold = hold;
    }

    /// Weight of `src` at sample `i` of the next block (valid for `i < remaining()`).
    #[inline]
    pub(crate) fn gains_at(&self, i: usize) -> ([f32; SOURCES], bool) {
        let k = self.pos as usize + i;
        let hold = self.hold as usize;
        if k < hold {
            return (self.start, false);
        }
        let f = k - hold;
        if f + 1 >= self.len as usize {
            return (self.target, true);
        }
        let a = (f + 1) as f32 / self.len as f32;
        let mut g = [0.0; SOURCES];
        for (s, g) in g.iter_mut().enumerate() {
            *g = self.start[s] + (self.target[s] - self.start[s]) * a;
        }
        (g, false)
    }

    /// Sources with a non-zero start or target weight (the ones a fade must read).
    pub(crate) fn active_sources(&self) -> [bool; SOURCES] {
        let mut a = [false; SOURCES];
        for (s, a) in a.iter_mut().enumerate() {
            *a = self.start[s] != 0.0 || self.target[s] != 0.0;
        }
        a
    }

    /// Advances by `n` rendered samples. Returns true if a fade completed within them.
    pub(crate) fn advance(&mut self, n: usize) -> bool {
        if !self.is_fading() {
            return false;
        }
        let pos = self.pos as usize + n;
        if pos + 1 >= (self.hold + self.len) as usize {
            self.start = self.target;
            self.pos = 0;
            self.len = 0;
            self.hold = 0;
            true
        } else {
            self.pos = pos as u32;
            false
        }
    }

    /// The current gains with sources renamed by `map[old] = new` (weights of sources mapped to
    /// the same new source add up). For replacements: the old instance's WET becomes AUX.
    pub(crate) fn remapped(&self, map: [usize; SOURCES]) -> [f32; SOURCES] {
        let cur = self.current();
        let mut g = [0.0; SOURCES];
        for (s, w) in cur.iter().enumerate() {
            g[map[s]] += w;
        }
        g
    }
}

/// Linear crossfade gain of sample `k` (0-based) of an `len`-sample fade (1.0 from `len − 1`).
/// The exact law every host crossfade uses, exposed for tests.
pub fn xfade_gain(k: usize, len: u32) -> f32 {
    if k + 1 >= len as usize {
        1.0
    } else {
        (k + 1) as f32 / len as f32
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)] // exact gains are the point of these tests
mod tests {
    use super::*;

    #[test]
    fn fade_is_linear_and_ends_on_target() {
        let mut m = Mixer::steady(WET);
        m.fade_to(DRY_LAT, 4);
        let g: Vec<[f32; SOURCES]> = (0..4).map(|i| m.gains_at(i).0).collect();
        assert_eq!(g[0][WET], 0.75);
        assert_eq!(g[0][DRY_LAT], 0.25);
        assert_eq!(g[2][DRY_LAT], 0.75);
        assert_eq!(g[3], one_hot(DRY_LAT));
        assert!(m.advance(4));
        assert_eq!(m.steady_source(), Some(DRY_LAT));
    }

    #[test]
    fn reversal_mid_fade_is_continuous() {
        let mut m = Mixer::steady(WET);
        m.fade_to(DRY_LAT, 10);
        assert!(!m.advance(3));
        let before = m.current();
        m.fade_to(WET, 10);
        assert_eq!(m.current(), before);
        let (g, _) = m.gains_at(0);
        assert!((g[WET] + g[DRY_LAT] - 1.0).abs() < 1e-6);
        assert!(g[WET] > before[WET]);
        assert!(!m.is_fading() || m.target_source() == WET);
    }

    #[test]
    fn hold_keeps_the_start_gains_then_fades() {
        let mut m = Mixer::steady(DRY0);
        m.fade_to_after(WET, 4, 3);
        let g: Vec<[f32; SOURCES]> = (0..7).map(|i| m.gains_at(i).0).collect();
        assert_eq!(g[0], one_hot(DRY0));
        assert_eq!(g[2], one_hot(DRY0));
        assert_eq!(g[3][WET], 0.25);
        assert_eq!(g[6], one_hot(WET));
        assert!(!m.advance(2));
        assert_eq!(m.current(), one_hot(DRY0));
        assert!(!m.advance(2));
        assert_eq!(m.current()[WET], 0.25);
        assert!(m.advance(3));
        assert_eq!(m.steady_source(), Some(WET));
    }

    #[test]
    fn remap_moves_weights() {
        let mut m = Mixer::steady(WET);
        m.fade_to(DRY_LAT, 4);
        m.advance(2);
        let g = m.remapped([AUX, AUX, DRY0, DRY_AUX, DRY_AUX]);
        assert_eq!(g[AUX], 0.5);
        assert_eq!(g[DRY_AUX], 0.5);
    }
}
