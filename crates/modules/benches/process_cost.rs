//! T-110: per-module `process()` CPU cost at 48 kHz for the realtime sub-block sizes (ADR-002
//! §2, §4: `MAX_BLOCK = 1024`), for every built-in module (noise gate, noise reduction,
//! parametric EQ, dynamics, true-peak limiter, gain). Run by `just bench`. T-704: Noise Reduction
//! runs with a captured noise print (the STFT path, reduction on); without one it is a delay line.
//!
//! Two passes over the same corpus:
//! - divan benches (`cargo bench`'s console table): detailed per-block-size distributions.
//! - a manual best-of-N pass (mirrors `true_peak_limiter.rs`'s own budget check), printing one
//!   `BENCH_RESULT` line per (module, block size) for `scripts/bench/summary.py`.
//!
//! No PROMPT/SPEC target names a single module's CPU cost in isolation (only the assembled rack,
//! `crates/rack/benches/full_rack.rs`, and the True-Peak Limiter's own SPEC-017 §4.5 budget,
//! already checked by `true_peak_limiter.rs`) — these are reported informationally
//! (`target: None`).

use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use divan::Bencher;
use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, OutputEvents, ProcessContext, ProcessMode, Transport,
    noise_profile,
};
use vox_modules::{Dynamics, Gain, NoiseGate, NoiseReduction, ParametricEq, TruePeakLimiter};
use vox_testkit::bench_report;
use vox_testkit::signal;

const RATE: u32 = 48_000;
const CORPUS_SECONDS: f64 = 5.0;
const PASSES: usize = 5;
const BLOCKS: [usize; 5] = [64, 128, 256, 512, 1024];

/// A "typical voice" corpus (voice-like tone bursts over pink noise), generated once.
fn corpus() -> &'static [f32] {
    static CORPUS: OnceLock<Vec<f32>> = OnceLock::new();
    CORPUS
        .get_or_init(|| {
            let voice =
                signal::voice_like(0x7051, 200.0, -6.0, 20.0, 0.2, 0.1, CORPUS_SECONDS, RATE)
                    .expect("voice_like");
            let pink = signal::pink_noise(0x7052, -30.0, CORPUS_SECONDS, RATE).expect("pink noise");
            voice
                .iter()
                .zip(&pink)
                .map(|(a, b)| a + b)
                .collect::<Vec<f32>>()
        })
        .as_slice()
}

/// T-704: a noise print of pink noise like the corpus's background. Without a print the Noise
/// Reduction module is an exact N-sample delay line (SPEC-014 §2.5): T-110's first version of this
/// bench built `NoiseReduction::new()` with no print and so measured that delay line (~300×
/// cheaper), not the STFT path with reduction on.
fn nr_print() -> &'static [u8] {
    static PRINT: OnceLock<Vec<u8>> = OnceLock::new();
    PRINT.get_or_init(|| {
        let noise = signal::pink_noise(0x7053, -30.0, 3.0, RATE).expect("pink noise");
        noise_profile(&NoiseReduction::new())
            .expect("NoiseProfile extension")
            .capture(&noise, f64::from(RATE), &[], &AtomicBool::new(false))
            .expect("capture")
    })
}

/// Noise Reduction at its defaults with [`nr_print`] loaded: the real STFT path.
fn noise_reduction_with_print() -> Box<dyn Module> {
    let mut m = NoiseReduction::new();
    m.load_state(&NoiseReduction::state_with_blob(Some(nr_print().to_vec())))
        .expect("load the NR state");
    Box::new(m)
}

/// Fails the bench if the NR rows would measure the delay line again.
fn assert_nr_measures_the_stft_path() {
    let mut m = NoiseReduction::new();
    m.load_state(&NoiseReduction::state_with_blob(Some(nr_print().to_vec())))
        .expect("load the NR state");
    m.activate(&activate_config(256)).expect("activate");
    assert!(
        m.has_active_print(),
        "the NR bench must run the STFT path (print loaded), not the no-print delay line"
    );
}

fn activate_config(block: usize) -> ActivateConfig {
    ActivateConfig {
        sample_rate: f64::from(RATE),
        max_block: block as u32,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

/// Per-iteration state for a divan bench: one activated module instance plus its scratch
/// buffers, over one block-sized window of [`corpus`].
struct BenchState {
    module: Box<dyn Module>,
    input: &'static [f32],
    output: Vec<f32>,
    out_events: OutputEvents,
}

fn setup(block: usize, make: fn() -> Box<dyn Module>) -> BenchState {
    let mut module = make();
    module.activate(&activate_config(block)).expect("activate");
    BenchState {
        module,
        input: &corpus()[..block],
        output: vec![0.0f32; block],
        out_events: OutputEvents::with_capacity(8),
    }
}

fn run_one_block(s: &mut BenchState) {
    s.out_events.clear();
    let mut ctx = ProcessContext::new(
        s.input.len() as u32,
        0,
        Transport {
            playing: true,
            position_samples: Some(0),
        },
        &[],
        &mut s.out_events,
    );
    let mut outs = [s.output.as_mut_slice()];
    s.module.process(&mut ctx, &[s.input], &mut outs);
}

fn bench_module(bencher: Bencher, block: usize, make: fn() -> Box<dyn Module>) {
    bencher
        .with_inputs(move || setup(block, make))
        .bench_local_refs(run_one_block);
}

#[divan::bench(args = BLOCKS)]
fn noise_gate(bencher: Bencher, block: usize) {
    bench_module(bencher, block, || Box::new(NoiseGate::new()));
}

#[divan::bench(args = BLOCKS)]
fn noise_reduction(bencher: Bencher, block: usize) {
    bench_module(bencher, block, noise_reduction_with_print);
}

#[divan::bench(args = BLOCKS)]
fn parametric_eq(bencher: Bencher, block: usize) {
    bench_module(bencher, block, || Box::new(ParametricEq::new()));
}

#[divan::bench(args = BLOCKS)]
fn dynamics(bencher: Bencher, block: usize) {
    bench_module(bencher, block, || Box::new(Dynamics::new()));
}

#[divan::bench(args = BLOCKS)]
fn true_peak_limiter(bencher: Bencher, block: usize) {
    bench_module(bencher, block, || Box::new(TruePeakLimiter::new()));
}

#[divan::bench(args = BLOCKS)]
fn gain(bencher: Bencher, block: usize) {
    bench_module(bencher, block, || Box::new(Gain::new()));
}

/// Best-of-[`PASSES`] percent of one real-time core: processing time of the whole corpus in
/// `block`-sized blocks, divided by the corpus's real-time duration.
fn percent_of_core(block: usize, make: fn() -> Box<dyn Module>) -> f64 {
    let input = corpus();
    let mut out = vec![0.0f32; block];
    let mut out_events = OutputEvents::with_capacity(8);
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let mut m = make();
        m.activate(&activate_config(block)).expect("activate");
        let started = Instant::now();
        for (i, chunk) in input.chunks(block).enumerate() {
            let pos = (i * block) as u64;
            out_events.clear();
            let mut ctx = ProcessContext::new(
                chunk.len() as u32,
                pos,
                Transport {
                    playing: true,
                    position_samples: Some(pos),
                },
                &[],
                &mut out_events,
            );
            m.process(&mut ctx, &[chunk], &mut [&mut out[..chunk.len()]]);
            std::hint::black_box(&out);
        }
        best = best.min(started.elapsed().as_secs_f64());
    }
    100.0 * best / (input.len() as f64 / f64::from(RATE))
}

type MakeModule = fn() -> Box<dyn Module>;

fn report() {
    assert_nr_measures_the_stft_path();
    println!(
        "per-module process() cost, {RATE} Hz, best of {PASSES} passes over {CORPUS_SECONDS} s \
         of voice-like + pink noise"
    );
    let modules: [(&str, MakeModule); 6] = [
        ("noise_gate", || Box::new(NoiseGate::new())),
        ("noise_reduction", noise_reduction_with_print),
        ("parametric_eq", || Box::new(ParametricEq::new())),
        ("dynamics", || Box::new(Dynamics::new())),
        ("true_peak_limiter", || Box::new(TruePeakLimiter::new())),
        ("gain", || Box::new(Gain::new())),
    ];
    for (name, make) in modules {
        for block in BLOCKS {
            let percent = percent_of_core(block, make);
            println!("  {name}, {block}-frame blocks: {percent:.4} % of one core");
            bench_report::result(
                "vox-modules",
                &format!("{name}_process_{block}f_pct_core"),
                percent,
                "pct_core",
                None,
            );
        }
    }
}

fn main() {
    report();
    divan::main();
}
