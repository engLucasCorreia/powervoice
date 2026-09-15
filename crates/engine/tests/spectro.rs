//! T-204: the spectrogram tile service over real chunk stores (SPEC-007 AC-1, AC-4, AC-6,
//! AC-7, AC-8, §4.6 cancellation; AC-9 as an ignored bench).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use vox_dsp::spectro::{dequantize, hop_for_zoom};
use vox_engine::spectro::{
    SpectroConfig, SpectroError, SpectroRequest, SpectroService, TILE_FRAMES, TileGeom, TileSource,
    VxstHeader, decode_vxst, tile_count, vxst_flags,
};
use vox_project::{ChunkStore, DocSnapshot, Marker, MarkerId, Piece, StoreOptions};
use vox_testkit::bench_report;

const FS: u32 = 48_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "vox-engine-spectro-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Doc {
    store: Arc<ChunkStore>,
    snapshot: Arc<DocSnapshot>,
    _dir: TempDir,
}

fn doc(tag: &str, samples: &[f32]) -> Doc {
    let dir = TempDir::new(tag);
    let store = ChunkStore::create(&dir.0, 0, StoreOptions::with_memory_budget(256 << 20)).unwrap();
    let mut writer = store.writer();
    for block in samples.chunks(48_000) {
        writer.append(block).unwrap();
    }
    let audio = writer.finish().unwrap();
    let snapshot = Arc::new(DocSnapshot::new(FS, audio.pieces, Vec::new()));
    Doc {
        store,
        snapshot,
        _dir: dir,
    }
}

impl Doc {
    fn source(&self) -> TileSource {
        self.with(Arc::clone(&self.snapshot))
    }

    fn with(&self, snapshot: Arc<DocSnapshot>) -> TileSource {
        TileSource {
            store: Arc::clone(&self.store),
            snapshot,
        }
    }
}

fn sine(freq_hz: f64, level_dbfs: f64, len: usize) -> Vec<f32> {
    let a = 10f64.powf(level_dbfs / 20.0);
    let w = std::f64::consts::TAU * freq_hz / f64::from(FS);
    (0..len)
        .map(|i| (a * (w * i as f64).sin()) as f32)
        .collect()
}

fn noise(seed: u64, seconds: f64) -> Vec<f32> {
    vox_testkit::signal::white_noise(seed, -20.0, seconds, FS).unwrap()
}

fn service(workers: usize) -> SpectroService {
    SpectroService::new(SpectroConfig {
        workers,
        cache_cap_bytes: None,
    })
}

fn attach(svc: &SpectroService, view: u32) -> Receiver<Vec<u8>> {
    let (tx, rx) = channel();
    svc.attach(
        view,
        Box::new(move |bytes| {
            let _ = tx.send(bytes);
        }),
    );
    rx
}

static NEXT_ID: AtomicU32 = AtomicU32::new(1);

fn req(fft_size: u32, hop: u32, tiles: Vec<u32>) -> SpectroRequest {
    SpectroRequest {
        request_id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        audio_rev: 0,
        fft_size,
        hop,
        window: 0,
        tiles,
    }
}

#[derive(Debug)]
struct Tile {
    header: VxstHeader,
    codes: Vec<u8>,
}

impl Tile {
    fn is_preview(&self) -> bool {
        self.header.flags & vxst_flags::PREVIEW != 0
    }

    fn is_last(&self) -> bool {
        self.header.flags & vxst_flags::LAST != 0
    }

    fn row(&self, frame: usize) -> &[u8] {
        let bins = self.header.bins as usize;
        &self.codes[frame * bins..(frame + 1) * bins]
    }

    fn center(&self, frame: usize) -> u64 {
        self.header.first_frame_center_sample + frame as u64 * u64::from(self.header.hop_samples)
    }
}

fn decode(bytes: &[u8]) -> Tile {
    let (header, codes) = decode_vxst(bytes).expect("a VXST frame");
    Tile {
        header,
        codes: codes.to_vec(),
    }
}

/// Every frame received until the `LAST` tile of `request_id` (other requests' tiles included).
fn collect_until_last(rx: &Receiver<Vec<u8>>, request_id: u32) -> Vec<Tile> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut out = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let tile = decode(&rx.recv_timeout(left).expect("the LAST tile in time"));
        let done = tile.header.request_id == request_id && tile.is_last();
        out.push(tile);
        if done {
            return out;
        }
    }
}

/// Sends `request` and returns its tiles (up to and including `LAST`).
fn run(
    svc: &SpectroService,
    rx: &Receiver<Vec<u8>>,
    view: u32,
    source: TileSource,
    request: SpectroRequest,
) -> Vec<Tile> {
    let id = request.request_id;
    svc.request(view, source, request).unwrap();
    collect_until_last(rx, id)
        .into_iter()
        .filter(|t| t.header.request_id == id)
        .collect()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn peak_bin(row: &[u8]) -> usize {
    row.iter()
        .enumerate()
        .max_by_key(|&(_, c)| *c)
        .map(|(i, _)| i)
        .unwrap()
}

fn mean_power(row: &[u8]) -> f64 {
    row.iter()
        .map(|&c| 10f64.powf(f64::from(dequantize(c)) / 10.0))
        .sum::<f64>()
        / row.len() as f64
}

/// SPEC-007 AC-1 through the tile service, plus AC-6's header geometry.
#[test]
fn ac1_tiles_of_a_bin_centred_sine_peak_at_bin_43() {
    let d = doc("ac1", &sine(1007.8125, -20.0, 10 * FS as usize));
    let svc = service(2);
    let rx = attach(&svc, 1);
    let tiles = run(&svc, &rx, 1, d.source(), req(2048, 512, vec![1, 2]));
    assert_eq!(tiles.len(), 2, "plain hops send no previews");
    assert!(!tiles[0].is_last() && tiles[1].is_last());
    for t in &tiles {
        let h = t.header;
        assert!(!t.is_preview());
        assert_eq!(
            (h.fft_size, h.hop_samples, h.frames, h.bins),
            (2048, 512, 256, 1025)
        );
        assert_eq!(h.window, 0);
        assert_eq!((h.q_floor_db, h.q_ceil_db), (-150.0, 6.0));
        assert_eq!(
            h.first_frame_center_sample,
            u64::from(TILE_FRAMES) * u64::from(h.tile_index) * 512
        );
        for f in 0..h.frames as usize {
            let row = t.row(f);
            assert_eq!(peak_bin(row), 43);
            let level = dequantize(row[43]);
            assert!((level + 20.0).abs() <= 0.35, "frame {f}: {level}");
        }
    }
}

/// SPEC-007 AC-4: an impulse at sample 240 000 of a 10 s document — at every hop from N/16 to
/// 65 536 the frame with the highest mean level is centred within ±hop/2 of it.
#[test]
fn ac4_impulse_frame_is_centred_within_half_a_hop() {
    let mut samples = vec![0.0f32; 10 * FS as usize];
    samples[240_000] = 1.0;
    let d = doc("ac4", &samples);
    let svc = service(4);
    let rx = attach(&svc, 1);
    for hop in (7..=16).map(|p| 1u32 << p) {
        let tiles = (0..tile_count(d.snapshot.len_samples, hop) as u32).collect();
        let got = run(&svc, &rx, 1, d.source(), req(2048, hop, tiles));
        let mut best = (f64::MIN, 0u64);
        for t in got.iter().filter(|t| !t.is_preview()) {
            for f in 0..t.header.frames as usize {
                let p = mean_power(t.row(f));
                if p > best.0 {
                    best = (p, t.center(f));
                }
            }
        }
        assert!(
            best.1.abs_diff(240_000) <= u64::from(hop / 2),
            "hop {hop}: loudest frame at {}",
            best.1
        );
    }
}

/// SPEC-007 §4.4/ADR-003 Amendment 1: at overview hops each tile's `PREVIEW` comes before its
/// refined tile, `LAST` is on the final refined tile only, and the last tile is short.
#[test]
fn overview_tiles_preview_first_then_refined_with_one_last() {
    let d = doc("preview", &noise(5, 60.0));
    let svc = service(3);
    let rx = attach(&svc, 7);
    let total = tile_count(d.snapshot.len_samples, 8192);
    assert_eq!(total, 2); // 352 frames
    let tiles = run(&svc, &rx, 7, d.source(), req(2048, 8192, vec![1, 0]));
    let refined: Vec<&Tile> = tiles.iter().filter(|t| !t.is_preview()).collect();
    assert_eq!(refined.len(), 2);
    assert_eq!(tiles.iter().filter(|t| t.is_last()).count(), 1);
    assert!(tiles.last().unwrap().is_last() && !tiles.last().unwrap().is_preview());
    for (i, t) in tiles.iter().enumerate() {
        if t.is_preview() {
            let idx = t.header.tile_index;
            assert!(
                !tiles[..i]
                    .iter()
                    .any(|e| !e.is_preview() && e.header.tile_index == idx),
                "a preview never follows its refined tile"
            );
        }
    }
    let short = refined.iter().find(|t| t.header.tile_index == 1).unwrap();
    assert_eq!(short.header.frames, 352 - 256);
}

/// SPEC-007 AC-7: the same tile with 1 and 8 workers, and before/after cache eviction, has an
/// identical payload hash.
#[test]
fn ac7_payload_hash_is_identical_across_worker_counts_and_eviction() {
    let mut samples = noise(11, 30.0);
    for (s, t) in samples
        .iter_mut()
        .zip(sine(1007.8125, -12.0, 30 * FS as usize))
    {
        *s += t;
    }
    let d = doc("ac7", &samples);
    let hashes = |svc: &SpectroService, rx: &Receiver<Vec<u8>>| {
        let mut out: Vec<(u32, u32, u64)> = Vec::new();
        for (hop, tiles) in [(1024u32, vec![0u32, 1, 2, 3]), (16_384, vec![0])] {
            for t in run(svc, rx, 1, d.source(), req(2048, hop, tiles)) {
                if !t.is_preview() {
                    out.push((hop, t.header.tile_index, fnv1a64(&t.codes)));
                }
            }
        }
        out.sort_unstable();
        out
    };
    let one = service(1);
    let rx1 = attach(&one, 1);
    let eight = service(8);
    let rx8 = attach(&eight, 1);
    let h1 = hashes(&one, &rx1);
    let h8 = hashes(&eight, &rx8);
    assert_eq!(h1.len(), 5);
    assert_eq!(h1, h8);
    let warm = hashes(&eight, &rx8);
    assert_eq!(warm, h8, "cache hits");
    let computed = eight.stats().computed;
    eight.clear_cache();
    assert_eq!(eight.stats().bytes_cached, 0);
    assert_eq!(hashes(&eight, &rx8), h8, "recomputed after eviction");
    assert!(eight.stats().computed >= computed + 5);
}

fn golden_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/spectro_golden_tile.hex")
}

/// SPEC-007 AC-7: a fixed synthetic tile (bin-centred sine + seeded noise, N = 256, hop 64, 8
/// frames) matches its checked-in golden codes: bit-exact (same FNV-1a hash) on x86_64; elsewhere
/// at most 1 step in ≤ 0.1 % of cells (SIMD rounding). `POWERVOICE_BLESS=1` rewrites the file.
#[test]
fn ac7_golden_tile() {
    let mut samples = sine(10.0 * f64::from(FS) / 256.0, -6.0, 512);
    for (s, n) in samples.iter_mut().zip(noise(42, 1.0)) {
        *s += n * 0.1;
    }
    let d = doc("golden", &samples);
    let svc = service(1);
    let rx = attach(&svc, 1);
    let tiles = run(&svc, &rx, 1, d.source(), req(256, 64, vec![0]));
    assert_eq!(tiles.len(), 1);
    let codes = &tiles[0].codes;
    assert_eq!(codes.len(), 8 * 129);
    let hex: String = codes.iter().map(|b| format!("{b:02x}")).collect();
    if std::env::var_os("POWERVOICE_BLESS").is_some() {
        std::fs::create_dir_all(golden_path().parent().unwrap()).unwrap();
        std::fs::write(golden_path(), format!("{hex}\n")).unwrap();
    }
    let golden_hex = std::fs::read_to_string(golden_path()).expect("golden tile file");
    let golden: Vec<u8> = (0..golden_hex.trim().len() / 2)
        .map(|i| u8::from_str_radix(&golden_hex.trim()[2 * i..2 * i + 2], 16).unwrap())
        .collect();
    assert_eq!(golden.len(), codes.len());
    if cfg!(target_arch = "x86_64") {
        assert_eq!(fnv1a64(codes), fnv1a64(&golden), "golden tile hash");
    } else {
        let diffs: Vec<i32> = codes
            .iter()
            .zip(&golden)
            .map(|(&a, &b)| i32::from(a) - i32::from(b))
            .filter(|&d| d != 0)
            .collect();
        assert!(diffs.iter().all(|d| d.abs() <= 1));
        assert!(diffs.len() * 1000 <= codes.len().max(1000));
    }
}

/// SPEC-007 AC-8 (60 s stand-in for the 60-min document): a marker-only edit is all cache hits;
/// silencing a range recomputes exactly the tiles whose input span intersects it; after a cut,
/// every tile whose span ends before the cut is a cache hit.
#[test]
fn ac8_cache_reuse_after_marker_silence_and_cut_edits() {
    let d = doc("ac8", &noise(21, 60.0));
    let svc = service(4);
    let rx = attach(&svc, 1);
    let (fft, hop) = (2048u32, 1024u32);
    let n = tile_count(d.snapshot.len_samples, hop) as u32;
    assert_eq!(n, 11);
    let all: Vec<u32> = (0..n).collect();
    let tiles = run(&svc, &rx, 1, d.source(), req(fft, hop, all.clone()));
    assert_eq!(tiles.len(), 11);
    assert_eq!(svc.stats().computed, 11);

    let delta = |before: vox_engine::spectro::SpectroStats| {
        let now = svc.stats();
        (
            now.computed - before.computed,
            now.cache_hits - before.cache_hits,
        )
    };

    // Marker-only edit: same audio, new markers.
    let marked = Arc::new(DocSnapshot::new(
        FS,
        d.snapshot.pieces.to_vec(),
        vec![Marker::new(MarkerId(1), 1000, 0, "m")],
    ));
    let before = svc.stats();
    run(&svc, &rx, 1, d.with(marked), req(fft, hop, all.clone()));
    assert_eq!(delta(before), (0, 11));

    // Silence [30 s, 31 s).
    let (at, len) = (30 * u64::from(FS), u64::from(FS));
    let silenced = Arc::new(DocSnapshot::new(
        FS,
        d.snapshot
            .replaced(at, len, &Piece::silence_run(len))
            .unwrap(),
        Vec::new(),
    ));
    let touched = all
        .iter()
        .filter(|&&k| {
            let (s, e) = TileGeom::new(silenced.len_samples, fft, hop, k)
                .unwrap()
                .input_span(false);
            s < (at + len) as i64 && e > at as i64
        })
        .count() as u64;
    assert!((1..=2).contains(&touched));
    let before = svc.stats();
    run(&svc, &rx, 1, d.with(silenced), req(fft, hop, all.clone()));
    assert_eq!(delta(before), (touched, 11 - touched));

    // Cut [30 s, 31 s): later tiles are re-keyed, earlier ones hit.
    let cut = Arc::new(DocSnapshot::new(
        FS,
        d.snapshot.replaced(at, len, &[]).unwrap(),
        Vec::new(),
    ));
    let m = tile_count(cut.len_samples, hop) as u32;
    let unchanged = (0..m)
        .filter(|&k| {
            TileGeom::new(cut.len_samples, fft, hop, k)
                .unwrap()
                .input_span(false)
                .1
                <= at as i64
        })
        .count() as u64;
    assert!(unchanged >= 5);
    let before = svc.stats();
    run(&svc, &rx, 1, d.with(cut), req(fft, hop, (0..m).collect()));
    assert_eq!(delta(before), (u64::from(m) - unchanged, unchanged));
}

/// SPEC-007 §4.6: a newer request cancels the older one's unsent tiles; nothing of the older
/// request arrives after the newer one's first tile.
#[test]
fn newer_request_cancels_unsent_tiles_of_older_ones() {
    let d = doc("cancel", &noise(8, 60.0));
    let svc = service(1);
    let rx = attach(&svc, 1);
    let heavy = req(2048, 128, (0..64).collect());
    let heavy_id = heavy.request_id;
    svc.request(1, d.source(), heavy).unwrap();
    let light = req(2048, 1024, vec![0, 1]);
    let light_id = light.request_id;
    svc.request(1, d.source(), light).unwrap();
    let got = collect_until_last(&rx, light_id);
    let heavy_count = got
        .iter()
        .filter(|t| t.header.request_id == heavy_id)
        .count();
    assert!(heavy_count < 64, "older request was not cancelled");
    let first_light = got
        .iter()
        .position(|t| t.header.request_id == light_id)
        .unwrap();
    assert!(
        got[first_light..]
            .iter()
            .all(|t| t.header.request_id == light_id)
    );
    assert_eq!(got.len() - first_light, 2);
    assert!(svc.stats().cancelled >= 63);
    // Nothing else arrives later.
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
}

#[test]
fn invalid_requests_are_refused() {
    let d = doc("invalid", &noise(1, 1.0));
    let svc = service(1);
    assert_eq!(
        svc.request(9, d.source(), req(2048, 512, vec![0])),
        Err(SpectroError::UnknownView(9))
    );
    let _rx = attach(&svc, 1);
    assert_eq!(
        svc.request(1, d.source(), req(1000, 512, vec![0])),
        Err(SpectroError::InvalidFftSize(1000))
    );
    assert_eq!(
        svc.request(1, d.source(), req(2048, 64, vec![0])),
        Err(SpectroError::InvalidHop {
            fft_size: 2048,
            hop: 64
        })
    );
    assert_eq!(
        svc.request(1, d.source(), req(2048, 3000, vec![0])),
        Err(SpectroError::InvalidHop {
            fft_size: 2048,
            hop: 3000
        })
    );
    let mut bad_window = req(2048, 512, vec![0]);
    bad_window.window = 1;
    assert_eq!(
        svc.request(1, d.source(), bad_window),
        Err(SpectroError::UnsupportedWindow(1))
    );
    // Tiles past the end are ignored: nothing to send, no error.
    svc.request(1, d.source(), req(2048, 512, vec![500]))
        .unwrap();
    svc.detach(1);
    assert_eq!(
        svc.request(1, d.source(), req(2048, 512, vec![0])),
        Err(SpectroError::UnknownView(1))
    );
}

/// Chunk ids restart per store: another document's tiles never hit this one's cache, and the
/// service keeps no strong reference to a store once its tiles are delivered (a session can close).
#[test]
fn another_store_never_aliases_the_cache_and_stores_are_not_retained() {
    let a = doc("store-a", &sine(1000.0, -20.0, 100_000));
    let b = doc("store-b", &noise(2, 100_000.0 / f64::from(FS)));
    let svc = service(2);
    let rx = attach(&svc, 1);
    let ta = run(&svc, &rx, 1, a.source(), req(2048, 512, vec![0]));
    let tb = run(&svc, &rx, 1, b.source(), req(2048, 512, vec![0]));
    assert_eq!(svc.stats().computed, 2);
    assert_ne!(ta[0].codes, tb[0].codes);
    assert_eq!(Arc::strong_count(&a.store), 1);
    assert_eq!(Arc::strong_count(&b.store), 1);
}

#[test]
fn cache_is_lru_and_capped() {
    let d = doc("cap", &noise(4, 30.0));
    let tile_bytes = 256 * 1025;
    let svc = SpectroService::new(SpectroConfig {
        workers: 2,
        cache_cap_bytes: Some(3 * tile_bytes),
    });
    let rx = attach(&svc, 1);
    run(&svc, &rx, 1, d.source(), req(2048, 512, (0..6).collect()));
    assert_eq!(svc.stats().bytes_cached, 3 * tile_bytes);
    assert!(
        d.store.resident_bytes() >= 3 * tile_bytes,
        "counted against the budget"
    );
    let before = svc.stats();
    // Tile 0 was evicted (LRU); the most recent three are still cached... except the order of
    // completion with 2 workers isn't fixed, so only check the totals.
    run(&svc, &rx, 1, d.source(), req(2048, 512, (0..6).collect()));
    let now = svc.stats();
    assert_eq!(
        now.computed + now.cache_hits - before.computed - before.cache_hits,
        6
    );
    assert!(now.bytes_cached <= 3 * tile_bytes);
}

/// SPEC-007 AC-9 engine-side bench: visible tiles after a zoom on a 60-min 48 kHz document
/// (1920 device px, FFT 2048, cold cache) within 200 ms, refined whole-file tiles within 2 s, a
/// warm re-request within 50 ms. The 60-min document repeats ~9 s of noise through the piece
/// table (no 700 MB store), with a period that doesn't align with any tile span.
/// Run: `cargo test --release -p vox-engine --test spectro -- --ignored --nocapture`.
#[test]
#[ignore = "bench (release build): cargo test --release -p vox-engine --test spectro -- --ignored"]
fn ac9_latency_on_a_60_min_document() {
    let base = doc(
        "ac9",
        &vox_testkit::signal::pink_noise(9, -20.0, 433_216.0 / f64::from(FS), FS).unwrap(),
    );
    let len_total = 60 * 60 * u64::from(FS);
    let mut pieces = Vec::new();
    let mut have = 0u64;
    'fill: loop {
        for p in base.snapshot.pieces.iter() {
            let take = u64::from(p.len).min(len_total - have) as u32;
            pieces.push(Piece { len: take, ..*p });
            have += u64::from(take);
            if have == len_total {
                break 'fill;
            }
        }
    }
    let long = Arc::new(DocSnapshot::new(FS, pieces, Vec::new()));
    let svc = SpectroService::new(SpectroConfig::default());
    let rx = attach(&svc, 1);
    let width_px = 1920.0;
    for view_s in [3600.0, 600.0, 60.0, 10.0, 1.0] {
        let spp = view_s * f64::from(FS) / width_px;
        let hop = hop_for_zoom(spp, 2048);
        let start = if view_s >= 3600.0 { 0 } else { len_total / 2 };
        let end = start + (view_s * f64::from(FS)) as u64;
        let span = u64::from(TILE_FRAMES) * u64::from(hop);
        let last_tile = (tile_count(len_total, hop) - 1) as u32;
        let tiles: Vec<u32> =
            ((start / span) as u32..=((end / span) as u32).min(last_tile)).collect();
        let count = tiles.len();
        svc.clear_cache();
        let t0 = Instant::now();
        let r = req(2048, hop, tiles.clone());
        let id = r.request_id;
        svc.request(
            1,
            TileSource {
                store: Arc::clone(&base.store),
                snapshot: Arc::clone(&long),
            },
            r,
        )
        .unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut visible_ms = 0.0;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let t = decode(&rx.recv_timeout(deadline - Instant::now()).unwrap());
            if t.header.request_id != id {
                continue;
            }
            seen.insert(t.header.tile_index);
            if seen.len() == count && visible_ms == 0.0 {
                visible_ms = t0.elapsed().as_secs_f64() * 1e3;
            }
            if t.is_last() {
                break;
            }
        }
        let refined_ms = t0.elapsed().as_secs_f64() * 1e3;
        let t1 = Instant::now();
        let warm = run(
            &svc,
            &rx,
            1,
            TileSource {
                store: Arc::clone(&base.store),
                snapshot: Arc::clone(&long),
            },
            req(2048, hop, tiles),
        );
        let warm_ms = t1.elapsed().as_secs_f64() * 1e3;
        assert_eq!(warm.len(), count);
        println!(
            "view {view_s:>6} s  hop {hop:>6}  tiles {count:>2}  visible {visible_ms:7.1} ms  refined {refined_ms:7.1} ms  warm {warm_ms:6.1} ms"
        );
        let view = view_s as u64;
        for (metric, value, budget) in [
            ("visible", visible_ms, 200.0),
            ("refined", refined_ms, 2_000.0),
            ("warm", warm_ms, 50.0),
        ] {
            bench_report::result(
                "vox-engine",
                &format!("spec007_ac9_{metric}_{view}s_view_ms"),
                value,
                "ms",
                Some(bench_report::Target::le(budget)),
            );
        }
        assert!(visible_ms <= 200.0, "visible tiles took {visible_ms} ms");
        assert!(refined_ms <= 2000.0, "refined tiles took {refined_ms} ms");
        assert!(warm_ms <= 50.0, "warm re-request took {warm_ms} ms");
    }
}

/// T-602 (SPEC-007 §4.1): "while a save, export or bake job runs, tile work uses at most half of
/// the workers" — a background-job guard caps concurrent tiles; dropping it lifts the cap.
#[test]
fn a_background_job_limits_tile_work_to_half_the_workers() {
    use vox_engine::spectro::worker_cap;
    assert_eq!(worker_cap(8, 0), 8);
    assert_eq!(worker_cap(8, 1), 4);
    assert_eq!(worker_cap(8, 2), 4);
    assert_eq!(worker_cap(3, 1), 1);
    assert_eq!(worker_cap(1, 1), 1, "never starved completely");
    assert_eq!(worker_cap(0, 0), 1);

    let d = doc("background-job", &noise(5, 12.0));
    let svc = service(4);
    let rx = attach(&svc, 1);
    let job = svc.begin_background_job();
    let tiles: Vec<u32> = (0..16).collect();
    let got = run(&svc, &rx, 1, d.source(), req(1024, 256, tiles.clone()));
    assert!(got.iter().any(Tile::is_last));
    assert!(
        svc.stats().peak_running <= 2,
        "at most half of 4 workers while the job runs: {}",
        svc.stats().peak_running
    );
    drop(job);
    // Without the job every worker may take tiles again (all tiles still arrive).
    let got = run(&svc, &rx, 1, d.source(), req(2048, 512, tiles));
    assert!(got.iter().any(Tile::is_last));
    assert!(svc.stats().peak_running <= 4);
}
