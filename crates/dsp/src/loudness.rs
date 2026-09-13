//! Integrated/short-term/momentary loudness, loudness range and sample/true peak per
//! ITU-R BS.1770-5 / EBU R128 (S4-01; docs/references.md), for the mono processed-or-source
//! signal the loudness analysis job and LUFS normalize measure. Wraps `ebur128` (feature
//! `precision-true-peak` for 4×-oversampled true peak, MEMORY.md gotcha) — the same crate
//! `vox-testkit` wraps independently as its own reference (ADR-001 §3), so this module's tests
//! cross-check against it. Mono channel weight G = 1.0, no dual-mono +3 dB compensation by
//! default (PROMPT §3.3).
//!
//! MEMORY.md: `ebur128`'s true peak reads ~+0.10 dB high near fs/4 and is not the normative TP
//! meter for pass/fail gates (the true-peak limiter uses its own oversampled detector,
//! [`crate::true_peak`], validated against testkit's 16× reference) — here it is a *display*
//! measurement (the loudness panel, and the true-peak-ceiling notice LUFS normalize shows
//! without limiting), well within this ticket's own ±0.1 LU/dB tolerances.
//!
//! Convention (matches `vox-testkit::measure`): exact digital silence, or a scope with no block
//! above the BS.1770 absolute gate, reports `-inf`; any non-finite input sample makes every
//! field `NaN`; `max_momentary_lufs`/`max_short_term_lufs` are `None` when the scope is shorter
//! than that window (400 ms / 3 s).

use ebur128::{EbuR128, Mode};

/// Wraps an `ebur128` failure — only reachable with a bad channel count/sample rate, which never
/// happens here (this module only ever constructs 1- or 2-channel state at a real sample rate).
#[derive(Debug, thiserror::Error)]
#[error("loudness measurement: {0}")]
pub struct LoudnessError(#[from] ebur128::Error);

/// One measurement's BS.1770/EBU R128 report (SPEC-011 pending; this is the S4-01 ticket's
/// report shape: I / S-max / M-max / LRA / sample peak / true peak).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoudnessReport {
    pub integrated_lufs: f64,
    /// `None` if the scope is shorter than the 400 ms momentary window.
    pub max_momentary_lufs: Option<f64>,
    /// `None` if the scope is shorter than the 3 s short-term window.
    pub max_short_term_lufs: Option<f64>,
    pub lra_lu: f64,
    pub sample_peak_dbfs: f64,
    pub true_peak_dbtp: f64,
}

const MOMENTARY_WINDOW_S: f64 = 0.4;
const SHORT_TERM_WINDOW_S: f64 = 3.0;

/// `20*log10(|linear|)`; exact digital silence maps to `-inf`, `NaN` propagates — no artificial
/// floor (the project-wide convention, `vox-testkit::units::linear_to_dbfs`; duplicated here
/// rather than shared, since `dsp` and `testkit` are independent leaves, ADR-001 §3).
///
/// `pub(crate)`: [`crate::acx`] reuses it for RMS/peak/noise-floor (S4-03) rather than
/// duplicating it a second time within this same crate.
pub(crate) fn linear_to_dbfs(linear: f64) -> f64 {
    20.0 * linear.abs().log10()
}

/// A streaming loudness meter: `push` blocks of any size (a rack-render block, a document read
/// chunk, …) then `finish`. Memory stays bounded regardless of the scope's length — an offline
/// render or a 60-minute read never needs to fit in one buffer.
pub struct LoudnessMeter {
    state: EbuR128,
    channels: u32,
    sample_rate_hz: u32,
    frames: u64,
    max_momentary: f64,
    max_short_term: f64,
    non_finite: bool,
}

impl LoudnessMeter {
    /// A mono meter — every production caller (PowerVoice documents and the rack are
    /// single-channel).
    pub fn new(sample_rate_hz: u32) -> Result<Self, LoudnessError> {
        Self::with_channels(1, sample_rate_hz)
    }

    /// A meter over `channels` interleaved channels, exposed only so this crate's own tests can
    /// cross-check the EBU Tech 3341 (stereo) conformance cases against `vox-testkit`; production
    /// code always uses [`Self::new`].
    pub fn with_channels(channels: u32, sample_rate_hz: u32) -> Result<Self, LoudnessError> {
        let mode = Mode::I | Mode::LRA | Mode::TRUE_PEAK | Mode::SAMPLE_PEAK;
        let state = EbuR128::new(channels, sample_rate_hz, mode)?;
        Ok(Self {
            state,
            channels,
            sample_rate_hz,
            frames: 0,
            max_momentary: f64::NEG_INFINITY,
            max_short_term: f64::NEG_INFINITY,
            non_finite: false,
        })
    }

    /// Feeds one block of interleaved samples (length a multiple of the channel count). A
    /// non-finite sample anywhere makes every [`LoudnessReport`] field `NaN` once [`Self::finish`]
    /// is called — checked here rather than left to `ebur128`, which does not document its
    /// behaviour on non-finite input.
    pub fn push(&mut self, block: &[f32]) -> Result<(), LoudnessError> {
        if self.non_finite {
            return Ok(());
        }
        if block.iter().any(|s| !s.is_finite()) {
            self.non_finite = true;
            return Ok(());
        }
        if block.is_empty() {
            return Ok(());
        }
        self.state.add_frames_f32(block)?;
        self.frames += block.len() as u64 / u64::from(self.channels);
        let momentary = self.state.loudness_momentary()?;
        if momentary.is_finite() {
            self.max_momentary = self.max_momentary.max(momentary);
        }
        let short_term = self.state.loudness_shortterm()?;
        if short_term.is_finite() {
            self.max_short_term = self.max_short_term.max(short_term);
        }
        Ok(())
    }

    /// Finalizes the measurement (BS.1770 gating runs here, over every block seen).
    pub fn finish(self) -> Result<LoudnessReport, LoudnessError> {
        if self.non_finite {
            return Ok(LoudnessReport {
                integrated_lufs: f64::NAN,
                max_momentary_lufs: Some(f64::NAN),
                max_short_term_lufs: Some(f64::NAN),
                lra_lu: f64::NAN,
                sample_peak_dbfs: f64::NAN,
                true_peak_dbtp: f64::NAN,
            });
        }
        let integrated_lufs = self.state.loudness_global()?;
        let lra_lu = self.state.loudness_range()?;
        let mut sample_peak_linear = 0.0f64;
        let mut true_peak_linear = 0.0f64;
        for channel in 0..self.channels {
            sample_peak_linear = sample_peak_linear.max(self.state.sample_peak(channel)?);
            true_peak_linear = true_peak_linear.max(self.state.true_peak(channel)?);
        }
        let duration_s = self.frames as f64 / f64::from(self.sample_rate_hz);
        Ok(LoudnessReport {
            integrated_lufs,
            max_momentary_lufs: (duration_s >= MOMENTARY_WINDOW_S).then_some(self.max_momentary),
            max_short_term_lufs: (duration_s >= SHORT_TERM_WINDOW_S).then_some(self.max_short_term),
            lra_lu,
            sample_peak_dbfs: linear_to_dbfs(sample_peak_linear),
            true_peak_dbtp: linear_to_dbfs(true_peak_linear),
        })
    }
}

/// One-shot convenience over a whole mono buffer (tests, and small scopes). Long-lived callers
/// (the analysis job, LUFS normalize's scan) stream through [`LoudnessMeter`] instead.
pub fn measure(samples: &[f32], sample_rate_hz: u32) -> Result<LoudnessReport, LoudnessError> {
    let mut meter = LoudnessMeter::new(sample_rate_hz)?;
    meter.push(samples)?;
    meter.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_testkit::{measure as testkit_measure, signal};

    const RATE: u32 = 48_000;

    /// Ticket AC: "testkit cross-check: mono 1 kHz −20 dBFS → −23.0 ± 0.1 LUFS" — cross-checked
    /// directly against `vox-testkit::measure::loudness`, the independent reference this module's
    /// production code must agree with.
    #[test]
    fn mono_1khz_minus20_dbfs_matches_the_reference_and_is_minus_23_lufs() {
        let s = signal::sine(1000.0, -20.0, 20.0, RATE).unwrap();
        let report = measure(&s, RATE).unwrap();
        assert!(
            (report.integrated_lufs - (-23.0)).abs() <= 0.1,
            "integrated {:.4} LUFS, expected -23.0 +/- 0.1",
            report.integrated_lufs
        );
        let reference = testkit_measure::loudness(&s, 1, RATE).unwrap();
        assert!(
            (report.integrated_lufs - reference.integrated_lufs).abs() <= 1e-6,
            "production {:.6} vs testkit reference {:.6}",
            report.integrated_lufs,
            reference.integrated_lufs
        );
        assert!((report.lra_lu - reference.lra_lu).abs() <= 1e-6);
        assert!((report.true_peak_dbtp - reference.true_peak_dbtp).abs() <= 1e-6);
    }

    /// Ticket AC: "EBU Tech 3341 cases 1-2 -> +/- 0.1 LU" (stereo conformance cases; exercised
    /// through [`LoudnessMeter::with_channels`], the same block-feeding/hop logic production code
    /// uses, generalized only for this cross-check — mono is the only production channel count).
    #[test]
    fn tech_3341_cases_are_within_tolerance_streamed_in_hops() {
        for (case, expected_lufs) in [(1u8, -23.0), (2u8, -33.0)] {
            let s = signal::tech3341_case(case, 20.0, RATE).unwrap();
            let mut meter = LoudnessMeter::with_channels(2, RATE).unwrap();
            // Feed in 100 ms hops (not one `push`) to exercise the same incremental usage the
            // analysis job's block loop relies on.
            let hop = (RATE as usize / 10) * 2; // 100ms * 2 channels
            for chunk in s.chunks(hop) {
                meter.push(chunk).unwrap();
            }
            let report = meter.finish().unwrap();
            assert!(
                (report.integrated_lufs - expected_lufs).abs() <= 0.1,
                "case {case}: integrated {:.4} LUFS, expected {expected_lufs} +/- 0.1",
                report.integrated_lufs
            );
        }
    }

    #[test]
    #[allow(clippy::float_cmp)] // deliberate exact-value check, not a tolerance comparison
    fn digital_silence_is_negative_infinity() {
        let s = signal::silence(2.0, RATE).unwrap();
        let report = measure(&s, RATE).unwrap();
        assert_eq!(report.integrated_lufs, f64::NEG_INFINITY);
        assert_eq!(report.sample_peak_dbfs, f64::NEG_INFINITY);
    }

    #[test]
    fn non_finite_sample_makes_every_field_nan() {
        let mut s = signal::sine(1000.0, -20.0, 1.0, RATE).unwrap();
        s[10] = f32::NAN;
        let report = measure(&s, RATE).unwrap();
        assert!(report.integrated_lufs.is_nan());
        assert!(report.max_momentary_lufs.unwrap().is_nan());
        assert!(report.lra_lu.is_nan());
        assert!(report.sample_peak_dbfs.is_nan());
        assert!(report.true_peak_dbtp.is_nan());
    }

    #[test]
    fn max_momentary_and_short_term_are_none_when_the_scope_is_too_short() {
        let s = signal::sine(1000.0, -20.0, 0.2, RATE).unwrap();
        let report = measure(&s, RATE).unwrap();
        assert!(report.max_momentary_lufs.is_none());
        assert!(report.max_short_term_lufs.is_none());

        let s = signal::sine(1000.0, -20.0, 1.0, RATE).unwrap();
        let report = measure(&s, RATE).unwrap();
        assert!(report.max_momentary_lufs.is_some());
        assert!(report.max_short_term_lufs.is_none());
    }

    /// Ticket AC: "processed vs source toggle differs by the rack gain (Gain -6 dB -> 6.0 +/- 0.1
    /// LU)" — this module doesn't know about the rack, but the underlying property it relies on
    /// (linear gain shifts integrated loudness by exactly that many dB) is this module's own
    /// correctness property, so it's checked directly here; `src-tauri::loudness` tests the actual
    /// processed-vs-source rack path end to end.
    #[test]
    fn a_linear_gain_shifts_integrated_loudness_by_the_same_db() {
        let s = signal::sine(1000.0, -20.0, 5.0, RATE).unwrap();
        let base = measure(&s, RATE).unwrap();
        let gain_db = -6.0;
        let gain = 10f64.powf(gain_db / 20.0) as f32;
        let gained: Vec<f32> = s.iter().map(|&x| x * gain).collect();
        let after = measure(&gained, RATE).unwrap();
        assert!(
            (after.integrated_lufs - (base.integrated_lufs + gain_db)).abs() <= 0.1,
            "expected {:.4}, got {:.4}",
            base.integrated_lufs + gain_db,
            after.integrated_lufs
        );
        assert!((after.true_peak_dbtp - (base.true_peak_dbtp + gain_db)).abs() <= 0.1);
    }
}
