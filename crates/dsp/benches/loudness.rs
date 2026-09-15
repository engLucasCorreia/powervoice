//! T-110: LUFS / true-peak measurement throughput (`LoudnessMeter::push`, S4-01, BS.1770/EBU
//! R128), pushed in `OFFLINE_BLOCK`-sized chunks (ADR-002 §2: the analysis job's own block
//! size). No PROMPT/SPEC number names measurement throughput directly (the loudness analysis
//! job's own responsiveness is out of this ticket's scope) — reported informationally
//! (`target: None`), but it directly bears on "open a 60-min WAV in < 3 s" (PROMPT §2) since the
//! import scan measures loudness once.

use std::sync::OnceLock;
use std::time::Instant;

use divan::Bencher;
use vox_dsp::loudness::LoudnessMeter;
use vox_testkit::bench_report;
use vox_testkit::signal;

const RATE: u32 = 48_000;
const CORPUS_SECONDS: f64 = 30.0;
/// ADR-002 §2 `OFFLINE_BLOCK`.
const PUSH_BLOCKS: [usize; 3] = [1024, 4096, 16_384];

fn corpus() -> &'static [f32] {
    static C: OnceLock<Vec<f32>> = OnceLock::new();
    C.get_or_init(|| {
        let voice = signal::voice_like(0xA051, 200.0, -6.0, 20.0, 0.2, 0.1, CORPUS_SECONDS, RATE)
            .expect("voice_like");
        let pink = signal::pink_noise(0xA052, -30.0, CORPUS_SECONDS, RATE).expect("pink noise");
        voice
            .iter()
            .zip(&pink)
            .map(|(a, b)| a + b)
            .collect::<Vec<f32>>()
    })
    .as_slice()
}

#[divan::bench(args = PUSH_BLOCKS)]
fn push(bencher: Bencher, block: usize) {
    bencher
        .with_inputs(|| LoudnessMeter::new(RATE).expect("meter"))
        .bench_local_refs(|meter: &mut LoudnessMeter| {
            for chunk in corpus().chunks(block) {
                meter.push(chunk).expect("push");
            }
        });
}

/// Seconds of audio measured per second of wall time, best of a few passes.
fn realtime_factor(block: usize) -> f64 {
    const PASSES: usize = 3;
    let input = corpus();
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let mut meter = LoudnessMeter::new(RATE).expect("meter");
        let started = Instant::now();
        for chunk in input.chunks(block) {
            meter.push(chunk).expect("push");
        }
        std::hint::black_box(meter.finish().expect("finish"));
        best = best.min(started.elapsed().as_secs_f64());
    }
    (input.len() as f64 / f64::from(RATE)) / best
}

fn report() {
    println!(
        "LoudnessMeter (LUFS + true peak) throughput, {CORPUS_SECONDS} s of voice-like + pink \
         noise, realtime factor (audio seconds measured per wall second)"
    );
    for block in PUSH_BLOCKS {
        let factor = realtime_factor(block);
        println!("  push({block}) blocks: {factor:.1}x realtime");
        bench_report::result(
            "vox-dsp",
            &format!("loudness_push_{block}f_realtime_factor"),
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
