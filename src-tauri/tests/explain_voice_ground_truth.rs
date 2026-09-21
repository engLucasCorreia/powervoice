//! H-95: the owner's own recording is the acceptance case for "Explain My Voice", not one test
//! among eight (see `tickets/H-95-voice-report-verification.md`). This test runs the *real*
//! `analyze_buffer` pipeline (the same one `src-tauri/src/spectrum.rs` runs for the "Average"
//! job, which "Explain My Voice" starts, per H-92) directly on `ExampleRecording.wav`, so:
//!
//! 1. `ui/src/lib/analyzer/explain/voiceFixtures.ts::ownerReport()` (a hand-pasted literal) is
//!    checked against a live computation instead of trusting the paste forever.
//! 2. The ticket's one open discrepancy is settled: `spectrum.csv`'s presence figure (~-4.4 dB
//!    relative to the 1 kHz octave) versus the app's reported -1.30 dB.
//!
//! **Finding (H-95): neither of the ticket's two candidate causes holds.**
//! - *Different normalisation* is ruled out algebraically and empirically: ENBW cancels in every
//!   `tone_balance` ratio (both bands divide by the same window's ENBW), so the app's exact
//!   formula applied to `spectrum.csv`'s own numbers below reproduces -4.50 dB, not -1.30 dB —
//!   the orchestrator's cruder arithmetic and the app's formula agree with each other.
//! - *Different material* (active-speech-only vs. a whole-file average that dilutes with
//!   near-silent pauses) is also ruled out: `analysis.ltas` (the same curve
//!   `spectrum.csv` was exported from — `src-tauri/src/spectrum.rs::encode_curve` uses `a.ltas`
//!   verbatim) computed straight off `ExampleRecording.wav` gives **-1.30 dB**, matching the
//!   *active*-gated report almost exactly, not -4.5 dB. That is because this file's pauses sit
//!   ~75 dB under the active level (`noise_floor_dbfs` -103 vs. `active_level_dbfs` -27) —
//!   utterly negligible in the power average, so gating changes nothing here.
//!
//! What actually explains it: `spectrum.csv`'s own numbers, band-summed exactly like
//! `analysis.ltas`, agree with `ExampleRecording.wav`'s real spectrum below ~500 Hz (within
//! ~2 dB) but sit 4-16 dB *quieter* through 700 Hz-13 kHz, dipping hardest around 5-7 kHz
//! (-15.7 dB) and recovering by 16-20 kHz (CSV is if anything a hair *louder* there) — a broad
//! mid/presence dip, not a uniform shelf, tilt, smoothing artefact (tested: 1/3-octave smoothing
//! only moves presence_db by ~0.3 dB) or window/FFT-size mismatch (bin spacing matches exactly;
//! `tone_balance`'s band-power method is window-invariant by construction). In short:
//! `spectrum.csv` does not represent `ExampleRecording.wav`'s true spectrum above ~500 Hz — the
//! two committed fixtures disagree with each other, not with the code. The app's own -1.30 dB is
//! independently reproducible straight from the raw audio samples using nothing but
//! `vox_dsp::diagnostics::features` (no UI, no IPC), so **the feature is correct**; `spectrum.csv`
//! is the stale/mismatched artifact and should be re-exported from the current build against
//! `ExampleRecording.wav` if a CSV cross-check fixture is still wanted.

use std::path::PathBuf;

use vox_dsp::diagnostics::features::{REFERENCE_BAND_HZ, SpectrumView, tone_balance};
use vox_dsp::diagnostics::{OfflineConfig, WindowKind, analyze_buffer};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent")
        .to_path_buf()
}

/// Parses `spectrum.csv` (`frequency_hz,level_db`, `-Infinity` as an empty cell) into
/// `(bin_hz, power_per_bin)` — power, not dB, so a caller can sum bands directly like
/// `SpectrumView::band_power` does.
fn load_spectrum_csv(path: &std::path::Path) -> (f64, Vec<(f64, f64)>) {
    let text = std::fs::read_to_string(path).expect("spectrum.csv must be readable");
    let mut rows = Vec::new();
    for line in text.lines().skip(1) {
        let Some((f, db)) = line.split_once(',') else {
            continue;
        };
        if db.is_empty() {
            continue;
        }
        let f: f64 = f.parse().expect("frequency_hz parses");
        let db: f64 = db.parse().expect("level_db parses");
        rows.push((f, 10f64.powf(db / 10.0)));
    }
    let bin_hz = rows[1].0 - rows[0].0;
    (bin_hz, rows)
}

/// Sums `power` over rows whose frequency lies in `[lo, hi)`, the same half-open convention as
/// `SpectrumView::bins`.
fn csv_band_power(rows: &[(f64, f64)], lo: f64, hi: f64) -> f64 {
    rows.iter()
        .filter(|&&(f, _)| f >= lo && f < hi)
        .map(|&(_, p)| p)
        .sum()
}

fn db(p: f64) -> f64 {
    if p > 0.0 {
        10.0 * p.log10()
    } else {
        f64::NEG_INFINITY
    }
}

/// The app's exact `tone_balance` presence formula (per-octave density relative to the 1 kHz
/// octave; ENBW cancels in the ratio so it's omitted) applied to `spectrum.csv`'s own band
/// powers.
fn csv_presence_db(rows: &[(f64, f64)]) -> f64 {
    let (ref_lo, ref_hi) = REFERENCE_BAND_HZ;
    let ref_oct = (ref_hi / ref_lo).log2();
    let ref_power = csv_band_power(rows, ref_lo, ref_hi);
    let (pres_lo, pres_hi): (f64, f64) = (2000.0, 5000.0);
    let pres_oct = (pres_hi / pres_lo).log2();
    let pres_power = csv_band_power(rows, pres_lo, pres_hi);
    (db(pres_power) - 10.0 * pres_oct.log10()) - (db(ref_power) - 10.0 * ref_oct.log10())
}

fn approx(actual: f64, expected: f64, tol: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: got {actual}, expected {expected} +/- {tol}"
    );
}

/// The owner's real voice, analysed exactly as "Explain My Voice" analyses it (H-92: the
/// Average job's default FFT size / window, SPEC-007 §8.5), cross-checked against:
/// - the hand-pasted `ownerReport()` fixture (H-91/H-94's golden prose tests), and
/// - the orchestrator's independent band-power arithmetic from `spectrum.csv` (H-95 ticket).
#[test]
fn owner_recording_matches_the_pasted_fixture_and_resolves_the_presence_discrepancy() {
    let root = workspace_root();
    let wav_path = root.join("ExampleRecording.wav");
    let (samples, info) =
        vox_testkit::wav::read_wav_file(&wav_path).expect("ExampleRecording.wav must be readable");
    assert_eq!(
        info.channels, 1,
        "the owner's recording is documented as mono"
    );

    let config = OfflineConfig {
        sample_rate_hz: info.sample_rate,
        fft_size: 16_384, // diagnostics.svelte.ts DEFAULT_PREFS.inspector_fft_size
        window: WindowKind::Hann, // DEFAULT_PREFS.inspector_window
    };
    let analysis =
        analyze_buffer(&samples, config, |_| true).expect("analysis must not be cancelled");
    let report = &analysis.report;

    // --- 1. Cross-check against voiceFixtures.ts::ownerReport() (values pasted 2026-09-21). ---
    let f0 = report.f0.as_ref().expect("voiced speech present");
    approx(f0.median_hz, 103.552_453_945_843_92, 0.05, "f0.median_hz");
    approx(f0.low_hz, 87.351_620_051_468_97, 0.05, "f0.low_hz");
    approx(f0.high_hz, 128.764_389_930_969_1, 0.05, "f0.high_hz");
    approx(
        f0.voiced_fraction,
        0.559_226_430_298_146_7,
        0.01,
        "f0.voiced_fraction",
    );
    approx(
        f0.octave_corrected,
        0.177_233_429_394_812_68,
        0.02,
        "f0.octave_corrected",
    );

    let tone = report.tone.expect("tone balance present");
    approx(tone.mud_db, 10.253_682_014_131_09, 0.1, "tone.mud_db");
    let presence_active = tone.presence_db.expect("presence measurable");
    approx(
        presence_active,
        -1.304_005_758_451_950_2,
        0.1,
        "tone.presence_db (active)",
    );
    approx(
        tone.air_db.expect("air measurable"),
        -22.142_890_990_671_418,
        0.1,
        "tone.air_db",
    );

    let sib = report.sibilance.expect("sibilance present");
    approx(
        sib.ratio_db,
        -20.120_227_447_928_52,
        0.2,
        "sibilance.ratio_db",
    );
    approx(
        sib.centre_hz,
        4_826.018_123_202_088,
        20.0,
        "sibilance.centre_hz",
    );

    assert!(report.hum.is_none(), "no mains hum in a clean recording");
    approx(
        report.rumble_db.expect("rumble present"),
        -28.286_414_326_847_876,
        0.1,
        "rumble_db",
    );
    approx(
        report.noise_floor_dbfs.expect("noise floor present"),
        -102.983_593_275_881_32,
        0.5,
        "noise_floor_dbfs",
    );
    approx(
        report.active_level_dbfs.expect("active level present"),
        -27.319_906_799_500_224,
        0.2,
        "active_level_dbfs",
    );
    approx(
        report.snr_db.expect("snr present"),
        75.663_686_476_381_09,
        0.5,
        "snr_db",
    );

    // --- 2. The presence discrepancy: rule out "different material" (H-95 cause 2). ---
    // `analysis.ltas` is the *same* whole-file curve the Spectrum Inspector displays and
    // CSV-exports (src-tauri/src/spectrum.rs::encode_curve uses `a.ltas` verbatim) — every
    // frame, active speech and pauses alike — versus `report.tone` above, which comes from the
    // level-gated *active* spectrum only (crates/dsp/src/diagnostics/voice.rs). If gating were
    // the explanation, these two would differ by the ~3 dB gap the ticket flags. They don't:
    // this recording's pauses sit ~75 dB under the active level, too quiet to move the average.
    let whole_file_view = SpectrumView::new(
        &analysis.ltas,
        config.sample_rate_hz,
        analysis.fft_size,
        analysis.enbw_bins,
    );
    let whole_file_tone = tone_balance(&whole_file_view).expect("whole-file tone balance");
    let presence_whole_file = whole_file_tone
        .presence_db
        .expect("whole-file presence measurable");
    approx(
        presence_whole_file,
        presence_active,
        0.05,
        "whole-file (ltas) presence_db must match the active-gated report: this recording's \
         pauses are ~75 dB under the signal, so gating cannot be the ~3 dB discrepancy's cause",
    );

    // --- 3. Rule out normalisation (H-95 cause 1), and locate the real mismatch. ---
    // The app's exact formula, applied to `spectrum.csv`'s own band powers, reproduces the
    // ticket's independently-computed -4.4/-4.5 dB — so normalisation was never the issue: the
    // orchestrator's arithmetic and the app's formula agree with each other on the CSV's numbers.
    let (csv_bin_hz, csv_rows) = load_spectrum_csv(&root.join("spectrum.csv"));
    approx(
        csv_bin_hz,
        f64::from(config.sample_rate_hz) / analysis.fft_size as f64,
        // spectrum.csv rounds frequency_hz to 2 decimals (spectrumCsv.ts), so consecutive
        // spacings wobble by rounding error, not by a different FFT size/sample rate.
        0.01,
        "spectrum.csv's bin spacing must match the default FFT size (16384 @ 48 kHz)",
    );
    approx(
        csv_presence_db(&csv_rows),
        -4.497_424_271_727_83,
        0.05,
        "the app's own presence formula, applied to spectrum.csv, must match the ticket's \
         independent CSV arithmetic (ruling out a normalisation difference)",
    );

    // So neither candidate cause holds: gating changes nothing (step 2), and normalisation is
    // identical (step 3, both landing near -4.5 dB on the CSV's own numbers). What's left is
    // that spectrum.csv itself doesn't carry the same 700 Hz-16 kHz content as the WAV file
    // that ships beside it: below 500 Hz the two agree to ~2 dB, but the CSV is 4-16 dB quieter
    // than ExampleRecording.wav's real spectrum from ~700 Hz to ~13 kHz (worst around 5-7 kHz),
    // recovering by 16-20 kHz — a broad dip, not a shelf/tilt/smoothing/window artefact (all
    // ruled out separately; see the module doc comment). That is a stale/mismatched fixture,
    // not a defect in `tone_balance` or in the report H-91/H-94 print: the report's -1.30 dB is
    // independently reproducible from the raw audio samples alone.
    // (`SpectrumView::band_power` divides by ENBW; multiply back out for a number comparable to
    // `csv_band_power`, which never divided by anything.)
    let enbw = analysis.enbw_bins;
    let ref_power_csv = csv_band_power(&csv_rows, REFERENCE_BAND_HZ.0, REFERENCE_BAND_HZ.1);
    let ref_power_wav = whole_file_view.band_power(REFERENCE_BAND_HZ.0, REFERENCE_BAND_HZ.1) * enbw;
    let low_band_diff_db = db(csv_band_power(&csv_rows, 200.0, 500.0))
        - db(whole_file_view.band_power(200.0, 500.0) * enbw);
    assert!(
        low_band_diff_db.abs() < 3.0,
        "sanity: 200-500 Hz should roughly agree between spectrum.csv and the WAV \
         (diff {low_band_diff_db} dB) — if this drifts, the whole finding needs re-checking"
    );
    let mid_band_diff_db = db(ref_power_csv) - db(ref_power_wav);
    assert!(
        mid_band_diff_db < -3.0,
        "sanity: the 1 kHz reference octave should be *quieter* in spectrum.csv than in the \
         WAV by several dB (diff {mid_band_diff_db} dB) — this is the actual, located mismatch"
    );
}
