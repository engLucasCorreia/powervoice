//! T-110: spectrogram tile computation cost (SPEC-007 §4.2–§4.4): one windowed real FFT frame
//! (`Stft::accumulate_power`) at each offered FFT size (SPEC-007 §2.6: 256 ..= 16384, powers of
//! two). No PROMPT/SPEC number names tile-compute throughput directly (the engine's worker pool
//! sizing, `SpectroConfig::default_workers`, is out of this ticket's scope) — reported
//! informationally (`target: None`), assuming a COLA hop of `fft_size / 2` for the realtime
//! factor.

use std::time::Instant;

use divan::Bencher;
use vox_dsp::spectro::Stft;
use vox_testkit::bench_report;
use vox_testkit::signal;

const RATE: u32 = 48_000;
/// SPEC-007 §2.6: 256 ..= 16384. 2048 is `auto_fft_size(48_000)`.
const FFT_SIZES: [usize; 5] = [256, 1024, 2048, 4096, 16_384];

fn frame(fft_size: usize) -> Vec<f32> {
    signal::sine(1000.0, -6.0, fft_size as f64 / f64::from(RATE), RATE).expect("sine frame")
}

struct BenchState {
    stft: Stft,
    input: Vec<f32>,
    acc: Vec<f64>,
}

#[divan::bench(args = FFT_SIZES)]
fn accumulate_power(bencher: Bencher, fft_size: usize) {
    bencher
        .with_inputs(|| {
            let stft = Stft::new(fft_size);
            let acc = vec![0.0; stft.bins()];
            BenchState {
                stft,
                input: frame(fft_size),
                acc,
            }
        })
        .bench_local_refs(|s: &mut BenchState| {
            s.stft.accumulate_power(&s.input, &mut s.acc);
        });
}

/// Realtime factor assuming a COLA hop of `fft_size / 2` (SPEC-007 §4.4): audio seconds of tile
/// grid computed per wall second, best of a few passes.
fn realtime_factor(fft_size: usize) -> f64 {
    const PASSES: usize = 5;
    const FRAMES: usize = 200;
    let input = frame(fft_size);
    let hop = fft_size / 2;
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let mut stft = Stft::new(fft_size);
        let mut acc = vec![0.0; stft.bins()];
        let started = Instant::now();
        for _ in 0..FRAMES {
            stft.accumulate_power(&input, &mut acc);
            std::hint::black_box(&acc);
        }
        best = best.min(started.elapsed().as_secs_f64());
    }
    (FRAMES * hop) as f64 / f64::from(RATE) / best
}

fn report() {
    println!(
        "spectrogram Stft::accumulate_power throughput (COLA hop = fft_size / 2), realtime \
         factor (audio seconds of tile grid computed per wall second)"
    );
    for fft_size in FFT_SIZES {
        let factor = realtime_factor(fft_size);
        println!("  fft_size {fft_size}: {factor:.1}x realtime");
        bench_report::result(
            "vox-dsp",
            &format!("spectro_fft{fft_size}_realtime_factor"),
            factor,
            "realtime_x",
            None,
        );
    }
}

fn main() {
    report();
    divan::main();
}
