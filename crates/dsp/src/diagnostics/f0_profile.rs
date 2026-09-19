//! Robust aggregation of YIN's per-frame estimates into the pitch profile the UI shows
//! (H-91, SPEC-007 §8.6): median, 10th and 90th percentile.
//!
//! YIN's characteristic error is the **octave**: a handful of frames of a voiced stretch come
//! back at F0/2 or F0×2 (a sub-harmonic dip of `d'` that happens to fall below the threshold
//! first, or a creaky/breathy frame whose period doubles). A plain median over every voiced
//! frame survives a few of those, but the **percentiles do not** — 10 % of frames an octave low
//! drags `low_hz` down by an octave and the displayed range becomes nonsense.
//!
//! So, before any percentile is taken:
//! 1. a **robust centre** is estimated in the log-frequency domain (pitch is logarithmic) from
//!    the *confident* frames — aperiodicity ≤ [`CONFIDENT_APERIODICITY`] — falling back to every
//!    voiced frame when too few are that confident;
//! 2. every voiced frame within [`OCTAVE_FOLD_TOLERANCE_OCT`] of an exact octave of that centre
//!    (up to ±[`MAX_FOLD_OCTAVES`]) is **folded** onto it — an octave error is corrected, not
//!    discarded, so the frame still counts toward the voiced fraction and the percentiles;
//! 3. the centre is re-estimated from the folded values and the fold is repeated once, so a
//!    centre that was itself pulled by the errors settles on the true one.
//!
//! Frames that are neither near the centre nor near an octave of it are left alone: they are
//! real pitch movement (emphasis, a question's rise), not tracker errors, and the percentiles
//! are supposed to show them.
//!
//! Nothing here is a second pitch tracker — [`super::pitch::Yin`] stays the only estimator; this
//! module only decides which of its frames the profile is built from.

/// One kept YIN frame: its estimate and how periodic that frame was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F0Frame {
    pub f0_hz: f64,
    /// `d'(τ*)` — 0 = perfectly periodic (see [`super::pitch::Pitch`]).
    pub aperiodicity: f64,
}

/// At or below this aperiodicity a frame is "confident" and may set the robust centre.
/// (YIN already refuses anything above [`super::pitch::YIN_MAX_APERIODICITY`] = 0.35.)
pub const CONFIDENT_APERIODICITY: f64 = 0.20;
/// Fewer confident frames than this share of the voiced frames → the centre is taken from every
/// voiced frame instead (a uniformly breathy voice is not an error).
pub const MIN_CONFIDENT_SHARE: f64 = 0.25;
/// At least this many confident frames are needed before they alone set the centre.
pub const MIN_CONFIDENT_FRAMES: usize = 3;
/// How close to an exact octave of the centre a frame must sit to count as an octave error
/// (octaves). A slipped frame is not at exactly half or double the centre — it is at half or
/// double *its own* pitch, which is itself moving, so the window has to be as wide as the
/// voice's own excursion: 1/4 octave (3 semitones) either side of the octave. The widest real
/// speech gestures stay well clear of it (a rise of a perfect fifth is 0.58 octave, the window
/// starts at 0.75), and a frame that genuinely lands within 3 semitones of an octave above the
/// speaker's median is far more likely a tracker slip than a real note.
pub const OCTAVE_FOLD_TOLERANCE_OCT: f64 = 0.25;
/// Octaves folded either side of the centre (±2 covers YIN's halving, doubling and the rare 4×).
pub const MAX_FOLD_OCTAVES: i32 = 2;
/// Fold, re-centre, fold again.
const FOLD_PASSES: usize = 2;

/// The pitch profile of a set of voiced frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct F0Profile {
    pub median_hz: f64,
    /// 10th percentile of the folded frames.
    pub low_hz: f64,
    /// 90th percentile of the folded frames.
    pub high_hz: f64,
    /// `1 − median aperiodicity` over the voiced frames, 0 … 1.
    pub confidence: f64,
    /// Share of the voiced frames that were an octave off the centre and were folded onto it.
    pub octave_corrected: f64,
    /// The robust centre, as log2(Hz) — what [`fold_toward`] folds toward.
    pub centre_log2: f64,
}

/// Nearest-rank percentile of an ascending slice.
fn percentile(sorted: &[f64], q: f64) -> f64 {
    let i = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// Median of `values` (sorts it in place); `None` when empty.
fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(percentile(values, 0.5))
}

/// `log2(f0)` folded onto `centre_log2` when it sits within [`OCTAVE_FOLD_TOLERANCE_OCT`] of an
/// exact octave of it; returns the folded value and whether it moved.
fn fold_log2(log2_f0: f64, centre_log2: f64) -> (f64, bool) {
    let d = log2_f0 - centre_log2;
    let k = d.round();
    if k == 0.0
        || k.abs() > f64::from(MAX_FOLD_OCTAVES)
        || (d - k).abs() > OCTAVE_FOLD_TOLERANCE_OCT
    {
        return (log2_f0, false);
    }
    (log2_f0 - k, true)
}

/// `f0_hz` folded onto the octave of `centre_log2` it belongs to (see [`F0Profile::centre_log2`]).
/// Used for the live "now" readout, so one octave-slipped frame can't make it jump.
pub fn fold_toward(f0_hz: f64, centre_log2: f64) -> f64 {
    if f0_hz <= 0.0 || !f0_hz.is_finite() {
        return f0_hz;
    }
    let (folded, _) = fold_log2(f0_hz.log2(), centre_log2);
    folded.exp2()
}

/// The robust centre of `frames` as log2(Hz): the median of the confident frames when there are
/// enough of them, else of every frame. `None` when `frames` is empty.
fn robust_centre_log2(frames: &[F0Frame], scratch: &mut Vec<f64>) -> Option<f64> {
    if frames.is_empty() {
        return None;
    }
    scratch.clear();
    scratch.extend(
        frames
            .iter()
            .filter(|f| f.aperiodicity <= CONFIDENT_APERIODICITY && f.f0_hz > 0.0)
            .map(|f| f.f0_hz.log2()),
    );
    let enough = scratch.len() >= MIN_CONFIDENT_FRAMES
        && scratch.len() as f64 >= MIN_CONFIDENT_SHARE * frames.len() as f64;
    if !enough {
        scratch.clear();
        scratch.extend(
            frames
                .iter()
                .filter(|f| f.f0_hz > 0.0)
                .map(|f| f.f0_hz.log2()),
        );
    }
    median(scratch)
}

/// The pitch profile of `frames` (order-independent; `frames` is not modified). `None` when
/// there is no usable frame.
pub fn profile(frames: &[F0Frame]) -> Option<F0Profile> {
    let mut scratch: Vec<f64> = Vec::with_capacity(frames.len());
    let mut centre = robust_centre_log2(frames, &mut scratch)?;
    let mut folded: Vec<f64> = Vec::with_capacity(frames.len());
    let fold = |centre: f64, folded: &mut Vec<f64>| -> usize {
        folded.clear();
        let mut corrected = 0;
        for f in frames.iter().filter(|f| f.f0_hz > 0.0) {
            let (value, moved) = fold_log2(f.f0_hz.log2(), centre);
            corrected += usize::from(moved);
            folded.push(value);
        }
        corrected
    };
    // Settle the centre: fold with it, re-take the median of the folded values, repeat.
    for _ in 0..FOLD_PASSES {
        fold(centre, &mut folded);
        centre = median(&mut folded)?;
    }
    // The profile itself comes from one last fold with the settled centre.
    let corrected = fold(centre, &mut folded);
    folded.sort_by(f64::total_cmp);
    let used = folded.len();
    if used == 0 {
        return None;
    }
    scratch.clear();
    scratch.extend(
        frames
            .iter()
            .filter(|f| f.f0_hz > 0.0)
            .map(|f| f.aperiodicity),
    );
    let confidence = 1.0 - median(&mut scratch).unwrap_or(1.0);
    Some(F0Profile {
        median_hz: percentile(&folded, 0.5).exp2(),
        low_hz: percentile(&folded, 0.1).exp2(),
        high_hz: percentile(&folded, 0.9).exp2(),
        confidence: confidence.clamp(0.0, 1.0),
        octave_corrected: corrected as f64 / used as f64,
        centre_log2: centre,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(f0s: &[f64], aper: f64) -> Vec<F0Frame> {
        f0s.iter()
            .map(|&f0_hz| F0Frame {
                f0_hz,
                aperiodicity: aper,
            })
            .collect()
    }

    fn cents(a: f64, b: f64) -> f64 {
        1200.0 * (a / b).log2()
    }

    /// A speaking voice: 120 Hz ± a few semitones, deterministic.
    fn speech(n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| 120.0 * 2f64.powf(0.18 * ((i as f64) * 0.7).sin()))
            .collect()
    }

    #[test]
    fn a_clean_track_is_unchanged() {
        let f0s = speech(200);
        let p = profile(&frames(&f0s, 0.05)).expect("profile");
        assert!(p.octave_corrected < 1e-12, "{p:?}");
        let mut sorted = f0s.clone();
        sorted.sort_by(f64::total_cmp);
        assert!(cents(p.median_hz, percentile(&sorted, 0.5)).abs() < 1.0);
        assert!(cents(p.low_hz, percentile(&sorted, 0.1)).abs() < 1.0);
        assert!(cents(p.high_hz, percentile(&sorted, 0.9)).abs() < 1.0);
        assert!((p.confidence - 0.95).abs() < 1e-9);
    }

    #[test]
    fn a_burst_of_halved_frames_does_not_move_the_profile() {
        let clean = speech(200);
        let reference = profile(&frames(&clean, 0.05)).unwrap();
        for share in [0.02, 0.05, 0.10, 0.20] {
            let bad = (clean.len() as f64 * share) as usize;
            let mut f = frames(&clean, 0.05);
            // A contiguous burst, as a real tracker slip is — and *confident*, so the guard
            // cannot lean on aperiodicity alone.
            for frame in f.iter_mut().take(bad) {
                frame.f0_hz /= 2.0;
            }
            let p = profile(&f).expect("profile");
            assert!(
                cents(p.median_hz, reference.median_hz).abs() < 5.0,
                "share {share}: median {} vs {}",
                p.median_hz,
                reference.median_hz
            );
            assert!(
                cents(p.low_hz, reference.low_hz).abs() < 25.0,
                "share {share}: low {} vs {}",
                p.low_hz,
                reference.low_hz
            );
            assert!(
                (p.octave_corrected - share).abs() < 0.02,
                "share {share}: corrected {}",
                p.octave_corrected
            );
        }
    }

    #[test]
    fn doubled_and_quadrupled_frames_are_folded_back() {
        let clean = speech(120);
        let reference = profile(&frames(&clean, 0.05)).unwrap();
        let mut f = frames(&clean, 0.05);
        for (i, frame) in f.iter_mut().enumerate() {
            if i % 11 == 0 {
                frame.f0_hz *= 2.0;
            } else if i % 17 == 0 {
                frame.f0_hz *= 4.0;
            }
        }
        let p = profile(&f).unwrap();
        assert!(cents(p.high_hz, reference.high_hz).abs() < 25.0, "{p:?}");
        assert!(cents(p.median_hz, reference.median_hz).abs() < 5.0, "{p:?}");
        assert!(p.octave_corrected > 0.1);
    }

    #[test]
    fn real_pitch_movement_is_kept() {
        // A question's rise: the last fifth of the frames climbs a perfect fifth (7 semitones,
        // nowhere near an octave). It must show in the 90th percentile.
        let mut f0s = vec![110.0; 80];
        f0s.extend(std::iter::repeat_n(110.0 * 2f64.powf(7.0 / 12.0), 20));
        let p = profile(&frames(&f0s, 0.1)).unwrap();
        assert!(p.octave_corrected < 1e-12, "{p:?}");
        assert!(
            cents(p.high_hz, 110.0 * 2f64.powf(7.0 / 12.0)).abs() < 1.0,
            "{p:?}"
        );
        assert!(cents(p.median_hz, 110.0).abs() < 1.0, "{p:?}");
    }

    #[test]
    fn the_centre_comes_from_the_confident_frames() {
        // Half the frames are confident at 200 Hz; the other half are barely-voiced frames an
        // octave down. The confident half sets the centre and the rest fold onto it.
        let mut f = frames(&[200.0; 50], 0.05);
        f.extend(frames(&[100.0; 50], 0.34));
        let p = profile(&f).unwrap();
        assert!(cents(p.median_hz, 200.0).abs() < 1.0, "{p:?}");
        assert!((p.octave_corrected - 0.5).abs() < 1e-9, "{p:?}");
        // Confidence reports the truth: these frames were mostly noisy.
        assert!((p.confidence - (1.0 - 0.34)).abs() < 0.15, "{p:?}");
    }

    #[test]
    fn a_uniformly_breathy_voice_still_gets_a_profile() {
        // Nothing is confident: the centre falls back to every frame rather than giving up.
        let f = frames(&speech(60), 0.30);
        let p = profile(&f).expect("profile");
        assert!(cents(p.median_hz, 120.0).abs() < 60.0, "{p:?}");
        assert!(p.octave_corrected < 1e-12, "{p:?}");
    }

    #[test]
    fn folding_is_idempotent_and_bounded() {
        let p = profile(&frames(&speech(80), 0.05)).unwrap();
        for f0 in [60.0, 120.0, 240.0, 480.0] {
            let once = fold_toward(f0, p.centre_log2);
            assert!((fold_toward(once, p.centre_log2) - once).abs() < 1e-9);
        }
        // Eight times the centre is more than MAX_FOLD_OCTAVES away: left alone, not mangled.
        let far = p.centre_log2.exp2() * 8.0;
        assert!((fold_toward(far, p.centre_log2) - far).abs() < 1e-9);
        // A fifth above the centre is not an octave error.
        let fifth = p.centre_log2.exp2() * 2f64.powf(7.0 / 12.0);
        assert!((fold_toward(fifth, p.centre_log2) - fifth).abs() < 1e-9);
        assert!(fold_toward(0.0, p.centre_log2).abs() < 1e-12);
    }

    #[test]
    fn nothing_in_nothing_out() {
        assert!(profile(&[]).is_none());
        assert!(profile(&frames(&[0.0, -1.0], 0.1)).is_none());
    }
}
