//! SPEC-017 §4.6 / AC-5 (subset): the 16× reference true-peak meter reads analytic sine peaks
//! within ±0.005 dB, the testkit fs/4 45° case reads −2.990 dBTP, and the band-limiting
//! helpers used by the limiter stress set behave.

use vox_testkit::bandlimit::{bandlimited_clicks, fade_and_pad, kaiser_lowpass};
use vox_testkit::measure::peak_dbfs;
use vox_testkit::signal::sine_with_phase;
use vox_testkit::true_peak::{
    true_peak_4x_dbtp, true_peak_reference_dbtp, true_peak_reference_range_dbtp,
};

/// Context kept on both sides of the measured range (≥ the 64-sample half-width).
const CONTEXT: usize = 200;

#[test]
fn reference_reads_analytic_sine_peaks() {
    let mut worst = 0.0_f64;
    for rate in [44_100u32, 48_000, 96_000] {
        let fs = f64::from(rate);
        for f in [
            20.0,
            100.0,
            997.0,
            5_000.0,
            10_000.0,
            15_000.0,
            0.4 * fs,
            0.45 * fs,
        ] {
            let phases = if f < 200.0 { 4 } else { 16 };
            for k in 0..phases {
                let phase_deg = 7.3 + 360.0 * f64::from(k) / f64::from(phases);
                let level = -3.0;
                // Half a period (+2) holds at least one peak.
                let span = (fs / f / 2.0).ceil() as usize + 2;
                let n = span + 2 * CONTEXT + 1;
                let x = sine_with_phase(f, level, phase_deg, n as f64 / fs, rate).unwrap();
                assert!(x.len() >= span + 2 * CONTEXT);
                let tp = true_peak_reference_range_dbtp(&x, CONTEXT..CONTEXT + span);
                let err = tp - level;
                worst = worst.max(err.abs());
                assert!(
                    err.abs() <= 0.005,
                    "{rate} Hz, {f:.1} Hz, phase {phase_deg:.1}°: {tp:.5} dBTP"
                );
            }
        }
    }
    println!("reference meter worst |error| on sines: {worst:.5} dB");
}

#[test]
fn faded_and_padded_sines_read_their_peak() {
    for (f, phase) in [(997.0, 11.0), (15_000.0, 63.0), (19_900.0, 170.0)] {
        let x = sine_with_phase(f, -1.0, phase, 0.2, 48_000).unwrap();
        let y = fade_and_pad(&x, 5.0, 50.0, 48_000);
        let tp = true_peak_reference_dbtp(&y);
        assert!((tp + 1.0).abs() <= 0.005, "{f} Hz: {tp:.5}");
        // The plain 4× points are a subset of the reference's points.
        assert!(true_peak_4x_dbtp(&y) <= tp + 1e-12);
    }
}

#[test]
fn fs_over_4_at_45_degrees_reads_minus_2_990() {
    // Sample peak −6.00 dBFS, analytic peak −6 + 3.0103 dB.
    let level = -6.0 + 20.0 * 2f64.sqrt().log10();
    let x = sine_with_phase(12_000.0, level, 45.0, 0.5, 48_000).unwrap();
    let y = fade_and_pad(&x, 5.0, 50.0, 48_000);
    assert!((peak_dbfs(&x) + 6.0).abs() < 1e-3);
    let tp = true_peak_reference_dbtp(&y);
    assert!((tp + 2.990).abs() <= 0.005, "{tp:.5}");
    let tp4 = true_peak_4x_dbtp(&y);
    assert!((tp4 + 2.990).abs() <= 0.005, "4x {tp4:.5}");
}

#[test]
fn silence_and_non_finite() {
    let silence = true_peak_reference_dbtp(&[0.0; 100]);
    assert!(silence.is_infinite() && silence < 0.0, "{silence}");
    assert!(true_peak_reference_dbtp(&[0.0, f32::NAN]).is_nan());
    assert!(true_peak_4x_dbtp(&[f32::INFINITY]).is_nan());
}

#[test]
fn kaiser_lowpass_passes_below_and_stops_above() {
    let rate = 48_000;
    for (f, lo, hi) in [(1_000.0, -1e-4, 1e-4), (18_900.0, -1e-3, 1e-3)] {
        let x = sine_with_phase(f, 0.0, 0.0, 0.2, rate).unwrap();
        let y = kaiser_lowpass(&x, 19_500.0, 1_000.0, 110.0, rate).unwrap();
        assert_eq!(y.len(), x.len());
        let mid = &y[2_000..7_000];
        let g = peak_dbfs(mid) - peak_dbfs(&x[2_000..7_000]);
        assert!(g > lo && g < hi, "{f} Hz: {g} dB");
    }
    let x = sine_with_phase(20_500.0, 0.0, 0.0, 0.2, rate).unwrap();
    let y = kaiser_lowpass(&x, 19_500.0, 1_000.0, 110.0, rate).unwrap();
    assert!(peak_dbfs(&y[2_000..7_000]) < -100.0);
    assert!(kaiser_lowpass(&x, 30_000.0, 1_000.0, 110.0, rate).is_err());
}

#[test]
fn fades_pad_and_clicks() {
    let x = vec![1.0f32; 1_000];
    let y = fade_and_pad(&x, 5.0, 50.0, 48_000);
    assert_eq!(y.len(), 1_000 + 2 * 2_400);
    assert!(y[..2_400].iter().all(|&v| v == 0.0));
    assert!(y[y.len() - 2_400..].iter().all(|&v| v == 0.0));
    assert!(y[2_400] > 0.0 && y[2_400] < 0.01);
    assert!((y[2_400 + 500] - 1.0).abs() < f32::EPSILON);

    let c = bandlimited_clicks(1, 40, 0.5, 1.0, 19_500.0, 1.0, 48_000).unwrap();
    assert_eq!(c.len(), 48_000);
    let p = peak_dbfs(&c);
    assert!(p > -7.0 && p < 1.0, "{p}");
}
