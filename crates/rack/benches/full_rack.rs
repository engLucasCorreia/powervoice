//! T-110: a typical voice-over rack's `process()` cost at 48 kHz, for the realtime sub-block
//! sizes (ADR-002 §2, §4: `MAX_BLOCK = 1024`). Chain: gain (input trim) → noise gate → noise
//! reduction → parametric EQ → dynamics → true-peak limiter (output ceiling) — a representative
//! order, not a spec requirement.
//!
//! PROMPT §2 performance target: "full rack at 48 kHz < 20% of one core on a mid-range laptop".
//! This machine is whatever `just bench` runs on, not necessarily "mid-range" — the target check
//! is directional, reported via `BENCH_RESULT` (`scripts/bench/summary.py`).

use std::time::Instant;

use divan::Bencher;
use vox_dsp::fp::DenormalGuard;
use vox_module_api::{ActivateConfig, ChannelLayout, Module, ProcessMode, Transport};
use vox_modules::{Dynamics, Gain, NoiseGate, NoiseReduction, ParametricEq, TruePeakLimiter};
use vox_rack::Chain;
use vox_testkit::bench_report::{self, Target};
use vox_testkit::signal;

const RATE: u32 = 48_000;
const CORPUS_SECONDS: f64 = 5.0;
const PASSES: usize = 5;
const BLOCKS: [usize; 5] = [64, 128, 256, 512, 1024];
/// PROMPT §2: "full rack at 48 kHz < 20% of one core on a mid-range laptop".
const FULL_RACK_BUDGET_PCT_CORE: f64 = 20.0;

fn corpus() -> &'static [f32] {
    use std::sync::OnceLock;
    static CORPUS: OnceLock<Vec<f32>> = OnceLock::new();
    CORPUS
        .get_or_init(|| {
            let voice =
                signal::voice_like(0x8051, 200.0, -6.0, 20.0, 0.2, 0.1, CORPUS_SECONDS, RATE)
                    .expect("voice_like");
            let pink = signal::pink_noise(0x8052, -30.0, CORPUS_SECONDS, RATE).expect("pink noise");
            voice
                .iter()
                .zip(&pink)
                .map(|(a, b)| a + b)
                .collect::<Vec<f32>>()
        })
        .as_slice()
}

fn typical_rack(block: usize) -> Chain {
    let mut chain = Chain::new();
    let modules: [Box<dyn Module>; 6] = [
        Box::new(Gain::new()),
        Box::new(NoiseGate::new()),
        Box::new(NoiseReduction::new()),
        Box::new(ParametricEq::new()),
        Box::new(Dynamics::new()),
        Box::new(TruePeakLimiter::new()),
    ];
    for m in modules {
        chain.push(m).expect("push module");
    }
    chain
        .activate(&ActivateConfig {
            sample_rate: f64::from(RATE),
            max_block: block as u32,
            mode: ProcessMode::Realtime,
            layout: ChannelLayout::MONO,
        })
        .expect("activate chain");
    chain
}

struct BenchState {
    chain: Chain,
    input: &'static [f32],
    output: Vec<f32>,
}

#[divan::bench(args = BLOCKS)]
fn full_rack(bencher: Bencher, block: usize) {
    bencher
        .with_inputs(move || BenchState {
            chain: typical_rack(block),
            input: &corpus()[..block],
            output: vec![0.0f32; block],
        })
        .bench_local_refs(|s: &mut BenchState| {
            let _fp = DenormalGuard::new();
            s.chain.process(
                Transport {
                    playing: true,
                    position_samples: Some(0),
                },
                s.input,
                &mut s.output,
            );
        });
}

/// Best-of-[`PASSES`] percent of one real-time core for the whole corpus, `block`-sized blocks.
fn percent_of_core(block: usize) -> f64 {
    let input = corpus();
    let mut out = vec![0.0f32; block];
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let mut chain = typical_rack(block);
        let _fp = DenormalGuard::new();
        let started = Instant::now();
        for (i, in_chunk) in input.chunks(block).enumerate() {
            let pos = (i * block) as u64;
            chain.process(
                Transport {
                    playing: true,
                    position_samples: Some(pos),
                },
                in_chunk,
                &mut out[..in_chunk.len()],
            );
            std::hint::black_box(&out);
        }
        best = best.min(started.elapsed().as_secs_f64());
    }
    100.0 * best / (input.len() as f64 / f64::from(RATE))
}

fn report() {
    println!(
        "full voice rack (gain, noise gate, noise reduction, parametric EQ, dynamics, \
         true-peak limiter) process() cost, {RATE} Hz, best of {PASSES} passes over \
         {CORPUS_SECONDS} s of voice-like + pink noise, budget {FULL_RACK_BUDGET_PCT_CORE} % \
         (PROMPT §2, \"mid-range laptop\" — directional on this machine)"
    );
    for block in BLOCKS {
        let percent = percent_of_core(block);
        println!("  {block}-frame blocks: {percent:.4} % of one core");
        bench_report::result(
            "vox-rack",
            &format!("full_rack_process_{block}f_pct_core"),
            percent,
            "pct_core",
            Some(Target::le(FULL_RACK_BUDGET_PCT_CORE)),
        );
    }
}

fn main() {
    report();
    divan::main();
}
