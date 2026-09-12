//! T-006 acceptance tests (see `tickets/T-006-testkit-cli.md`).

use vox_testkit::{golden, measure, signal};

const SAMPLE_RATE: u32 = 48_000;

#[test]
fn mono_1khz_sine_at_minus20_dbfs() {
    let s = signal::sine(1000.0, -20.0, 20.0, SAMPLE_RATE).unwrap();

    let peak = measure::peak_dbfs(&s);
    assert!(
        (peak - (-20.0)).abs() <= 0.01,
        "peak {peak:.4} dBFS, expected -20.00 +/- 0.01"
    );

    let rms = measure::rms_db(&s);
    assert!(
        (rms - (-23.01)).abs() <= 0.01,
        "rms {rms:.4} dB, expected -23.01 +/- 0.01"
    );

    let report = measure::loudness(&s, 1, SAMPLE_RATE).expect("loudness measurement");
    assert!(
        (report.integrated_lufs - (-23.0)).abs() <= 0.1,
        "integrated {:.4} LUFS, expected -23.0 +/- 0.1",
        report.integrated_lufs
    );
}

#[test]
fn tech_3341_case_1_minus23_dbfs_per_channel() {
    let s = signal::tech3341_case(1, 20.0, SAMPLE_RATE).unwrap();
    let report = measure::loudness(&s, 2, SAMPLE_RATE).expect("loudness measurement");

    for (name, value) in [
        ("integrated", report.integrated_lufs),
        (
            "max momentary",
            report
                .max_momentary_lufs
                .expect("20s input covers the 400ms window"),
        ),
        (
            "max short-term",
            report
                .max_short_term_lufs
                .expect("20s input covers the 3s window"),
        ),
    ] {
        assert!(
            (value - (-23.0)).abs() <= 0.1,
            "{name} {value:.4} LUFS, expected -23.0 +/- 0.1"
        );
    }
}

#[test]
fn tech_3341_case_2_minus33_dbfs_per_channel() {
    let s = signal::tech3341_case(2, 20.0, SAMPLE_RATE).unwrap();
    let report = measure::loudness(&s, 2, SAMPLE_RATE).expect("loudness measurement");

    for (name, value) in [
        ("integrated", report.integrated_lufs),
        (
            "max momentary",
            report
                .max_momentary_lufs
                .expect("20s input covers the 400ms window"),
        ),
        (
            "max short-term",
            report
                .max_short_term_lufs
                .expect("20s input covers the 3s window"),
        ),
    ] {
        assert!(
            (value - (-33.0)).abs() <= 0.1,
            "{name} {value:.4} LUFS, expected -33.0 +/- 0.1"
        );
    }
}

#[test]
fn tech_3342_lra_of_two_level_stereo_tone() {
    // EBU Tech 3342 LRA conformance case 1: 20 s at -20 dBFS/ch then 20 s at
    // -30 dBFS/ch, stereo 1 kHz -> LRA should read the ~10 LU step between
    // the two loudness levels (10th-95th percentile gating absorbs the
    // instantaneous transition, so the tolerance is wider than the I/M/S
    // cases above).
    let mut s = signal::sine_stereo(1000.0, -20.0, 20.0, SAMPLE_RATE).unwrap();
    s.extend(signal::sine_stereo(1000.0, -30.0, 20.0, SAMPLE_RATE).unwrap());

    let report = measure::loudness(&s, 2, SAMPLE_RATE).expect("loudness measurement");
    assert!(
        (report.lra_lu - 10.0).abs() <= 1.0,
        "LRA {:.4} LU, expected 10 +/- 1",
        report.lra_lu
    );
}

#[test]
fn true_peak_detects_inter_sample_overshoot() {
    // At freq == fs/4 with a 45 degree phase offset, every discrete sample
    // lands at +/- sqrt(2)/2 of the analytic (continuous-time) peak, so the
    // sample peak reads 3.0103 dB *below* the true (analytic) peak. Solve for
    // the amplitude that makes the *sample* peak land at -6 dBFS.
    const TARGET_SAMPLE_PEAK_DBFS: f64 = -6.0;
    const SAMPLE_TO_ANALYTIC_DB: f64 = 3.0103;
    let level_dbfs = TARGET_SAMPLE_PEAK_DBFS + SAMPLE_TO_ANALYTIC_DB;
    let analytic_true_peak_dbtp = level_dbfs;

    let freq_hz = f64::from(SAMPLE_RATE) / 4.0;
    let s = signal::sine_with_phase(freq_hz, level_dbfs, 45.0, 1.0, SAMPLE_RATE).unwrap();

    let sample_peak = measure::peak_dbfs(&s);
    assert!(
        (sample_peak - TARGET_SAMPLE_PEAK_DBFS).abs() <= 0.05,
        "sample peak {sample_peak:.4} dBFS, expected -6.00 +/- 0.05"
    );

    let report = measure::loudness(&s, 1, SAMPLE_RATE).expect("loudness measurement");
    assert!(
        report.true_peak_dbtp > sample_peak,
        "true peak {:.4} dBTP should exceed sample peak {:.4} dBFS",
        report.true_peak_dbtp,
        sample_peak
    );
    assert!(
        (report.true_peak_dbtp - analytic_true_peak_dbtp).abs() <= 0.3,
        "true peak {:.4} dBTP, expected within +/- 0.3 dB of analytic {:.4}",
        report.true_peak_dbtp,
        analytic_true_peak_dbtp
    );
}

#[test]
fn noise_floor_of_bursts_with_quiet_gaps() {
    // 10s tone bursts (1s on) with 1s gaps of -70 dBFS white noise.
    let s =
        signal::tone_bursts(123, 1000.0, -10.0, Some(-70.0), 1.0, 1.0, 20.0, SAMPLE_RATE).unwrap();
    let floor = measure::noise_floor_db(&s, 1, SAMPLE_RATE, measure::DEFAULT_NOISE_FLOOR_WINDOW_MS)
        .expect("20s input covers the 500ms window");
    assert!(
        (floor - (-70.0)).abs() <= 1.0,
        "noise floor {floor:.4} dB, expected -70 +/- 1"
    );
}

/// Golden hashes pin exact generator output for a fixed seed. Unlike the
/// vacuous "does this process agree with itself" version of this test, an
/// accidental change to any generator's arithmetic will flip these and be
/// caught here. FNV-1a (not `DefaultHasher`/SipHash, which carries no
/// cross-Rust-release stability guarantee) over raw `f32::to_bits`, so it
/// depends only on the PRNG and the generator's floating-point arithmetic,
/// not on `sin`/`powf` transcendental-function ULP differences (all three
/// signals below are PRNG-driven, not sine-based, for exactly that reason).
#[test]
fn generator_output_matches_golden_hashes() {
    let white = signal::white_noise(1, 0.0, 0.01, SAMPLE_RATE).unwrap();
    assert_eq!(
        golden::fnv1a_hash(&white),
        0x7ec8_b1ec_3ae2_9c76,
        "white_noise(seed=1, 0 dBFS, 10ms, 48kHz) golden hash mismatch"
    );

    let pink = signal::pink_noise(1, 0.0, 0.01, SAMPLE_RATE).unwrap();
    assert_eq!(
        golden::fnv1a_hash(&pink),
        0x5c07_f78d_3571_d474,
        "pink_noise(seed=1, 0 dBFS, 10ms, 48kHz) golden hash mismatch"
    );

    let bursts =
        signal::tone_bursts(1, 1000.0, 0.0, Some(0.0), 0.005, 0.005, 0.01, SAMPLE_RATE).unwrap();
    assert_eq!(
        golden::fnv1a_hash(&bursts),
        0x84d5_52a8_3735_6523,
        "tone_bursts(seed=1, 0 dBFS tone + 0 dBFS gap noise, 10ms, 48kHz) golden hash mismatch"
    );
}

#[test]
fn generators_are_deterministic_for_a_given_seed() {
    let sine_a = signal::sine(440.0, -12.0, 0.25, SAMPLE_RATE).unwrap();
    let sine_b = signal::sine(440.0, -12.0, 0.25, SAMPLE_RATE).unwrap();
    assert_eq!(sine_a, sine_b);

    let white_a = signal::white_noise(7, -20.0, 0.25, SAMPLE_RATE).unwrap();
    let white_b = signal::white_noise(7, -20.0, 0.25, SAMPLE_RATE).unwrap();
    assert_eq!(white_a, white_b);

    let pink_a = signal::pink_noise(7, -20.0, 0.25, SAMPLE_RATE).unwrap();
    let pink_b = signal::pink_noise(7, -20.0, 0.25, SAMPLE_RATE).unwrap();
    assert_eq!(pink_a, pink_b);

    // Different seeds must (overwhelmingly likely) diverge.
    let white_c = signal::white_noise(8, -20.0, 0.25, SAMPLE_RATE).unwrap();
    assert_ne!(white_a, white_c);
}
