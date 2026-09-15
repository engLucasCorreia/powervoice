//! H-42: the analyzer diagnostics' cost (SPEC-007 §8.11). The live voice tracker runs on the
//! engine's control tick next to the analyzer, one tick's worth of samples at a time; its
//! budget is the analyzer consumer's own (SPEC-007 AC-18: ≤ 2 % of one core at 60 Hz, i.e.
//! ≤ 333 µs per frame). The Spectrum Inspector adds one windowed FFT per 30 Hz frame at the
//! chosen size, and the long-term average job is reported as a realtime factor
//! (informational).

use std::time::Instant;

use divan::Bencher;
use vox_dsp::diagnostics::{
    OfflineConfig, PowerSpectrum, VoiceTracker, WindowKind, analyze_buffer,
};
use vox_testkit::bench_report::{self, Target};
use vox_testkit::signal;

const RATE: u32 = 48_000;
/// One 60 Hz control tick of samples at 48 kHz.
const TICK: usize = 800;
const INSPECTOR_SIZES: [usize; 4] = [1024, 4096, 16_384, 32_768];

/// Speech-like bursts (a 140 Hz tone over noise, 1.2 s on / 0.8 s off).
fn speech(seconds: f64) -> Vec<f32> {
    signal::voice_like(1, 140.0, -20.0, 30.0, 1.2, 0.8, seconds, RATE).expect("signal")
}

struct TrackerState {
    tracker: VoiceTracker,
    input: Vec<f32>,
    pos: usize,
}

/// One control tick: push 800 samples (YIN every 10 ms, the diagnostics FFT every 100 ms).
#[divan::bench]
fn voice_tracker_tick(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let input = speech(10.0);
            let mut tracker = VoiceTracker::new(RATE);
            tracker.push(&input[..RATE as usize * 5]);
            TrackerState {
                tracker,
                input,
                pos: RATE as usize * 5,
            }
        })
        .bench_local_refs(|s: &mut TrackerState| {
            if s.pos + TICK > s.input.len() {
                s.pos = 0;
            }
            s.tracker.push(&s.input[s.pos..s.pos + TICK]);
            s.pos += TICK;
        });
}

/// A report (sorting the F0 history, the level statistics, the spectral features).
#[divan::bench]
fn voice_tracker_report(bencher: Bencher) {
    bencher
        .with_inputs(|| {
            let mut tracker = VoiceTracker::new(RATE);
            tracker.push(&speech(12.0));
            tracker
        })
        .bench_local_refs(|t: &mut VoiceTracker| std::hint::black_box(t.report()));
}

/// One Spectrum Inspector frame at each FFT size.
#[divan::bench(args = INSPECTOR_SIZES)]
fn inspector_frame(bencher: Bencher, fft_size: usize) {
    bencher
        .with_inputs(|| {
            let spectrum = PowerSpectrum::new(fft_size, WindowKind::Hann);
            let out = vec![0.0; spectrum.bins()];
            (spectrum, speech(1.0)[..fft_size].to_vec(), out)
        })
        .bench_local_refs(|(s, x, out)| s.power(x, out));
}

/// Mean live cost per 60 Hz analyzer frame: 800 samples pushed + a report every 6th frame
/// (10 Hz), over 20 s of speech-like signal, best of three passes.
fn live_cost_per_frame_us() -> f64 {
    let input = speech(20.0);
    let mut best = f64::INFINITY;
    for _ in 0..3 {
        let mut tracker = VoiceTracker::new(RATE);
        let started = Instant::now();
        let mut frames = 0usize;
        for chunk in input.chunks(TICK) {
            tracker.push(chunk);
            if frames.is_multiple_of(6) {
                std::hint::black_box(tracker.report());
            }
            frames += 1;
        }
        best = best.min(started.elapsed().as_secs_f64() * 1e6 / frames as f64);
    }
    best
}

fn inspector_cost_us(fft_size: usize) -> f64 {
    let mut s = PowerSpectrum::new(fft_size, WindowKind::Hann);
    let x = speech(1.0)[..fft_size].to_vec();
    let mut out = vec![0.0; s.bins()];
    let n = 200;
    let started = Instant::now();
    for _ in 0..n {
        s.power(&x, &mut out);
        std::hint::black_box(&out);
    }
    started.elapsed().as_secs_f64() * 1e6 / f64::from(n)
}

fn offline_realtime_factor() -> f64 {
    let input = speech(60.0);
    let config = OfflineConfig {
        sample_rate_hz: RATE,
        fft_size: 16_384,
        window: WindowKind::Hann,
    };
    let started = Instant::now();
    std::hint::black_box(analyze_buffer(&input, config, |_| true));
    60.0 / started.elapsed().as_secs_f64()
}

fn report() {
    let live = live_cost_per_frame_us();
    println!(
        "live voice diagnostics: {live:.1} µs per 60 Hz frame (budget 333 µs = 2 % of a core)"
    );
    bench_report::result(
        "vox-dsp",
        "diagnostics_live_per_frame",
        live,
        "us",
        Some(Target::le(333.0)),
    );
    for fft in INSPECTOR_SIZES {
        let us = inspector_cost_us(fft);
        println!(
            "  inspector frame, FFT {fft}: {us:.1} µs (30 Hz → {:.2} % of a core)",
            us * 30.0 / 1e4
        );
        bench_report::result(
            "vox-dsp",
            &format!("inspector_frame_fft{fft}"),
            us,
            "us",
            None,
        );
    }
    let rt = offline_realtime_factor();
    println!("long-term average job (FFT 16 384 + diagnostics): {rt:.0}x realtime");
    bench_report::result(
        "vox-dsp",
        "ltas_offline_realtime_factor",
        rt,
        "realtime_x",
        None,
    );
}

fn main() {
    report();
    divan::main();
}
