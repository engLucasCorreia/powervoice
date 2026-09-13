# SPEC-018 — Sessions: sidecar `.vo.json`, save & autosave policy, recent files

- **Status:** approved (autonomous, T-300)
- **Milestone:** M3 (T-306). Depends on T-301 (journal `state`/`saved`, recovery), T-302/T-303 (edits,
  markers), T-103 (`RackModel` + placeholders), T-104 (settings file), T-201/T-202 (save/open
  pipelines).
- **Related:** SPEC-000 (glossary: session, document), SPEC-004 (§2.6 save and "modified", §2.7
  recovery, §2.8 cleanup, AC-8, AC-9, AC-12), SPEC-005 (§2.1 entry points, §2.6 save format, §2.7
  Save/Save As, §2.9 cue markers), SPEC-006 (view state), SPEC-007 (§2.1 spectral pane settings),
  SPEC-009 (markers, §2.13 precedence), SPEC-012 (§2.2 persistence, placeholders, AC-13) · ADR-001 (§3
  rule 4: `project` never depends on `rack`; §4 `project` owns the sidecar), ADR-004 (§1 session
  directory and lock, §6 journal `state`/`saved`, §8 save sequence, §9 recovery), ADR-005 (§2
  placeholders, §10 slot serialization, `blob_base64`) · PROMPT §2 (LOCKED: "Persistence — plain audio
  file + sidecar `name.vo.json` (rack, markers, noise profile, view state) + autosave/crash recovery"),
  §3.6 (autosave, recent files) · MEMORY D-012 (namespace), T-005 (`serde_json` `float_roundtrip`),
  T-104 (settings pattern)

## 1. Purpose

A voice-over user's work is more than the audio file. It is also the effects chain and noise print they
tuned, the markers they placed, and where they were looking. PROMPT §2 locks the shape of the answer: the
audio stays a plain file any program can open, and everything else goes in a small sidecar next to it.

The user must be able to trust that:
- reopening a file brings back exactly the rack, markers and view they saved, bit for bit;
- a sidecar never gets attached to the wrong audio;
- PowerVoice writes their folders only when they press Save, and never leaves a half-written file;
- a crash never loses work (the session journal's job, SPEC-004);
- recent files are one click away.

## 2. Behavior / UX

### 2.1 Where each kind of data lives

| Data | Audio file | Sidecar | Session store (journal, SPEC-004) | App settings |
|---|---|---|---|---|
| Samples | ✓ | — | ✓ | — |
| Markers: position, length, name | WAV `cue `/`adtl` (interop) | ✓ **authoritative** (SPEC-009 §2.13) | ✓ | — |
| Marker ids and kinds | — | ✓ | ✓ | — |
| Rack: slots, parameters, bypass, state blobs; **the noise profile is the NR slot's state blob** (ADR-005 §11) | — | ✓ | ✓ `state` (≤ 2 s old) | — |
| View state (§2.6.5) | — | ✓ | ✓ `state` | last-used spectral settings as defaults |
| Save format (container, bit depth, dither) | implied by the header | ✓ | ✓ | default recording format |
| Undo/redo history | — | — | ✓ | — |
| Recent files | — | — | — | ✓ |

- **The sidecar describes one audio file.** It lives next to that file, so that copying the pair
  copies the work.
- **The session store is private working storage** (ADR-004 §1). It is never next to the user's file.

### 2.2 Sidecar file name and location
- **Name.** The sidecar of `‹dir›/‹file name›` is `‹dir›/‹file name›.vo.json`. The full file name
  includes its extension: `chapter01.wav` → `chapter01.wav.vo.json`.
- **Decided (autonomous, T-300):** PROMPT §2 says `name.vo.json`, and "name" is read as the full file
  name. The stem form (`chapter01.vo.json`) would make `chapter01.wav` and `chapter01.flac` in one
  folder share and overwrite one sidecar, which is a realistic case (a master next to a delivery file).
  ⚠ Flagged as an interpretation of LOCKED wording, not a change of it.
- **Lookup.** At open, only that exact name is looked for. The match is case-sensitive, except on
  Windows and macOS, where the file system decides.
- **Renaming.** A pair renamed consistently outside PowerVoice keeps working: identity is checked by
  content (§2.5), and the stored file name is informational only.

### 2.3 When PowerVoice writes the sidecar
**Decided (autonomous, T-300): PowerVoice writes a sidecar only as part of a successful Save or Save
As.** Consequences:
- A **new recording** has no sidecar until its first Save As.
- **Nothing is written** on open, close, quit, idle, timers, crash recovery, export or rack edits.
- **Export** writes no sidecar. Exported files are deliverables (M6).
- **Save As** writes the sidecar next to the *new* file. A sidecar at the old path is left untouched.

Rationale: the user's folders change only when the user asks. That is predictable for version control
and sync folders (Dropbox, Syncthing), and no partial or surprising writes land in them (the ADR-004 §1
reasoning). Crash safety does not need file writes (§2.10).

**Save sequence** (extends ADR-004 §8 and SPEC-005 §2.7):
1. **Pre-flight.** SPEC-005 §2.7's checks, plus §2.9's two new ones: *changed on disk* and *sidecar
   replaceable*. All of them run before any byte is written.
2. **Audio.** Written when §2.4 says it is needed: temp file → `fdatasync` → rename → directory fsync.
3. **Backup.** If §2.8's backup rule applies, the existing sidecar is copied to `‹sidecar›.bak`
   atomically (temp → `fdatasync` → rename).
4. **Sidecar.** Temp file `.‹sidecar name›.powervoice-tmp-‹pid›` in the same folder → `fdatasync` →
   rename over the target → directory fsync (POSIX). On Windows, `MoveFileExW(REPLACE_EXISTING)`
   through `std::fs::rename`.
5. **Journal.** Journal `saved { path, seq, format, audio_crc32, sidecar: true }` (§4.5).

**Crash windows**, for AC-6:
- **Before step 2's rename:** the old audio and the old sidecar remain.
- **Between steps 2 and 4:** the new audio remains with the old sidecar. The old sidecar's fingerprint
  no longer matches, so it is ignored at the next open (§2.5). Because `saved` was never journaled, the
  session is still recoverable (SPEC-004 §2.7), and recovering, then saving, restores everything.
- **After step 4:** consistent.

### 2.4 Modified state and sidecar-only saves (extends SPEC-004 §2.6)
The document tracks two flags:

| Flag | Meaning | Source |
|---|---|---|
| `dirty` | Audio or markers differ from the last save (journal `seq` ≠ saved `seq`) | SPEC-004 AC-8 |
| `sidecar_dirty` | The **persisted content** (save format + markers + rack, §4.3) differs from what was last written or read | this spec |

- **Title and prompts.** The title shows `*` when `dirty || sidecar_dirty`. The close, open-another and
  quit prompts (SPEC-004 §2.8) appear under the same condition. When only `sidecar_dirty` is set, the
  prompt adds the line "Effect settings changed."
- **View state never marks the document modified.** Zoom, scroll, selection and spectral view changes
  are saved with the next Save but never set `*` and never prompt.
  - **Decided (autonomous, T-300):** otherwise, scrolling a file would nag the user at every close.
- **Changes back to the saved state are clean.** The digest compares content, so reverting a rack
  parameter to its saved value clears `sidecar_dirty`, just as undo to the saved `seq` clears `dirty`.
- ⚠ **Extension, flagged:** SPEC-004 §2.2 keeps rack edits out of the undo history (OD-4). They now
  *do* mark the document modified. This is consistent with OD-4, since "modified" is not "undoable",
  but it is new behavior for the SPEC-004 title rule.

**Save writes the audio only when needed.** The audio file is rewritten iff at least one of:
- `audio_rev` changed since the last save or open;
- the markers changed and the container stores markers (WAV);
- the save format changed (Save As, or a mapped format, SPEC-005 §2.6);
- the file is missing or changed on disk (§2.9).

Otherwise **Save writes only the sidecar**: the audio file keeps its bytes and mtime, and `dirty` is
cleared as usual (journal `saved`).
- **Decided (autonomous, T-300):** re-rendering 60 min of audio to persist a rack tweak or a FLAC
  document's markers would take seconds and rewrite a large file for nothing. The result on disk is
  identical either way.
- ⚠ This refines SPEC-005 §2.7 ("Save writes the current revision"): the file already *is* that
  revision.
- **Save with nothing modified** is still allowed. It writes only the sidecar, which is how a user
  deliberately persists the view state.

### 2.5 Opening: which sidecar is used
- **When.** After the import completes (SPEC-005 §2.3 step 5), PowerVoice reads `‹file›.vo.json` and
  classifies it. Rack, markers and view are applied before playback and editing unlock, in the same step
  that emits `document_changed`.
- **Document identity** = `(sample_rate_hz, len_samples, audio_crc32)` (§4.2). A sidecar **matches**
  when all three equal the imported document's.
  - `file_name`, `file_size_bytes` and `file_mtime` are informational only: they are shown in
    diagnostics and used for §2.9's changed-on-disk check.
  - **Decided (autonomous, T-300):** content identity survives renames, copies, and another program
    rewriting only the metadata (e.g. cue chunks, SPEC-009 §2.13 case 3), and it catches a wrong or stale
    sidecar.

**Outcomes:**

| Case | Detection | Result | Notice |
|---|---|---|---|
| **None** | no `‹file›.vo.json` | WAV cue markers (SPEC-009 case 5); rack **carried over** (below); SPEC-006 default view (zoom full) | — |
| **Valid, matching** | parses, format id and compat OK (§2.8), identity equal | markers (SPEC-009 §2.13), rack, view and save format loaded | — |
| **Belongs to other audio** | identity differs | ignored, as in "None" | `notice.sidecar.mismatch` "‹file›.vo.json belongs to a different version of this audio and was ignored. It will be kept as a backup when you save." |
| **Corrupt** | not JSON, not an object, wrong `format`, missing `version`, a section of the wrong type, or larger than 64 MiB | ignored, as in "None" | `notice.sidecar.corrupt` "‹file›.vo.json is damaged and was ignored…" (same backup sentence) |
| **Too new** | `compat_version` > 1 | ignored, as in "None" | `notice.sidecar.too_new` "‹file›.vo.json was written by a newer PowerVoice (‹app_version›) and can't be read…" (same backup sentence) |
| **Unreadable** | permission or I/O error | ignored, as in "None" | `notice.sidecar.unreadable` |
| **Partly invalid** | valid file with some invalid items (§2.6.3, §2.6.4) | valid items loaded; invalid markers dropped; unparseable slots become placeholders | `notice.sidecar.items_dropped` "3 markers in ‹file›.vo.json were invalid and were skipped" |

- **Rack carry-over.** A document opened without a usable sidecar keeps the rack that is currently
  loaded. A new recording also keeps it. A document with a valid sidecar replaces the rack with its own.
  - **Decided (autonomous, T-300):** a voice-over book is recorded chapter by chapter through the same
    chain. Carrying the rack over means chapter 2 plays through it at once, and its first Save stores it.
    The carried rack is the baseline for `sidecar_dirty`, so opening a file never sets `*` by itself.
  - The app starts with an empty rack unless it recovers a session.
- **Whole-rack A/B** is never stored and is off after open (SPEC-012 §2.3).
- **The sidecar never blocks opening.** Every failure falls back to "None", never panics, and leaves
  the document fully usable (AC-8).

### 2.6 Schema v1
One JSON object, UTF-8 without BOM. It is **normative**: T-306 implements exactly these names and
types.

```json
{
  "format": "org.powervoice.sidecar",
  "version": 1,
  "compat_version": 1,
  "writer": {
    "app": "PowerVoice",
    "app_version": "0.3.0",
    "written_at": "2026-09-13T08:41:07Z"
  },
  "document": {
    "file_name": "chapter01.wav",
    "file_size_bytes": 518400076,
    "file_mtime": "2026-09-13T08:41:06Z",
    "sample_rate_hz": 48000,
    "len_samples": 172800000,
    "audio_crc32": "9f3a51c2",
    "fingerprint": "crc32-ieee/f32le/v1",
    "save_format": { "container": "wav", "sample_format": "pcm24", "dither": "tpdf" }
  },
  "markers": {
    "items": [
      { "id": 1, "pos_samples": 0, "len_samples": 0, "name": "Intro", "kind": "user" },
      { "id": 4, "pos_samples": 144000, "len_samples": 0, "name": "Dropout 10 ms", "kind": "dropout" },
      { "id": 7, "pos_samples": 200000, "len_samples": 4800, "name": "Take 2", "kind": "user" }
    ]
  },
  "rack": {
    "slots": [
      { "module": "org.powervoice.gain@1.0.0", "bypass": false,
        "state": { "format_version": 1, "params": { "gain_db": -6.0 } } },
      { "module": "org.powervoice.noise-reduction@1.0.0", "bypass": false,
        "state": { "format_version": 1, "params": { "reduction_db": 12.0 }, "blob": "AAECAw==" } },
      { "module": "com.acme.x@2.0.0", "bypass": true, "state": { "format_version": 3, "params": {}, "blob": "…" } }
    ]
  },
  "view": {
    "waveform": {
      "start_sample": 0, "samples_per_pixel": 86400.0, "vertical_zoom": 1.0,
      "amplitude_ruler_mode": "dbfs", "time_ruler_format": "timecode",
      "selection": null, "cursor_samples": 0
    },
    "spectral": {
      "visible": false, "split_ratio": 50.0, "fft_size": "auto", "freq_scale": "log",
      "display_floor_db": -120.0, "display_ceil_db": 0.0, "colormap": "inferno"
    },
    "markers_panel": { "sort": { "column": "start", "direction": "asc" }, "type_filter": "all" },
    "rack_panel": { "collapsed_slots": [], "collapsed_groups": [] }
  }
}
```

#### 2.6.1 Header
| Field | Type | Rule |
|---|---|---|
| `format` | string | Exactly `"org.powervoice.sidecar"`. Anything else → corrupt. The namespace follows D-012 and is permanent from the first sidecar (SPEC-012 OD-2). |
| `version` | integer ≥ 0 | The schema version the writer used. v1 writes 1. |
| `compat_version` | integer ≥ 0 | The lowest reader version that can read this file safely, ignoring unknown fields. v1 writes 1. Missing → equal to `version`. |
| `writer.app`, `writer.app_version`, `writer.written_at` | strings | App name, its semver, and the RFC 3339 UTC time with seconds and `Z`. Informational only (created-by). |

#### 2.6.2 `document`
| Field | Type | Rule |
|---|---|---|
| `file_name` | string | File name at write time (informational) |
| `file_size_bytes` | integer | Size of the audio file written or verified at save (informational; §2.9) |
| `file_mtime` | RFC 3339 string | Its mtime, truncated to whole seconds (informational; §2.9) |
| `sample_rate_hz` | integer | Identity |
| `len_samples` | integer | Identity |
| `audio_crc32` | 8 lower-case hex digits | Identity (§4.2) |
| `fingerprint` | string | The algorithm id, `"crc32-ieee/f32le/v1"`. An unknown id → the sidecar is treated as *belongs to other audio*. |
| `save_format` | object | `container` ∈ {`wav`, `flac`}; `sample_format` ∈ {`pcm16`, `pcm24`, `f32`}; `dither` ∈ {`tpdf`, `none`}. It restores the Save As dialog's choices. The file header stays authoritative for what the file *is*. |

#### 2.6.3 `markers`
- **Items.** `items` holds `{ id: integer ≥ 1, pos_samples, len_samples, name, kind }`, in canonical
  order (SPEC-009 §2.1). `kind` is `"user"`, `"dropout"`, or any other string (preserved, §2.7).
- **Validation on read.** An item is dropped (and counted for the `items_dropped` notice) if:
  - `pos_samples > len_samples(document)`;
  - `pos + len > len_samples(document)`;
  - its `id` is a duplicate (later occurrences are dropped);
  - a field has the wrong type.

  Names are normalized as in SPEC-009 §2.4, and an empty name becomes a default name.
- **Order.** Items are re-sorted canonically after loading.
- **Cap.** At most 100 000 items are loaded (SPEC-009 §2.12).

#### 2.6.4 `rack`
- **Slots.** `slots` is the ordered slot list in ADR-005 §10's slot format, which is also the format of
  the CLI's `--rack` files (SPEC-012 §2.8): `module` (`id@version`), `bypass`, and `state` (a
  `ModuleState`: `format_version`, `params` as a key → f64 map sorted by key, optional `blob` as
  standard base64).
- **Placeholders** (unknown id, or `StateError::TooNew`, SPEC-012 §2.2) are written back **verbatim**:
  their slot object is JSON-equal to what was read, including fields PowerVoice does not know.
- A slot object that is not even a well-formed slot (e.g. no `module` string) becomes an "Unreadable
  module" placeholder and is also written back verbatim.
- **Ownership.** `project` stores `rack` as an **opaque JSON value**, because ADR-001 rule 4 forbids
  `project → rack`. The engine (the composition layer) converts `RackModel` to and from that value with
  `rack`'s serde, exactly as the OD-4 bake attachment is opaque (ADR-004 Amendment 1).
- **Noise profile.** It is the NR slot's `state.blob` (ADR-005 §11). No separate field exists.

#### 2.6.5 `view`
All fields are optional on read. A missing or invalid field takes its default: SPEC-006/007 defaults,
or, for `spectral`, the app's last-used spectral settings.

| Field | Rule |
|---|---|
| `waveform.start_sample` (int), `samples_per_pixel` (f64), `vertical_zoom` (f64) | Restored exactly, then clamped to SPEC-006's ranges for the current viewport width. An out-of-range `samples_per_pixel` → zoom full. |
| `waveform.amplitude_ruler_mode`, `time_ruler_format` | SPEC-006 enums |
| `waveform.selection` (`null` or `{start_sample, end_sample}`), `cursor_samples` | Restored when within `[0, L]` and `start ≤ end`; otherwise `null` / 0 |
| `spectral.*` | SPEC-007 §3 values. **Decided (autonomous, T-300):** per document from M3, as SPEC-007 §2.1 announced. The app settings keep the last-used values as defaults for documents without a sidecar. `freq_range` is not stored (SPEC-007 §2.4). |
| `markers_panel.sort`, `type_filter` | SPEC-009 §2.8 |
| `rack_panel.collapsed_slots` (slot indices), `collapsed_groups` (list of `{slot, group_key}`) | ADR-005 §13 collapse state |

- **Typing.** `view` is typed by a DTO in `src-tauri` (ts-rs), because it is UI state. `project` stores
  it as an opaque JSON value, like `rack`.
- **Not stored:** playhead-follow and zero-crossing snap (app preferences, SPEC-003/006), the panel's
  text filter, and whole-rack A/B.

#### 2.6.6 Encoding rules
- **Integers** are JSON integers. Positions are u64, and serde_json reads and writes them exactly.
- **Floats** are written with serde_json's `float_roundtrip` (workspace feature, MEMORY T-005). Every
  f64 therefore reads back **bit-identical**. Non-finite values are never written: `ModuleState` is
  finite by contract (ADR-005 §10), and a non-finite view float is replaced by its default before
  writing. A non-finite value reaching the writer is a bug and fails the write with an error, never a
  corrupted file.
- **Deterministic text:**
  - pretty-printed, 2-space indentation, LF line ends, one trailing LF;
  - known fields in the order of this section;
  - map keys sorted where the type is a map (`params`);
  - preserved unknown fields after the known fields of their object (their relative order is not
    guaranteed).

  So two writes of the same state are byte-identical except for `writer.written_at` and
  `document.file_mtime`, which keeps diffs of sidecars in version control readable.
- **Size cap on read:** 64 MiB. Larger → corrupt, and PowerVoice never allocates beyond the cap.

### 2.7 Unknown fields are preserved
PowerVoice keeps what it does not understand, so an older build never destroys a newer build's data:
- **Where.**
  - **Top level:** unknown members are preserved verbatim.
  - **Inside known objects** (`writer`, `document`, `save_format`, `markers`, `rack`, `view` and its
    sub-objects): unknown members are preserved.
  - **Marker items:** preserved per item, matched **by id**. A marker deleted in PowerVoice takes its
    unknown fields with it; a moved or renamed one keeps them.
  - **Slots:** preserved per slot instance through the `RackModel` (T-103 keeps an `extra` map per
    slot). Reordering keeps them, removing a slot drops them, and placeholders keep everything (§2.6.4).
  - **Unknown marker `kind` strings** are kept (`MarkerKind::Other`, SPEC-009 §2.1). The marker behaves
    as a user marker in the UI and is written back with its original kind.
- **How** (T-104's settings pattern): every schema struct has `#[serde(flatten)] extra:
  serde_json::Map`, so values round-trip JSON-equal. Member order is not guaranteed.
- **Rule for future schema versions** (normative for whoever writes v2): a field added in a later
  version must make sense even after an older writer has edited the rest of the file and carried the
  field through unchanged. For example, an extension referring to markers must carry the marker ids it
  refers to, and must tolerate their absence.

### 2.8 Versions, migration and backups
- **Reading by version:**

  | File | Reader v1 does |
  |---|---|
  | `compat_version` ≤ 1 and `version` = 1 | reads it |
  | `compat_version` ≤ 1 and `version` > 1 (a newer but compatible writer) | reads it as v1; unknown fields preserved |
  | `compat_version` > 1 | *too new* (§2.5): ignored, backed up on save |
  | `version` < 1 | migrated **in memory** through the chain `migrate_v(n) → v(n+1)` (pure functions `Value → Value`), then read |

- **The file is never rewritten on open.** A migrated or newer-compatible file is rewritten only on the
  next Save, and always as `version: 1, compat_version: 1`.
- There is no v0 in the wild: v1 is the first released schema. The migration framework exists now and
  is tested with a synthetic test-only step (AC-9).
- **Backup rule.** **Decided (autonomous, T-300): before overwriting a sidecar that PowerVoice couldn't
  fully read, or whose `version` differs from 1, PowerVoice first copies it to `‹sidecar›.bak`.**
  - "Couldn't fully read" covers the mismatch, corrupt, too-new and unreadable cases and the
    partly-invalid case.
  - The `.bak` is one generation: it overwrites an older `.bak`.
  - The backup step itself is atomic. If it fails, the Save fails before the sidecar is touched
    (`error.save.sidecar_backup`).
  - Rationale: nothing PowerVoice couldn't interpret is ever destroyed, and a newer PowerVoice, or a
    person, can recover it.

### 2.9 New save pre-flight checks (extend SPEC-005 §2.7's list; they run first)
- **Changed on disk.** If the bound audio file's size or mtime differ from what PowerVoice recorded at
  open or last save:
  - the dialog "‹name› was changed on disk since PowerVoice opened it. Saving will replace that
    version." appears, with **Overwrite** / **Save As…** / **Cancel** (default Cancel);
  - Overwrite performs a **full** save (audio + sidecar);
  - a missing file needs no prompt: Save recreates it (SPEC-004 §2.7).
  - This covers other programs and a second PowerVoice instance (§2.11).
- **Sidecar replaceable.** If an existing sidecar can't be replaced (on Windows, a read-only attribute;
  anywhere, a directory in its place), the Save is refused before the audio is written, with
  `error.save.sidecar_locked` "‹file›.vo.json can't be replaced — use Save As".
  - On POSIX, renaming over a read-only file in a writable folder succeeds, so a mode-0444 sidecar is
    replaced normally.
- **Read-only folders.** Opening works, and the sidecar is read. Save fails in SPEC-005's pre-flight
  (`error.save.permission`) before any byte is written, and the error offers Save As. Save As to a
  writable folder writes both files there.

**Runtime failure after the audio succeeded** (e.g. disk full while writing the sidecar):
- The audio save stands, `dirty` is cleared, and `sidecar_dirty` **stays set**, so `*` remains.
- The old sidecar is untouched (or absent), never partial.
- The notice `notice.save.sidecar_failed` explains the loss:
  - WAV: "Saved ‹name›, but its settings file couldn't be written (‹reason›). Markers are in the file;
    effects and view settings are not."
  - FLAC: "…Markers, effects and view settings are not."
- The next Save retries with a sidecar-only write (§2.4).

### 2.10 "Autosave" and crash safety
- **Definition.** **Decided (autonomous, T-300): PowerVoice's autosave (PROMPT §2, §3.6) is the
  session journal, and nothing else.**
  - Every edit is `fdatasync`ed into the session before it reports success.
  - The rack and view state are journaled as `state` within **2 s** of their last change (SPEC-004
    `state_debounce_s`).
  - There is **no** timer that writes the audio file or the sidecar, and no "auto-save to file"
    preference in v1.
- **Rationale:**
  - crash recovery (SPEC-004 §2.7) already restores everything, full undo history included, which a
    periodic file save could not;
  - silent writes to user files are exactly what this spec rules out (§2.3).
- ⚠ **Interpretation of LOCKED wording, flagged:** PROMPT §2's "autosave/crash recovery" is fulfilled
  by the journal. A user who expects Audition-style "save to file every N minutes" gets recovery
  instead. A v1.x preference could add it.
- **`state` record payload.** Exactly the sidecar's `rack` and `view` sections, as JSON (§4.5). Markers
  are journaled as edits (SPEC-009).
- **After recovery:**
  - the document opens modified (SPEC-004);
  - `sidecar_dirty` is computed against the sidecar on disk, if it matches, or else against the empty
    baseline;
  - the rack and view are as of their last `state` record, at most ~2 s old.

### 2.11 A file already open in another PowerVoice instance
- **Detection.** Before importing, Open scans `‹app_local_data_dir›/sessions/` for a session whose
  advisory lock is held (ADR-004 §1) and whose `meta.json` source path equals the requested path after
  canonicalization (§4.6). A session of **this** instance is excluded; opening this instance's own path
  is SPEC-005's Revert.
- **Dialog.** "‹name› is already open in another PowerVoice window. Editing it in two places can
  overwrite changes." **Open Anyway** / **Cancel** (default **Cancel**).
  - **Cancel** creates nothing: no session directory, no recent-files change.
  - **Open Anyway** opens normally, as an independent session. §2.9's changed-on-disk check then
    protects whichever instance saves second.
- **Decided (autonomous, T-300): warn and allow**, instead of refusing. Opening a file read-only for
  listening in a second window is legitimate, and the save check prevents silent loss.
- **Scan cost.** The scan reads only `meta.json` of locked sessions and takes ≤ 50 ms with 20 sessions.

### 2.12 Recent files
- **Storage.** The T-104 settings file gains an additive field `recent_files: [{ "path": string,
  "opened_at": RFC 3339 UTC }]`. It is ordered most recent first and holds at most **10** entries.
  Settings writes are atomic (T-104), and the settings version stays 1 because the field is additive
  with a default.
- **When an entry is added or moved to the top:**
  - after a successful open, meaning the import completed (not cancelled, not failed), including Revert;
  - after a successful Save As, for the new path;
  - after the first Save of a new recording (a Save As);
  - for a recovered document, when it is saved.

  Failed or cancelled opens and exports never touch the list. Session and temp paths are never added.
- **Dedup and overflow.** Entries are de-duplicated by canonical path (§4.6). Re-adding an existing
  path moves it to the top with a new `opened_at`. An 11th distinct path drops the oldest entry.
- **No pinning.** **Decided (autonomous, T-300):** ten entries cover a voice-over project's active
  chapters, and Audition's Open Recent has no pinning either. Pinning would add UI and ordering rules
  for little gain.
- **File → Open Recent** submenu:
  - one item per entry, labelled with the file name plus its parent folder, dimmed and middle-truncated
    to 60 characters (`chapter01.wav — ~/VO/Book 3`);
  - a separator, then **Clear Recent Files**, disabled when the list is empty;
  - picking an entry follows the normal Open flow: the unsaved-changes prompt, disabled while recording
    (SPEC-005 §2.1).
- **Missing files:**
  - When the File menu opens, existence is checked off the UI thread with a total budget of 300 ms.
    Entries not answered in time are shown normally. Network paths must not freeze the menu.
  - A missing entry (`NotFound`) is shown greyed with "(missing)" and a **Remove** action.
  - Choosing a missing entry shows "‹path› can't be found." with **Remove from List** / **Cancel**.
  - An entry that exists but fails to open (e.g. unsupported) shows SPEC-005's error and stays in the
    list.
- **Clear Recent Files** empties the list without confirmation. **Decided (autonomous, T-300):** it
  deletes no user data.
- **Compressed imports.** Opening `x.mp3` adds it. Its Save (SPEC-005 §2.6: Save As WAV 24-bit) writes
  `x.wav` + `x.wav.vo.json` next to each other, adds `x.wav` to the list, and writes nothing next to the
  MP3.

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `sidecar_suffix` | Sidecar name | — | — | `‹file name›.vo.json` | fixed | §2.2 |
| `sidecar_format_id` | Format id | — | — | `org.powervoice.sidecar` | fixed | permanent |
| `sidecar_version` | Schema version written | — | — | 1 (`compat_version` 1) | fixed | §2.8 |
| `sidecar_max_bytes` | Largest sidecar read | MiB | — | 64 | fixed | larger = corrupt |
| `fingerprint_algo` | Document identity | — | — | `crc32-ieee/f32le/v1` | fixed | §4.2 |
| `state_debounce_s` | Rack/view journaling delay | s | — | 2 | fixed | SPEC-004 |
| `recent_max` | Recent files | entries | — | 10 | fixed | no pinning |
| `recent_check_budget_ms` | Existence check when the menu opens | ms | — | 300 | fixed | §2.12 |
| `already_open_scan_ms` | Session-lock scan at Open | ms | — | ≤ 50 (20 sessions) | fixed | §2.11 |
| `sidecar_write_budget_ms` | Write (10 000 markers, 16 slots, 1 MiB of blobs) | ms | — | ≤ 150 p95 | fixed | AC-18 |
| `sidecar_load_budget_ms` | Read + validate + apply (same content) | ms | — | ≤ 150 p95 | fixed | AC-18 |
| `backup_generations` | `.bak` copies | — | — | 1 | fixed | §2.8 |

## 4. Algorithm / implementation notes

### 4.1 Ownership (ADR-001)
- **`project::sidecar`** owns:
  - the schema structs with `extra` maps;
  - read (size cap → parse → header → migrate → validate → classify);
  - write (serialize → pre-flight → backup → atomic write);
  - the `.bak` rule and the fingerprint;
  - the persisted-content digest (§4.3);
  - stale temp-file cleanup (the dead-pid rule, SPEC-004 AC-12).

  The `rack` and `view` sections are opaque `serde_json::Value`s in `project`.
- **`engine`** converts `RackModel` ↔ the `rack` Value through `rack`'s serde. **`src-tauri`** converts
  the view DTO ↔ the `view` Value. Neither adds a forbidden crate edge.
- **No new dependencies.**
  - RFC 3339 UTC timestamps use a small hand-written civil-date formatter (days-from-civil), because
    there is no `chrono`/`time` in the workspace.
  - Base64 reuses ADR-005's `blob_base64` helper.
  - CRC uses `crc32fast` (ADR-004).

### 4.2 Fingerprint `crc32-ieee/f32le/v1`
- **Definition.** CRC-32 (IEEE, `crc32fast`) over the little-endian bytes of every document sample, as
  f32, in order: exactly the bytes the importer puts in the chunk store (SPEC-005 §2.3 conversion
  table).
- **At open:** zero extra I/O. ADR-004 already computes a CRC32 per committed chunk over the same bytes,
  and the document CRC is their `crc32fast::Hasher::combine` in chunk order.
- **At save:** the save pipeline hashes the samples **as a reader will decode them**:
  - WAV/FLAC 16 and 24-bit: `q / 2^(bits−1)` as f32 for each written integer `q`, after dither;
  - WAV 32f: the written floats.

  So the saved sidecar matches the file even though a dithered file differs from the in-session
  document. That value also becomes the "file fingerprint" used for later sidecar-only saves.
- **Strength.** A 32-bit CRC plus exact length and rate: the chance that an unrelated audio of the same
  length and rate matches is 2⁻³² per comparison. That is accepted for a sidecar match (the consequence
  would be restoring the wrong rack or markers, which is visible and undoable, not data loss).
  `fingerprint` names the algorithm, so a later version can move to a 64-bit hash.

### 4.3 Persisted-content digest (for `sidecar_dirty`)
- **Digest.** CRC-32 of the canonical serialization (§2.6.6, no pretty-printing) of
  `{ document.save_format, markers, rack }`, excluding `writer`, `view`, the informational document
  fields, and unknown fields.
- **Baseline.** It is recorded after a successful load and after every successful sidecar write, and
  compared after every marker, rack or save-format change. For "None" (no usable sidecar), the baseline
  is the digest of the state right after open (carried rack, WAV markers).
- **Cost.** ≤ 5 ms at 10 000 markers. It runs on the control thread after the change is committed and
  is debounced like `state`, at most once per 100 ms.

### 4.4 Read pipeline
1. `metadata`; > 64 MiB → corrupt.
2. Read and parse to `serde_json::Value`.
3. Check the header (`format`, `version`, `compat_version`).
4. Migrate if `version < 1`.
5. Deserialize into the typed structs (with `extra`). Markers are validated per item; `rack` and `view`
   are kept as Values.
6. Check identity against the imported document.
7. The engine resolves the rack (registry → instances or placeholders, SPEC-012). `src-tauri` applies
   the view.
8. Record the digest baseline, the `.bak`-needed flag and the raw file's `version`.

Every failure maps to a §2.5 outcome, and none panics (fuzz: AC-8).

### 4.5 Journal (ADR-004 §6), additive
- `state { rack, view }` carries the two sections as JSON.
- `saved` gains `audio_crc32` (the file fingerprint) and `sidecar: bool` (false when the sidecar write
  failed, §2.9).
- Recovery rebuilds the file fingerprint and `sidecar_dirty` from them. No structural change is made:
  the fields are additive. ⚠ Flagged as an ADR-004 §6 detail for T-301/T-306.

### 4.6 Canonical paths
- **Where used:** recent-files dedup, the already-open scan, and matching the session's source path.
- **Existing files:** `std::fs::canonicalize` (resolves symlinks, `.` and `..`).
- **Missing files:** the lexically normalized absolute path.
- **Comparison:** byte-exact on Linux; case-insensitive (Unicode simple case folding) on Windows and
  macOS, whose default file systems are case-insensitive. ⚠ macOS/Windows behavior is unverified
  (MEMORY risk "Windows/macOS untested").

### 4.7 IPC (ADR-003)
- **Open.** `document_open` gains the outcome `NeedsConfirmation { kind: already_open { path } }`. The
  UI re-issues with `confirm_already_open: true`.
- **Save.** `document_save` / `document_save_as` gain `NeedsConfirmation { kind: changed_on_disk }` and
  the errors `error.save.sidecar_locked` and `error.save.sidecar_backup`. `SaveResult` gains
  `{ audio_written: bool, sidecar_written: bool }`.
- **Recent files:** the command `recent_files_get() -> RecentFileDto[] { path, name, folder, exists:
  bool | null }` (existence resolved within the budget), plus `recent_files_remove(path)`,
  `recent_files_clear()`, and the event `recent_files_changed`.
- **`document_changed`** gains `sidecar_dirty` alongside `dirty`.
- **Notice keys:** `notice.sidecar.mismatch`, `.corrupt`, `.too_new`, `.unreadable`, `.items_dropped`,
  and `notice.save.sidecar_failed`.
- **Dialog keys:** `dialog.already_open.*`, `dialog.changed_on_disk.*`, `dialog.recent_missing.*`.
- **Menu keys:** `menu.file.open_recent`, `menu.file.clear_recent`, `recent.missing`.

## 5. Acceptance criteria

Fixtures are generated in test code (testkit noise and sines, hand-written sidecar JSON). "JSON-equal"
means equal as `serde_json::Value` trees, with numbers compared exactly.

- **AC-1 (location, and no silent writes).** Given a scratch folder watched by a file-system snapshot
  (names, sizes, mtimes and hashes of every entry):
  - after New Recording → 5 s take, the folder is unchanged;
  - after Save As `take.wav`, it contains exactly `take.wav` and `take.wav.vo.json`;
  - after reopening it, editing markers and the rack, idling 10 min with the `state` debounce firing,
    exporting to another folder, closing with Don't Save and reopening, the snapshot equals the
    post-Save-As snapshot;
  - only an explicit Save changes it.
- **AC-2 (schema v1, determinism).** Given a fixed document state (3 markers of every kind, a
  3-slot rack including an NR blob and a placeholder, a non-default view):
  - the written sidecar equals the golden file `crates/project/tests/data/sidecar-v1.golden.json`
    byte for byte, after substituting `writer.written_at` and `document.file_mtime`;
  - two consecutive writes differ only in those two fields;
  - the output is UTF-8 without BOM, with LF endings and one trailing LF.
- **AC-3 (rack round trip is bit-exact).** Given a rack of Gain `gain_db = −6.123456789012345`, a test
  module with wide-range parameters set to `0.1 + 0.2`, `1e-300`, `5e-324`, `−1.7976931348623157e308`
  and `1/3`, an NR slot with a 48 KiB seeded blob, and one bypassed slot, when saved, closed and
  reopened:
  - every parameter value is `f64::to_bits`-identical;
  - blobs are byte-identical, and bypass flags and slot order are equal;
  - an offline render (`rack::offline::render`) of 10 s of pink noise through the reloaded rack is
    bit-identical (FNV-1a) to the render before saving;
  - a second save writes a byte-identical `rack` section.
- **AC-4 (placeholders are preserved).** Given a sidecar slot
  `{"module":"com.acme.x@2.0.0","bypass":true,"state":{"format_version":3,"params":{"k":1.5},"blob":"AAEC","x_nested":{"a":[1,2]}},"x_slot":"keep"}`
  and a Gain slot whose state has `format_version` 99 (TooNew):
  - both load as placeholders (SPEC-012 AC-13): dry, latency 0, with the missing / "requires a newer
    version" messages;
  - after moving the Gain placeholder to position 0 and saving, both slot objects are **JSON-equal** to
    the originals, and the order is as edited.
- **AC-5 (unknown fields are preserved).** Given a v1 sidecar with an unknown member in each of: the top
  level (`"x_v2": {"deep": [1, {"b": null}]}`), `writer`, `document`, `save_format`, `markers`, marker
  item id 4, `rack`, a slot, and `view.waveform`; plus a marker with `"kind": "chapter"`; when opened,
  marker 7 renamed, marker 4 moved, marker 1 deleted and saved, then:
  - every unknown member is JSON-equal to the original, except marker 1's, which is gone with it;
  - marker 4 keeps its unknown member at its new position;
  - the "chapter" marker is written back with `"kind": "chapter"`, and in the UI it behaved as a user
    marker (Type "Point").
- **AC-6 (atomic write under kill).** Given a document with 10 000 markers (sidecar ≈ 1.5 MB) and an
  existing sidecar S₀, when a child process performing Save (sidecar-only, and separately full WAV +
  sidecar) is `SIGKILL`ed at **20** seeded random points:
  - the sidecar on disk is always either byte-identical to S₀ or complete and parseable as the new
    state, never partial;
  - the audio obeys SPEC-004 AC-14;
  - any leftover `.‹name›.powervoice-tmp-‹dead pid›` is deleted at the next save into that folder, and
    one whose pid is alive is not;
  - for kills between the audio rename and the sidecar rename: the next open shows
    `notice.sidecar.mismatch`, the recovery dialog lists the session, and Recover + Save yields a
    matching sidecar with the pre-crash rack.
- **AC-7 (identity mismatch).** Given `a.wav` and its sidecar, and a `b.wav` equal to `a.wav` except
  that one sample differs by 1 LSB (same length and rate), with `a.wav.vo.json` copied to
  `b.wav.vo.json`:
  - opening `b.wav` ignores the sidecar, shows `notice.sidecar.mismatch`, uses `b.wav`'s cue markers,
    and keeps the carried-over rack;
  - after Save, `b.wav.vo.json.bak` is byte-identical to the copied sidecar, and the new sidecar
    matches `b.wav`;
  - the same happens when only `len_samples` or only `sample_rate_hz` differ, or when `fingerprint` is
    an unknown id;
  - renaming `a.wav` + its sidecar to `c.wav` + `c.wav.vo.json` still loads everything, with no notice.
- **AC-8 (corrupt, too new, never blocks opening).** Given sidecars that are: truncated JSON, 64 KiB of
  seeded random bytes, a JSON array, `format` "org.other", a 65 MiB file, `compat_version: 2`, and an
  unreadable file (mode 000):
  - each open shows its §2.5 notice, the document is fully usable, and nothing panics;
  - the first Save writes `.bak` then a valid v1 sidecar;
  - 10 000 seeded structural mutations of the golden sidecar (deleted or retyped members, huge numbers,
    deep nesting to 10 000 levels) never panic, and each either loads or maps to a §2.5 outcome;
  - `version: 2, compat_version: 1` with unknown members loads normally and is written back as
    `version: 1` with the unknown members preserved and a `.bak` of the original.
- **AC-9 (migration framework).** With a test-only registry containing a synthetic step `v0 → v1` (it
  renames a `marker_list` member to `markers.items`), given a v0 fixture:
  - the markers load exactly;
  - the file's bytes and mtime are unchanged after open and after close without saving;
  - Save writes `version: 1`, plus a `.bak` byte-identical to the v0 file;
  - a chain of two synthetic steps (`v-1 → v0 → v1`) applies in order;
  - a step that returns an error maps to *corrupt*, with no panic.
- **AC-10 [with SPEC-009 AC-18] (marker precedence).** Precedence cases 1–5 of SPEC-009 §2.13 hold end
  to end through T-306's open path, including kind inheritance and the notices.
- **AC-11 (modified flags, sidecar-only save).** Given a saved 60-min WAV document:
  - a rack parameter change sets `*` (`sidecar_dirty`), and setting it back to the saved value clears
    `*` without a save;
  - a view-only change (zoom, scroll, selection, spectral toggle) never sets `*`, and closing then shows
    no prompt;
  - with the rack changed, Save completes in ≤ 200 ms: the WAV's bytes and mtime are unchanged
    (`audio_written = false`), the sidecar is rewritten, and `*` clears;
  - a marker rename on the WAV document triggers a full save (cue chunk), while on a FLAC document it
    triggers a sidecar-only save;
  - an audio edit always triggers a full save;
  - the close prompt with only the rack changed shows "Effect settings changed.".
- **AC-12 (changed on disk; second instance).**
  - After opening `a.wav`, a test changes its mtime. Save then shows the changed-on-disk dialog. Cancel
    writes nothing (folder snapshot equal); Overwrite performs a full save.
  - With instance 1 holding `a.wav` open, instance 2's Open of `a.wav` (also via a symlink and via
    `./x/../a.wav`) returns `already_open`. Cancel leaves no session directory and no recent-files
    change; Open Anyway opens normally.
  - After instance 1 exits cleanly, instance 2's Open shows no dialog.
  - The scan takes ≤ 50 ms with 20 sessions present.
- **AC-13 (read-only locations).**
  - Given a folder with mode 0555 containing `a.wav` and a valid sidecar, open loads the sidecar. Save
    returns `error.save.permission` before writing: target, sidecar and folder snapshot are unchanged,
    with no temp file. Save As to a writable folder writes both files there and binds the document to
    the new path.
  - A mode-0444 sidecar in a writable folder is replaced normally (POSIX).
  - Through the fake file-system seam, a Windows-style non-replaceable sidecar yields
    `error.save.sidecar_locked` with the audio untouched.
- **AC-14 (partial failure).** With disk-full injected after the audio rename but during the sidecar
  write:
  - Save reports audio saved, `sidecar_written = false`, and `notice.save.sidecar_failed` (the WAV
    wording, or the FLAC wording for FLAC);
  - `dirty` is false, `sidecar_dirty` is true and `*` stays;
  - the old sidecar is byte-identical (or still absent), and no temp file remains;
  - the next Save writes only the sidecar;
  - journal `saved` has `sidecar: false`, and a crash plus recovery at that point shows `*`.
- **AC-15 (autosave = journal; recovery of rack and view).** Given a scripted 30-min session with no
  Save, a rack parameter change every 10 s and view changes:
  - the user folder snapshot never changes;
  - after `SIGKILL` 2.5 s after the last change, Recover restores a `rack` section JSON-equal to the
    pre-crash one and the same view state;
  - after `SIGKILL` 0.5 s after a change, the recovered rack equals either the pre-change or the
    post-change state (debounce), never a mixture;
  - the recovered document shows "(recovered)" and `*`.
- **AC-16 (recent files).** Given a fresh settings file:
  - opening A, B, C, then A gives `[A, C, B]`, and 12 distinct successful opens keep the 10 newest in
    order;
  - A reached as `./dir/../A.wav` and through a symlink yields one entry;
  - a cancelled import and a failed open (unsupported file) change nothing;
  - Save As Z puts Z at the top, and saving an imported MP3 as WAV adds the WAV;
  - with B deleted from disk, the menu shows B greyed with "(missing)". Remove drops it, and the
    settings file is rewritten atomically with unknown settings fields preserved (T-104);
  - Clear empties the list, and the list survives an app restart;
  - with 10 entries on an unreachable network mount (simulated with a stalled `stat`), the menu opens
    within 300 ms plus one frame, with those entries shown as unknown.
- **AC-17 (compressed import).** Opening `x.mp3`, then Save:
  - Save opens Save As with WAV 24-bit and `x.wav` preselected (SPEC-005 AC-17);
  - afterwards the folder holds `x.mp3` (byte-identical), `x.wav` and `x.wav.vo.json`, and no
    `x.mp3.vo.json`;
  - the sidecar's identity matches `x.wav` on reopen.
- **AC-18 (performance, reference NVMe, release build).** Given 10 000 markers and a 16-slot rack
  holding 1 MiB of blobs in total:
  - a sidecar write (serialize → temp → `fdatasync` → rename) takes ≤ 150 ms p95 over 20 runs;
  - read + validate + apply takes ≤ 150 ms p95;
  - the fingerprint adds 0 bytes of extra reads to opening the 60-min fixture, and ≤ 50 ms of wall time
    (SPEC-005 AC-11 still holds).
- **AC-19 (view state restore).**
  - A saved view (`start_sample` 1 234 567, `samples_per_pixel` 37.25, `vertical_zoom` 4.0, selection
    [2 000 000, 2 480 000), spectral visible at `split_ratio` 62.5 with the viridis colormap, markers
    panel sorted by Name descending with the Regions filter) is restored exactly on reopen (f64 bits
    and u64 values equal) at the same viewport width.
  - At a narrower window, `start_sample` and `samples_per_pixel` are clamped per SPEC-006 AC-4.
  - A view with `selection.end_sample` beyond L restores `selection = null`, and `samples_per_pixel =
    -1` restores zoom full.
  - A document without a sidecar gets SPEC-006 defaults and the app's last-used spectral settings.

## 6. Test plan

| AC | Unit (`project`) | Integration (engine / `src-tauri`, child process, temp dirs) | Vitest (UI, mockIPC) | Manual smoke (owner, Linux) |
|---|---|---|---|---|
| AC-1 | write only from the save API | folder-snapshot harness across the scripted session | — | check the folder after a session without Save |
| AC-2 | golden file; determinism | — | — | open a sidecar in a text editor |
| AC-3 | f64 bits round trip; blob base64 | engine: RackModel ↔ Value; offline render hash before/after | — | save and reopen a tuned rack, A/B by ear |
| AC-4 | opaque slot preservation | registry resolution → placeholders → save | "Missing module" slot text | — |
| AC-5 | `extra` maps; per-id marker extras | open-edit-save flow | — | — |
| AC-6 | temp/rename/fsync sequence with fault injection | `SIGKILL` at 20 points (child process); recovery flow | — | `kill -9` during a save |
| AC-7 | identity classification | open flows with the copied sidecar; `.bak` content | notice text | copy a sidecar next to the wrong file |
| AC-8 | fuzz/mutation loop; size cap; version matrix | open flows per case | notices | open with a hand-broken sidecar |
| AC-9 | migration chain with a test registry | — | — | — |
| AC-10 | cue projection compare (SPEC-009) | end-to-end cases 1–5 | notices | edit markers in REAPER, reopen |
| AC-11 | digest baseline and comparisons | save paths: audio vs sidecar-only; timing | title `*`, prompt text | tweak the rack, Save, check the WAV mtime |
| AC-12 | canonical path compare | two app instances (child processes) + lock | already-open and changed-on-disk dialogs | open the same file twice; `touch` a file, then Save |
| AC-13 | pre-flight checks; fake-FS seam | chmod 0555 / 0444 temp folders | error surfaces | save from a read-only mount |
| AC-14 | fault injection between steps | disk-full provider | notice wording, `*` | — |
| AC-15 | `state` payload = sections | scripted session + `SIGKILL` + recover | — | kill during rack tweaking, recover |
| AC-16 | ordering/dedup/cap; atomic settings write | stalled-stat provider | Open Recent menu, missing/Remove/Clear | use Open Recent across restarts |
| AC-17 | — | MP3 → Save As WAV flow | Save → Save As redirect | save an imported MP3 |
| AC-18 | serialize/parse bench (`just bench`) | open-time delta on the 60-min fixture | — | — |
| AC-19 | view Value validation and clamps | — | restore into SPEC-006/007 state at two widths | reopen a file and check the view |

**Fixtures.**
- The golden sidecar and hand-written variants are committed under `crates/project/tests/data/`. They
  are small text files; audio is still never committed.
- Audio comes from testkit at test time. Folder snapshots and two-instance runs use temp dirs.

## 7. Out of scope
- **Other files:** project/session files that bundle several audio files (multitrack, PROMPT §3.8); a
  sidecar for exports; reading Audition's `.sesx`, `.pkf` or XMP marker metadata; Save a Copy.
- **Other autosave or history:** "auto-save to file every N minutes" (§2.10); persistent undo across
  sessions (SPEC-004 §7).
- **Recent-files extras:** pinning, recent folders, and a welcome screen listing recent files.
- **Multi-device concerns:** sync-conflict resolution for cloud folders beyond the changed-on-disk
  check, and sidecar encryption or signing.
- **Rack presets.** Their storage format is T-406. Presets use the same slot format but are not
  sidecars.
