# SPEC-004 — Document model & undo

- **Status:** approved (owner, M0 checkpoint 2026-09-12)
- **Milestone:** M1 (T-101: snapshots, chunk store, markers, take writer). The history-facing parts
  land in M3 (T-301: undo/redo, budgets, journal, recovery; T-302/T-303 edit and marker ops; T-306
  sidecar). Each AC is tagged with its milestone.
- **Related:** SPEC-000 (glossary), SPEC-002 (takes), SPEC-003 (stop semantics, playhead
  extrapolation), SPEC-012 (rack state) · ADR-004 (structure) · ADR-002 §3, §5, §8 · ADR-003
  (`document_changed`, `audio_rev`) · ADR-005 open question 3

## 1. Purpose
A voice-over editor is used for hours of cut-listen-undo. The user must be able to trust five things:
1. undo always brings back exactly what was there;
2. long files don't make the app slow or hungry for memory;
3. a crash never loses an edit that appeared to succeed;
4. playback never plays stale audio;
5. PowerVoice cleans up after itself.

This spec defines that behavior. ADR-004 defines how it is built.

## 2. Behavior / UX

### 2.1 Documents and revisions (snapshot semantics)
- **One document.** One document is open at a time (PROMPT §2, LOCKED): mono samples at one sample
  rate, plus markers.
- **Revisions.** Every committed change creates a new immutable **revision** (snapshot). `rev` changes
  on every change; `audio_rev` changes only when samples change. A marker edit therefore never
  invalidates the waveform or spectrogram.
- **Consistency.** Every consumer works on **one revision from start to finish**: playback, save,
  export, peaks, spectrogram and analysis. Edits made while a save or export runs never leak into it.
  Editing never waits for such a job.
- **All-or-nothing.** A long edit (e.g. normalizing 60 min) shows progress and can be cancelled. A
  cancelled or failed edit leaves the document exactly as it was: same revision, no undo entry, and a
  notice for failures.
- **Undo floor.** Opening a file makes the imported audio the undo floor. A new recording's floor is
  the empty document.

### 2.2 What is undoable

| Action | Undo entry | Stops playback | Notes |
|---|---|---|---|
| Cut, delete, paste, trim, silence, insert silence | one each | yes | M3 (T-302) |
| Peak normalize, LUFS normalize | one each | yes | M3 (T-305), M6 (T-603) |
| **Bake rack** | one | yes | Restores the audio **and** the pre-bake rack (OD-4 default); redo re-applies the reset. M6 (T-602) |
| Recording take (new file; M3 at cursor/punch) | one, including markers placed during it and dropout markers | n/a | SPEC-002 |
| Recovered take applied | one | n/a | §2.7 |
| Marker add, rename, move, delete | one each | **no** | M3 (T-303) |
| Undo/redo of a marker-only entry | — | **no** | |
| Undo/redo of an audio entry | — | yes | |
| Copy to clipboard | none | no | not a document change |
| Selection, zoom, scroll, view state, playhead | none | no | |
| Rack edits: slots, parameters, bypass, presets, noise-print capture | **none in v1** (OD-4 default) | no | persisted as session `state` (≤ 2 s old after a crash) |
| Settings | none | no | |
| Save / Save As / Export | none | no | Undo history survives a save |
| Open | none | — | becomes the undo floor |

- **Menu labels.** The Edit menu shows **Undo ‹label›** / **Redo ‹label›** (i18n keys, e.g.
  `history.cut`) and disables the item when the stack is empty.
- **Shortcuts.** Undo **Ctrl+Z** / Redo **Ctrl+Shift+Z** (⌘Z / ⌘⇧Z on macOS). These agree with
  PROMPT §3.6 and one secondary source (killerkeys.com). The official Adobe page returns 403, so the
  final binding is left to SPEC-019 (T-701). Add marker = **M** (two independent sources, see
  SPEC-002).
- **Depth.** Undo depth is unlimited; only the disk budget (§2.5) limits it, never memory. A new edit
  clears the redo stack. Undo and redo feel instant (AC-3).
- **Markers.**
  - A marker added **during playback** lands at the heard position under the key press, as
    extrapolated by the UI (ADR-002 §8, SPEC-003 §2.2), in document samples.
  - Audio edits move markers:
    - markers at or after an insertion point shift right;
    - markers inside a deleted range move to the range start;
    - markers after a deleted range shift left.
  - These are ADR-004's proposed defaults; the M3 editing spec may refine them.
  - Several markers may share a position.

### 2.3 Destructive edits vs playback and recording
- **An audio edit during playback,** including undo/redo of an audio entry:
  - playback stops with a ~5 ms fade (no click) *before* the edit commits;
  - the playhead stays at the heard position where playback stopped, clamped to the new length
    (an engine-initiated stop behaves like Pause, SPEC-003 §2.1);
  - playback does not resume by itself.
  The user never hears audio from a revision that is no longer current.
- **A marker-only change during playback** has no audible effect: the output is identical to
  uninterrupted playback.
- **During recording,** audio edits and undo/redo are refused, with their controls disabled and
  "Not available while recording". Adding markers is allowed and lands in the take (SPEC-002 §2.2).

### 2.4 Memory budget
- **Default.** Resident memory used for audio data is bounded by the **memory budget**:
  clamp(RAM/4, 512 MiB, 4 GiB).
- **Setting.** It can be changed under Settings → Performance → "Memory for audio", from 256 MiB to
  16 GiB in 256 MiB steps. A change applies without a restart.
- **Over budget.** The least-recently-used audio is released from memory **silently**. It stays on
  disk and is reloaded on demand. The user sees no notice and loses no data; at worst the first access
  to far-away audio is slightly slower. Playback never glitches because of it: the audio being played
  and the segment being written are never released.
- **What doesn't count.** RAM use does not grow with document length or undo depth.

### 2.5 Disk budget
- **Settings.** Settings → Recovery & storage shows "Session storage: 3.2 GB (history 2.1 GB)".
- **Limits.** The session's soft limit is max(8 GiB, 8 × document size), and at least 2 GiB must stay
  free on the volume (ADR-004 §4).
- **When a limit is exceeded** (checked after every committed edit, and every 10 s while idle):
  1. PowerVoice first reclaims unused space automatically (compaction). This is silent.
  2. If that is not enough, it applies **OD-1**. Default: it removes the oldest undo steps and shows
     "Low disk space: the 12 oldest undo steps were removed (freed 4.1 GB)." The current document and
     the redo steps are never touched.
- **Disk nearly full.** If free space is still below 2 GiB with nothing left to reclaim, a persistent
  warning appears: "Disk almost full — save your work."
- **Recording** has its own disk rules (SPEC-002 §2.5). Compaction and undo removal never run while
  recording.

> **OWNER DECISION OD-1 — Disk pressure policy (ADR-004 open question 1).**
> - **A.** Automatically drop the oldest undo steps, with a notice, once compaction can't reclaim
>   enough.
> - **B.** Only warn and let the disk fill. History stays intact, but an edit or recording may then
>   fail for lack of space.
> - **C.** Ask each time with a modal ("Remove oldest undo steps?").
>
> **✅ Decided by the owner at the M0 checkpoint: A.** Losing the oldest history is
> recoverable by the user (save a copy); a full disk mid-recording is not.

### 2.6 Save and "modified"
- **Modified mark.** The title shows `*` while the document differs from the last saved state. Undoing
  back to the saved state removes the `*`, and redoing away from it sets it again.
- **History lifetime.** History survives Save and ends when the document is closed. There is no
  persistent undo across sessions.
- **When the user's file is written.** PowerVoice writes the user's audio file only on explicit Save,
  Save As or Export to a new file. Save is atomic: a crash or power loss during Save leaves either the
  complete old file or the complete new one. At most a hidden temp file `.<name>.powervoice-tmp-<pid>`
  remains next to it. PowerVoice deletes such leftovers (dead pid only) the next time it saves into that
  folder.
- **Sidecar autosave.** Sidecar autosave policy belongs to the M3 sidecar spec (T-306). Crash safety
  never depends on it; it comes from the session journal (§2.7).

### 2.7 Crash recovery
- **Start-up.** After an unclean exit, a dialog appears before any document opens: "PowerVoice didn't
  shut down properly". It lists each recoverable session with:
  - the file name, or "Untitled recording";
  - the original path;
  - the last edit time;
  - the number of unsaved changes;
  - whether a recording was in progress, with its recovered length.
- **Per-session actions.**
  - **Recover.** Only one document can be open, so the other sessions stay listed.
  - **Discard.** It asks for confirmation: "Permanently delete the unsaved changes to ‹name›?".
- **Dialog-wide action: Decide later.** It keeps everything and asks again at the next start. The same
  list is available under Settings → Recovery & storage.
- **A recovered document** opens **modified**, bound to its original path, with the title suffix
  "(recovered)" until the first save. The **full undo/redo history** is back, as of the last committed
  edit. Rack and view state are back as of their last save, at most ~2 s old.
- **Interrupted take.** The dialog asks "A recording was in progress (2:31 recovered)":
  - **Apply as recorded** is the default. The result is the same as if Stop had been pressed: one
    undoable edit.
  - **Open as new document.**
  - **Discard take.**
- **The source file changed** on disk since the session opened (size or mtime differ):
  "‹name› changed on disk since you opened it. Recovering keeps your version; saving will overwrite
  the file."
- **The source file is missing:** recovery still works, because the session holds all audio. Save
  recreates the file.
- **Damaged data.** Recovery keeps everything up to the last intact record. If later records are
  damaged, a notice says "The last N changes could not be recovered." If stored audio fails its
  checksum, recovery returns to the newest state whose audio is intact, with the same notice. If
  nothing is intact, only Discard is offered. PowerVoice never crashes on damaged recovery data.
- **Sessions in use** by another running PowerVoice instance are neither listed nor touched.

### 2.8 Session cleanup
- **Normal close.** After Save or Don't Save, the session directory is deleted within 5 s, including
  take backups. If that deletion is interrupted, the next start finishes it.
- **Start-up.** Cleanly closed sessions are removed silently.
- **Recoverable sessions** are **never deleted silently** (OD-2).
- **Manual clearing.** Settings → Recovery & storage → "Recovery data (X GB) — Clear…" lists
  recoverable sessions with their sizes and deletes only the ones the user confirms. Sessions in use
  are excluded.
- **Quitting with unsaved changes** shows Save / Don't Save / Cancel. Don't Save discards the session.

> **OWNER DECISION OD-2 — Recovery retention (ADR-004 open question 2).**
> - **A.** Keep recoverable sessions until the user discards them. Settings shows their total size.
> - **B.** Auto-delete them after N days (e.g. 30), with a reminder notice at start-up a week before.
>
> **✅ Decided by the owner at the M0 checkpoint: A.** Voice-over sessions are irreplaceable, and the
> start-up dialog plus the Settings total keep them visible.

> **OWNER DECISION OD-3 — Import cost (ADR-004 open question 3).**
> Opening a file copies it into the session store as 32-bit float. That uses ≈ 1.33× the file size
> for 24-bit WAV, 2× for 16-bit, and much more relative to compressed MP3/AAC sources (≈ 691 MB per
> hour at 48 kHz). The copy costs ≈ 1–2 s per hour of audio on NVMe; slow HDDs will miss the < 3 s
> open target (PROMPT §4).
> - **A.** Accept it. The < 3 s target is specified for SSD/NVMe only, and the waveform appears
>   progressively on HDDs.
> - **B.** Reference uncompressed WAV sources in place, without importing. ADR-004 rejected this: it
>   needs page faults on a foreign file and breaks save-over-source on Windows.
>
> **✅ Decided by the owner at the M0 checkpoint: A.**

> **OWNER DECISION OD-4 — Are rack edits undoable? (ADR-005 open question 3, ADR-004 §6 `state`).**
> - **A.** Rack edits are not in the document history (v1). The exception is **bake**, whose undo
>   entry restores the pre-bake rack along with the audio. Otherwise undoing a bake would leave the
>   user without the rack settings they just baked.
> - **B.** One history for everything. Parameter drags are coalesced into one entry per gesture, and
>   undoing a rack entry does not stop playback. Ctrl+Z then often undoes slider tweaks instead of the
>   last cut, which is surprising in an editor centred on destructive edits.
> - **C.** A separate rack history, used when the rack panel has focus. It is more code and harder to
>   explain.
>
> **✅ Decided by the owner at the M0 checkpoint: A.** Structural consequence (ADR-004 amended): undo entries
> and journal `edit` records need an optional opaque rack-state attachment. `project` stores it as
> opaque data, keeping ADR-001's "`project` never depends on `rack`". That is an ADR-004 amendment,
> flagged in SPEC-000 §4.

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `memory_budget` | Memory for audio | MiB | 256 … 16 384 | clamp(RAM/4, 512, 4096) | step 256 | Settings → Performance |
| `disk_soft_limit` | Session disk limit | bytes | — | max(8 GiB, 8 × document bytes) | fixed | ADR-004 §4 |
| `disk_free_floor` | Minimum free space | GiB | — | 2 | fixed | |
| `compact_threshold` | Reclaim when unused ≥ | % | — | 25 | fixed | ADR-004 §4 |
| `disk_check_interval_s` | Idle disk check | s | — | 10 | fixed | also after every edit |
| `state_debounce_s` | Rack/view state save delay | s | — | 2 | fixed | ADR-004 §6 |
| `close_cleanup_s` | Session deletion after close | s | — | ≤ 5 | fixed | |
| `recovery_retention` | Recoverable-session retention | — | keep / N days | keep | — | OD-2 |

## 4. Algorithm / implementation notes
- Structure: ADR-004 §1–§10 (session layout, chunks, piece table, undo stacks of snapshots,
  eviction, compaction, journal and fsync policy, recovery, GC). Nothing here changes it, except the
  OD-4 attachment.
- "Command succeeded" means the journal record has been appended and `fdatasync`ed (ADR-004 §6).
  The UI shows an edit only after that, which is what makes AC-9 hold.
- Undo/redo is an `Arc` swap plus one journal append. The ≤ 50 ms bound in AC-3 is dominated by
  `fdatasync` on SSD; HDD latency is not guaranteed.
- A destructive commit follows the ADR-004 §3 sequence: stop → acknowledgement → commit → hand the
  reader the new snapshot. The audio thread never holds a snapshot (ADR-002 §2).
- Resident-memory accounting (AC-4, AC-5) is the store's own counter of mapped bytes plus the
  spectrogram tile cache and transient op buffers, as ADR-004 §4 counts them. Process RSS is checked
  manually only, because WebView memory dominates it.
- Journal fault injection (AC-9) truncates the file at every byte offset inside the last three
  records, and separately flips one byte per record. Recovery must stop at the first bad CRC or torn
  line (ADR-004 §9).

## 5. Acceptance criteria
- **AC-1 [M1] (snapshot isolation).** Given a reader that has started streaming revision r of a
  10-min document, when 50 audio edits commit while it streams, then every sample it reads equals the
  corresponding sample of r read with no concurrent edits (FNV-1a hash equal). *[M2/M6 extension:]*
  the same holds for Save and Export of r: the file's hash is independent of concurrent edits.
- **AC-2 [M3] (all-or-nothing).** Given a whole-file normalize of a 60-min document, when it is
  cancelled at ≈ 50 % progress, then `rev`, `audio_rev`, the audio hash and the undo/redo depths are
  unchanged and no undo entry is added. The same holds when the op fails with an injected I/O error,
  which additionally posts a notice.
- **AC-3 [M3] (undo/redo exactness and speed).** Given a 10-min document and a seeded random sequence
  of 200 operations (cut, paste, delete, silence, insert silence, normalize, marker
  add/rename/move/delete), when all are undone, then the audio hash and the marker list equal the
  original. When all are redone, they equal the final state. On a 60-min document with 20 000 pieces
  on SSD, each undo or redo command completes in ≤ 50 ms, including its journal `fdatasync`.
- **AC-4 [M3] (depth not bounded by memory).** Given a 60-min 48 kHz document and the disk limit
  raised to 100 GiB, when 100 whole-file normalizes are applied, then all 100 remain undoable and the
  store's resident-memory counter never exceeds the memory budget + 128 MiB.
- **AC-5 [M1] (memory budget).** Given a 512 MiB budget and a 60-min document (691 MB of samples),
  when it is played start to end with 200 random seeks, then:
  - the store's resident-memory counter stays ≤ 512 MiB + 128 MiB (the pinned tail and playback
    segments) at every 100 ms sample;
  - the fake backend reports 0 output underruns;
  - no notice is shown.
- **AC-6 [M1 engine / M3 ops] (destructive edits stop playback; marker edits don't).** Given
  playback on the fake backend:
  - When an audio edit is issued, then within 6 ms of the stop command the output fades to digital
    silence, no output sample after the commit comes from the old revision, the playhead equals the
    heard position at stop (±1 sample), and playback does not resume.
  - When a marker is added, renamed, moved or deleted, or a marker entry is undone, then the output
    is bit-identical to a control run without the marker operation.
- **AC-7 [M3] (marker placement and shifting).**
  - Given playback, when Add marker (M) is pressed at app time T, then the marker lies within
    ±10 ms of the heard position at T (the SPEC-003 AC-6 bound).
  - Given markers at 1 s, 3 s and 10 s on a 48 kHz document, when [2 s, 4 s) is cut, then they are
    at 1 s, 2 s and 8 s exactly (in samples).
  - When 1 s of silence is inserted at 1 s, the 1 s marker moves to 2 s.
- **AC-8 [M3] (modified mark).** An edit sets `dirty`. Save clears it. Undo sets it. Redo back to the
  saved state clears it. Undo history survives the save: its depth is unchanged.
- **AC-9 [M3] (crash durability).**
  - Given a scripted editing session, when the process is `SIGKILL`ed immediately after an edit
    command reports success (at 50 random points), then recovery reproduces exactly the pre-crash
    audio hash, marker list, undo depth, redo depth and entry labels.
  - Given journal fault injection (§4), recovery never panics and yields exactly the state after the
    last intact record, with the "could not be recovered" notice when records were lost.
- **AC-10 [M3] (recovery flow).** Given a crash with 3 unsaved edits and an interrupted 2.5 s take:
  - at next start, the dialog lists the file name, the original path, the last edit time,
    "3 unsaved changes" and "recording 0:02.5";
  - **Recover** opens the document modified, titled "(recovered)", bound to the original path, with 3
    undo steps;
  - **Apply as recorded** adds one undoable "Record" edit;
  - **Decide later** keeps the session and lists it again at the next start;
  - if the source file's mtime changed, the changed-on-disk warning appears;
  - a session locked by a second running instance is not listed.
- **AC-11 [M3] (recovery time).** Given a 60-min session with 1 000 journal records on NVMe, when it
  is recovered, then the document is editable within 5 s of pressing Recover. This includes journal
  replay and checksum verification of all referenced audio.
- **AC-12 [M1 GC / M3 UI] (cleanup).**
  - After Save + close, the session directory is gone within 5 s.
  - After 20 open → edit → save → close cycles, `sessions/` is empty.
  - An interrupted deletion is completed at the next start.
  - Recoverable sessions survive 5 restarts with "Decide later".
  - Settings → Clear deletes exactly the confirmed sessions and never a locked one.
  - A stale `.powervoice-tmp-<dead pid>` file is deleted at the next save into that folder; one whose
    pid is alive is not.
- **AC-13 [M3] (disk pressure, OD-1 default A).** Given a session over its disk limit:
  - when ≥ 25 % of its data is unreachable, compaction runs first, with no undo steps removed and no
    notice;
  - otherwise the oldest undo entries are removed until usage is under the limit, the notice reports
    the count and the freed size, and the current audio hash and redo stack are unchanged;
  - neither action runs while recording.
- **AC-14 [M2] (atomic save).** Given Save of a 10-min document, when the process is killed at 20
  random points during it, then the target file is either bit-identical to its previous version or
  complete and identical to the rendered revision. It is never partial.
- **AC-15 [M1] (refused while recording).** Given a recording in progress:
  - audio edit, undo and redo commands return an `IpcError` whose key is `error.not_while_recording`
    and change nothing;
  - an Add marker command succeeds and its marker appears in the take (SPEC-002 AC-15).
- **AC-16 [M4/M6] (OD-4 default A).**
  - Given a cut followed by a rack parameter change, when Undo is pressed, then the cut is undone and
    the parameter keeps its new value.
  - Given a bake, when Undo is pressed, then the audio hash **and** the rack description (slots,
    parameter values, bypass flags, state blobs) equal their pre-bake values. Redo restores the baked
    audio and the reset rack.

## 6. Test plan

| AC | Unit | Integration | Manual smoke (owner) |
|---|---|---|---|
| AC-1 | project: reader over snapshot while committing | engine: save/export job during edits (M2/M6) | — |
| AC-2 | project: ChunkWriter cancel/fail leaves state untouched | engine job cancel | cancel a long normalize |
| AC-3 | project: seeded random op sequence (property test) | timing bench with a 60-min fixture (`just fixtures`) | undo/redo a long session |
| AC-4 | project: store accounting after 100 normalizes | — | watch memory in a long session |
| AC-5 | project: eviction policy | fake backend playback + seeks over a 60-min fixture | play a 60-min file with a 512 MiB budget |
| AC-6 | — | fake backend: stop-ack-commit sequence; marker ops bit-identical | edit while playing; add markers while playing |
| AC-7 | project: marker shift rules | fake backend + UI extrapolation (Vitest) | press M while playing |
| AC-8 | project: dirty vs saved seq | — | edit / save / undo |
| AC-9 | journal parser fault injection (every offset) | child process + `SIGKILL` at 50 points, recover, compare | `kill -9` mid-session |
| AC-10 | recovery classification | end-to-end with a crashed child and a second instance | real crash, walk through the dialog |
| AC-11 | — | bench with a generated 60-min session | — |
| AC-12 | GC classification | open/save/close cycles; lock contention | check `~/.local/share/<id>/sessions` |
| AC-13 | budget decision logic | fake volume-size provider | — |
| AC-14 | — | kill during save at 20 points (M2) | — |
| AC-15 | — | fake backend recording + command attempts | try Ctrl+Z while recording |
| AC-16 | engine: bake entry carries the rack attachment (M6) | undo/redo after bake | bake, undo, check the rack |

Fixtures come from `just fixtures` (60-min generated WAVs) and seeded testkit signals. Sessions are
generated by test scripts, not committed.

## 7. Out of scope
- Edit-op details (clipboard across documents, zero-crossing snap, marker panel UI): M3 editing and
  markers specs.
- The sidecar schema and sidecar autosave (T-306). WAV/cue writing (T-201). Export (M6).
- Multiple documents, persistent undo across sessions, a History panel.
- Rack presets and the rack's own persistence format (SPEC-012, T-406).
