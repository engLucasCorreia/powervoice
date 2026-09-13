# SPEC-008 — Editing operations: cut, copy, paste, delete, trim, silence, insert silence

- **Status:** approved (autonomous, T-300)
- **Milestone:** M3 (T-302). Depends on T-301 (undo/redo, journal, budgets) and on the M2 selection
  model (SPEC-006, T-206).
- **Related:** SPEC-000 (glossary: document time, piece, snapshot, edit), SPEC-003 (transport, engine-
  initiated stop = Pause), SPEC-004 (undo semantics, markers, all-or-nothing, stop-before-commit,
  refused while recording), SPEC-006 (selection model, edit cursor, zero-crossing snap), SPEC-010
  (normalize — the other M3 destructive op), SPEC-019 (full shortcut map, M7) · ADR-004 §3 (piece
  table, clipboard as `Vec<Piece>`, marker shifting, destructive-edit sequence), §4 (reachability,
  compaction), §6 (journal `edit` records), follow-up "T-302 clipboard across document switches" ·
  ADR-001 §4 (`project` owns edit ops and the clipboard) · ADR-002 (workers, generation-token
  cancellation) · ADR-003 (commands, `job_progress`, `notice`) · PROMPT §2 (LOCKED), §3.3, §3.6, §3.8

## 1. Purpose

A voice-over editor cuts and joins speech all day. They remove a flubbed line, paste a retake over it,
drop in a second of room tone, and trim the head and tail before export. They need these operations
to be:
- **exact**: what is selected is precisely what is removed, and a cut followed by a paste gives back the
  original bit for bit;
- **instant**, even on a 60-minute file with thousands of earlier edits;
- **predictable**: the same keys as Audition, and a known place for the cursor, selection and markers
  afterwards;
- **safe**: one undo step each, never half-applied, never audible from a stale revision.

This spec defines the seven operations and the clipboard. SPEC-004 defines what "undoable", "stops
playback" and "all-or-nothing" mean. ADR-004 defines how the piece table makes the operations cheap.

## 2. Behavior / UX

### 2.1 The operations

Notation: the document is `A` with length `L` samples. The selection is `[S, E)` in document samples,
with `S < E ≤ L`, as stored by SPEC-006 §2.2 (exclusive end). The **cursor** `c` is the playhead
position when the transport is stopped (SPEC-006 §2.9). The clipboard holds `n` samples `K`. `‖` means
concatenation.

| Op | Requires | Resulting document | Clipboard |
|---|---|---|---|
| **Cut** | non-empty selection | `A[0,S) ‖ A[E,L)` | replaced by `A[S,E)` |
| **Copy** | non-empty selection | unchanged (not an edit) | replaced by `A[S,E)` |
| **Paste** at the cursor (no selection) | non-empty clipboard | `A[0,c) ‖ K ‖ A[c,L)` (insert) | unchanged |
| **Paste** over a selection | non-empty clipboard | `A[0,S) ‖ K ‖ A[E,L)` (replace; `n` may differ from `E − S`) | unchanged |
| **Delete** | non-empty selection | `A[0,S) ‖ A[E,L)` | unchanged |
| **Trim to selection** (Audition: *Crop*) | non-empty selection | `A[S,E)` | unchanged |
| **Silence** | non-empty selection | `A[0,S) ‖ zeros(E − S) ‖ A[E,L)`, same length | unchanged |
| **Insert silence…** | a duration `d` from the dialog (§2.5) | `A[0,p) ‖ zeros(m) ‖ A[p,L)`, with `p = S` if a selection exists, otherwise `p = c`, and `m` from §2.5 | unchanged |

- **Delete ripples.** Delete and Cut close the gap, as in Audition's Waveform view. There is no
  "delete and leave silence" variant: that is Silence.
- **Sample-exact.** Every operation acts on exactly the stored `[S, E)` or `c`. Nothing is rounded to
  pixels, snapped again at operation time (§2.7) or faded (§2.8).
- **Silence is digital zero.** Silenced and inserted samples are exactly `0.0` (positive zero). They
  are stored as `Source::Silence` pieces (ADR-004 §3), so they cost no disk space.
- **Insert silence with a selection** inserts at the selection start and removes nothing (inserting
  never destroys audio).
  - **Decided (autonomous, T-300):** this follows Adobe's help wording "position the playhead or select
    existing audio, then choose Edit > Insert > Silence". An insert command that silently replaced the
    selection would duplicate Silence.
- **Whole-document edge cases.**
  - Cutting or deleting `[0, L)` leaves an empty document (`L = 0`). This is allowed and undoable.
  - Trim of `[0, L)` is a **no-op**: success with `changed = false`, no undo entry, playback not
    stopped.
  - Paste and Insert silence into an empty document insert at 0.

### 2.2 "No selection" and enablement

"No selection" means `selection == null` **or** an empty selection (`S == E`, which SPEC-006 allows).
An empty selection at `S` counts as the cursor at `S`.

| Op | Enabled when | With no selection |
|---|---|---|
| Cut, Copy, Delete, Trim, Silence | a document is open, a non-empty selection exists, and nothing below disables it | **disabled** |
| Paste | a document is open and the clipboard is non-empty | inserts at the cursor |
| Insert silence… | a document is open | inserts at the cursor |

- **Decided (autonomous, T-300): selection-based ops are disabled without a selection, never applied
  to the whole file.** A stray Delete or Ctrl+X with nothing selected must not wipe or crop a
  60-minute recording, and Ctrl+A (SPEC-006) is one keystroke away. Normalize differs on purpose: its
  whole-file fallback is LOCKED (PROMPT §2) and is harmless because it preserves relative levels
  (SPEC-010).
- **Disabled everywhere at once:**
  - "Disabled" applies to the Edit menu item, the waveform context menu item and the shortcut (a
    no-op that shows nothing).
  - A command that reaches Rust anyway with an empty range returns `IpcError` key
    `error.no_selection` and changes nothing. A Paste with an empty clipboard returns
    `error.clipboard_empty`.
- **Disabled for all seven operations, Copy included:**
  - **while recording** (SPEC-004 §2.3; tooltip "Not available while recording", command error
    `error.not_while_recording`). Copy is included because during a take the selection can cover
    live, uncommitted audio that no snapshot contains yet.
  - **while a document job runs:** a normalize (SPEC-010), a long paste (§2.6) or an import/open.
    The job's modal progress dialog owns the window meanwhile.

### 2.3 Selection and cursor after each operation

| Op | Selection after | Playhead / cursor after |
|---|---|---|
| Cut, Delete | none | `S` |
| Copy | unchanged | unchanged |
| Paste (insert at `c`) | `[c, c + n)` (the pasted audio) | `c` |
| Paste (over `[S, E)`) | `[S, S + n)` | `S` |
| Trim | none | `0` |
| Silence | unchanged `[S, E)` | unchanged |
| Insert silence at `p` | `[p, p + m)` (the inserted silence) | `p` |

- **Decided (autonomous, T-300): pasted and inserted audio becomes the selection.** This makes an
  otherwise invisible edit visible. Shift+Space (SPEC-003, play from selection start) immediately
  plays it, Delete removes it, and a normalize favorite (SPEC-010) applies to just it. Audacity behaves
  the same way, and Audition is believed to (not verified; the official page is 403).
  - Consequence: pressing Ctrl+V twice replaces the first paste with the same content instead of
    appending a second copy. To append again, the user clicks at the end of the pasted audio first.
    This is accepted; it is the Audacity behavior.
- **Decided (autonomous, T-300): Trim leaves no selection and puts the cursor at 0**, the start of what
  was kept.
- **During playback,** the destructive op first stops the transport (§2.9). That is an engine-initiated
  stop, so it behaves like Pause (SPEC-003 AC-9): the playhead is at the heard position. Then:
  - with no selection, that heard position **is** the cursor `c` used by Paste and Insert silence;
  - the playhead then moves as in the table above.

  ⚠ This refines SPEC-004 §2.3 bullet 2 and AC-6, which say the playhead stays at the heard position
  after an edit during playback. Here, SPEC-004 AC-6's "playhead equals the heard position" holds at
  the stop step; after the commit, this table sets the playhead. For Silence (and Normalize,
  SPEC-010) the two readings agree, because the playhead is "unchanged".
  - Rationale: after deleting a flub that was heard while playing, leaving the playhead at the old
    heard sample number would point at different audio, shifted by the deletion length.
- **After undo or redo of one of these operations** (T-301 implements it): the selection is cleared and
  the playhead goes to the edit's first affected position `min(at)` over its `Replace` ops (§4.1),
  clamped to the length.
  - **Decided (autonomous, T-300):** this is derivable from the journal record alone, so no view state
    enters the undo history (SPEC-004 §2.2 keeps selection out of it). Reported as a SPEC-004
    extension.

### 2.4 Markers

Markers (`Marker { pos_samples, len_samples }`, ADR-004 §3) move with every structural edit, so each
keeps pointing at the same audio. This **confirms** ADR-004's proposed defaults and SPEC-004 §2.2 /
AC-7, and refines them for replacement, trim and region markers:
- markers before the edit point don't move;
- markers at or after an **insertion** point shift right by the inserted length;
- markers **inside** a removed range move to the range start;
- markers **after** a removed range shift left by the removed length;
- a **replacement** (Paste over a selection) is one operation: markers inside `[S, E)` move to `S`, the
  start of the new content, and markers at or after `E` shift by `n − (E − S)`;
- **Silence** (and Normalize, SPEC-010) don't change the length, so **no marker moves**;
- **Trim `[S, E)`:** markers before `S` move to 0, markers in `[S, E)` shift left by `S`, and markers at
  or after `E` move to the new end `E − S`.
  - **Decided (autonomous, T-300): audio edits never delete markers.** Markers are the user's notes
    ("retake line 12"). An edit may stack several at one position (SPEC-004 allows it), and the
    markers panel (SPEC-009, T-303) deletes them in one action. Undo restores the exact prior
    positions either way.
- **Region markers** (`len_samples > 0`) map their start like a point marker. Their **exclusive end**
  `e = pos + len` uses the rule in §4.2, so:
  - inserting exactly at a region's end does not extend it;
  - inserting strictly inside it grows it;
  - removing part of it shrinks it;
  - a region whose length becomes 0 becomes a point marker at the mapped start. **Decided (autonomous,
    T-300)**, same rationale as above.
- **Undo.** The marker changes are part of the op's single undo entry (ADR-004 §6 `marker_ops`), so
  undo restores every marker exactly.

### 2.5 Insert silence dialog

- **Opening.** Edit → Insert Silence… opens a small modal dialog:
  - **Duration** field;
  - the computed length, e.g. "= 48 000 samples at 48 kHz";
  - the insertion point, e.g. "at 0:12.345";
  - OK and Cancel. Enter = OK, Esc = Cancel.
- **Accepted input.** The field accepts:
  - plain seconds: `1`, `1.5`, `0.250`;
  - timecode `[[hh:]mm:]ss[.fff]`: `0:01.500`, `00:00:02`;
  - an integer followed by `smp`: `48000 smp`.

  The value is shown in the current time-ruler format (SPEC-006 §2.5). Samples mode shows the sample
  count. Unparseable text shows an error outline, and OK is disabled.
- **Conversion.** `m = round(d_s × sample_rate_hz)` in f64, round half away from zero (`f64::round`). A
  `smp` value is used as is. So 1.000 s is exactly 48 000 samples at 48 kHz and 44 100 at 44.1 kHz.
- **Range.** From 1 sample to 3 600 s. Out-of-range values show "Duration must be between 1 sample and
  1 hour" and disable OK.
  - **Decided (autonomous, T-300):** the 1 h ceiling guards against typos, e.g. `10000` meant as samples
    but typed as seconds. Room-tone and pause insertion is seconds long in practice.
- **Default.** 1.000 s. The last accepted value is remembered for the rest of the app session, not
  persisted.
- **Instant.** The inserted silence is `Silence` pieces, so the insert itself is instant (§2.10).

### 2.6 The clipboard

- **One in-app clipboard.** It holds audio only: pieces plus the source sample rate. It holds no
  markers, and PowerVoice does not read or write the operating-system clipboard.
  - **Decided (autonomous, T-300):** one clipboard, not Audition's multiple internal clipboards. It is
    simpler, matches every other desktop app's Ctrl+C/V model, and OS audio-clipboard interoperation is
    platform-specific with no voice-over use case in v1.
- **Lifetime.**
  - Cut and Copy replace the clipboard **only after** the op succeeds (Cut: after its commit is
    journaled). A failed Cut leaves both the document and the clipboard unchanged.
  - Undo and redo never change the clipboard. Undoing a cut restores the audio, and the clipboard still
    holds it.
  - The clipboard lasts until the next Cut/Copy or until the app quits. It is not persisted across app
    restarts or crash recovery: after recovery it is empty.
- **Same document.** Clipboard pieces reference the session's chunks, and the clipboard is a
  reachability root for compaction (ADR-004 §4). Paste is then a pure splice.
- **Across documents.** **Decided (autonomous, T-300): the clipboard survives opening another
  document.** Copying room tone or a pickup line from one file into another is a normal voice-over
  task, and Audition allows it.
  - **Materialize at close.** When the document is closed (one document at a time, PROMPT §2) and the
    clipboard is non-empty, its samples are written to `<app_local_data_dir>/clipboard/` (raw f32 LE
    plus a small `meta.json` with rate and length) **before** the session is garbage-collected
    (ADR-004 follow-up for T-302). SPEC-004 AC-12's "session directory gone within 5 s" still holds:
    materializing a 10-min clip is ~115 MB, well under 1 s on NVMe.
    - If materializing fails (e.g. disk full), the clipboard is cleared and a notice
      `notice.clipboard_lost` explains it. The close itself never fails because of the clipboard.
  - **First paste into the new document** imports the clip into the new session's chunk store through
    a `ChunkWriter`. That is sample I/O, so it is a **job** (§2.6.1).
  - **Later pastes.** After a same-rate import the clipboard is re-bound to the new session's chunks,
    and the materialized file is deleted, so later pastes are pure splices again. After a converted
    import (next bullet), the converted pieces are cached for later pastes into the **same** document.
    The original-rate file is kept for any other document, so audio is never converted twice.
  - The clipboard directory is deleted at app quit and, if stale, at start-up.
- **Sample-rate mismatch** (only possible across documents). **Decided (autonomous, T-300): paste
  converts.** The clip is resampled from its rate to the document rate with the same fixed-ratio
  resampler as playback's rate-mismatch path (`dsp::resample`, `rubato::Fft`, SPEC-003 §2.4, with its
  output delay compensated). The result has exactly `round(n × r_doc / r_clip)` samples, and a notice
  `notice.paste_resampled` says "Pasted audio was converted from 44.1 kHz to 48 kHz".
  - Refusing would leave no workaround, because v1 has no document sample-rate conversion.
  - Sample format never mismatches: documents are always f32 internally (PROMPT §2).

#### 2.6.1 Long pastes (jobs)

A paste that needs sample I/O (cross-document import or conversion) runs on a worker (ADR-002).
- **Before the job.** The transport stops before the job starts (§2.9), and the document is busy
  (§2.2) while it runs.
- **Progress.** A modal progress dialog "Pasting…" appears if the job is still running after
  **250 ms**, with Cancel (Esc). Progress arrives through `job_progress` (≤ 10 Hz, ADR-003) and is
  monotonic.
- **Cancel or failure** (SPEC-004 all-or-nothing):
  - the document is exactly as before: same `rev`, `audio_rev`, hash, undo and redo depths, no undo
    entry;
  - the clipboard is unchanged;
  - chunks already written are unreachable and are reclaimed by compaction;
  - Cancel takes effect within **100 ms**;
  - a failure additionally posts a notice.

### 2.7 Zero-crossing snap (SPEC-006 §2.10)

- **Snapping happens only when a selection boundary is placed or dragged**, if `snapToZeroCrossing` is
  on (SPEC-006). The operations use the stored selection exactly as it is and **never snap again at
  operation time**. Paste and Insert silence at a plain cursor (not a selection) are not snapped
  either, because SPEC-006 decided cursor placement is never snapped.
  - **Decided (autonomous, T-300):** what the view shows is exactly what is edited. An op-time snap
    would silently move an edit by up to 512 samples away from the visible boundary. Users who want
    click-free joins turn snap on before selecting, or select inside pauses and room tone, which is
    the normal voice-over practice.
- **Adjust commands.** Audition's separate "Zero Crossings → Adjust selection inward/outward/…"
  commands (Shift+I, Shift+O, Shift+H/J/K/L: killerkeys.com and pie-menu.com agree) are **out of scope
  for v1**. They are listed for SPEC-019 so that those keys stay free.

### 2.8 No micro-fades at edit boundaries

**Decided (autonomous, T-300): no fade or crossfade is applied at any edit boundary, and there is no
preference for one in v1.** Edits are sample-exact splices.
- **Exactness.** The ACs require exact sample equality (cut + paste round trips, silence equals exact
  zeros).
- **Cost.** A boundary fade would synthesize new audio at every join, and every Cut, Delete and Paste
  would stop being a pure piece splice with no sample I/O (ADR-004 §3).
- **Scope.** Fades are out of scope for v1 (PROMPT §3.8).
- **Avoiding clicks.** The tools for click-free joins are zero-crossing snap (§2.7) and editing inside
  pauses.

⚠ **Contradiction, not silently resolved:** this ticket's brief states "Audition does none by default".
Community evidence suggests otherwise. Audition's Preferences → Data has "Smooth delete and cut
boundaries" (2 ms crossfade) and "Smooth all edit boundaries by crossfading" (5 ms). An Adobe community
thread ([paste and silent artifacts](https://community.adobe.com/t5/audition-discussions/paste-and-silent-artifacts/m-p/9882257))
calls 5 ms "the default" and treats unchecked boxes as a user change, which implies they may be on by
default. The official Adobe pages return HTTP 403 to automated fetches, so this could not be settled.
The decision above stands on PowerVoice's own grounds (exactness, pure splices, PROMPT §3.8). It is
flagged in case Audition parity on this point matters later; an opt-in "smooth edit boundaries" setting
would then be a v1.x item.

### 2.9 Playback, recording and undo

- **Stops playback.** Cut, Paste, Delete, Trim, Silence and Insert silence are destructive audio edits
  (SPEC-004 §2.2):
  - the transport stops with the ~5 ms fade and waits for the audio thread's acknowledgement **before**
    the commit (ADR-004 §3, SPEC-004 AC-6);
  - playback does not resume by itself;
  - for jobs (§2.6.1), the stop happens before the job starts.
- **Doesn't stop playback.** Copy is not an edit: playback continues bit-identically. Trim of the whole
  document is a no-op and does not stop playback either.
- **Refused while recording** (§2.2).
- **Undo entries.** Each operation creates **exactly one** undo entry, including Trim, which is two
  splices internally. Labels are i18n keys, shown as "Undo ‹label›" (SPEC-004 §2.2):

  | Op | `label_key` | English |
  |---|---|---|
  | Cut | `history.cut` | Cut |
  | Paste | `history.paste` | Paste |
  | Delete | `history.delete` | Delete |
  | Trim to selection | `history.trim` | Trim to Selection |
  | Silence | `history.silence` | Silence |
  | Insert silence | `history.insert_silence` | Insert Silence |

  Copy creates none.

### 2.10 Performance

- **Pure splices.** Cut, Copy, same-document Paste, Delete, Trim, Silence and Insert silence are pure
  piece-table splices. They perform **no sample I/O** on the chunk store (0 bytes read, 0 bytes
  written), commit no chunks and compute no peaks (ADR-004 §3, §5). `audio_rev` still changes, so
  views refetch peaks, which are served from the existing per-chunk pyramids.
- **Latency.** On a 60-min, 48 kHz document with 20 000 pieces and 1 000 markers:
  - the splice itself (new piece vector, prefix sums and marker mapping) takes **≤ 5 ms**;
  - the whole command, from receipt with the transport stopped to its success reply, including the
    journal append and `fdatasync` on SSD, takes **≤ 50 ms**;
  - Copy, which has no journal record, takes ≤ 5 ms.
- **Fragmentation stays bounded.** After every splice, adjacent pieces are **merged** when they
  reference contiguous ranges of the same chunk, or are both Silence (§4.1). Cutting a range and
  pasting it back therefore restores the original piece table exactly.

### 2.11 Where the operations live, and shortcuts

- **Edit menu.** Cut, Copy, Paste, Delete, Trim to Selection, Silence, Insert Silence… in that order,
  under Undo/Redo.
- **Waveform right-click menu.** The same items with the same enablement.
  - Audition puts Silence under Effects. **Decided (autonomous, T-300):** PowerVoice keeps all seven in
    Edit, so there is one place for every destructive structural edit.
- **Focus.** Shortcuts act when the editor has focus and **no text field or modal dialog** does.
  Inside a text field, Ctrl+X/C/V/Delete keep their native text meaning. Bindings go through the
  keymap registry (T-104), are not remappable in v1 (PROMPT §2), and SPEC-019 (M7) audits them.

| Command | Windows / Linux | macOS | Status | Sources |
|---|---|---|---|---|
| Cut | Ctrl+X | ⌘X | **Verified** | [tutorialtactic](https://tutorialtactic.com/blog/adobe-audition-shortcuts/), [killerkeys](https://www.killerkeys.com/adobe-audition-keyboard-shortcuts) |
| Copy | Ctrl+C | ⌘C | **Verified** | tutorialtactic, killerkeys, [pie-menu](https://www.pie-menu.com/shortcuts/adobe-audition) |
| Paste | Ctrl+V | ⌘V | **Verified** | tutorialtactic, killerkeys |
| Delete | Delete | ⌫ (Delete) | **Verified** | killerkeys ("Delete key"), pie-menu (⌫) |
| Trim to selection (Crop) | Ctrl+T | ⌘T | **Verified** (two secondary sources) | killerkeys ("Crop: Ctrl + T"), pie-menu ("Crop: ⌘ + t") |
| Silence | — | — | **No default** in any source; menu only | Adobe help (search summary): *Effects > Silence*, no shortcut |
| Insert silence… | — | — | **No default** in any source; menu only | Adobe help (search summary): *Edit > Insert > Silence*, no shortcut |

- **Not bound.** Backspace is not bound in v1: Audition's Windows default is the Delete key, and
  SPEC-019 may add it.
- **Reserved.** "Mix Paste" (sources disagree: Shift+V vs Ctrl+Shift+V) and "Copy to New"
  (Alt+Shift+C) are not implemented in v1, and their keys are left unbound for SPEC-019.
- **Official page.** `helpx.adobe.com/audition/desktop/keyboard-shortcuts/default-keyboard-shortcuts.html`
  still returns HTTP 403 to automated fetches (as in SPEC-003 and SPEC-006).

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `insert_silence_duration` | Insert silence duration | s (or samples) | 1 sample … 3 600 s | 1.000 s | typed; `round(d × rate)` | remembered per app session (§2.5) |
| `clipboard_slots` | Clipboards | — | — | 1 | fixed | §2.6 |
| `edit_boundary_fade_ms` | Edit-boundary fade | ms | — | 0 | fixed | no micro-fades (§2.8) |
| `splice_budget_ms` | Pure splice time (60 min, 20 000 pieces, 1 000 markers) | ms | — | ≤ 5 | fixed | §2.10 |
| `edit_command_budget_ms` | Command → success reply, incl. journal `fdatasync` (SSD) | ms | — | ≤ 50 | fixed | §2.10, transport stopped |
| `job_dialog_delay_ms` | Progress dialog appears after | ms | — | 250 | fixed | long pastes (§2.6.1) |
| `job_cancel_latency_ms` | Cancel → job stopped | ms | — | ≤ 100 | fixed | §2.6.1 |

## 4. Algorithm / implementation notes

### 4.1 One primitive: `Replace`

Every structural edit is one or more `Replace { at, remove_len, pieces }` ops. This is exactly
ADR-004 §6's journal op; `insert_len` is the sum of the pieces' lengths.

| Op | Replace ops (applied in order, each on the result of the previous) | Marker mapping |
|---|---|---|
| Cut, Delete `[S,E)` | `Replace{S, E−S, []}` | §4.2 |
| Paste at `c` | `Replace{c, 0, K}` | §4.2 |
| Paste over `[S,E)` | `Replace{S, E−S, K}` | §4.2 |
| Trim `[S,E)` | `Replace{E, L−E, []}`, then `Replace{0, S, []}` (ops with `remove_len = 0` and no pieces are dropped) | §4.2, composed |
| Silence `[S,E)` | `Replace{S, E−S, [Silence(E−S)]}` | **identity** (length unchanged) |
| Insert silence at `p` | `Replace{p, 0, [Silence(m)]}` | §4.2 |

- **Splice.** Find the pieces covering `at` and `at + remove_len` by binary search on the prefix sums
  (`starts`, ADR-004 §3). Split at most two pieces, then build the new `Vec<Piece>` and prefix sums.
  That is O(pieces), well under 1 ms at 20 000 pieces per ADR-004.
- **Merge.** Afterwards, merge neighbours `(Chunk(c), o₁, l₁)` + `(Chunk(c), o₁ + l₁, l₂)` into
  `(Chunk(c), o₁, l₁ + l₂)`, and `Silence(l₁)` + `Silence(l₂)` into `Silence(l₁ + l₂)`. Merging is
  only done while the merged length fits `Piece.len: u32`.
- **Long silences.** A Silence op longer than `u32::MAX` samples (about 24.8 h at 48 kHz) is split into
  several pieces. With the 1 h insert limit this only matters for Silence over very long selections.
- **Copy** clones the covered pieces (sub-ranges trimmed at the ends) into the clipboard. No snapshot
  is created.
- **Commit** (ADR-004 §3, SPEC-004 §4):
  1. validate the request (§4.3);
  2. stop the transport and wait for the acknowledgement;
  3. build the snapshot;
  4. append the journal `edit {seq, label_key, ops, marker_ops}` and `fdatasync` it;
  5. swap the `Arc`;
  6. hand the reader the new snapshot;
  7. reply.

  Only then does Cut replace the clipboard. `rev` and `audio_rev` both bump. A no-op (whole-document
  Trim) returns before step 2.

### 4.2 Position mapping for markers

For `Replace{at = a, remove_len = r, insert_len = n}`:

```
map_start(p):                       // point markers and region starts
  p <  a                → p
  r == 0 and p ≥ a      → p + n     // insertion: at or after shifts right
  r >  0 and p < a + r  → a         // inside the removed range → its start
  r >  0 and p ≥ a + r  → p − r + n // after it → shifted

map_end(e):                         // region exclusive end e = pos + len, only when len > 0
  e ≤ a                 → e
  r == 0 and e > a      → e + n
  r >  0 and e ≤ a + r  → a
  r >  0 and e > a + r  → e − r + n

new_len = map_end(e) − map_start(pos)   // always ≥ 0 by construction; 0 turns the region into a point
```

- This matches SPEC-004 AC-7. Cut `[2 s, 4 s)` sends markers at 1/3/10 s to 1/2/8 s, and inserting 1 s
  at 1 s moves the 1 s marker to 2 s.
- For Trim, composing the two ops gives: `p < S → 0`, `S ≤ p < E → p − S`, `p ≥ E → E − S`.
- Marker ids and names never change, and the order of markers at the same position is stable.

### 4.3 Validation and IPC

- **Commands** (names are T-302's choice; this is the required shape). Each takes the `rev` the UI
  based its selection on (`base_rev`) and an explicit target:
  - `edit_cut {base_rev, range}`, `edit_copy {base_rev, range}`, `edit_delete {base_rev, range}`,
    `edit_trim {base_rev, range}`, `edit_silence {base_rev, range}`;
  - `edit_paste {base_rev, target: cursor | range}`;
  - `edit_insert_silence {base_rev, target: cursor | range, len_samples}`.

  `target: cursor` means "the playhead after the stop", which the engine resolves (§2.3), so the UI
  never guesses the heard position.
- **Result.** Each returns `EditResult { changed, rev, audio_rev, len_samples, selection, playhead_samples }`,
  or a `job_id` for long pastes, which finish with `document_changed` plus a result event. Rejections
  change nothing:

  | Condition | `IpcError.key` |
  |---|---|
  | `base_rev ≠ rev` | `error.document_changed`; the UI refreshes and the user retries |
  | `S ≥ E` | `error.no_selection` |
  | `E > L`, or `c > L` | `error.invalid_range` |
  | clipboard empty | `error.clipboard_empty` |
  | `len_samples` outside §2.5 | `error.insert_silence_duration` |
  | recording | `error.not_while_recording` |
  | a job is running | `error.document_busy` |

- **Clipboard state.** A new low-rate event `clipboard_changed { len_samples, sample_rate_hz }` (or
  `null` when empty) lets the UI enable Paste. It is one more ADR-003 §1 event (≤ 10 Hz), not a new
  mechanism.
- **i18n keys.** Menu and context items are `edit.cut`, `edit.copy`, `edit.paste`, `edit.delete`,
  `edit.trim`, `edit.silence`, `edit.insert_silence`. The dialog uses `dialog.insert_silence.*`, the
  job label is `job.paste`, and the notices are `notice.paste_resampled` and `notice.clipboard_lost`.
  Undo labels are in §2.9.

### 4.4 Cross-document paste

- **Materialize at close.** Streaming the clipboard's pieces through the snapshot sample reader into
  `clipboard/clip.f32` is sequential, allocation-bounded and not fsynced: the clipboard is not
  crash-durable.
- **Paste into the new document.** Stream the file (through `dsp::resample` when the rates differ) into
  a `ChunkWriter`. Chunk commits compute peaks (ADR-004 §2, §5). Then commit one `Replace` whose pieces
  reference the new chunks.
- **Cancellation** uses the worker's generation token and is checked at least once per chunk
  (65 536 samples), which bounds cancel latency far below 100 ms on SSD.
- **Resampler.** It is primed, its output delay is discarded, the input is zero-padded at the end to
  flush, and the output is truncated to exactly `round(n × r_doc / r_clip)` samples. This is the same
  alignment contract as SPEC-003 §4.

## 5. Acceptance criteria

All sample comparisons use testkit (`fnv1a_hash`, `null_test_db`, `peak_dbfs`). `-inf` is digital
silence and `NaN` is non-finite input (SPEC-000 §2.4). Unless stated, the document is 10 min of seeded
white noise at −20 dBFS RMS at 48 kHz, imported (one piece per chunk) and then fragmented by 500 seeded
random splices.

- **AC-1 (cut + paste round trip is exact).** Given any seeded `[S, E)` (100 cases, including `S = 0`,
  `E = L`, and boundaries inside and at chunk edges), when Cut and then Paste at `S` are applied:
  - the document's FNV-1a hash and length equal the original;
  - the piece table equals the original piece table after merging (§4.1);
  - between the two steps, the clipboard's samples hash equal to the original `A[S, E)`.
- **AC-2 (copy).** Given a selection, when Copy is invoked:
  - `rev`, `audio_rev` and the undo depth are unchanged, and no journal record is written;
  - the clipboard hashes equal to `A[S, E)`;
  - during fake-backend playback, the output is bit-identical to a control run without the Copy.
- **AC-3 (paste inserts at the cursor / replaces a selection).**
  - With no selection and the cursor at `c`, Paste gives exactly `A[0,c) ‖ K ‖ A[c,L)`, with
    `L' = L + n`, selection `[c, c + n)` and playhead `c`.
  - With a selection `[S, E)`, Paste gives exactly `A[0,S) ‖ K ‖ A[E,L)` for `n < E − S`,
    `n = E − S` and `n > E − S`, with selection `[S, S + n)`.
  - `c = 0` and `c = L` (append) are included.
- **AC-4 (delete and trim).**
  - Delete `[S, E)` gives exactly `A[0,S) ‖ A[E,L)`, with no selection, playhead `S` and the clipboard
    unchanged (hash equal to before).
  - Trim `[S, E)` gives exactly `A[S,E)` with length `E − S`, no selection and playhead 0.
  - Trim `[0, L)` returns `changed = false` with no undo entry and no transport stop.
  - Delete `[0, L)` gives length 0 and is undoable.
- **AC-5 (silence is exact zeros).** Given a selection `[S, E)`, Silence gives:
  - every sample in `[S, E)` exactly `+0.0`; `peak_dbfs` of that range is `-inf`;
  - samples outside it bit-identical to before (hashes of `[0,S)` and `[E,L)` equal);
  - the length unchanged;
  - the selection still `[S, E)`;
  - no marker moved;
  - 0 bytes written to the chunk store.
- **AC-6 (insert silence durations).**
  - At 48 kHz, "1.000" inserts **48 000** samples of exact `+0.0` at `c`, and `L' = L + 48 000`.
  - At 44.1 kHz, "1.000" inserts 44 100 and "0.5" inserts 22 050.
  - At 48 kHz, "0:01.500" inserts 72 000 and "480 smp" inserts 480.
  - The inserted range becomes the selection.
  - With a selection `[S, E)`, silence is inserted at `S` and `A[S, E)` follows it unchanged.
  - The dialog rejects "0", "3600.001" and "abc" (OK disabled), accepts "3600", and a bypassing command
    with `len_samples = 0` returns `error.insert_silence_duration`.
- **AC-7 (marker mapping).** Given a 48 kHz document of 20 s with point markers at 1, 3, 5, 10 and
  20 s (the last one at `L`) and a region [6 s, 8 s):

  | Op | Expected markers (s) | Expected region |
  |---|---|---|
  | Cut or Delete [2, 4) | 1, 2, 3, 8, 18 | [4, 6) |
  | Paste of 1.5 s at 5 | 1, 3, 6.5, 11.5, 21.5 | [7.5, 9.5) |
  | Paste of 1 s over [2, 4) | 1, 2, 4, 9, 19 | [5, 7) |
  | Insert silence 1 s at 1 | 2, 4, 6, 11, 21 | [7, 9) |
  | Insert silence 1 s at 8 (the region's end) | 1, 3, 5, 11, 21 | [6, 8) (not extended) |
  | Insert silence 1 s at 7 (inside the region) | 1, 3, 5, 11, 21 | [6, 9) |
  | Trim [4, 12) | 0, 0, 1, 6, 8 | [2, 4) |
  | Delete [5.5, 9) | 1, 3, 5, 6.5, 16.5 | point at 5.5 (length 0) |
  | Silence [2, 12) | 1, 3, 5, 10, 20 (unchanged) | [6, 8) |

  All positions are exact in samples (seconds × 48 000). The numbers of markers, their ids and their
  names are unchanged in every row, and undo restores the original list exactly.
- **AC-8 (no selection, disabled states).**
  - With `selection = null` and with an empty selection `[S, S)`, Cut, Copy, Delete, Trim and Silence
    are disabled in the Edit menu, the context menu and the keymap (Vitest).
  - The corresponding commands called directly return `error.no_selection` and change nothing (`rev`
    and hash equal).
  - Paste is disabled when the clipboard is empty (`error.clipboard_empty` via direct command).
  - Insert Silence… is enabled with no selection.
  - All seven are disabled while recording or while a document job runs.
- **AC-9 (zero-crossing snap interplay).** Given `snapToZeroCrossing` on and a stored selection
  `[S, E)` whose boundaries are deliberately not at zero crossings (set programmatically), Cut removes
  exactly `[S, E)`. Given the cursor at a non-crossing sample `c`, Paste inserts exactly at `c`. No op
  reads samples to snap.
- **AC-10 (no micro-fades).** Given a document of constant DC `0.5` (every sample exactly `0.5f32`),
  after Cut, Delete, Paste at a cursor, Paste over a selection and Trim, every sample of the result is
  exactly `0.5f32`: there is no dip at any join.
- **AC-11 (stops playback before commit; Copy doesn't).** Given fake-backend playback, for each of Cut,
  Paste, Delete, Trim, Silence and Insert silence (SPEC-004 AC-6):
  - the output fades to digital silence within **6 ms** of the stop command;
  - no output sample after the commit comes from the old revision;
  - the transport-stop event precedes the snapshot swap;
  - playback does not resume;
  - the playhead then equals the §2.3 value. For Paste and Insert silence without a selection, the
    insertion point equals the heard position at the stop (±1 sample).

  Copy during playback leaves the output bit-identical to a control run.
- **AC-12 (refused while recording).** Given a recording in progress, each of the seven commands
  returns `error.not_while_recording`, and the document, the clipboard and the take are unchanged.
- **AC-13 (undo/redo, labels, clipboard independence).**
  - Each op except Copy adds exactly one undo entry with the `label_key` from §2.9. Trim adds one entry
    containing two `Replace` ops.
  - Undo restores the prior audio hash and marker list, and redo restores the post-op ones.
  - Undoing a Cut leaves the clipboard hash unchanged.
  - After undo, the selection is `null` and the playhead is at `min(at)` (§2.3).
  - The Edit menu reads "Undo Trim to Selection" after a Trim (en locale).
- **AC-14 (performance, no sample I/O).** Given a 60-min 48 kHz document with **20 000** pieces (chunk
  and silence mixed, built by a seeded splice script) and 1 000 markers, for 100 seeded runs of each
  op (Copy, Cut, same-document Paste, Delete, Trim, Silence, Insert silence) with the transport stopped:
  - the pure splice plus marker mapping takes ≤ **5 ms** p95 (release build);
  - command receipt → success reply, including the journal `fdatasync` on the reference NVMe SSD,
    takes ≤ **50 ms** p95;
  - the chunk store's counters show **0** sample bytes read, **0** bytes written, **0** chunks committed
    and **0** peak pyramids computed across all 700 runs.
- **AC-15 (cross-document paste, same rate).** Given a 2.000 s Copy from document A (48 kHz), when A
  is closed (Don't Save) and document B (48 kHz) is opened:
  - Paste into B inserts samples whose hash equals A's copied range;
  - A's session directory is gone within 5 s of the close (SPEC-004 AC-12);
  - a second Paste into B is a pure splice (0 bytes written).
- **AC-16 (cross-document paste with conversion).** Given a Copy of 1.000 s of a 1 kHz sine at
  −6 dBFS from a 44.1 kHz document, when pasted into a 48 kHz document:
  - the inserted length is exactly **48 000** samples;
  - its dominant frequency is 1 kHz ± 0.05 %;
  - the sample peak of its middle 0.5 s is −6.00 ± 0.10 dBFS;
  - the notice `notice.paste_resampled` is posted;
  - it is one undo entry.

  A second paste into the same document reuses the converted pieces (0 bytes written). Pasting into a
  third 44.1 kHz document (after closing the 48 kHz one) inserts samples hash-equal to the original
  44.1 kHz copy, not converted twice.
- **AC-17 (long paste is all-or-nothing).** Given a cross-document paste of a 30-min clip, when it is
  cancelled at ≈ 50 % progress:
  - the job stops within 100 ms;
  - `rev`, `audio_rev`, the audio hash, the undo/redo depths and the clipboard are unchanged, and no
    undo entry is added.

  The same holds with an injected I/O error, which additionally posts a notice.
- **AC-18 (stale selection rejected).** Given a UI selection based on revision r, when the document
  moves to r + 1 (e.g. a marker edit) before the command arrives, then the command returns
  `error.document_changed` and changes nothing.
- **AC-19 (shortcuts and focus).** In Vitest with the keymap registry:
  - Ctrl+X, Ctrl+C, Ctrl+V, Delete and Ctrl+T dispatch Cut, Copy, Paste, Delete and Trim on
    Windows/Linux, and ⌘X/⌘C/⌘V/⌫/⌘T on macOS;
  - Silence and Insert Silence have no binding;
  - with focus in a text input, or with a modal dialog open, these keys dispatch nothing.

## 6. Test plan

| AC | Unit (`project`) | Integration (engine, fake backend) | Vitest (UI, mockIPC) | Manual smoke (owner, Linux) |
|---|---|---|---|---|
| AC-1 | splice + merge property test, seeded ranges | — | — | cut a word and paste it back, listen |
| AC-2 | copy = piece clone, no snapshot | output bit-identity during Copy | — | Ctrl+C while playing |
| AC-3 | insert/replace splices, hash vs concatenated reference | — | selection/playhead applied from `EditResult` | paste a retake over a flub |
| AC-4 | delete/trim splices, whole-document cases | — | — | trim head/tail of a take |
| AC-5 | silence pieces read back as `+0.0`; store write counter | — | — | silence a cough |
| AC-6 | duration → samples conversion | — | dialog parsing and validation matrix | insert 1 s at a pause |
| AC-7 | `map_start`/`map_end` table test + composed trim | — | markers redraw at mapped positions | cut across markers, check the panel |
| AC-8 | command validation errors | — | menu/context/keymap enablement matrix | try Delete with nothing selected |
| AC-9 | ops never call the sample reader | — | snap on + programmatic selection | snap on, cut, confirm the removed range |
| AC-10 | DC-0.5 join test for all ops | — | — | listen to joins inside a steady tone |
| AC-11 | — | stop → ack → commit ordering, heard-position cursor | — | edit while playing |
| AC-12 | refusal API | fake recording + commands | disabled controls while recording | try Ctrl+X while recording |
| AC-13 | one entry per op, labels, clipboard after undo | undo/redo via engine | "Undo ‹label›" text | Ctrl+Z / Ctrl+Shift+Z each op |
| AC-14 | splice bench (divan, `just bench`) + I/O counters | command latency bench on a 60-min fixture (`just fixtures`) | — | edit a 60-min file, feel for lag |
| AC-15 | clipboard materialize / rebind | close A → open B → paste | Paste enabled from `clipboard_changed` | copy room tone between files |
| AC-16 | resample length contract | 44.1 k → 48 k paste; testkit frequency and peak | notice shown | paste between rates |
| AC-17 | `ChunkWriter` cancel/fail | job cancel at 50 %, injected I/O error | progress dialog after 250 ms, Cancel/Esc | cancel a long paste |
| AC-18 | `base_rev` check | — | UI refresh on `error.document_changed` | — |
| AC-19 | — | — | keymap + focus tests | shortcuts in the running app |

Fixtures are seeded testkit signals (`white_noise`, `sine`, a DC constant), the 60-min fixture from
`just fixtures`, and piece tables built by seeded splice scripts in test code. Nothing is committed.

## 7. Out of scope

- Mix Paste, Paste to New, Copy to New, multiple clipboards, and OS clipboard interoperation (§2.6,
  §2.11).
- Fades, crossfades and edit-boundary smoothing (PROMPT §3.8, §2.8).
- Zero-crossing *adjust* commands (Shift+I/O/H/J/K/L). Listed for SPEC-019.
- Copying or pasting markers with audio.
- Record at cursor, overwrite and punch-in (T-304, own spec).
- Normalize (SPEC-010), LUFS normalize and bake (M6).
- The markers panel (SPEC-009, T-303).
- Undo/redo mechanics, the journal and recovery (SPEC-004, T-301), beyond the post-undo cursor rule in
  §2.3.
- Document sample-rate conversion. Stereo (PROMPT §2: mono only).
