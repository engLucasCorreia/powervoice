//! Acceptance tests from tickets/S1-02-wav-peaks.md, checked against `vox_testkit::wav` as an
//! independent measuring stick (ADR-001 §3 amendment): open->save round trip is bit-exact at
//! 16/24-bit and 32-bit float, stereo L=+sine/R=-sine downmixes to exact silence, and the 16-bit
//! TPDF dither residual lands at -96.33 dBFS +/- 0.5 dB.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use vox_io::{BitDepth, read_wav, write_wav};
use vox_testkit::{signal, wav as tkwav};

fn tmp_path(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "vox-io-acceptance-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("f.wav")
}

#[test]
fn round_trip_16_24_32f_bit_exact_when_already_grid_exact() {
    let sine = signal::sine(997.0, -6.0, 0.2, 48_000).unwrap();
    let cases = [
        ("16", BitDepth::Int16, tkwav::BitDepth::Int16),
        ("24", BitDepth::Int24, tkwav::BitDepth::Int24),
        ("32f", BitDepth::Float32, tkwav::BitDepth::Float32),
    ];
    for (tag, io_bits, tk_bits) in cases {
        // A reference file at this bit depth: every sample is already exactly representable
        // there, so a bit-exact open->save round trip must not introduce any dither noise.
        let path = tmp_path(&format!("roundtrip-{tag}"));
        tkwav::write_wav_file(&path, &sine, 1, 48_000, tk_bits).unwrap();
        let (before, _) = tkwav::read_wav_file(&path).unwrap();

        let (rate, channels, mut source) = read_wav(&path).unwrap();
        assert_eq!(rate, 48_000);
        assert_eq!(channels, 1);
        let mut mono = vec![0.0f32; before.len()];
        let n = source.read_mono(&mut mono).unwrap();
        assert_eq!(n, mono.len(), "{tag}-bit: every sample read back");

        let out_path = tmp_path(&format!("roundtrip-out-{tag}"));
        let report = write_wav(&out_path, rate, io_bits, &mono).unwrap();
        assert_eq!(report.clipped_samples, 0);

        let (after, _) = tkwav::read_wav_file(&out_path).unwrap();
        assert_eq!(
            before, after,
            "{tag}-bit: open->save of an unedited file must be bit-exact"
        );
    }
}

#[test]
fn silence_stays_silence_through_a_16_bit_round_trip() {
    let silence = signal::silence(0.05, 48_000).unwrap();
    let path = tmp_path("silence-roundtrip");
    let report = write_wav(&path, 48_000, BitDepth::Int16, &silence).unwrap();
    assert_eq!(report.clipped_samples, 0);
    let (decoded, _) = tkwav::read_wav_file(&path).unwrap();
    assert!(
        decoded.iter().all(|&s| s == 0.0),
        "digital silence must stay digital silence, not be dithered into noise"
    );
}

#[test]
fn stereo_opposite_channels_downmix_to_exact_silence() {
    let sine = signal::sine(440.0, -6.0, 0.05, 48_000).unwrap();
    let mut interleaved = Vec::with_capacity(sine.len() * 2);
    for &s in &sine {
        interleaved.push(s); // left = +sine
        interleaved.push(-s); // right = -sine
    }
    let path = tmp_path("stereo-cancel");
    tkwav::write_wav_file(&path, &interleaved, 2, 48_000, tkwav::BitDepth::Float32).unwrap();

    let (rate, channels, mut source) = read_wav(&path).unwrap();
    assert_eq!(rate, 48_000);
    assert_eq!(channels, 2, "original channel count is still reported");
    let mut mono = vec![1.0f32; sine.len()];
    let n = source.read_mono(&mut mono).unwrap();
    assert_eq!(n, sine.len());
    assert!(
        mono.iter().all(|&s| s == 0.0),
        "L=+sine, R=-sine must downmix (average) to exact silence"
    );
}

#[test]
fn multichannel_average_downmix_matches_manual_average() {
    // 3 channels, distinct deterministic content per channel; downmix must equal the exact
    // per-frame average.
    let a = signal::sine(220.0, -12.0, 0.05, 48_000).unwrap();
    let b = signal::sine(330.0, -12.0, 0.05, 48_000).unwrap();
    let c = signal::white_noise(1, -20.0, 0.05, 48_000).unwrap();
    let mut interleaved = Vec::with_capacity(a.len() * 3);
    for i in 0..a.len() {
        interleaved.push(a[i]);
        interleaved.push(b[i]);
        interleaved.push(c[i]);
    }
    let path = tmp_path("tri-downmix");
    tkwav::write_wav_file(&path, &interleaved, 3, 48_000, tkwav::BitDepth::Float32).unwrap();

    let (_, channels, mut source) = read_wav(&path).unwrap();
    assert_eq!(channels, 3);
    let mut mono = vec![0.0f32; a.len()];
    assert_eq!(source.read_mono(&mut mono).unwrap(), a.len());
    for i in 0..a.len() {
        let expected = (f64::from(a[i]) + f64::from(b[i]) + f64::from(c[i])) / 3.0;
        assert!(
            (f64::from(mono[i]) - expected).abs() < 1e-6,
            "frame {i}: {} vs {expected}",
            mono[i]
        );
    }
}

#[test]
fn tpdf_dither_residual_matches_spec_16_bit() {
    // SPEC-005 §2.8: 16-bit TPDF residual RMS = -96.33 dBFS (testkit's 20*log10(rms) convention),
    // independent of the signal's own level.
    let sine = signal::sine(997.0, -20.0, 1.0, 48_000).unwrap();
    let path = tmp_path("tpdf-residual");
    write_wav(&path, 48_000, BitDepth::Int16, &sine).unwrap();
    let (decoded, info) = tkwav::read_wav_file(&path).unwrap();
    assert_eq!(info.bits_per_sample, 16);

    let residual: Vec<f32> = sine
        .iter()
        .zip(decoded.iter())
        .map(|(&a, &b)| a - b)
        .collect();
    let residual_db = vox_testkit::measure::rms_db(&residual);
    assert!(
        (residual_db - (-96.33)).abs() < 0.5,
        "16-bit TPDF residual {residual_db:.2} dBFS, expected -96.33 +/- 0.5"
    );
}

#[test]
fn out_of_range_samples_are_clipped_and_reported() {
    let samples = [1.5f32, -2.0, 0.0, 0.5];
    let path = tmp_path("clip");
    let report = write_wav(&path, 48_000, BitDepth::Int16, &samples).unwrap();
    assert_eq!(report.clipped_samples, 2);
    let (decoded, _) = tkwav::read_wav_file(&path).unwrap();
    assert!((decoded[0] - 1.0).abs() < 1e-3);
    assert!((decoded[1] - (-1.0)).abs() < 1e-3);

    let report = write_wav(&path, 48_000, BitDepth::Float32, &samples).unwrap();
    assert_eq!(report.clipped_samples, 0, "float32 never clips");
}
