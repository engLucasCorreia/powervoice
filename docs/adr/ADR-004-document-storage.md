# ADR-004 — Document storage, snapshots, undo, journal
- Status: accepted (owner, M0 checkpoint 2026-09-12)
- Date: 2026-09-12
- Deciders: owner, orchestrator

## Context
- **Size.** 60 min of mono audio at 48 kHz is 172.8 M f32 samples ≈ **691 MB**. Holding a whole-file
  normalize in RAM would double that.
- **Requirements (§3.3, §3.6):**
  - unlimited undo within a memory budget;
  - destructive edits, including bake;
  - autosave and crash recovery;
  - open 60 min in < 3 s with a progressive waveform.
- **Playback and recording (ADR-002).** The audio thread never touches the store. The reader thread
  streams snapshots.
- **Storage is not persistence.** The plain audio file and the `name.vo.json` sidecar remain the
  user's persistence format (§2). This ADR covers the working store behind them.

## Decision

### 1. Session directory
Each open document has one session directory at `<app_local_data_dir>/sessions/<session-id>/`.
The root is passed in by the composition root (Tauri `app_local_data_dir()` for the identifier
`app.powervoice.editor`), so `project` has no Tauri or `directories` dependency.

```
session.lock          PID, host, start time; OS advisory lock held while the session is open
meta.json             store format version, current generation, source path, doc rate
chunks.<gen>.f32      append-only chunk store (64 MiB segments)
peaks.<gen>.bin       append-only per-chunk peak pyramids (derived data, never fsynced)
journal.<gen>.jsonl   append-only edit journal
takes/take-NNNN.wav   crash-safe capture files
```

Rationale for the location:
- **Local data dir, not the cache dir.** OS or cleanup tools may purge cache dirs, and recovery data
  must survive.
- **Not next to the audio file.** Synced folders, network shares and removable media are unsafe for
  memory mapping and multi-GB scratch data.
- **Local, not roaming.** `%LOCALAPPDATA%` on Windows keeps it off roaming profiles.

### 2. Chunks: **65 536 samples (256 KiB)**
- **Immutability.** A chunk is up to 65 536 f32 LE samples, and is immutable once committed.
  `ChunkId = u32`, assigned sequentially.
- **Placement.**
  - Chunks start 4 KiB-aligned inside 64 MiB segments and never straddle a segment.
  - Waste is ≤ 4 KiB per chunk plus ≤ 256 KiB per segment.
  - A 60-min document is 2 637 chunks in 11 segments.
- **Rationale for 64 k over 256 k.**
  - Pieces can reference any sub-range of a chunk, so chunk size does not force rewriting unchanged
    audio at any size. Size only sets the granularity of peaks, CRCs, recovery and eviction.
  - With 64 k, a recording chunk commits every 1.37 s, which is the live-peak and recovery
    granularity.
  - One chunk equals the top level of the peak pyramid (65 536 spp).
  - 256 KiB is a multiple of Windows' 64 KiB mapping granularity.
  - Index overhead stays trivial (< 3 k entries per hour).
- **Writes.** Writes go through a read-write `memmap2` mapping of the tail segment, and reads use the
  same mapping.
  - Mapped views are coherent with each other on every OS, whereas the Win32 docs say mapped files
    and `ReadFile`/`WriteFile` "are not necessarily coherent"
    (https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-createfilemappinga).
  - To avoid SIGBUS when the disk is full, each new segment is **preallocated**: `posix_fallocate` on
    Linux, `F_PREALLOCATE` on macOS (through `libc`), and `SetEndOfFile` on Windows. Disk-full then
    surfaces as a normal error at segment creation.
- **Commit.** A chunk is committed when it is full or its producer finishes:
  1. compute its peaks and CRC32;
  2. flush the range (`msync`/`FlushViewOfFile`);
  3. only then may a journal record reference it.

### 3. Snapshots and the piece table
```rust
pub enum Source { Chunk(ChunkId), Silence }            // Silence: no storage at all
pub struct Piece { pub source: Source, pub offset: u32, pub len: u32 }
pub struct Marker { pub id: MarkerId, pub pos_samples: u64, pub len_samples: u64, pub name: Arc<str> }
pub struct DocSnapshot {
    pub rev: u64,             // bumps on every change
    pub audio_rev: u64,       // bumps only when samples change (IPC staleness, ADR-003)
    pub sample_rate_hz: u32,
    pub pieces: Arc<[Piece]>,
    pub starts: Arc<[u64]>,   // prefix sums → O(log n) position lookup
    pub len_samples: u64,
    pub markers: Arc<[Marker]>,
}
// shared as Arc<DocSnapshot>
```
- **Edits** build a new `Vec<Piece>`, which is O(pieces) and < 1 ms for 20 k pieces.
  - New audio is streamed by a `ChunkWriter` on a worker thread, cancellable with progress. A
    whole-file normalize holds a few chunks in RAM and writes 691 MB to disk, not to memory.
  - The control thread commits by swapping the current `Arc`.
  - Copy, paste, cut and trim are pure piece splices with no sample I/O. The clipboard is a
    `Vec<Piece>`.
- **Markers** live in the snapshot, so they are undoable. A marker-only edit shares `pieces`.
  - Every audio edit also applies marker shifts.
  - Proposed default, to be confirmed by the editing spec: markers at or after an insertion point
    shift right; markers inside a deleted range move to its start; markers after it shift left.
- **Destructive edits stop playback.**
  1. The control thread stops the transport (≈ 5 ms fade) and waits for the audio thread's
     acknowledgement.
  2. It commits the edit.
  3. It hands the new snapshot to the reader.

  Marker edits, and undo/redo of marker-only entries, do not stop playback. Destructive edits are
  refused while recording.

### 4. Undo, memory budget, eviction, disk budget
- **Undo stacks.** Undo and redo are stacks of `Arc<DocSnapshot>`. Undo pushes the current snapshot
  onto redo, and a new edit clears redo. Snapshots are only piece tables, so **undo depth is not
  bounded by memory**.
- **The memory budget bounds resident data.** Default `clamp(RAM/4, 512 MiB, 4 GiB)`, configurable.
  It counts:
  - mapped store segments;
  - the spectrogram tile cache;
  - transient op buffers.
- **Eviction.**
  - Segments are mapped lazily. Readers hold `Arc` segment guards.
  - Over budget, the least-recently-used segments with no live guard are unmapped. Their pages are
    clean, so the data stays on disk and is remapped on demand.
  - The tile cache is LRU.
  - The tail segment and segments backing the playback position are never evicted.
- **Disk budget.** Undo depth is bounded only by disk.
  - Soft limit: max(8 GiB, 8 × document bytes).
  - Keep ≥ 2 GiB free on the volume.
- **When the disk limit is exceeded:**
  1. **Compact** if unreachable chunks are ≥ 25 %. Chunks become unreachable after redo truncation,
     or when they are referenced by no current, undo, redo or clipboard snapshot.
  2. Otherwise, drop the oldest undo entries (with a notice), then compact.
- **Compaction** runs on a worker, never while recording, and is crash-safe by generation:
  1. Copy live chunks into `chunks.<gen+1>` (chunk ids are kept; only locations change).
  2. Write `journal.<gen+1>` starting with a checkpoint; fsync both.
  3. Atomically replace `meta.json` (temp file + rename).
  4. Delete the old generation.

### 5. Peaks cached per chunk id
When a chunk commits, its min/max pyramid is computed from the data already in hand. Levels are
spp = 64, 256, 1024, 4096, 16384, 65536: 1 365 buckets × 8 bytes ≈ 10.9 KB per chunk, ≈ 29 MB per
hour. Each pyramid is appended to `peaks.<gen>.bin`, keyed by chunk id. **Only new chunks ever
compute peaks.** Edits reuse every untouched chunk's pyramid.

Document-level buckets are the conservative union (min of mins, max of maxes) of the chunk buckets
that overlap each piece's range:
- they never under-report amplitude;
- they are exact to ±1 bucket in time, which is ≤ 1 px because the UI picks spp ≤ pixel spp;
- below 64 spp the UI reads raw samples (ADR-003);
- silence pieces are zero.

If the peaks file is missing or corrupt, pyramids are recomputed.

### 6. Journal and fsync policy
- **Format.** One record per line: `<crc32 hex>\t<compact JSON>\n`, CRC32 over the JSON bytes
  (`crc32fast`). JSON keeps it inspectable, and records are small because piece-table *deltas* are
  stored.
- **Records:**
  - `open` (source path, size, mtime, format, rate)
  - `chunks` (batched: id, byte offset, len, crc32)
  - `edit {seq, label_key, ops: [Replace{at, remove_len, pieces}], marker_ops}`
  - `undo` / `redo {seq}`
  - `take_begin {take, mode: new|insert|overwrite|punch, at, len}`
  - `state` (sidecar-level rack/view state; not in the document undo history in v1, since whether
    rack edits are undoable is ADR-005's open question 3)
  - `saved {path, seq, format}`
  - `checkpoint` (full chunk index, stacks, current snapshot)
  - `close`
- **Checkpoints** are written at session start, after compaction, and every 256 records, which bounds
  replay time.

| What | Durability action |
|---|---|
| Chunk data | Flushed before any record references it |
| `edit`/`undo`/`redo`/marker/`saved`/`close` | Append + `fdatasync` immediately, before the command reports success (control thread; ~ms on SSD) |
| `state` (rack/view) | Debounced 2 s, then `fdatasync` |
| Take WAV | Data appended continuously; RIFF/data sizes patched + `fdatasync` every ~1 s |
| `peaks` file | Never fsynced (derived) |

Guarantees:
- Every edit whose command succeeded survives a crash.
- A recording loses at most the last **250 ms** on a process crash (audio still in the capture
  ring / writer hands; the writer drains ≥ every 50 ms) and at most **~1.5 s** on power loss
  (header patched + `fdatasync` every ~1 s). Everything recovered is bit-identical to what was
  captured (SPEC-002 §2.3, AC-6; amended at T-008 — previously "loses nothing").

### 7. Recording
1. **Start.** Create `takes/take-NNNN.wav` as 32-bit float at the document rate: lossless for any
   device format, with no dither decision at capture time. Journal `take_begin`.
2. **Capture.** The capture-writer drains the 10 s capture ring (ADR-002), resampling if the device
   rate differs.
   - It appends to the WAV, patching the header every ~1 s.
   - It also appends to the chunk store through a `ChunkWriter`, which drives live `VXRP` peaks.
   - Beyond the 4 GiB RIFF limit it rolls over to the next take file.
3. **Stop.** Commit the final chunk, patch and fsync the WAV, then journal `chunks` + one `edit`.
   **The take becomes one undoable edit.** The WAV stays as a backup until the session is
   garbage-collected.
4. **Failure.** Device loss or capture-ring overflow finalizes the take at the last good sample,
   commits it and posts a notice.

### 8. Open and save
- **Open = import.**
  - A worker decodes the file through `io`, downmixing if needed (T-202). It converts to f32 at the
    source rate, so the document rate is the source rate.
  - It writes chunks and peaks in one sequential pass and emits `peaks_progress`, so the waveform
    appears progressively.
  - Playback and editing unlock when the pass completes. The imported state is the undo floor.
  - Estimate for 60 min at 24-bit: ≈ 518 MB read and 691 MB written, about 1–2 s on NVMe.
  - The source file is never mapped or kept open, so it can be saved over safely on every OS.
- **Save.**
  1. A worker renders the current `Arc<DocSnapshot>` to `<target dir>/.<name>.powervoice-tmp-<pid>`
     through `io`. Bit depth and dither follow the WAV/export specs. Editing may continue, because
     snapshots are immutable.
  2. `fdatasync`.
  3. `rename` over the target: `MoveFileExW(REPLACE_EXISTING)` via `std::fs::rename` on Windows,
     then fsync the directory on POSIX.
  4. Write the sidecar the same way (T-306).
  5. Journal `saved`. `dirty` means the current `seq` ≠ the last saved `seq`.
- **Save does not render the rack.** Only export does (ADR-001 jobs). The undo history survives a
  save.

### 9. Crash recovery
1. **Scan sessions at start-up.** Skip any whose advisory lock is held (another instance is using it).
2. **Classify each unlocked session:**
   - **Clean:** it ends in `close`, or has no unsaved `seq` and no uncommitted take. Garbage-collect it.
   - **Recoverable:** anything else.
3. **Offer recoverable sessions** in a dialog listing the file name, last edit time, number of unsaved
   edits and whether a take is present. The choices are Recover or Discard.
4. **Recover:**
   1. Read `meta.json` for the generation.
   2. Replay the journal from the last valid checkpoint. Stop at the first bad CRC or torn line and
      truncate there.
   3. Verify that every referenced chunk lies within the file and matches its CRC.
   4. Rebuild the current snapshot, undo/redo stacks, markers and rack/view `state`.
   5. A `take_begin` without its edit becomes a separate recovered take. The WAV is authoritative,
      with its length inferred from the file size when the header is stale. The user chooses where
      it goes.
   6. The document opens as *modified*, bound to its original path.
5. **Discard** garbage-collects the session.

### 10. Garbage collection
A session directory is deleted in three cases:
- on normal document close after save or discard (journal `close`);
- at start-up, when it is classified clean;
- when the user discards it from the recovery dialog or from Settings → "Recovery data (X GB) — clear".

Recoverable sessions are **never deleted silently**.

## Consequences
**Positive**
- RAM use is independent of document length and undo depth. A 691 MB normalize costs disk, not
  memory.
- Undo and redo are pointer swaps.
- Edits recompute peaks only for new chunks.
- Crash recovery is exact for committed edits, and the take WAV is a user-visible safety net.

**Negative**
- Import costs a full write of the file (1.33× for 24-bit sources).
- Disk use grows with history until compaction.
- Mapped-file I/O errors still raise SIGBUS. This is accepted because the session directory is on
  local disk.
- Preallocation needs a little `unsafe` code per OS.

**Follow-ups**
- T-101: chunk store, snapshots, take writer.
- T-301: undo, budgets, journal, recovery.
- T-203: peaks.
- T-302: preserving the clipboard across document switches (materialize its samples before GC).
- New dependencies named here: `memmap2 0.9`, `crc32fast`, `libc` (unix preallocation). They go into
  ADR-007's licence table.

## Alternatives considered
- **16 k or 256 k chunks**:
  - 16 k means 4× more index, CRC and peak entries, and recording chunks every 0.34 s.
  - 256 k means 5.5 s recovery/peak granularity for recordings and coarser eviction, with no
    edit-cost benefit because pieces address sub-ranges.
- **Whole document in RAM**: fails the 691 MB / normalize-doubling constraint.
- **Referencing the source file instead of importing**: faster open, but compressed and 16/24-bit
  sources would need decoding on read, the reader would page-fault in a foreign file, and it breaks
  save-over-source on Windows.
- **Cache directory or next to the audio file**: purgeable, or unsafe for mmap/sync (see §1).
- **Undo as inverse operations (command pattern)**: more code per op and error-prone. Snapshots are
  exact and cheap.
- **SQLite for journal and chunks**: robust transactions, but no zero-copy mmap reads, a heavier
  dependency, and we need only append-only semantics.
- **Writing via `pwrite` with an mmap only for reads**: coherent on Linux/macOS but not guaranteed on
  Windows.
- **Full piece table per journal record**: up to hundreds of KB per edit on fragmented documents.
  Deltas plus periodic checkpoints are smaller.

## Open questions (owner)
1. **Disk pressure policy.** Is it acceptable to drop the oldest undo steps automatically (with a
   notice) when the session exceeds the disk budget (proposed max(8 GiB, 8 × document size), ≥ 2 GiB
   free)? The alternative is to only warn and let the disk fill.
2. **Recovery retention.** Keep recoverable sessions until the user discards them (proposed), or
   auto-delete them after N days?
3. **Import cost.** Opening converts the whole file into the session store, using disk ≈ 1.33× the
   file for 24-bit sources. Is that acceptable, including for users on slow HDDs, where the < 3 s
   target may not hold?

## Amendment 1 — M0 checkpoint (2026-09-12): bake undo carries the rack (SPEC-004 OD-4 = A)
The owner decided that rack edits are not part of the document undo history, except that undoing a
**bake** restores the pre-bake rack. Therefore every undo entry and every journal `edit` record may
carry an optional opaque `attachment: Option<Vec<u8>>`. `project` stores and returns it verbatim and
never interprets it (ADR-001: `project` must not depend on `rack`). The engine supplies the serialized
pre-bake `RackModel` when it commits a bake and consumes it on undo/redo. Recovery restores
attachments exactly like the rest of the journal.

## Amendment 2 — T-300 (SPEC-009/018), 2026-09-13
- **Marker kind:** `Marker` gains `kind: MarkerKind` (`User`, `Dropout`, plus `Unknown(String)` kept
  verbatim so kinds written by newer versions survive a round trip). Point vs region stays derived from
  `len_samples` (0 = point). Markers are kept sorted by `(pos_samples, id)`. Implemented in T-303.
- **Journal `saved` record** gains additive fields `audio_crc32` (CRC-32 of the samples as written, used
  by the sidecar identity check) and `sidecar` (path + whether it was written). Implemented in T-306.
- The rack and view sections of the sidecar are stored by `project` as **opaque JSON** (like the
  Amendment 1 attachment), preserving ADR-001's "`project` never depends on `rack`".
- T-301's journal label-params change becomes **Amendment 3**.

## Amendment 4 — T-300 (SPEC-022 punch-in), 2026-09-13
(Amendment 3 is reserved for T-301's journal label params.)
- `take_begin` gains `mode` (`New` | `Insert` | `Overwrite` | `Punch`), `at`, and for Punch the
  replaced window `[start, end)`, pre/post-roll and the applied recording offset.
- New records: `take_window {take, start, end}` (the part of the take file that becomes document audio,
  after latency compensation) and `take_cancel {take}` (pre-roll cancel; the take file stays as a backup
  until GC).
- Edits gain a marker mapping choice: Overwrite and Punch keep markers in place (`MarkerMapping::Identity`),
  Insert shifts them (SPEC-008 §4.2). Crossfades at punch boundaries are rendered into new chunks inside
  the replaced range, so audio outside the selection stays bit-identical.
- The capture writer keeps ~2 s of look-back so a take can start at a sample that is already in the past
  when the start position becomes known.

## Amendment 5 — T-101 implementation notes, 2026-09-13
- **Journal line format:** `<crc32 lowercase hex>\t<json>\n`; one write + `fdatasync` per append;
  `Journal::open_append` truncates a torn tail to the last valid line before appending.
- **Records added by T-101:** `take_discard {take}` (discarded or empty take; an empty take adds no undo
  entry); `edit.take` pairs an edit with its `take_begin`; `edit.mapping` = `"shift" | "identity"`
  (per-op marker mapping, SPEC-008 §4.2; Identity is refused for length-changing ops); attachments are
  lowercase hex; checkpoints carry `next_rev` and `next_audio_rev`.
- **Durability rule:** committing a chunk only copies it into the mapping and publishes it (readers see
  it immediately); **`ChunkStore::sync()`** (async flush per dirty mapped segment + one `sync_data()` of
  the chunk file; `F_FULLFSYNC` on macOS via std) must run before any journal record references new
  chunks — `Session` is the single place that enforces it. A dirty segment evicted before a sync is
  flushed before unmapping and stays marked. Measured: 60-min import 0.88 s + 0.11 s sync on btrfs
  (was 8.1 s with per-chunk `msync`).
- **Import floor:** `Session::set_floor` (before any edit/take) makes imported audio the undo floor
  (journal `chunks` + checkpoint, clean document, no undo entry).
- **Takes:** `commit_take(&FinishedTake, &[Marker])` borrows (a failed commit leaves the take open for
  retry); take markers are clamped into the take range. `TakeSyncHandle` (default
  `TakeSyncMode::Background`) lets the capture side run the ~1 s header patch + `fdatasync` on another
  thread; the header never claims more data than written.
- **Preallocation:** on `EOPNOTSUPP` the segment is still created but counted in
  `unreserved_segments()` so the engine can warn; copy-on-write file systems (btrfs, ZFS, APFS) weaken
  the no-SIGBUS guarantee (documented).
