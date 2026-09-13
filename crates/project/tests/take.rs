//! Crash-safe take writer (ADR-004 §6–§7) and WAV-tail recovery (SPEC-002 AC-6, project part).

mod common;

use std::time::{Duration, Instant};

use common::*;
use vox_project::take::{
    MAX_TAKE_DATA_BYTES, TAKE_HEADER_BYTES, TakeSyncMode, TakeWriter, TakeWriterOptions,
    read_take_samples, recover_take, recover_take_file, repair_take_header, take_header,
    take_part_path,
};
use vox_testkit::prng::Pcg32;
use vox_testkit::wav::{SampleFormat, read_wav_file};

fn header_data_bytes(path: &std::path::Path) -> u32 {
    let bytes = std::fs::read(path).unwrap();
    u32::from_le_bytes(bytes[40..44].try_into().unwrap())
}

#[test]
fn finished_take_is_a_mono_f32_wav_readable_by_testkit() {
    let tmp = TempDir::new("take-wav");
    let source = noise(21, RATE as usize);
    let mut writer = TakeWriter::create(tmp.path(), 1, RATE, TakeWriterOptions::default()).unwrap();
    for block in source.chunks(480) {
        writer.append(block).unwrap();
    }
    let files = writer.finish().unwrap();
    assert_eq!(files.samples, source.len() as u64);
    assert_eq!(files.parts, vec![take_part_path(tmp.path(), 1, 0)]);
    let (samples, info) = read_wav_file(&files.parts[0]).unwrap();
    assert_eq!(info.channels, 1);
    assert_eq!(info.sample_rate, RATE);
    assert_eq!(info.bits_per_sample, 32);
    assert_eq!(info.sample_format, SampleFormat::Float);
    assert!(bits_equal(&samples, &source));
    let len = std::fs::metadata(&files.parts[0]).unwrap().len();
    assert_eq!(len, TAKE_HEADER_BYTES + 4 * source.len() as u64);
}

#[test]
fn header_is_patched_about_every_second() {
    let tmp = TempDir::new("take-sync");
    let options = TakeWriterOptions {
        sync_mode: TakeSyncMode::Inline,
        ..TakeWriterOptions::default()
    };
    let mut writer = TakeWriter::create(tmp.path(), 1, RATE, options).unwrap();
    // The writer's sync clock starts at creation, i.e. before t0.
    let t0 = Instant::now();
    let path = take_part_path(tmp.path(), 1, 0);
    let block = noise(22, 2400); // 50 ms drains
    let mut t = t0;
    let mut patched_at = Vec::new();
    for i in 1..=60 {
        t += Duration::from_millis(50);
        writer.append_at(&block, t).unwrap();
        // Data always reaches the OS immediately (no user-space batching).
        let len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(len, TAKE_HEADER_BYTES + 4 * 2400 * i);
        let header = header_data_bytes(&path);
        if u64::from(header) == 4 * 2400 * i {
            patched_at.push(i);
        }
    }
    // 3 s of 50 ms drains: patched at ~1 s, ~2 s, ~3 s — never more than 1 s stale.
    assert_eq!(patched_at, vec![20, 40, 60]);
}

#[test]
fn take_rolls_over_past_the_riff_limit() {
    let tmp = TempDir::new("take-rollover");
    let options = TakeWriterOptions {
        max_data_bytes: 4000, // 1 000 samples per part for the test
        ..TakeWriterOptions::default()
    };
    let source = noise(23, 2500);
    let mut writer = TakeWriter::create(tmp.path(), 7, RATE, options).unwrap();
    for block in source.chunks(333) {
        writer.append(block).unwrap();
    }
    let files = writer.finish().unwrap();
    assert_eq!(files.samples, 2500);
    assert_eq!(
        files.parts,
        (0..3)
            .map(|p| take_part_path(tmp.path(), 7, p))
            .collect::<Vec<_>>()
    );
    let mut joined = Vec::new();
    for (part, expected) in files.parts.iter().zip([1000usize, 1000, 500]) {
        let (samples, _) = read_wav_file(part).unwrap();
        assert_eq!(samples.len(), expected);
        joined.extend(samples);
    }
    assert!(bits_equal(&joined, &source));
    let recovered = recover_take(tmp.path(), 7).unwrap();
    assert_eq!(recovered.iter().map(|p| p.samples).sum::<u64>(), 2500);
    assert!(recovered.iter().all(|p| !p.header_stale()));
    assert_eq!(MAX_TAKE_DATA_BYTES, u64::from(u32::MAX) - 39);
}

/// SPEC-002 AC-6 (project part): a take WAV truncated at 1 000 random byte offsets with a stale
/// header recovers `floor((size − header) / 4)` samples, bit-identical, with no panic.
#[test]
fn truncated_take_with_stale_header_recovers_by_file_size() {
    let tmp = TempDir::new("take-ac6");
    let source = noise(24, RATE as usize);
    for patch_midway in [false, true] {
        let take = if patch_midway { 2 } else { 1 };
        let options = TakeWriterOptions {
            sync_interval: Duration::from_secs(3600),
            ..TakeWriterOptions::default()
        };
        let mut writer = TakeWriter::create(tmp.path(), take, RATE, options).unwrap();
        let half = source.len() / 2;
        writer.append(&source[..half]).unwrap();
        if patch_midway {
            // The header claims half the take; the file then grows past it (stale again).
            writer.sync().unwrap();
        }
        writer.append(&source[half..]).unwrap();
        let path = writer.parts()[0].clone();
        drop(writer); // "crash": no final patch
        let full_len = std::fs::metadata(&path).unwrap().len();
        let claimed = u64::from(header_data_bytes(&path)) / 4;
        assert_eq!(claimed, if patch_midway { half as u64 } else { 0 });

        let mut rng = Pcg32::new(25 + u64::from(take), 9);
        let mut offsets: Vec<u64> = (0..1000)
            .map(|_| u64::from(rng.next_u32()) % (full_len + 1))
            .collect();
        offsets.sort_unstable_by(|a, b| b.cmp(a));
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let mut buf = vec![0.0f32; source.len()];
        for size in offsets {
            file.set_len(size).unwrap();
            match recover_take_file(&path) {
                Ok(rec) => {
                    assert!(size >= TAKE_HEADER_BYTES);
                    let expected = (size - TAKE_HEADER_BYTES) / 4;
                    assert_eq!(rec.samples, expected, "size {size}");
                    assert_eq!(rec.sample_rate_hz, RATE);
                    let n = read_take_samples(&rec, 0, &mut buf).unwrap();
                    assert_eq!(n as u64, expected);
                    assert!(
                        bits_equal(&buf[..n], &source[..n]),
                        "size {size}: samples differ"
                    );
                }
                Err(_) => assert!(size < TAKE_HEADER_BYTES, "size {size} failed to recover"),
            }
        }
    }
}

/// H-05: a crash while rolling over (or right after `begin_take`) can leave the newest part torn
/// inside its header. It holds no samples, so it must not make the whole take unrecoverable; a
/// damaged part that is *not* the newest is still an error.
#[test]
fn torn_trailing_part_does_not_fail_the_take() {
    let tmp = TempDir::new("take-torn-part");
    let options = TakeWriterOptions {
        max_data_bytes: 4000, // 1 000 samples per part
        ..TakeWriterOptions::default()
    };
    let source = noise(28, 2000);
    let mut writer = TakeWriter::create(tmp.path(), 4, RATE, options).unwrap();
    writer.append(&source).unwrap(); // fills parts 0 and 1 exactly
    drop(writer); // "crash"
    let header = take_header(RATE, 0);
    let torn = take_part_path(tmp.path(), 4, 2);
    for len in [0usize, 20, 43, 44] {
        std::fs::write(&torn, &header[..len]).unwrap();
        let parts = recover_take(tmp.path(), 4).unwrap_or_else(|e| panic!("len {len}: {e}"));
        let total: u64 = parts.iter().map(|p| p.samples).sum();
        assert_eq!(total, 2000, "torn part of {len} bytes");
        let mut joined = Vec::new();
        for part in &parts {
            let mut buf = vec![0.0f32; part.samples as usize];
            read_take_samples(part, 0, &mut buf).unwrap();
            joined.extend(buf);
        }
        assert!(bits_equal(&joined, &source), "len {len}");
    }
    // A torn part 0 alone is an empty take, not an error.
    let alone = take_part_path(tmp.path(), 5, 0);
    std::fs::write(&alone, &header[..10]).unwrap();
    assert!(recover_take(tmp.path(), 5).unwrap().is_empty());
    // Damage before the newest part is not a torn rollover.
    std::fs::write(&torn, header).unwrap();
    let middle = take_part_path(tmp.path(), 4, 1);
    std::fs::OpenOptions::new()
        .write(true)
        .open(&middle)
        .unwrap()
        .set_len(20)
        .unwrap();
    assert!(recover_take(tmp.path(), 4).is_err());
}

#[test]
fn repaired_take_is_a_valid_wav() {
    let tmp = TempDir::new("take-repair");
    let source = noise(26, 10_000);
    let options = TakeWriterOptions {
        sync_interval: Duration::from_secs(3600),
        ..TakeWriterOptions::default()
    };
    let mut writer = TakeWriter::create(tmp.path(), 1, RATE, options).unwrap();
    writer.append(&source).unwrap();
    let path = writer.parts()[0].clone();
    drop(writer);
    // Torn mid-sample.
    let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_len(TAKE_HEADER_BYTES + 4 * 6000 + 2).unwrap();
    let rec = recover_take_file(&path).unwrap();
    assert!(rec.header_stale());
    assert_eq!(rec.samples, 6000);
    repair_take_header(&rec).unwrap();
    let (samples, _) = read_wav_file(&path).unwrap();
    assert!(bits_equal(&samples, &source[..6000]));
    assert!(!recover_take_file(&path).unwrap().header_stale());
}

/// The default background mode: `append` never syncs; a `TakeSyncHandle` on another thread
/// patches the header to exactly the data written so far, and follows a rollover.
#[test]
fn sync_handle_patches_the_header_from_another_thread() {
    let tmp = TempDir::new("take-handle");
    let options = TakeWriterOptions {
        max_data_bytes: 4000, // 1 000 samples per part
        ..TakeWriterOptions::default()
    };
    assert_eq!(options.sync_mode, TakeSyncMode::Background);
    let mut writer = TakeWriter::create(tmp.path(), 3, RATE, options).unwrap();
    let handle = writer.sync_handle();
    let part0 = take_part_path(tmp.path(), 3, 0);
    let source = noise(27, 1500);

    writer
        .append_at(&source[..600], Instant::now() + Duration::from_secs(10))
        .unwrap();
    assert_eq!(header_data_bytes(&part0), 0, "append never syncs itself");
    let h = handle.clone();
    std::thread::spawn(move || h.sync().unwrap())
        .join()
        .unwrap();
    assert_eq!(header_data_bytes(&part0), 600 * 4);

    writer.append(&source[600..]).unwrap();
    let part1 = take_part_path(tmp.path(), 3, 1);
    assert_eq!(
        header_data_bytes(&part0),
        1000 * 4,
        "rollover finalized part 0"
    );
    assert_eq!(header_data_bytes(&part1), 0);
    std::thread::spawn(move || handle.sync().unwrap())
        .join()
        .unwrap();
    assert_eq!(
        header_data_bytes(&part1),
        500 * 4,
        "the handle follows the new part"
    );

    let files = writer.finish().unwrap();
    let mut joined = read_wav_file(&files.parts[0]).unwrap().0;
    joined.extend(read_wav_file(&files.parts[1]).unwrap().0);
    assert!(bits_equal(&joined, &source));
}
