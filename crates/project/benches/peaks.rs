//! T-110: peak-pyramid generation and overview query throughput (ADR-004 §5).
//! - `ChunkPeaks::compute`: builds one chunk's 6-level min/max pyramid (import/record path).
//! - `peaks_query::peaks`: the waveform view's overview query, at each pyramid level.
//!
//! No PROMPT/SPEC number names either throughput directly — reported informationally
//! (`target: None`), but pyramid generation bears on "open a 60-min WAV in < 3 s" (PROMPT §2)
//! and overview queries on "60 fps scroll/zoom".

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Instant;

use divan::Bencher;
use vox_project::store::ChunkPeaks;
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, StoreOptions, peaks};
use vox_testkit::bench_report;
use vox_testkit::signal;

const RATE: u32 = 48_000;
/// A full chunk (ADR-004 `CHUNK_SAMPLES = 65_536`).
const CHUNK_SAMPLES: usize = 65_536;
/// ADR-004 §5 pyramid levels.
const SPP_LEVELS: [u32; 6] = [64, 256, 1024, 4096, 16_384, 65_536];
/// A generous document for the overview-query benches.
const DOC_SECONDS: f64 = 120.0;

fn chunk_of_voice() -> &'static [f32] {
    static C: OnceLock<Vec<f32>> = OnceLock::new();
    C.get_or_init(|| {
        let s = signal::voice_like(
            0xD051,
            200.0,
            -6.0,
            20.0,
            0.2,
            0.1,
            CHUNK_SAMPLES as f64 / f64::from(RATE),
            RATE,
        )
        .expect("voice_like");
        s[..CHUNK_SAMPLES.min(s.len())].to_vec()
    })
    .as_slice()
}

#[divan::bench]
fn compute(bencher: Bencher) {
    bencher.bench(|| ChunkPeaks::compute(divan::black_box(chunk_of_voice())));
}

/// Seconds of chunk audio pyramid-computed per second of wall time, best of a few passes.
fn compute_realtime_factor() -> f64 {
    const PASSES: usize = 5;
    let chunk = chunk_of_voice();
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let started = Instant::now();
        let p = ChunkPeaks::compute(chunk);
        std::hint::black_box(&p);
        best = best.min(started.elapsed().as_secs_f64());
    }
    (chunk.len() as f64 / f64::from(RATE)) / best
}

/// A scratch directory removed on drop, even on panic (best-effort).
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("vox-bench-peaks-{tag}-{}", std::process::id()));
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

struct Fixture {
    store: Arc<ChunkStore>,
    snapshot: Arc<DocSnapshot>,
    _scratch: ScratchDir,
}

fn fixture(tag: &str) -> Fixture {
    let scratch = ScratchDir::new(tag);
    let store = ChunkStore::create(&scratch.0, 0, StoreOptions::default()).expect("create store");
    let mut writer = ChunkWriter::new(store.clone());
    let audio = signal::voice_like(0xD052, 200.0, -6.0, 20.0, 0.2, 0.1, DOC_SECONDS, RATE)
        .expect("voice_like");
    writer.append(&audio).expect("append");
    let written = writer.finish().expect("finish");
    // Pre-warm the pyramid cache (matches steady-state waveform-view usage).
    for &id in &written.chunks {
        store.chunk_peaks(id).expect("chunk_peaks");
    }
    let snapshot = Arc::new(DocSnapshot::new(RATE, written.pieces, Vec::new()));
    Fixture {
        store,
        snapshot,
        _scratch: scratch,
    }
}

#[divan::bench(args = SPP_LEVELS)]
fn overview_query(bencher: Bencher, spp: u32) {
    let f = fixture("divan");
    bencher.bench_local(|| {
        peaks(&f.store, &f.snapshot, spp, 0, 1000).expect("peaks");
    });
}

/// Queries of 1000 buckets served per second of wall time, best of a few passes.
fn overview_realtime_factor(spp: u32) -> f64 {
    const PASSES: usize = 5;
    const QUERIES: usize = 50;
    let f = fixture(&format!("report-{spp}"));
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let started = Instant::now();
        for _ in 0..QUERIES {
            let out = peaks(&f.store, &f.snapshot, spp, 0, 1000).expect("peaks");
            std::hint::black_box(&out);
        }
        best = best.min(started.elapsed().as_secs_f64());
    }
    QUERIES as f64 / best
}

fn report() {
    let factor = compute_realtime_factor();
    println!(
        "ChunkPeaks::compute (one {CHUNK_SAMPLES}-sample chunk), realtime factor: \
         {factor:.1}x realtime"
    );
    bench_report::result(
        "vox-project",
        "peaks_compute_realtime_factor",
        factor,
        "realtime_x",
        None,
    );

    println!("peaks() overview query (1000 buckets, {DOC_SECONDS} s document), queries/s");
    for spp in SPP_LEVELS {
        let qps = overview_realtime_factor(spp);
        println!("  spp {spp}: {qps:.0} queries/s");
        bench_report::result(
            "vox-project",
            &format!("peaks_overview_spp{spp}_queries_per_s"),
            qps,
            "queries_per_s",
            None,
        );
    }
}

fn main() {
    report();
    divan::main();
}
