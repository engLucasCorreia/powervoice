//! T-110: the callback-time histogram (ADR-002 §2 follow-up). Drives the **real** output
//! callback path (`FakeBackend` + `ManualEngine`, the same machinery `crates/engine/tests/
//! playback.rs` uses) with a typical voice rack for N seconds, recording each callback's real
//! wall time into a preallocated `vox_engine::histogram::Histogram` via
//! `FakeBackend::set_rt_guard` (no allocation in the callback — wrapped in `no_alloc`, same as
//! every other RT integration test), then reports p50/p95/p99/max against the block deadline
//! (`block frames / 48 000 Hz`).
//!
//! Plain `main`, not divan (a fixed-length simulated run reported as percentiles, like
//! `crates/sandbox/benches/clap_round_trip.rs` and `crates/sandbox-ipc/benches/round_trip.rs`).
//! Exposed as `just bench-callback`. `VOX_ENGINE_BENCH_SECONDS` overrides the 3 s per row
//! (default).

vox_module_api::install_test_allocator!();

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::histogram::Histogram;
use vox_engine::{EngineConfig, HostId, ManualEngine, PlaybackDoc, TransportCommand};
use vox_module_api::test_util::no_alloc;
use vox_module_api::{Module, ModuleRef, Version};
use vox_modules::{Dynamics, Gain, NoiseGate, NoiseReduction, ParametricEq, TruePeakLimiter};
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, StoreOptions};
use vox_rack::{RackModel, Registry, SlotModel};
use vox_testkit::bench_report::{self, Target};
use vox_testkit::signal;

const RATE: u32 = 48_000;
const MS_NS: u64 = 1_000_000;
/// Realtime sub-block sizes (ADR-002 §2, §4), the callback sizes exercised here.
const BLOCKS: [u32; 4] = [128, 256, 512, 1024];

fn run_seconds() -> f64 {
    std::env::var("VOX_ENGINE_BENCH_SECONDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0)
}

/// One slot at its module's default parameter values.
fn default_slot(id: &str, version: Version, m: Box<dyn Module>) -> SlotModel {
    let state = m.save_state().expect("save_state");
    SlotModel::new(
        &ModuleRef {
            id: id.into(),
            version,
        },
        false,
        &state,
    )
}

/// The same 6-module voice rack as `crates/rack/benches/full_rack.rs`.
fn typical_rack() -> RackModel {
    RackModel {
        slots: vec![
            default_slot(Gain::ID, Gain::VERSION, Box::new(Gain::new())),
            default_slot(
                NoiseGate::ID,
                NoiseGate::VERSION,
                Box::new(NoiseGate::new()),
            ),
            default_slot(
                NoiseReduction::ID,
                NoiseReduction::VERSION,
                Box::new(NoiseReduction::new()),
            ),
            default_slot(
                ParametricEq::ID,
                ParametricEq::VERSION,
                Box::new(ParametricEq::new()),
            ),
            default_slot(Dynamics::ID, Dynamics::VERSION, Box::new(Dynamics::new())),
            default_slot(
                TruePeakLimiter::ID,
                TruePeakLimiter::VERSION,
                Box::new(TruePeakLimiter::new()),
            ),
        ],
    }
}

struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "vox-bench-callback-histogram-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch dir");
        Self(path)
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A snapshot of [`Histogram`] readings (plain data: the histogram itself stays owned by the
/// `FakeBackend`'s RT guard closure for as long as any clone of the fake backend is alive, so
/// [`run`] reads it rather than trying to reclaim it).
struct Summary {
    count: u64,
    p50_us: f64,
    p95_us: f64,
    p99_us: f64,
    max_us: f64,
}

fn summarize(h: &Histogram) -> Summary {
    let us = |d: Option<Duration>| d.map_or(f64::NAN, |d| d.as_secs_f64() * 1_000_000.0);
    Summary {
        count: h.count(),
        p50_us: us(h.percentile(0.50)),
        p95_us: us(h.percentile(0.95)),
        p99_us: us(h.percentile(0.99)),
        max_us: us(h.max()),
    }
}

/// Runs `seconds` of simulated playback at a fixed `block`-frame callback size through the
/// typical rack, and returns a summary of real wall time per output callback.
fn run(block: u32, seconds: f64) -> Summary {
    assert!(
        vox_module_api::test_util::alloc_checks_active(),
        "the allocation checker must be installed"
    );
    let scratch = ScratchDir::new(&block.to_string());

    let fake = FakeBackend::new(0xC0FFEE);
    let dev = FakeDirection::new(2, &[RATE], RATE).default_buffer(block);
    fake.plug(HostId::Alsa, FakeDevice::new("DAC").with_output(dev));

    let histogram = Arc::new(Histogram::new());
    let violations = Arc::new(AtomicU64::new(0));
    let h = histogram.clone();
    let v = violations.clone();
    fake.set_rt_guard(move |f| {
        let started = Instant::now();
        let result = no_alloc(f);
        h.record(started.elapsed());
        match result {
            Ok(()) => 0,
            Err(n) => {
                v.fetch_add(u64::from(n), Ordering::Relaxed);
                n
            }
        }
    });

    // Enough document for the run plus headroom (never reaches end-of-document mid-run).
    let doc_seconds = seconds + 2.0;
    let audio = signal::voice_like(0xC1, 200.0, -6.0, 20.0, 0.2, 0.1, doc_seconds, RATE)
        .expect("voice_like");
    let store = ChunkStore::create(&scratch.0, 0, StoreOptions::default()).expect("create store");
    let mut writer = ChunkWriter::new(store.clone());
    writer.append(&audio).expect("append");
    let written = writer.finish().expect("finish");
    let snapshot = Arc::new(DocSnapshot::new(RATE, written.pieces, Vec::new()));

    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.rack = typical_rack();
    let clock_fake = fake.clone();
    cfg.clock = Arc::new(move || clock_fake.now_ns());

    let mut eng = ManualEngine::new(cfg);
    eng.set_document(Some(PlaybackDoc { store, snapshot }));
    eng.poll_devices();

    // Give the output stream a moment to open before starting playback (as Rig::run_ms does).
    for _ in 0..50 {
        fake.advance_by(MS_NS);
        eng.tick();
    }
    eng.transport(TransportCommand::Play);

    let steps = (seconds * 1000.0).round() as u64;
    for _ in 0..steps {
        fake.advance_by(MS_NS);
        eng.tick();
    }

    assert_eq!(
        violations.load(Ordering::Relaxed),
        0,
        "the output callback allocated or deallocated"
    );
    summarize(&histogram)
}

fn report() {
    let seconds = run_seconds();
    println!(
        "output callback wall time, {RATE} Hz, typical 6-module voice rack, {seconds} s per row \
         (real callback path: FakeBackend + ManualEngine, ADR-002 §2 follow-up)"
    );
    println!("| block | deadline µs | callbacks | p50 µs | p95 µs | p99 µs | max µs |");
    println!("|---|---|---|---|---|---|---|");
    for block in BLOCKS {
        let deadline = Duration::from_secs_f64(f64::from(block) / f64::from(RATE));
        let deadline_us = deadline.as_secs_f64() * 1_000_000.0;
        let s = run(block, seconds);
        println!(
            "| {block} | {deadline_us:.1} | {} | {:.1} | {:.1} | {:.1} | {:.1} |",
            s.count, s.p50_us, s.p95_us, s.p99_us, s.max_us
        );
        bench_report::result(
            "vox-engine",
            &format!("callback_{block}f_p99_us"),
            s.p99_us,
            "us",
            Some(Target::le(deadline_us)),
        );
        bench_report::result(
            "vox-engine",
            &format!("callback_{block}f_max_us"),
            s.max_us,
            "us",
            Some(Target::le(deadline_us)),
        );
    }
}

fn main() {
    report();
}
