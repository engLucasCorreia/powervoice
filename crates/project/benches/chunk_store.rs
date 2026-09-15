//! T-110: chunk-store read/prefetch throughput (ADR-004 §2: 65 536-sample chunks; the CLAUDE.md
//! rule "the audio thread never reads the memory-mapped chunk store — a reader thread prefetches
//! into a ring"). Benchmarks [`SnapshotReader::read`], the real playback-reader path (ADR-002
//! §5), at read sizes around the ~200 ms `PREFETCH` top-up (SPEC-000 §3). No PROMPT/SPEC number
//! names read throughput directly — reported informationally (`target: None`), but it bears on
//! "playback start < 50 ms" and "open a 60-min WAV in < 3 s" (PROMPT §2).

use std::cell::Cell;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use divan::Bencher;
use vox_project::{ChunkStore, ChunkWriter, DocSnapshot, SnapshotReader, StoreOptions};
use vox_testkit::bench_report;
use vox_testkit::signal;

const RATE: u32 = 48_000;
/// A generous document: well past several chunks (ADR-004 `CHUNK_SAMPLES = 65_536`).
const CORPUS_SECONDS: f64 = 120.0;
/// ~200 ms `PREFETCH` top-up (SPEC-000 §3) and smaller/larger neighbours.
const READ_SIZES: [usize; 3] = [4096, 9600, 32_768];

/// A scratch directory removed on drop, even on panic (best-effort).
struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "vox-bench-chunk-store-{tag}-{}",
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

/// A populated store + snapshot over [`CORPUS_SECONDS`] of voice-like audio, and the scratch
/// directory backing it (kept alive: dropping it removes the mmap-backed files).
struct Fixture {
    store: Arc<ChunkStore>,
    snapshot: Arc<DocSnapshot>,
    _scratch: ScratchDir,
}

fn fixture(tag: &str) -> Fixture {
    let scratch = ScratchDir::new(tag);
    let store = ChunkStore::create(&scratch.0, 0, StoreOptions::default()).expect("create store");
    let mut writer = ChunkWriter::new(store.clone());
    let audio = signal::voice_like(0xB051, 200.0, -6.0, 20.0, 0.2, 0.1, CORPUS_SECONDS, RATE)
        .expect("voice_like");
    writer.append(&audio).expect("append");
    let written = writer.finish().expect("finish");
    let snapshot = Arc::new(DocSnapshot::new(RATE, written.pieces, Vec::new()));
    Fixture {
        store,
        snapshot,
        _scratch: scratch,
    }
}

struct BenchState {
    reader: SnapshotReader,
    pos: Cell<u64>,
    len: u64,
    out: Vec<f32>,
}

#[divan::bench(args = READ_SIZES)]
fn read(bencher: Bencher, block: usize) {
    let f = fixture("divan");
    bencher
        .with_inputs(|| BenchState {
            reader: SnapshotReader::new(f.store.clone(), f.snapshot.clone()),
            pos: Cell::new(0),
            len: f.snapshot.len_samples,
            out: vec![0.0f32; block],
        })
        .bench_local_refs(|s: &mut BenchState| {
            let n = s.reader.read(s.pos.get(), &mut s.out).expect("read");
            let next = s.pos.get() + n as u64;
            s.pos.set(if next >= s.len { 0 } else { next });
        });
}

/// Seconds of document read per second of wall time, sequential reads, best of a few passes.
fn realtime_factor(block: usize) -> f64 {
    const PASSES: usize = 3;
    let f = fixture("report");
    let len = f.snapshot.len_samples;
    let mut best = f64::INFINITY;
    for _ in 0..PASSES {
        let mut reader = SnapshotReader::new(f.store.clone(), f.snapshot.clone());
        let mut out = vec![0.0f32; block];
        let started = Instant::now();
        let mut pos = 0u64;
        while pos < len {
            let n = reader.read(pos, &mut out).expect("read");
            std::hint::black_box(&out);
            if n == 0 {
                break;
            }
            pos += n as u64;
        }
        best = best.min(started.elapsed().as_secs_f64());
    }
    (len as f64 / f64::from(RATE)) / best
}

fn report() {
    println!(
        "SnapshotReader::read throughput, {CORPUS_SECONDS} s document, sequential reads, \
         realtime factor (document seconds read per wall second)"
    );
    for block in READ_SIZES {
        let factor = realtime_factor(block);
        println!("  read({block}) blocks: {factor:.1}x realtime");
        bench_report::result(
            "vox-project",
            &format!("chunk_store_read_{block}f_realtime_factor"),
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
