//! Big-document tests, ignored by default (minutes of I/O, ~5 GB scratch). Run with
//! `just test-big`, which builds in release mode and puts scratch files under `target/big-tests`.

mod common;

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use common::*;
use vox_project::take::{
    MAX_TAKE_DATA_BYTES, TAKE_HEADER_BYTES, TakeWriter, TakeWriterOptions, read_take_samples,
    recover_take_file,
};
use vox_project::{DocSnapshot, SEGMENT_BYTES, SnapshotReader};
use vox_testkit::prng::Pcg32;

const SIXTY_MIN: u64 = 60 * 60 * RATE_U64;

/// `fixtures/generated/long-60min-48k-mono.wav` (`just fixtures`), or the same fixture generated
/// into the scratch dir with testkit's generator when it hasn't been built.
fn fixture_path(tmp: &Path) -> PathBuf {
    let generated = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/generated/long-60min-48k-mono.wav");
    if generated.exists() {
        return generated;
    }
    let path = tmp.join("long-60min-48k-mono.wav");
    let t = Instant::now();
    vox_testkit::signal::write_long_fixture(&path, 3600.0, RATE, 42).unwrap();
    println!(
        "generated the 60-min fixture in {:.1} s",
        t.elapsed().as_secs_f64()
    );
    path
}

/// Offset and length of the `data` chunk of a WAV file.
fn data_chunk(file: &mut File) -> (u64, u64) {
    let mut head = [0u8; 12];
    file.read_exact(&mut head).unwrap();
    assert_eq!(&head[0..4], b"RIFF");
    let mut pos = 12u64;
    loop {
        let mut chunk = [0u8; 8];
        file.seek(SeekFrom::Start(pos)).unwrap();
        file.read_exact(&mut chunk).unwrap();
        let size = u64::from(u32::from_le_bytes(chunk[4..8].try_into().unwrap()));
        if &chunk[0..4] == b"data" {
            return (pos + 8, size);
        }
        pos += 8 + size + (size & 1);
    }
}

/// SPEC-004 AC-5 (store part): 512 MiB budget, 60-min document, played start to end with 200
/// random seeks: the resident-memory counter stays ≤ budget + 128 MiB at every read (and so at
/// every 100 ms sample), and every sample read back is bit-identical to the fixture.
#[test]
#[ignore = "60-min document (691 MB); run with `just test-big`"]
fn ac5_sixty_minute_playback_with_seeks_stays_within_budget() {
    let tmp = TempDir::new("big-ac5");
    let fixture = fixture_path(tmp.path());
    let budget = 512 * MIB;
    let limit = budget + 128 * MIB;
    let store_dir = tmp.path().join("store");
    std::fs::create_dir_all(&store_dir).unwrap();
    let store = store_in(&store_dir, budget);

    // Decode and hash the fixture first (untimed), then time only the store: ChunkWriter
    // append + finish, and the single sync that makes the import durable.
    let mut file = File::open(&fixture).unwrap();
    let (offset, len) = data_chunk(&mut file);
    assert_eq!(len, SIXTY_MIN * 4);
    file.seek(SeekFrom::Start(offset)).unwrap();
    let mut bytes = vec![0u8; len as usize];
    file.read_exact(&mut bytes).unwrap();
    let (words, _) = bytes.as_chunks::<4>();
    let source: Vec<f32> = words.iter().map(|w| f32::from_le_bytes(*w)).collect();
    drop(bytes);
    let mut import_hash = Fnv::new();
    import_hash.update(&source);

    let t = Instant::now();
    let mut writer = store.writer();
    for block in source.chunks(1 << 20) {
        writer.append(block).unwrap();
    }
    let written = writer.finish().unwrap();
    let import_s = t.elapsed().as_secs_f64();
    let t = Instant::now();
    store.sync().unwrap();
    let sync_s = t.elapsed().as_secs_f64();
    assert_eq!(store.unsynced_segments(), 0);
    drop(source);
    assert_eq!(written.len_samples, SIXTY_MIN);
    assert_eq!(written.chunks.len(), 2637, "ADR-004 §2: 2 637 chunks");
    assert_eq!(store.segment_count(), 11, "ADR-004 §2: 11 segments");
    let import_peak = store.peak_resident_bytes();
    assert!(import_peak <= limit, "import peak {import_peak}");

    let snapshot = Arc::new(DocSnapshot::new(RATE, written.pieces, Vec::new()));
    store.reset_peak_resident();
    let stop = Arc::new(AtomicBool::new(false));
    let sampler = {
        let store = Arc::clone(&store);
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut samples = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                samples.push(store.resident_bytes());
                std::thread::sleep(Duration::from_millis(100));
            }
            samples
        })
    };

    // Playback in 100 ms blocks; every 180 blocks a 1 s detour to a random position (200 seeks),
    // then back to the sequential position.
    let t = Instant::now();
    let mut reader = SnapshotReader::new(Arc::clone(&store), Arc::clone(&snapshot));
    let block = 4800usize;
    let blocks = (SIXTY_MIN / block as u64) as usize;
    let mut rng = Pcg32::new(77, 5);
    let mut buf = vec![0.0f32; block];
    let mut playback_hash = Fnv::new();
    let mut max_resident = 0u64;
    let mut seeks = 0;
    for b in 0..blocks {
        if b % 180 == 90 {
            let pos = (u64::from(rng.next_u32()) * 4096) % (SIXTY_MIN - 10 * block as u64);
            for k in 0..10 {
                reader.read(pos + (k * block) as u64, &mut buf).unwrap();
                max_resident = max_resident.max(store.resident_bytes());
            }
            seeks += 1;
        }
        let n = reader.read((b * block) as u64, &mut buf).unwrap();
        assert_eq!(n, block);
        playback_hash.update(&buf);
        max_resident = max_resident.max(store.resident_bytes());
        assert!(
            max_resident <= limit,
            "resident {max_resident} at block {b}"
        );
    }
    let playback_s = t.elapsed().as_secs_f64();
    stop.store(true, Ordering::Relaxed);
    let sampled = sampler.join().unwrap();
    let sampled_max = sampled.iter().copied().max().unwrap_or(0);

    assert_eq!(seeks, 200);
    assert_eq!(
        playback_hash.finish(),
        import_hash.finish(),
        "bit-identical playback"
    );
    assert!(store.peak_resident_bytes() <= limit);
    assert!(sampled_max <= limit);
    println!(
        "AC-5: import append+finish {import_s:.2} s + sync {sync_s:.2} s (peak resident {} MiB); \
         playback+200 seeks {playback_s:.2} s; \
         peak resident {} MiB, max at reads {} MiB, max of {} 100-ms samples {} MiB \
         (budget 512, limit 640); mapped segments at end {}",
        import_peak / MIB,
        store.peak_resident_bytes() / MIB,
        max_resident / MIB,
        sampled.len(),
        sampled_max / MIB,
        store.mapped_segments(),
    );
    assert!(store.mapped_bytes() <= budget + 2 * SEGMENT_BYTES);
}

/// The take writer rolls over at the real RIFF limit (≈ 6 h 12 min at 48 kHz, SPEC-002 §2.2).
#[test]
#[ignore = "writes a 4 GiB take; run with `just test-big`"]
fn take_rolls_over_at_the_real_4_gib_riff_limit() {
    let tmp = TempDir::new("big-riff");
    let per_part = MAX_TAKE_DATA_BYTES / 4;
    let total = per_part + u64::from(RATE);
    let t = Instant::now();
    let mut writer = TakeWriter::create(tmp.path(), 1, RATE, TakeWriterOptions::default()).unwrap();
    let value = |i: u64| (i % 1_000_003) as f32;
    let mut block = vec![0f32; 1 << 20];
    let mut written = 0u64;
    while written < total {
        let n = (block.len() as u64).min(total - written) as usize;
        for (i, s) in block[..n].iter_mut().enumerate() {
            *s = value(written + i as u64);
        }
        writer.append(&block[..n]).unwrap();
        written += n as u64;
    }
    let files = writer.finish().unwrap();
    let write_s = t.elapsed().as_secs_f64();
    assert_eq!(files.samples, total);
    assert_eq!(files.parts.len(), 2);
    let len0 = std::fs::metadata(&files.parts[0]).unwrap().len();
    assert_eq!(len0, TAKE_HEADER_BYTES + MAX_TAKE_DATA_BYTES);
    let rec0 = recover_take_file(&files.parts[0]).unwrap();
    let rec1 = recover_take_file(&files.parts[1]).unwrap();
    assert_eq!((rec0.samples, rec1.samples), (per_part, u64::from(RATE)));
    assert!(!rec0.header_stale() && !rec1.header_stale());
    let mut edge = [0f32; 2];
    read_take_samples(&rec0, per_part - 2, &mut edge).unwrap();
    assert_eq!(
        edge.map(f32::to_bits),
        [value(per_part - 2), value(per_part - 1)].map(f32::to_bits)
    );
    read_take_samples(&rec1, 0, &mut edge).unwrap();
    assert_eq!(
        edge.map(f32::to_bits),
        [value(per_part), value(per_part + 1)].map(f32::to_bits)
    );
    let (samples, _) = vox_testkit::wav::read_wav_file(&files.parts[1]).unwrap();
    assert_eq!(samples.len(), RATE as usize);
    println!(
        "RIFF rollover: {} samples ({:.2} GiB) in {write_s:.1} s; parts {} + {} samples",
        total,
        (total * 4) as f64 / (1u64 << 30) as f64,
        rec0.samples,
        rec1.samples
    );
}
