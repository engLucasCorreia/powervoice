//! Crash-safe take files (ADR-004 §6–§7, SPEC-002 §2.3, AC-6).
//!
//! A take is captured as mono 32-bit float WAV at the document rate: a canonical 44-byte header
//! (`WAVE_FORMAT_IEEE_FLOAT`, 16-byte `fmt `) followed by the `data` chunk, which is always the
//! last chunk. Samples go straight to the OS with one positional write per [`TakeWriter::append`]
//! (no user-space buffering). About every second the RIFF/data sizes are patched and the file is
//! `fdatasync`ed. Past the 4 GiB RIFF limit the take rolls over to a further part file.
//!
//! Recovery treats the file as authoritative and derives the length from the file size:
//! `floor((size − 44) / 4)` samples, whatever the (possibly stale) header says.
//!
//! # Crash-loss bounds (SPEC-002 §2.3)
//! - **Process crash (≤ 250 ms).** Data handed to `write` sits in the OS page cache and survives
//!   a process crash, so only audio the capture-writer has not appended yet is lost: the capture
//!   ring plus the block in hand. The bound holds if (1) the engine drains the ring and appends
//!   at least every 50 ms and (2) `append` never waits for an `fdatasync`. For (2) the default
//!   [`TakeSyncMode::Background`] leaves the ~1 s header patch + `fdatasync` to a
//!   [`TakeSyncHandle`] driven by another thread; with [`TakeSyncMode::Inline`] each sync (tens to
//!   hundreds of ms on a busy disk) stalls the drain while audio waits in the ring. `write` itself
//!   can still block under heavy dirty-page throttling; the 10 s capture ring absorbs that, but
//!   audio waiting in the ring is lost if the process dies meanwhile.
//! - **Power loss (~1.5 s).** Only `fdatasync`ed data is durable: at most the sync interval
//!   (~1 s) plus the drain interval is lost.

use std::fmt;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::fs_util::{read_exact_at, sync_dir, write_all_at};
use crate::{ProjectError, Result};

/// Size of the header our writer emits; samples start here.
pub const TAKE_HEADER_BYTES: u64 = 44;
/// Largest `data` payload a RIFF file can describe with a 44-byte header, rounded down to whole
/// samples: `riff_size = 36 + data ≤ u32::MAX`.
pub const MAX_TAKE_DATA_BYTES: u64 = (u32::MAX as u64 - 36) / 4 * 4;
/// Default header patch + `fdatasync` interval (ADR-004 §6).
pub const DEFAULT_TAKE_SYNC_INTERVAL: Duration = Duration::from_secs(1);

const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
/// How far into a take file the recovery parser looks for the `data` chunk.
const MAX_HEADER_SCAN_BYTES: u64 = 4096;

/// Who performs the periodic header patch + `fdatasync`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TakeSyncMode {
    /// Another thread calls [`TakeSyncHandle::sync`] about every `sync_interval`; `append` never
    /// syncs, so the capture-writer's drain never waits for the disk (recommended, the default).
    #[default]
    Background,
    /// `append` syncs inline once `sync_interval` has elapsed (simpler, but blocks the drain).
    Inline,
}

/// Take writer settings.
#[derive(Clone, Debug)]
pub struct TakeWriterOptions {
    /// Header patch + `fdatasync` interval (default 1 s).
    pub sync_interval: Duration,
    /// Maximum `data` bytes per part file before rolling over (default: the RIFF limit).
    /// Rounded down to whole samples and capped at [`MAX_TAKE_DATA_BYTES`].
    pub max_data_bytes: u64,
    /// Who syncs (default [`TakeSyncMode::Background`]).
    pub sync_mode: TakeSyncMode,
}

impl Default for TakeWriterOptions {
    fn default() -> Self {
        TakeWriterOptions {
            sync_interval: DEFAULT_TAKE_SYNC_INTERVAL,
            max_data_bytes: MAX_TAKE_DATA_BYTES,
            sync_mode: TakeSyncMode::Background,
        }
    }
}

/// Path of part `part` (0-based) of take `take`: `take-0001.wav`, `take-0001-2.wav`, …
pub fn take_part_path(takes_dir: &Path, take: u32, part: u32) -> PathBuf {
    if part == 0 {
        takes_dir.join(format!("take-{take:04}.wav"))
    } else {
        takes_dir.join(format!("take-{take:04}-{}.wav", part + 1))
    }
}

/// The canonical 44-byte mono f32 WAV header.
pub fn take_header(sample_rate_hz: u32, data_bytes: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&data_bytes.saturating_add(36).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&WAVE_FORMAT_IEEE_FLOAT.to_le_bytes());
    h[22..24].copy_from_slice(&1u16.to_le_bytes());
    h[24..28].copy_from_slice(&sample_rate_hz.to_le_bytes());
    h[28..32].copy_from_slice(&sample_rate_hz.saturating_mul(4).to_le_bytes());
    h[32..34].copy_from_slice(&4u16.to_le_bytes());
    h[34..36].copy_from_slice(&32u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

/// The files of a finished take.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakeFiles {
    pub parts: Vec<PathBuf>,
    pub samples: u64,
    pub sample_rate_hz: u32,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// State shared between the writer and its [`TakeSyncHandle`]s.
#[derive(Debug)]
struct SyncShared {
    sample_rate_hz: u32,
    /// A handle on the current part; swapped (under the lock) at rollover.
    target: Mutex<File>,
    /// `data` bytes written to the current part (stored after each write, reset at rollover
    /// under the `target` lock). A sync never describes more than was written.
    data_bytes: AtomicU64,
}

impl SyncShared {
    fn patch_and_sync(&self, file: &File) -> std::io::Result<()> {
        let data = self.data_bytes.load(Ordering::Acquire);
        let header = take_header(self.sample_rate_hz, u32::try_from(data).unwrap_or(u32::MAX));
        write_all_at(file, &header, 0)?;
        file.sync_data()
    }

    fn sync(&self) -> Result<()> {
        let file = lock(&self.target);
        self.patch_and_sync(&file)
            .map_err(ProjectError::io("syncing the take file"))
    }
}

/// Patches the take header and `fdatasync`s from another thread while the capture-writer keeps
/// appending (the default [`TakeSyncMode::Background`]). Cheap to clone, `Send + Sync`. T-106
/// runs it about every second (`DEFAULT_TAKE_SYNC_INTERVAL`) until the take is finished.
#[derive(Clone)]
pub struct TakeSyncHandle {
    shared: Arc<SyncShared>,
}

impl TakeSyncHandle {
    /// Patches the current part's RIFF/data sizes to the samples appended so far and
    /// `fdatasync`s. Blocks only against a rollover; the writer's appends never wait for it.
    pub fn sync(&self) -> Result<()> {
        self.shared.sync()
    }
}

impl fmt::Debug for TakeSyncHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TakeSyncHandle")
            .field(
                "data_bytes",
                &self.shared.data_bytes.load(Ordering::Relaxed),
            )
            .finish()
    }
}

/// Writes one take: owned by the capture-writer thread (ADR-002 §1).
#[derive(Debug)]
pub struct TakeWriter {
    takes_dir: PathBuf,
    take: u32,
    sample_rate_hz: u32,
    max_data_bytes: u64,
    sync_interval: Duration,
    sync_mode: TakeSyncMode,
    file: File,
    shared: Arc<SyncShared>,
    part_data_bytes: u64,
    total_samples: u64,
    last_sync: Instant,
    parts: Vec<PathBuf>,
    scratch: Vec<u8>,
}

impl TakeWriter {
    /// Creates part 0 of take `take` in `takes_dir` (which must exist) with an empty header,
    /// `fdatasync`s it and the directory.
    pub fn create(
        takes_dir: &Path,
        take: u32,
        sample_rate_hz: u32,
        options: TakeWriterOptions,
    ) -> Result<TakeWriter> {
        if sample_rate_hz == 0 {
            return Err(ProjectError::InvalidArgument("sample rate must be > 0"));
        }
        let max_data_bytes = (options.max_data_bytes.min(MAX_TAKE_DATA_BYTES) / 4) * 4;
        if max_data_bytes == 0 {
            return Err(ProjectError::InvalidArgument(
                "max_data_bytes must hold at least one sample",
            ));
        }
        let path = take_part_path(takes_dir, take, 0);
        let file = create_part(&path, sample_rate_hz)?;
        let target = file
            .try_clone()
            .map_err(ProjectError::io("creating the take file"))?;
        Ok(TakeWriter {
            takes_dir: takes_dir.to_path_buf(),
            take,
            sample_rate_hz,
            max_data_bytes,
            sync_interval: options.sync_interval,
            sync_mode: options.sync_mode,
            file,
            shared: Arc::new(SyncShared {
                sample_rate_hz,
                target: Mutex::new(target),
                data_bytes: AtomicU64::new(0),
            }),
            part_data_bytes: 0,
            total_samples: 0,
            last_sync: Instant::now(),
            parts: vec![path],
            scratch: Vec::new(),
        })
    }

    /// A handle that syncs this take from another thread.
    pub fn sync_handle(&self) -> TakeSyncHandle {
        TakeSyncHandle {
            shared: Arc::clone(&self.shared),
        }
    }

    /// Appends samples (one positional write per part touched), rolling over at the RIFF limit.
    /// In [`TakeSyncMode::Inline`] it then syncs if the interval has elapsed.
    pub fn append(&mut self, samples: &[f32]) -> Result<()> {
        self.append_at(samples, Instant::now())
    }

    /// [`Self::append`] with an explicit clock (tests).
    pub fn append_at(&mut self, samples: &[f32], now: Instant) -> Result<()> {
        let mut rest = samples;
        while !rest.is_empty() {
            let room = ((self.max_data_bytes - self.part_data_bytes) / 4) as usize;
            if room == 0 {
                self.roll_over()?;
                continue;
            }
            let n = room.min(rest.len());
            self.scratch.clear();
            for s in &rest[..n] {
                self.scratch.extend_from_slice(&s.to_le_bytes());
            }
            write_all_at(
                &self.file,
                &self.scratch,
                TAKE_HEADER_BYTES + self.part_data_bytes,
            )
            .map_err(ProjectError::io("writing the take file"))?;
            self.part_data_bytes += n as u64 * 4;
            self.shared
                .data_bytes
                .store(self.part_data_bytes, Ordering::Release);
            self.total_samples += n as u64;
            rest = &rest[n..];
        }
        if self.sync_mode == TakeSyncMode::Inline {
            self.maybe_sync(now)?;
        }
        Ok(())
    }

    /// Patches the header and `fdatasync`s if at least the sync interval passed since the last
    /// inline sync; returns whether it synced.
    pub fn maybe_sync(&mut self, now: Instant) -> Result<bool> {
        if now.saturating_duration_since(self.last_sync) < self.sync_interval {
            return Ok(false);
        }
        self.sync_at(now)?;
        Ok(true)
    }

    /// Patches the current part's header to its real length and `fdatasync`s now.
    pub fn sync(&mut self) -> Result<()> {
        self.sync_at(Instant::now())
    }

    fn sync_at(&mut self, now: Instant) -> Result<()> {
        self.shared.sync()?;
        self.last_sync = now;
        Ok(())
    }

    fn roll_over(&mut self) -> Result<()> {
        let part = u32::try_from(self.parts.len())
            .map_err(|_| ProjectError::InvalidArgument("too many take parts"))?;
        let path = take_part_path(&self.takes_dir, self.take, part);
        let file = {
            // Finish the old part and switch every sync handle to the new one atomically.
            let mut target = lock(&self.shared.target);
            self.shared
                .patch_and_sync(&target)
                .map_err(ProjectError::io("syncing the take file"))?;
            let file = create_part(&path, self.sample_rate_hz)?;
            *target = file
                .try_clone()
                .map_err(ProjectError::io("creating the take file"))?;
            self.shared.data_bytes.store(0, Ordering::Release);
            file
        };
        self.file = file;
        self.parts.push(path);
        self.part_data_bytes = 0;
        Ok(())
    }

    /// Samples written over all parts.
    pub fn samples_written(&self) -> u64 {
        self.total_samples
    }

    /// Paths of the parts written so far.
    pub fn parts(&self) -> &[PathBuf] {
        &self.parts
    }

    /// Patches + `fdatasync`s the last part and returns the take's files.
    pub fn finish(mut self) -> Result<TakeFiles> {
        self.sync()?;
        Ok(TakeFiles {
            parts: std::mem::take(&mut self.parts),
            samples: self.total_samples,
            sample_rate_hz: self.sample_rate_hz,
        })
    }
}

fn create_part(path: &Path, sample_rate_hz: u32) -> Result<File> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(ProjectError::io("creating the take file"))?;
    write_all_at(&file, &take_header(sample_rate_hz, 0), 0)
        .and_then(|()| file.sync_data())
        .map_err(ProjectError::io("creating the take file"))?;
    if let Some(dir) = path.parent() {
        sync_dir(dir).map_err(ProjectError::io("creating the take file"))?;
    }
    Ok(file)
}

/// What a take file holds, as recovered from its size (ADR-004 §9 step 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveredTake {
    pub path: PathBuf,
    pub sample_rate_hz: u32,
    /// Byte offset of the first sample.
    pub data_offset: u64,
    /// Whole samples present: `floor((file size − data_offset) / 4)`.
    pub samples: u64,
    /// Samples the header claims (stale after a crash).
    pub header_samples: u64,
}

impl RecoveredTake {
    /// `true` if the header doesn't describe the recovered length.
    pub fn header_stale(&self) -> bool {
        self.header_samples != self.samples
    }
}

/// Parses a mono f32 WAV header from the first bytes of a file of `file_len` bytes and derives
/// the recoverable sample count from `file_len`. Never panics on any input.
pub fn parse_take_header(head: &[u8], file_len: u64) -> Result<(u32, u64, u64, u64)> {
    let truncated = ProjectError::InvalidTakeFile("truncated header");
    let u16_at = |at: usize| -> Option<u16> {
        Some(u16::from_le_bytes(head.get(at..at + 2)?.try_into().ok()?))
    };
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(head.get(at..at + 4)?.try_into().ok()?))
    };
    if head.len() < 12 {
        return Err(truncated);
    }
    if &head[0..4] != b"RIFF" || &head[8..12] != b"WAVE" {
        return Err(ProjectError::InvalidTakeFile("not a RIFF/WAVE file"));
    }
    let mut pos = 12usize;
    let mut format: Option<(u16, u16, u32, u16)> = None;
    loop {
        let (Some(id), Some(size)) = (head.get(pos..pos + 4), u32_at(pos + 4)) else {
            return Err(truncated);
        };
        let body = pos + 8;
        if id == b"data" {
            let (tag, channels, rate, bits) =
                format.ok_or(ProjectError::InvalidTakeFile("data chunk before fmt chunk"))?;
            if tag != WAVE_FORMAT_IEEE_FLOAT || channels != 1 || bits != 32 || rate == 0 {
                return Err(ProjectError::InvalidTakeFile("not a mono 32-bit float WAV"));
            }
            let data_offset = body as u64;
            if file_len < data_offset {
                return Err(truncated);
            }
            let samples = (file_len - data_offset) / 4;
            return Ok((rate, data_offset, samples, u64::from(size) / 4));
        }
        if id == b"fmt " {
            if size < 16 {
                return Err(ProjectError::InvalidTakeFile("fmt chunk too short"));
            }
            match (
                u16_at(body),
                u16_at(body + 2),
                u32_at(body + 4),
                u16_at(body + 14),
            ) {
                (Some(tag), Some(ch), Some(rate), Some(bits)) => {
                    format = Some((tag, ch, rate, bits));
                }
                _ => return Err(truncated),
            }
        }
        let next = body as u64 + u64::from(size) + u64::from(size & 1);
        if next >= MAX_HEADER_SCAN_BYTES {
            return Err(ProjectError::InvalidTakeFile(
                "no data chunk in the first 4 KiB",
            ));
        }
        pos = next as usize;
    }
}

/// Recovers one take file: length from the file size when the header is stale (SPEC-002 AC-6).
pub fn recover_take_file(path: &Path) -> Result<RecoveredTake> {
    let file = File::open(path).map_err(ProjectError::io("opening a take file"))?;
    let file_len = file
        .metadata()
        .map_err(ProjectError::io("opening a take file"))?
        .len();
    let mut head = vec![0u8; file_len.min(MAX_HEADER_SCAN_BYTES) as usize];
    read_exact_at(&file, &mut head, 0).map_err(ProjectError::io("reading a take file"))?;
    let (sample_rate_hz, data_offset, samples, header_samples) =
        parse_take_header(&head, file_len)?;
    Ok(RecoveredTake {
        path: path.to_path_buf(),
        sample_rate_hz,
        data_offset,
        samples,
        header_samples,
    })
}

/// Recovers every existing part of take `take`, in order.
pub fn recover_take(takes_dir: &Path, take: u32) -> Result<Vec<RecoveredTake>> {
    let mut parts = Vec::new();
    for part in 0.. {
        let path = take_part_path(takes_dir, take, part);
        if !path.exists() {
            break;
        }
        parts.push(recover_take_file(&path)?);
    }
    Ok(parts)
}

/// Reads recovered samples `[start, start + out.len())` (clipped); returns the count read.
pub fn read_take_samples(take: &RecoveredTake, start: u64, out: &mut [f32]) -> Result<usize> {
    if start >= take.samples {
        return Ok(0);
    }
    let n = (out.len() as u64).min(take.samples - start) as usize;
    let file = File::open(&take.path).map_err(ProjectError::io("opening a take file"))?;
    let mut bytes = vec![0u8; n * 4];
    read_exact_at(&file, &mut bytes, take.data_offset + start * 4)
        .map_err(ProjectError::io("reading a take file"))?;
    let (words, _) = bytes.as_chunks::<4>();
    for (sample, word) in out.iter_mut().zip(words) {
        *sample = f32::from_le_bytes(*word);
    }
    Ok(n)
}

/// Rewrites a recovered file as a valid WAV: drops a torn trailing partial sample and patches the
/// header to the recovered length.
pub fn repair_take_header(take: &RecoveredTake) -> Result<()> {
    if take.data_offset != TAKE_HEADER_BYTES {
        return Err(ProjectError::InvalidTakeFile(
            "only canonical 44-byte take headers can be repaired",
        ));
    }
    let data_bytes = u32::try_from(take.samples * 4)
        .map_err(|_| ProjectError::InvalidTakeFile("take part exceeds the RIFF limit"))?;
    let file = OpenOptions::new()
        .write(true)
        .open(&take.path)
        .map_err(ProjectError::io("repairing a take file"))?;
    file.set_len(take.data_offset + take.samples * 4)
        .and_then(|()| write_all_at(&file, &take_header(take.sample_rate_hz, data_bytes), 0))
        .and_then(|()| file.sync_data())
        .map_err(ProjectError::io("repairing a take file"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_canonical() {
        let h = take_header(48_000, 400);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 436);
        assert_eq!(
            parse_take_header(&h, 44 + 400).unwrap(),
            (48_000, 44, 100, 100)
        );
        assert_eq!(
            parse_take_header(&h, 44 + 402).unwrap(),
            (48_000, 44, 100, 100)
        );
        assert_eq!(parse_take_header(&h, 44).unwrap(), (48_000, 44, 0, 100));
    }

    #[test]
    fn riff_limit_arithmetic() {
        assert_eq!(MAX_TAKE_DATA_BYTES, 4_294_967_256);
        assert!(36 + MAX_TAKE_DATA_BYTES <= u64::from(u32::MAX));
        assert!(MAX_TAKE_DATA_BYTES.is_multiple_of(4));
    }

    #[test]
    fn garbage_headers_are_errors_not_panics() {
        let h = take_header(48_000, 0);
        for cut in 0..44 {
            assert!(
                parse_take_header(&h[..cut], cut as u64).is_err(),
                "cut {cut}"
            );
        }
        let mut bad = h;
        bad[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_take_header(&bad, 10_000).is_err());
        let mut pcm = h;
        pcm[20] = 1;
        assert!(parse_take_header(&pcm, 100).is_err());
        let mut state = 0x1234_5678u32;
        for _ in 0..2000 {
            let mut junk = h.to_vec();
            for byte in junk.iter_mut().skip(12) {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if state >> 28 == 0 {
                    *byte = (state >> 8) as u8;
                }
            }
            let _ = parse_take_header(&junk, u64::from(state));
        }
    }
}
