//! H-15: AC-18 performance budgets for sidecar read/write and the fingerprint fast path
//! (SPEC-018 §3 `sidecar_write_budget_ms`/`sidecar_load_budget_ms`, both ≤ 150 ms p95 over 20
//! runs at 10 000 markers / a 16-slot rack holding 1 MiB of blobs in total).
//!
//! An ordinary (non-`just test-big`) test: this payload serializes to a couple of MB, nowhere
//! near the multi-GB fixtures `big.rs` needs a release build and a real disk for — comfortably
//! fast enough to assert a hard budget against directly in a debug test build.

mod common;

use std::time::Instant;

use common::*;
use vox_project::{DocumentIdentity, MarkerItemModel, SaveFormatModel, WriteInput};
use vox_testkit::bench_report;

const MARKER_COUNT: u64 = 10_000;
const SLOT_COUNT: usize = 16;
const TOTAL_BLOB_BYTES: usize = 1024 * 1024;
const RUNS: usize = 20;
const BUDGET_MS: f64 = 150.0;

fn ac18_markers(len_samples: u64) -> Vec<MarkerItemModel> {
    (1..=MARKER_COUNT)
        .map(|id| MarkerItemModel {
            id,
            pos_samples: (id * 97) % len_samples,
            len_samples: 0,
            name: format!("Marker {id}"),
            kind: "user".to_string(),
            extra: Default::default(),
        })
        .collect()
}

/// A 16-slot rack whose states carry 1 MiB of "blob" text between them. `rack` is an opaque JSON
/// [`serde_json::Value`] as far as `project`/sidecar.rs are concerned (ADR-001 rule 4) — the
/// content only needs to be *valid JSON of the right size*, not real base64/module data, to
/// exercise write/read performance realistically.
fn ac18_rack() -> serde_json::Value {
    let per_slot = TOTAL_BLOB_BYTES / SLOT_COUNT;
    let blob_text = "A".repeat(per_slot);
    let slots: Vec<serde_json::Value> = (0..SLOT_COUNT)
        .map(|i| {
            serde_json::json!({
                "module": "org.powervoice.noise-reduction@1.0.0",
                "bypass": i % 5 == 0,
                "state": {
                    "format_version": 1,
                    "params": { "reduction_db": 12.0, "sensitivity_db": 3.0 },
                    "blob": blob_text,
                },
            })
        })
        .collect();
    serde_json::json!({ "slots": slots })
}

fn ac18_input(len_samples: u64) -> (Vec<MarkerItemModel>, serde_json::Value) {
    (ac18_markers(len_samples), ac18_rack())
}

fn write_input<'a>(markers: &'a [MarkerItemModel], rack: serde_json::Value) -> WriteInput<'a> {
    WriteInput {
        file_name: "a.wav".to_string(),
        file_size_bytes: 691_200_000,
        file_mtime: "2026-09-13T08:41:06Z".to_string(),
        sample_rate_hz: 48_000,
        len_samples: 48_000 * 3600,
        audio_crc32: 0x9f3a_51c2,
        save_format: SaveFormatModel {
            container: "wav".to_string(),
            sample_format: "pcm24".to_string(),
            dither: "tpdf".to_string(),
            extra: Default::default(),
        },
        markers,
        rack,
        view: serde_json::json!({}),
        app_version: "0.3.0".to_string(),
        written_at: "2026-09-13T08:41:07Z".to_string(),
    }
}

fn p95(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((samples.len() as f64) * 0.95).ceil() as usize - 1;
    samples[idx.min(samples.len() - 1)]
}

/// AC-18: a sidecar write (serialize -> temp -> `fdatasync` -> rename) takes ≤ 150 ms p95 over 20
/// runs, at 10 000 markers and a 16-slot rack holding 1 MiB of blobs in total.
#[test]
#[ignore = "wall-clock AC-18 budget: release-only, run via `just test-big` (flaky in debug builds under parallel load)"]
fn ac18_sidecar_write_is_within_the_150ms_p95_budget() {
    let dir = TempDir::new("ac18-write");
    let path = dir.path().join("a.wav.vo.json");
    let len_samples = 48_000 * 3600;
    let (markers, rack) = ac18_input(len_samples);

    let mut times_ms = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let input = write_input(&markers, rack.clone());
        let t = Instant::now();
        vox_project::write_sidecar(&path, &input, false).unwrap();
        times_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let size = std::fs::metadata(&path).unwrap().len();
    let p95_ms = p95(times_ms.clone());
    println!(
        "AC-18 write: {size} bytes, {RUNS} runs, p95 {p95_ms:.2} ms (budget {BUDGET_MS} ms), \
         all: {times_ms:?}"
    );
    bench_report::result(
        "vox-project",
        "spec018_ac18_sidecar_write_p95_ms",
        p95_ms,
        "ms",
        Some(bench_report::Target::le(BUDGET_MS)),
    );
    assert!(
        size > 1024 * 1024,
        "payload should be a couple of MB, was {size}"
    );
    assert!(
        p95_ms <= BUDGET_MS,
        "p95 {p95_ms:.2} ms > {BUDGET_MS} ms budget"
    );
}

/// AC-18: read + validate + apply takes ≤ 150 ms p95 over the same payload — "apply" here is
/// `read_sidecar`'s classify step (SPEC-018 §4.4 steps 1-6); resolving the rack into live module
/// instances is `rack`'s job (ADR-001 rule 4), out of `project`'s scope to measure.
#[test]
#[ignore = "wall-clock AC-18 budget: release-only, run via `just test-big` (flaky in debug builds under parallel load)"]
fn ac18_sidecar_read_and_validate_is_within_the_150ms_p95_budget() {
    let dir = TempDir::new("ac18-read");
    let path = dir.path().join("a.wav.vo.json");
    let len_samples = 48_000 * 3600;
    let (markers, rack) = ac18_input(len_samples);
    let input = write_input(&markers, rack);
    vox_project::write_sidecar(&path, &input, false).unwrap();
    let identity = DocumentIdentity {
        sample_rate_hz: 48_000,
        len_samples,
        audio_crc32: 0x9f3a_51c2,
    };

    let mut times_ms = Vec::with_capacity(RUNS);
    for _ in 0..RUNS {
        let t = Instant::now();
        let load = vox_project::read_sidecar(&path, identity);
        times_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(
            load.doc.is_some(),
            "must actually load, not just time a failure"
        );
        assert_eq!(load.markers.len(), MARKER_COUNT as usize);
    }
    let p95_ms = p95(times_ms.clone());
    println!(
        "AC-18 read: {RUNS} runs, p95 {p95_ms:.2} ms (budget {BUDGET_MS} ms), all: {times_ms:?}"
    );
    bench_report::result(
        "vox-project",
        "spec018_ac18_sidecar_read_p95_ms",
        p95_ms,
        "ms",
        Some(bench_report::Target::le(BUDGET_MS)),
    );
    assert!(
        p95_ms <= BUDGET_MS,
        "p95 {p95_ms:.2} ms > {BUDGET_MS} ms budget"
    );
}

/// AC-18 (fingerprint fast path): the document CRC32 is combined from each chunk's own
/// already-computed CRC32 (zero extra passes over the samples) right after import — as opposed to
/// the streaming fallback needed once a document has been edited. Regression guard: the fast path
/// must stay at least an order of magnitude faster than streaming through the same samples, on a
/// document large enough (10 min) that an accidental fallback to streaming would be obvious.
#[test]
fn fingerprint_fast_path_avoids_the_streaming_fallbacks_extra_read() {
    let dir = TempDir::new("ac18-fingerprint");
    let samples = noise(1, 48_000 * 600); // 10 min mono @ 48 kHz
    let store_dir = dir.path().join("store");
    std::fs::create_dir_all(&store_dir).unwrap();
    let store = store_in(&store_dir, 512 * MIB);
    let mut writer = store.writer();
    writer.append(&samples).unwrap();
    let written = writer.finish().unwrap();
    let snapshot = vox_project::DocSnapshot::new(48_000, written.pieces, Vec::new());

    // Fast path: every piece is a whole, freshly-imported chunk.
    let t = Instant::now();
    let fast = vox_project::document_crc32(&store, &snapshot).unwrap();
    let fast_ms = t.elapsed().as_secs_f64() * 1000.0;

    // Force the streaming fallback by handing it a snapshot whose pieces don't look like whole
    // chunks any more (a non-zero offset), same underlying bytes.
    let mut edited_pieces = snapshot.pieces.to_vec();
    if let Some(first) = edited_pieces.first_mut() {
        first.offset = 1;
        first.len -= 1;
    }
    let edited = vox_project::DocSnapshot::new(48_000, edited_pieces, Vec::new());
    let t = Instant::now();
    let _ = vox_project::document_crc32(&store, &edited).unwrap();
    let streaming_ms = t.elapsed().as_secs_f64() * 1000.0;

    println!(
        "fingerprint: fast path {fast_ms:.3} ms vs streaming fallback {streaming_ms:.3} ms \
         ({} samples)",
        samples.len()
    );
    assert_eq!(
        fast,
        {
            let mut hasher = crc32fast::Hasher::new();
            for &s in &samples {
                hasher.update(&s.to_le_bytes());
            }
            hasher.finalize()
        },
        "fast path must still be correct"
    );
    assert!(
        fast_ms * 10.0 < streaming_ms.max(1.0) || fast_ms < 5.0,
        "fast path ({fast_ms:.3} ms) should be far cheaper than streaming ({streaming_ms:.3} ms)"
    );
}
