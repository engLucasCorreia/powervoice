//! T-704: SPEC-003 AC-1 / PROMPT §2 "playback start < 50 ms" as `BENCH_RESULT` lines (`just
//! bench`). Drives the real engine (`ManualEngine`: the same transport and output-callback path the
//! app runs) on the `FakeBackend`'s simulated clock — the harness of `tests/playback.rs`'s
//! `start_latency_is_under_50_ms` — with the output stream already open and idle (emitting
//! silence), then measures Play → the first non-silent frame:
//! - `written`: the first [`TICK_NS`] tick after which that frame has been written into the device
//!   buffer (SPEC-003 AC-1's definition; the tick makes it an upper bound by ≤ 0.1 ms), target
//!   < 50 ms;
//! - `heard`: the same frame leaving the device (adds the device's own output latency, one period
//!   here), informational.
//!
//! Worst of [`RUNS`] runs, each with Play landing at a different phase of the callback period,
//! for 64/256/1024-frame device buffers (SPEC-001's range), with an empty rack and with the
//! typical voice rack (gate → NR → EQ → dynamics → limiter at their defaults). The rack's own
//! latency is not pre-rolled: the first audible frame comes out after the rack latency (NR N = 2048
//! alone is 42.7 ms, SPEC-014 §2.7; the limiter adds 257 samples), so the voice-rack rows are the
//! engine's start time plus ~48 ms (T-704 finding: at the edge of the 50 ms budget).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{Direction, EngineConfig, HostId, ManualEngine, PlaybackDoc, TransportCommand};
use vox_module_api::{Module, ModuleRef, ModuleState, Version};
use vox_modules::{Dynamics, NoiseGate, NoiseReduction, ParametricEq, TruePeakLimiter};
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, StoreOptions};
use vox_rack::{RackModel, Registry, SlotModel};
use vox_testkit::bench_report::{self, Target};

const RATE: u32 = 48_000;
const MS: u64 = 1_000_000;
/// Measurement resolution after Play.
const TICK_NS: u64 = 100_000;
/// How long after Play to look for the first audible frame.
const WATCH_MS: u64 = 120;
const RUNS: u64 = 8;
const BUFFERS: [u32; 3] = [64, 256, 1024];
const BUDGET_MS: f64 = 50.0;

/// A scratch directory removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "vox-bench-playback-start-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create scratch dir");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn slot(id: &str, version: Version, module: &dyn Module) -> SlotModel {
    SlotModel::new(
        &ModuleRef {
            id: id.into(),
            version,
        },
        false,
        &ModuleState {
            format_version: module.descriptor().state_format_version,
            params: BTreeMap::new(),
            blob: None,
        },
    )
}

/// Gate → NR → EQ → dynamics → limiter at their defaults (NR without a print keeps its full
/// N-sample latency, SPEC-014 §2.1).
fn voice_rack() -> RackModel {
    RackModel {
        slots: vec![
            slot(NoiseGate::ID, NoiseGate::VERSION, &NoiseGate::new()),
            slot(
                NoiseReduction::ID,
                NoiseReduction::VERSION,
                &NoiseReduction::new(),
            ),
            slot(
                ParametricEq::ID,
                ParametricEq::VERSION,
                &ParametricEq::new(),
            ),
            slot(Dynamics::ID, Dynamics::VERSION, &Dynamics::new()),
            slot(
                TruePeakLimiter::ID,
                TruePeakLimiter::VERSION,
                &TruePeakLimiter::new(),
            ),
        ],
    }
}

struct Measured {
    written_ms: f64,
    heard_ms: f64,
}

fn recorded(fake: &FakeBackend) -> Option<vox_engine::backend::fake::RecordedOutput> {
    let id = fake
        .streams()
        .iter()
        .rev()
        .find(|s| s.info.direction == Direction::Output)
        .map(|s| s.info.id)?;
    fake.recorded_output(id)
}

/// One run: Play lands `phase_us` µs into a 1 ms tick, after 100 ms of idle output.
fn measure(buffer: u32, rack: &RackModel, phase_us: u64) -> Measured {
    let latency_ns = u64::from(buffer) * 1_000_000_000 / u64::from(RATE);
    let fake = FakeBackend::new(42);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(
            FakeDirection::new(2, &[RATE], RATE)
                .default_buffer(buffer)
                .latency_ns(latency_ns)
                .record_output(),
        ),
    );
    let dir = Scratch::new();
    let store = ChunkStore::create(&dir.0, 0, StoreOptions::with_memory_budget(64 << 20))
        .expect("create store");
    let mut writer = ChunkWriter::new(store.clone());
    writer
        .append(&vec![0.5f32; 2 * RATE as usize])
        .expect("append");
    let audio = writer.finish().expect("finish");
    let snapshot = Arc::new(DocSnapshot::new(RATE, audio.pieces, Vec::new()));
    let registry =
        Arc::new(Registry::with_factories(vox_modules::builtin_factories()).expect("registry"));
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    cfg.rack = rack.clone();
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    eng.set_document(Some(PlaybackDoc { store, snapshot }));
    eng.poll_devices();
    // The output stream opens here; its frame f is heard at open + latency + f / rate (the
    // `tests/playback.rs` start-latency formula). `playback_ns` stamps use the stream's own clock.
    let open_ns = fake.now_ns();

    for _ in 0..100 {
        fake.advance_by(MS);
        eng.tick();
    }
    fake.advance_by(phase_us * 1_000);
    eng.tick();
    assert!(
        recorded(&fake).is_some_and(|out| out.samples.iter().all(|&x| x == 0.0)),
        "the idle stream emits silence"
    );

    let t0 = fake.now_ns();
    assert!(eng.transport(TransportCommand::Play).playing);
    let mut written_ns = None;
    for _ in 0..WATCH_MS * MS / TICK_NS {
        fake.advance_by(TICK_NS);
        eng.tick();
        if recorded(&fake).is_some_and(|out| out.samples.iter().any(|&x| x != 0.0)) {
            written_ns = Some(fake.now_ns());
            break;
        }
    }
    let out = recorded(&fake).expect("recorded output");
    let first = out
        .samples
        .iter()
        .position(|&x| x != 0.0)
        .expect("audible output") as u64;
    let heard_ns = open_ns + latency_ns + first * 1_000_000_000 / u64::from(RATE);
    Measured {
        written_ms: (written_ns.expect("audible output") - t0) as f64 / 1e6,
        heard_ms: heard_ns.saturating_sub(t0) as f64 / 1e6,
    }
}

fn main() {
    println!(
        "SPEC-003 AC-1 playback start (FakeBackend clock, {RATE} Hz, worst of {RUNS} Play phases)"
    );
    let racks = [
        ("empty_rack", RackModel { slots: Vec::new() }),
        ("voice_rack", voice_rack()),
    ];
    for (rack_name, rack) in &racks {
        for buffer in BUFFERS {
            let mut written = 0.0f64;
            let mut heard = 0.0f64;
            for run in 0..RUNS {
                let m = measure(buffer, rack, run * 1_000 / RUNS);
                written = written.max(m.written_ms);
                heard = heard.max(m.heard_ms);
            }
            println!(
                "  {rack_name}, {buffer}-frame buffer: written {written:.2} ms, heard {heard:.2} ms \
                 (budget {BUDGET_MS} ms to written)"
            );
            bench_report::result(
                "vox-engine",
                &format!("playback_start_{rack_name}_{buffer}f_written_max_ms"),
                written,
                "ms",
                Some(Target::le(BUDGET_MS)),
            );
            bench_report::result(
                "vox-engine",
                &format!("playback_start_{rack_name}_{buffer}f_heard_max_ms"),
                heard,
                "ms",
                None,
            );
        }
    }
}
