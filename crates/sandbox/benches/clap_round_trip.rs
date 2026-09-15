//! T-110: sandbox IPC round trip per block through the real CLAP backend (ADR-008, T-803), using
//! the in-repo test CLAP plugin (`vox-test-clap`) hosted by a real `powervoice-sandbox` process.
//!
//! Complements `crates/sandbox-ipc/benches/round_trip.rs` (the raw IPC channel against the
//! built-in "test" backend): this measures the full `Module::process()` round trip through
//! `vox_plugin_host::SandboxFactory`, the path the live rack actually uses (a CLAP effect is a
//! `ProxyModule` like any other slot) — under the `no_alloc` check, since an installed plugin's
//! `process()` runs on the audio thread and must obey the same RT contract as a built-in module.
//!
//! Plain `main`, not divan (matches `sandbox-ipc/benches/round_trip.rs`'s own convention): one
//! sandbox process per block size, percentile latency over many blocks, `VOX_SBX_BENCH_SECONDS`
//! overrides the per-row duration (default 2 s — this is one real child process per row, unlike
//! the in-process benches elsewhere in this ticket).

vox_module_api::install_test_allocator!();

#[path = "../tests/common/mod.rs"]
mod common;

use std::time::{Duration, Instant};

use common::*;
use vox_module_api::test_util::no_alloc;
use vox_module_api::{
    ActivateConfig, ChannelLayout, ModuleFactory, OutputEvents, ProcessContext, ProcessMode,
    Transport,
};
use vox_test_clap as tc;
use vox_testkit::bench_report::{self, Target};

const WARMUP_BLOCKS: usize = 50;
const BLOCKS: [usize; 4] = [64, 128, 256, 512];

fn seconds() -> f64 {
    std::env::var("VOX_SBX_BENCH_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2.0)
}

fn config(block: usize) -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block: block as u32,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

fn run(block: usize, seconds: f64) {
    let factory = clap_factory(tc::ID_GAIN, default_options());
    let mut m = factory.create().expect("create CLAP gain proxy");
    m.activate(&config(block)).expect("activate");

    let input = sine(block, 220.0, 0.25);
    let mut output = vec![0.0f32; block];
    let mut out_events = OutputEvents::with_capacity(8);
    let mut process_one = |steady: u64| {
        out_events.clear();
        let mut ctx = ProcessContext::new(
            block as u32,
            steady,
            Transport {
                playing: true,
                position_samples: Some(steady),
            },
            &[],
            &mut out_events,
        );
        let mut outs = [output.as_mut_slice()];
        m.process(&mut ctx, &[input.as_slice()], &mut outs);
    };

    for i in 0..WARMUP_BLOCKS {
        no_alloc(|| process_one(i as u64)).expect("process allocated (warmup)");
    }

    let block_deadline_us = 1_000_000.0 * block as f64 / RATE;
    let mut durations_us: Vec<f64> = Vec::new();
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    let mut steady = WARMUP_BLOCKS as u64;
    while Instant::now() < deadline {
        let started = Instant::now();
        no_alloc(|| process_one(steady)).expect("process allocated");
        durations_us.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        steady += 1;
    }
    durations_us.sort_by(|a, b| a.total_cmp(b));
    let n = durations_us.len();
    let pct = |p: f64| durations_us[((n as f64 - 1.0) * p).round() as usize];
    let (p50, p95, p99, max) = (pct(0.50), pct(0.95), pct(0.99), durations_us[n - 1]);
    println!(
        "| {block} | {block_deadline_us:.1} | {n} | {p50:.1} | {p95:.1} | {p99:.1} | {max:.1} |"
    );
    bench_report::result(
        "powervoice-sandbox",
        &format!("clap_round_trip_{block}f_p99_us"),
        p99,
        "us",
        Some(Target::le(block_deadline_us)),
    );
}

fn main() {
    let seconds = seconds();
    println!(
        "sandbox IPC round trip, 48 kHz, CLAP test gain plugin in a real powervoice-sandbox \
         process, {seconds} s per row"
    );
    println!("| block | deadline µs | blocks | p50 µs | p95 µs | p99 µs | max µs |");
    println!("|---|---|---|---|---|---|---|");
    for block in BLOCKS {
        run(block, seconds);
    }
}
