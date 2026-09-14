//! Append-only chunk store (ADR-004 §2, §4, §5).
//!
//! One file, `chunks.<gen>.f32`, grows in preallocated 64 MiB **segments**. Each committed chunk
//! (≤ 65 536 f32 LE samples) starts 4 KiB-aligned inside one segment, never straddles two, and is
//! never written again. Segments are mapped read-write on demand (`memmap2::MmapRaw`); writers copy
//! into the mapping, readers copy out of the same mapping (mapped views are coherent). Each chunk
//! has a CRC32 (checked the first time the chunk is read) and a min/max peak pyramid in
//! `peaks.<gen>.bin`.
//!
//! Durability: a committed chunk is readable at once but only durable after
//! [`ChunkStore::sync`]. ADR-004 §2's "flush before a journal record references it" is enforced in
//! one place, [`crate::Session`], which calls `sync()` before appending any record that names new
//! chunks. One sync per commit (instead of one `msync` per chunk) keeps a 60-min import fast.
//!
//! Memory: the store counts its mapped bytes plus external reservations (tile cache, op buffers)
//! against a budget. When mapping a segment would exceed it, the least-recently-used segments
//! that are neither the tail (being written) nor guarded by a reader or writer are unmapped. They
//! are remapped on the next access (SPEC-004 §2.4).

mod peaks;
mod prealloc;
mod writer;

pub use peaks::{ChunkPeaks, PEAK_BUCKETS_PER_CHUNK, PEAK_LEVELS_SPP};
pub use writer::{CancelToken, ChunkWriter, WriterProgress, WrittenAudio};

use std::borrow::Cow;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use memmap2::{MmapOptions, MmapRaw};
use serde::{Deserialize, Serialize};

use crate::snapshot::DocSnapshot;
use crate::{CHUNK_ALIGN_BYTES, CHUNK_SAMPLES, ProjectError, Result, SEGMENT_BYTES};

use prealloc::Reservation;

/// Sequential id of a committed chunk (ADR-004 §2).
pub type ChunkId = u32;

/// Where a committed chunk lives: the payload of the journal's `chunks` record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkLocation {
    pub id: ChunkId,
    /// Byte offset in `chunks.<gen>.f32`.
    pub offset: u64,
    /// Length in samples (1 ..= 65 536).
    pub len: u32,
    /// CRC32 (`crc32fast`) over the chunk's f32 LE bytes.
    pub crc32: u32,
}

const UNVERIFIED: u8 = 0;
const VERIFIED: u8 = 1;
const CORRUPT: u8 = 2;

struct ChunkEntry {
    loc: ChunkLocation,
    state: AtomicU8,
}

impl ChunkEntry {
    /// An id with no chunk behind it (T-301: a reopened or compacted store keeps ids stable and
    /// never reuses one, so ids that were never journaled, or were compacted away, are holes).
    fn hole(id: ChunkId) -> Self {
        ChunkEntry {
            loc: ChunkLocation {
                id,
                offset: 0,
                len: 0,
                crc32: 0,
            },
            state: AtomicU8::new(CORRUPT),
        }
    }

    fn is_hole(&self) -> bool {
        self.loc.len == 0
    }
}

/// Called with the segment index before each segment is preallocated; an `Err` is treated
/// exactly like a failed preallocation. Test-only fault injection (disk full at segment creation).
#[doc(hidden)]
pub type SegmentHook = Arc<dyn Fn(u32) -> io::Result<()> + Send + Sync>;

/// Lower clamp of the default memory budget (SPEC-004 §3 `memory_budget`).
pub const MIN_DEFAULT_MEMORY_BUDGET_BYTES: u64 = 512 * 1024 * 1024;
/// Upper clamp of the default memory budget.
pub const MAX_DEFAULT_MEMORY_BUDGET_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// `clamp(RAM / 4, 512 MiB, 4 GiB)`; 512 MiB when the RAM size is unknown (SPEC-004 §2.4).
pub fn default_memory_budget(total_ram_bytes: Option<u64>) -> u64 {
    total_ram_bytes.map_or(MIN_DEFAULT_MEMORY_BUDGET_BYTES, |ram| {
        (ram / 4).clamp(
            MIN_DEFAULT_MEMORY_BUDGET_BYTES,
            MAX_DEFAULT_MEMORY_BUDGET_BYTES,
        )
    })
}

/// Total physical RAM where it can be read without extra dependencies (Linux `/proc/meminfo`);
/// `None` elsewhere, in which case the composition root may pass its own value.
pub fn detect_total_ram_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
        let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        kib.checked_mul(1024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Construction options of a [`ChunkStore`].
#[derive(Clone)]
pub struct StoreOptions {
    /// Resident-memory budget in bytes (mapped segments + external reservations).
    pub memory_budget_bytes: u64,
    /// Test-only fault-injection hook ([`SegmentHook`]); `None` in production.
    #[doc(hidden)]
    pub segment_hook: Option<SegmentHook>,
}

impl StoreOptions {
    /// Options with the given budget and no hook.
    pub fn with_memory_budget(memory_budget_bytes: u64) -> Self {
        StoreOptions {
            memory_budget_bytes,
            segment_hook: None,
        }
    }
}

impl Default for StoreOptions {
    fn default() -> Self {
        StoreOptions::with_memory_budget(default_memory_budget(detect_total_ram_bytes()))
    }
}

impl fmt::Debug for StoreOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoreOptions")
            .field("memory_budget_bytes", &self.memory_budget_bytes)
            .field("segment_hook", &self.segment_hook.is_some())
            .finish()
    }
}

#[derive(Debug)]
struct Memory {
    budget: AtomicU64,
    mapped: AtomicU64,
    external: AtomicU64,
    peak: AtomicU64,
}

impl Memory {
    fn resident(&self) -> u64 {
        self.mapped.load(Ordering::Relaxed) + self.external.load(Ordering::Relaxed)
    }

    fn note_peak(&self) {
        self.peak.fetch_max(self.resident(), Ordering::Relaxed);
    }
}

/// Bytes counted against the store's memory budget on behalf of another cache (spectrogram
/// tiles, transient op buffers — ADR-004 §4). Released on drop.
pub struct MemoryReservation {
    memory: Arc<Memory>,
    bytes: u64,
}

impl MemoryReservation {
    /// The reserved size.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        self.memory
            .external
            .fetch_sub(self.bytes, Ordering::Relaxed);
    }
}

impl fmt::Debug for MemoryReservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryReservation")
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// A live reference to one mapped segment; while it exists the segment is never unmapped.
pub(crate) struct SegmentGuard {
    seg: u32,
    map: Arc<MmapRaw>,
}

struct SegmentSlot {
    map: Option<Arc<MmapRaw>>,
    last_use: u64,
    /// Written since the last [`ChunkStore::sync`].
    dirty: bool,
}

struct Segments {
    slots: Vec<SegmentSlot>,
    tick: u64,
    /// The segment holding the allocation cursor: never evicted (ADR-004 §4).
    tail: Option<u32>,
}

struct Alloc {
    next_offset: u64,
    segments: u32,
}

/// Maps evicted under the segment lock and dropped (unmapped) after it is released. Dirty
/// victims already started their write-back under the lock (see `evict_locked`).
type Evicted = Vec<Arc<MmapRaw>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read<T>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(PoisonError::into_inner)
}

/// The session's chunk store. Shared as `Arc<ChunkStore>` between the control thread, the reader
/// thread and writer threads (see the crate-level concurrency contract).
pub struct ChunkStore {
    chunks_path: PathBuf,
    peaks_path: PathBuf,
    file: File,
    peaks_file: File,
    /// Lock order: `alloc` → `segments`. Readers never take `alloc`.
    alloc: Mutex<Alloc>,
    segments: Mutex<Segments>,
    index: RwLock<Vec<ChunkEntry>>,
    memory: Arc<Memory>,
    unreserved_segments: AtomicU32,
    /// Serializes `sync()` calls, so one call's flag clearing can't race another's flush.
    sync_lock: Mutex<()>,
    segment_hook: Option<SegmentHook>,
}

impl fmt::Debug for ChunkStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChunkStore")
            .field("chunks_path", &self.chunks_path)
            .field("chunks", &self.chunk_count())
            .field("mapped_bytes", &self.mapped_bytes())
            .finish_non_exhaustive()
    }
}

impl ChunkStore {
    /// Creates `chunks.<generation>.f32` and `peaks.<generation>.bin` in `dir` (both must not
    /// exist yet).
    pub fn create(dir: &Path, generation: u32, options: StoreOptions) -> Result<Arc<ChunkStore>> {
        let chunks_path = dir.join(format!("chunks.{generation}.f32"));
        let peaks_path = dir.join(format!("peaks.{generation}.bin"));
        let open = |path: &Path| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(ProjectError::io("creating the chunk store"))
        };
        let file = open(&chunks_path)?;
        let peaks_file = open(&peaks_path)?;
        Ok(Arc::new(ChunkStore {
            chunks_path,
            peaks_path,
            file,
            peaks_file,
            alloc: Mutex::new(Alloc {
                next_offset: 0,
                segments: 0,
            }),
            segments: Mutex::new(Segments {
                slots: Vec::new(),
                tick: 0,
                tail: None,
            }),
            index: RwLock::new(Vec::new()),
            memory: Arc::new(Memory {
                budget: AtomicU64::new(options.memory_budget_bytes),
                mapped: AtomicU64::new(0),
                external: AtomicU64::new(0),
                peak: AtomicU64::new(0),
            }),
            unreserved_segments: AtomicU32::new(0),
            sync_lock: Mutex::new(()),
            segment_hook: options.segment_hook,
        }))
    }

    /// Reopens `chunks.<generation>.f32` in `dir` for crash recovery (ADR-004 §9) with the chunk
    /// index the journal describes. Ids missing from `locations` become holes (never reused);
    /// a location that doesn't fit the file is marked corrupt. New chunks are appended after the
    /// last indexed one. The peaks file is reopened (or recreated: derived data, ADR-004 §5).
    pub(crate) fn open_existing(
        dir: &Path,
        generation: u32,
        options: StoreOptions,
        locations: &[ChunkLocation],
    ) -> Result<Arc<ChunkStore>> {
        const CONTEXT: &str = "opening the chunk store";
        let chunks_path = dir.join(format!("chunks.{generation}.f32"));
        let peaks_path = dir.join(format!("peaks.{generation}.bin"));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&chunks_path)
            .map_err(ProjectError::io(CONTEXT))?;
        let peaks_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&peaks_path)
            .map_err(ProjectError::io(CONTEXT))?;
        let mut file_len = file.metadata().map_err(ProjectError::io(CONTEXT))?.len();
        if !file_len.is_multiple_of(SEGMENT_BYTES) {
            // A partial segment (an extension interrupted mid-way): complete it so a mapping of
            // the whole segment never reaches past the end of the file.
            file_len = file_len.next_multiple_of(SEGMENT_BYTES);
            file.set_len(file_len).map_err(ProjectError::io(CONTEXT))?;
        }
        let segments = u32::try_from(file_len / SEGMENT_BYTES)
            .map_err(|_| ProjectError::InvalidArgument("store segment space exhausted"))?;
        let len = locations
            .iter()
            .map(|l| l.id as usize + 1)
            .max()
            .unwrap_or(0);
        let mut index: Vec<ChunkEntry> = (0..len).map(|id| ChunkEntry::hole(id as u32)).collect();
        let mut next_offset = 0u64;
        for loc in locations {
            let end = loc.offset + u64::from(loc.len) * 4;
            let fits = loc.len >= 1
                && loc.len as usize <= CHUNK_SAMPLES
                && end <= file_len
                && loc.offset / SEGMENT_BYTES == (end - 1) / SEGMENT_BYTES;
            if fits {
                next_offset = next_offset.max(end);
            }
            index[loc.id as usize] = ChunkEntry {
                loc: *loc,
                state: AtomicU8::new(if fits { UNVERIFIED } else { CORRUPT }),
            };
        }
        let slots = (0..segments)
            .map(|_| SegmentSlot {
                map: None,
                last_use: 0,
                dirty: false,
            })
            .collect();
        Ok(Arc::new(ChunkStore {
            chunks_path,
            peaks_path,
            file,
            peaks_file,
            alloc: Mutex::new(Alloc {
                next_offset,
                segments,
            }),
            segments: Mutex::new(Segments {
                slots,
                tick: 0,
                tail: None,
            }),
            index: RwLock::new(index),
            memory: Arc::new(Memory {
                budget: AtomicU64::new(options.memory_budget_bytes),
                mapped: AtomicU64::new(0),
                external: AtomicU64::new(0),
                peak: AtomicU64::new(0),
            }),
            unreserved_segments: AtomicU32::new(0),
            sync_lock: Mutex::new(()),
            segment_hook: options.segment_hook,
        }))
    }

    /// Path of the chunk file.
    pub fn chunks_path(&self) -> &Path {
        &self.chunks_path
    }

    /// Path of the peaks file.
    pub fn peaks_path(&self) -> &Path {
        &self.peaks_path
    }

    /// A new [`ChunkWriter`] appending to this store.
    pub fn writer(self: &Arc<Self>) -> ChunkWriter {
        ChunkWriter::new(Arc::clone(self))
    }

    /// The next chunk id (ids below it are committed chunks or holes).
    pub fn chunk_count(&self) -> u32 {
        u32::try_from(read(&self.index).len()).unwrap_or(u32::MAX)
    }

    /// Location of a committed chunk (`None` for an unknown id or a hole).
    pub fn location(&self, id: ChunkId) -> Option<ChunkLocation> {
        read(&self.index)
            .get(id as usize)
            .filter(|e| !e.is_hole())
            .map(|e| e.loc)
    }

    /// Locations of all committed chunks, in id order.
    pub fn locations(&self) -> Vec<ChunkLocation> {
        read(&self.index)
            .iter()
            .filter(|e| !e.is_hole())
            .map(|e| e.loc)
            .collect()
    }

    /// Bytes the chunk file occupies (whole preallocated segments).
    pub fn chunk_file_bytes(&self) -> u64 {
        u64::from(self.segment_count()) * SEGMENT_BYTES
    }

    /// Makes the next committed chunk get id `next_id` or higher, so ids are never reused
    /// across a compaction (T-301).
    pub(crate) fn reserve_ids_below(&self, next_id: ChunkId) {
        let mut index = write(&self.index);
        while index.len() < next_id as usize {
            let id = index.len() as ChunkId;
            index.push(ChunkEntry::hole(id));
        }
    }

    /// Number of preallocated segments (the chunk file is `segment_count × 64 MiB` long).
    pub fn segment_count(&self) -> u32 {
        lock(&self.alloc).segments
    }

    /// Segments whose disk space could not be reserved (the file system doesn't support
    /// preallocation, see `store::prealloc`): writes into them could still hit a full disk as
    /// `SIGBUS`. The engine may warn when this is non-zero.
    pub fn unreserved_segments(&self) -> u32 {
        self.unreserved_segments.load(Ordering::Relaxed)
    }

    /// Commits one chunk (ADR-004 §2): peaks + CRC32 from the samples in hand, copy into the tail
    /// segment's mapping, mark the segment dirty, then publish the index entry. The chunk is
    /// readable immediately; it is durable after the next [`Self::sync`].
    pub(crate) fn commit_chunk(&self, samples: &[f32]) -> Result<ChunkLocation> {
        self.commit_chunk_as(samples, None)
    }

    /// [`Self::commit_chunk`] under a given id, which must be a hole or past the end of the
    /// index (compaction copies live chunks keeping their ids, ADR-004 §4).
    pub(crate) fn commit_chunk_with_id(
        &self,
        id: ChunkId,
        samples: &[f32],
    ) -> Result<ChunkLocation> {
        self.commit_chunk_as(samples, Some(id))
    }

    fn commit_chunk_as(&self, samples: &[f32], fixed_id: Option<ChunkId>) -> Result<ChunkLocation> {
        if samples.is_empty() || samples.len() > CHUNK_SAMPLES {
            return Err(ProjectError::InvalidArgument(
                "a chunk holds 1 ..= 65 536 samples",
            ));
        }
        let len = samples.len() as u32;
        let byte_len = samples.len() * 4;
        let peaks = ChunkPeaks::compute(samples);
        let bytes = le_bytes(samples);
        let crc32 = crc32fast::hash(&bytes);

        let (offset, seg) = self.reserve(byte_len as u64)?;
        let guard = self.acquire_segment(seg)?;
        let in_seg = (offset - u64::from(seg) * SEGMENT_BYTES) as usize;
        // SAFETY: `reserve` handed out `[offset, offset + byte_len)` exclusively to this call and
        // never lets a chunk straddle a segment, so `[in_seg, in_seg + byte_len)` lies inside this
        // 64 MiB mapping. No reader can look at the range before the index entry is published
        // below, and `guard` keeps the mapping alive during the copy.
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                guard.map.as_mut_ptr().add(in_seg),
                byte_len,
            );
        }
        // Mark dirty before publishing, so any `sync()` that starts after the chunk is visible
        // covers it.
        if let Some(slot) = lock(&self.segments).slots.get_mut(seg as usize) {
            slot.dirty = true;
        }
        drop(guard);

        let loc = {
            let mut index = write(&self.index);
            let id = match fixed_id {
                Some(id) => {
                    while index.len() <= id as usize {
                        let hole = index.len() as ChunkId;
                        index.push(ChunkEntry::hole(hole));
                    }
                    if !index[id as usize].is_hole() {
                        return Err(ProjectError::InvalidArgument("chunk id already in use"));
                    }
                    id
                }
                None => ChunkId::try_from(index.len())
                    .map_err(|_| ProjectError::InvalidArgument("chunk id space exhausted"))?,
            };
            let loc = ChunkLocation {
                id,
                offset,
                len,
                crc32,
            };
            let entry = ChunkEntry {
                loc,
                state: AtomicU8::new(UNVERIFIED),
            };
            if (id as usize) < index.len() {
                index[id as usize] = entry;
            } else {
                index.push(entry);
            }
            loc
        };
        // Peaks are derived data (never fsynced, recomputed when missing or corrupt, ADR-004 §5),
        // so failing to write them must not fail the commit.
        let _ = peaks.write_record(&self.peaks_file, loc.id);
        Ok(loc)
    }

    /// Makes every chunk committed so far durable (ADR-004 §2 step 2): starts write-back of each
    /// dirty mapped segment (`msync(MS_ASYNC)`, `FlushViewOfFile` on Windows), then one
    /// `fdatasync` of the chunk file (`F_FULLFSYNC` on macOS, through std). A journal record may
    /// reference a chunk only after a `sync()` that started after the chunk was committed.
    pub fn sync(&self) -> Result<()> {
        let _serialized = lock(&self.sync_lock);
        let dirty: Vec<(usize, Option<Arc<MmapRaw>>)> = {
            let mut segments = lock(&self.segments);
            segments
                .slots
                .iter_mut()
                .enumerate()
                .filter(|(_, slot)| slot.dirty)
                .map(|(i, slot)| {
                    slot.dirty = false;
                    (i, slot.map.clone())
                })
                .collect()
        };
        let result = dirty
            .iter()
            .try_for_each(|(_, map)| map.as_ref().map_or(Ok(()), |m| m.flush_async()))
            .and_then(|()| self.file.sync_data());
        if let Err(e) = result {
            let mut segments = lock(&self.segments);
            for (i, _) in &dirty {
                if let Some(slot) = segments.slots.get_mut(*i) {
                    slot.dirty = true;
                }
            }
            return Err(ProjectError::from_io("syncing the chunk store", e));
        }
        Ok(())
    }

    /// Segments written since the last [`Self::sync`].
    pub fn unsynced_segments(&self) -> usize {
        lock(&self.segments)
            .slots
            .iter()
            .filter(|s| s.dirty)
            .count()
    }

    /// Reserves a 4 KiB-aligned slot of `byte_len` bytes that doesn't straddle a segment,
    /// preallocating new segments as needed. Returns `(file offset, segment index)`.
    fn reserve(&self, byte_len: u64) -> Result<(u64, u32)> {
        let mut alloc = lock(&self.alloc);
        let mut offset = alloc.next_offset.next_multiple_of(CHUNK_ALIGN_BYTES);
        let mut seg = offset / SEGMENT_BYTES;
        if offset + byte_len > (seg + 1) * SEGMENT_BYTES {
            seg += 1;
            offset = seg * SEGMENT_BYTES;
        }
        let seg = u32::try_from(seg)
            .map_err(|_| ProjectError::InvalidArgument("store segment space exhausted"))?;
        while alloc.segments <= seg {
            self.create_segment(alloc.segments)?;
            alloc.segments += 1;
        }
        alloc.next_offset = offset + byte_len;
        lock(&self.segments).tail = Some(seg);
        Ok((offset, seg))
    }

    /// Preallocates segment `index` (ADR-004 §2): disk-full surfaces here as an error instead of
    /// as `SIGBUS` on a later write through the mapping.
    fn create_segment(&self, index: u32) -> Result<()> {
        const CONTEXT: &str = "creating a store segment";
        if let Some(hook) = &self.segment_hook {
            hook(index).map_err(ProjectError::io(CONTEXT))?;
        }
        let offset = u64::from(index) * SEGMENT_BYTES;
        match prealloc::preallocate(&self.file, offset, SEGMENT_BYTES) {
            Ok(Reservation::Reserved) => {}
            Ok(Reservation::Unreserved) => {
                self.unreserved_segments.fetch_add(1, Ordering::Relaxed);
            }
            Err(e) => {
                // Roll back a partial extension so the file stays a whole number of segments.
                let _ = self.file.set_len(offset);
                return Err(ProjectError::from_io(CONTEXT, e));
            }
        }
        lock(&self.segments).slots.push(SegmentSlot {
            map: None,
            last_use: 0,
            dirty: false,
        });
        Ok(())
    }

    /// A guard on segment `seg`, mapping it (after evicting LRU segments over budget) if needed.
    pub(crate) fn acquire_segment(&self, seg: u32) -> Result<SegmentGuard> {
        let mut evicted = Vec::new();
        let result = self.acquire_segment_locked(seg, &mut evicted);
        // Unmap evicted segments only after the segment-table lock is released.
        drop(evicted);
        result
    }

    fn acquire_segment_locked(&self, seg: u32, evicted: &mut Evicted) -> Result<SegmentGuard> {
        let mut segments = lock(&self.segments);
        segments.tick += 1;
        let tick = segments.tick;
        match segments.slots.get_mut(seg as usize) {
            None => return Err(ProjectError::InvalidArgument("segment does not exist")),
            Some(slot) => {
                if let Some(map) = &slot.map {
                    let map = Arc::clone(map);
                    slot.last_use = tick;
                    return Ok(SegmentGuard { seg, map });
                }
            }
        }
        self.evict_locked(&mut segments, SEGMENT_BYTES, evicted);
        let map = MmapOptions::new()
            .offset(u64::from(seg) * SEGMENT_BYTES)
            .len(SEGMENT_BYTES as usize)
            .map_raw(&self.file)
            .map_err(ProjectError::io("mapping a store segment"))?;
        let map = Arc::new(map);
        let slot = &mut segments.slots[seg as usize];
        slot.map = Some(Arc::clone(&map));
        slot.last_use = tick;
        self.memory
            .mapped
            .fetch_add(SEGMENT_BYTES, Ordering::Relaxed);
        self.memory.note_peak();
        Ok(SegmentGuard { seg, map })
    }

    /// Unmaps LRU segments until `incoming` more bytes fit the budget, skipping the tail and any
    /// guarded segment. Evicted maps are pushed to `evicted` so the caller unmaps them after
    /// releasing the lock. A dirty segment stays marked dirty for the next [`Self::sync`].
    fn evict_locked(&self, segments: &mut Segments, incoming: u64, evicted: &mut Evicted) {
        let budget = self.memory.budget.load(Ordering::Relaxed);
        while self.memory.resident() + incoming > budget {
            let tail = segments.tail;
            let victim = segments
                .slots
                .iter()
                .enumerate()
                .filter(|(i, slot)| {
                    Some(*i as u32) != tail
                        // Guards clone the Arc only under this lock, so a count of 1 (the table's
                        // own reference) means no reader or writer holds the segment.
                        && slot.map.as_ref().is_some_and(|m| Arc::strong_count(m) == 1)
                })
                .min_by_key(|(_, slot)| slot.last_use)
                .map(|(i, _)| i);
            let Some(i) = victim else { break };
            let slot = &mut segments.slots[i];
            if let Some(map) = slot.map.take() {
                if slot.dirty {
                    // Start write-back of a dirty view before it becomes unreachable (on Windows
                    // its dirty pages must be flushed through the view). Done under the segment
                    // lock, so a concurrent `sync()` never sees it unmapped but not yet flushed.
                    let _ = map.flush_async();
                }
                evicted.push(map);
                self.memory
                    .mapped
                    .fetch_sub(SEGMENT_BYTES, Ordering::Relaxed);
            }
        }
    }

    /// Copies `out.len()` samples of chunk `id` starting at sample `offset` into `out`, verifying
    /// the chunk's CRC32 the first time it is read. `cache` keeps the last segment guard between
    /// calls (see [`crate::SnapshotReader`]).
    pub(crate) fn read_chunk_into(
        &self,
        id: ChunkId,
        offset: u32,
        out: &mut [f32],
        cache: &mut Option<SegmentGuard>,
    ) -> Result<()> {
        let (loc, state) = {
            let index = read(&self.index);
            let entry = index
                .get(id as usize)
                .filter(|e| !e.is_hole())
                .ok_or(ProjectError::UnknownChunk(id))?;
            (entry.loc, entry.state.load(Ordering::Acquire))
        };
        if u64::from(offset) + out.len() as u64 > u64::from(loc.len) {
            return Err(ProjectError::OutOfBounds {
                chunk: id,
                offset,
                len: u32::try_from(out.len()).unwrap_or(u32::MAX),
            });
        }
        if state == CORRUPT {
            return Err(ProjectError::ChecksumMismatch { chunk: id });
        }
        let seg = (loc.offset / SEGMENT_BYTES) as u32;
        if !matches!(cache, Some(g) if g.seg == seg) {
            // Release the old guard first so its segment is evictable while mapping the new one.
            *cache = None;
            *cache = Some(self.acquire_segment(seg)?);
        }
        let Some(guard) = cache.as_ref() else {
            return Err(ProjectError::InvalidArgument("segment guard missing"));
        };
        let in_seg = (loc.offset - u64::from(seg) * SEGMENT_BYTES) as usize;
        // SAFETY: a chunk never straddles a segment, so its bytes `[in_seg, in_seg + len * 4)`
        // lie inside this mapping, which `guard` keeps alive. Committed chunk bytes are never
        // written again, and the index entry was published (under the index lock) only after the
        // bytes were written, so these reads can't race a write.
        let base = unsafe { guard.map.as_ptr().add(in_seg) };
        if state != VERIFIED {
            // SAFETY: as above, the chunk's `len * 4` committed bytes.
            let bytes = unsafe { std::slice::from_raw_parts(base, loc.len as usize * 4) };
            let ok = crc32fast::hash(bytes) == loc.crc32;
            if let Some(entry) = read(&self.index).get(id as usize) {
                entry
                    .state
                    .store(if ok { VERIFIED } else { CORRUPT }, Ordering::Release);
            }
            if !ok {
                return Err(ProjectError::ChecksumMismatch { chunk: id });
            }
        }
        // SAFETY: `offset + out.len() <= loc.len` was checked above, so the source range lies
        // inside the chunk's committed bytes.
        unsafe { copy_le_f32(base.add(offset as usize * 4), out) };
        Ok(())
    }

    /// Copies samples `[offset, offset + out.len())` of chunk `id` into `out`.
    pub fn read_chunk(&self, id: ChunkId, offset: u32, out: &mut [f32]) -> Result<()> {
        self.read_chunk_into(id, offset, out, &mut None)
    }

    /// Recomputes chunk `id`'s CRC32 from the store now (even if it was verified before).
    pub fn verify_chunk(&self, id: ChunkId) -> Result<()> {
        match read(&self.index).get(id as usize) {
            Some(entry) if !entry.is_hole() => entry.state.store(UNVERIFIED, Ordering::Release),
            _ => return Err(ProjectError::UnknownChunk(id)),
        }
        self.read_chunk_into(id, 0, &mut [], &mut None)
    }

    /// Copies `[pos, pos + out.len())` of `snapshot` into `out` (clipped at its end) and returns
    /// the number of samples written. Prefer a [`crate::SnapshotReader`] for streaming, which keeps
    /// its segment guard between calls.
    pub fn read(&self, snapshot: &DocSnapshot, pos: u64, out: &mut [f32]) -> Result<usize> {
        crate::reader::read_range(self, snapshot, pos, out, &mut None)
    }

    /// The peak pyramid of chunk `id`, from `peaks.<gen>.bin`, or recomputed from the chunk when
    /// the record is missing or corrupt (ADR-004 §5).
    pub fn chunk_peaks(&self, id: ChunkId) -> Result<ChunkPeaks> {
        let loc = self.location(id).ok_or(ProjectError::UnknownChunk(id))?;
        if let Some(peaks) = ChunkPeaks::read_record(&self.peaks_file, id)
            && peaks.len_samples() == loc.len
        {
            return Ok(peaks);
        }
        let mut samples = vec![0.0; loc.len as usize];
        self.read_chunk(id, 0, &mut samples)?;
        let peaks = ChunkPeaks::compute(&samples);
        let _ = peaks.write_record(&self.peaks_file, id);
        Ok(peaks)
    }

    /// Current memory budget in bytes.
    pub fn memory_budget(&self) -> u64 {
        self.memory.budget.load(Ordering::Relaxed)
    }

    /// Changes the memory budget; applies immediately (evicting unguarded segments if needed).
    pub fn set_memory_budget(&self, bytes: u64) {
        self.memory.budget.store(bytes, Ordering::Relaxed);
        self.evict_to_budget();
    }

    fn evict_to_budget(&self) {
        let mut evicted = Vec::new();
        {
            let mut segments = lock(&self.segments);
            self.evict_locked(&mut segments, 0, &mut evicted);
        }
        drop(evicted);
        self.memory.note_peak();
    }

    /// Bytes of currently mapped segments.
    pub fn mapped_bytes(&self) -> u64 {
        self.memory.mapped.load(Ordering::Relaxed)
    }

    /// The resident-memory counter of SPEC-004 §4: mapped bytes + external reservations.
    pub fn resident_bytes(&self) -> u64 {
        self.memory.resident()
    }

    /// Highest [`Self::resident_bytes`] observed since creation or the last reset.
    pub fn peak_resident_bytes(&self) -> u64 {
        self.memory.peak.load(Ordering::Relaxed)
    }

    /// Resets [`Self::peak_resident_bytes`] to the current value.
    pub fn reset_peak_resident(&self) {
        self.memory
            .peak
            .store(self.memory.resident(), Ordering::Relaxed);
    }

    /// Number of currently mapped segments.
    pub fn mapped_segments(&self) -> usize {
        lock(&self.segments)
            .slots
            .iter()
            .filter(|s| s.map.is_some())
            .count()
    }

    /// Counts `bytes` of another cache against the budget until the reservation is dropped,
    /// evicting unguarded segments to make room.
    pub fn reserve_external(&self, bytes: u64) -> MemoryReservation {
        self.memory.external.fetch_add(bytes, Ordering::Relaxed);
        self.evict_to_budget();
        MemoryReservation {
            memory: Arc::clone(&self.memory),
            bytes,
        }
    }
}

/// The f32 LE encoding of `samples` (borrowed on little-endian targets).
fn le_bytes(samples: &[f32]) -> Cow<'_, [u8]> {
    #[cfg(target_endian = "little")]
    {
        // SAFETY: `f32` is 4 bytes with no padding, `u8` has alignment 1, and the slice covers
        // exactly the `samples.len() * 4` initialized bytes of `samples` for its lifetime. On a
        // little-endian target these bytes are the f32 LE encoding.
        Cow::Borrowed(unsafe {
            std::slice::from_raw_parts(samples.as_ptr().cast::<u8>(), samples.len() * 4)
        })
    }
    #[cfg(not(target_endian = "little"))]
    {
        Cow::Owned(samples.iter().flat_map(|s| s.to_le_bytes()).collect())
    }
}

/// Copies `out.len()` f32 LE samples from `src` into `out`.
///
/// # Safety
/// `src` must be valid for reads of `out.len() * 4` bytes.
unsafe fn copy_le_f32(src: *const u8, out: &mut [f32]) {
    #[cfg(target_endian = "little")]
    // SAFETY: the caller guarantees `src` is readable for `out.len() * 4` bytes; the destination
    // is `out`'s own storage, a byte copy has no alignment requirement, and every bit pattern is a
    // valid `f32`.
    unsafe {
        std::ptr::copy_nonoverlapping(src, out.as_mut_ptr().cast::<u8>(), out.len() * 4);
    }
    #[cfg(not(target_endian = "little"))]
    for (i, sample) in out.iter_mut().enumerate() {
        let mut b = [0u8; 4];
        // SAFETY: `i * 4 + 4 <= out.len() * 4`, within the range the caller guarantees.
        unsafe { std::ptr::copy_nonoverlapping(src.add(i * 4), b.as_mut_ptr(), 4) };
        *sample = f32::from_le_bytes(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget_clamps() {
        const GIB: u64 = 1024 * 1024 * 1024;
        assert_eq!(default_memory_budget(None), 512 * 1024 * 1024);
        assert_eq!(default_memory_budget(Some(GIB)), 512 * 1024 * 1024);
        assert_eq!(default_memory_budget(Some(8 * GIB)), 2 * GIB);
        assert_eq!(default_memory_budget(Some(64 * GIB)), 4 * GIB);
    }
}
