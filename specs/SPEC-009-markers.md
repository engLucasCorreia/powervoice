# SPEC-009 — Markers: model, add, rename, move, delete, navigate, Markers panel

- **Status:** approved (autonomous, T-300)
- **Milestone:** M3 (T-303). Depends on T-301 (undo/redo, journal), the M2 marker drawing (SPEC-006
  §2.11) and WAV cue I/O (SPEC-005 §2.9, T-201/T-202). Sidecar persistence is T-306 (SPEC-018), which
  implements §2.13's precedence rules.
- **Related:** SPEC-000 (glossary: document time, heard position, edit), SPEC-002 (markers during
  recording, dropout markers), SPEC-003 (§2.2 extrapolation, seek semantics), SPEC-004 (§2.2 undo
  table, AC-6, AC-7), SPEC-005 (§2.9 cue/adtl read/write, sidecar precedence), SPEC-006 (§2.9 selection,
  §2.11 marker drawing, §4.1 pixel↔sample mapping), SPEC-008 (§2.4/§4.2 marker mapping under audio
  edits, §2.11 focus rule), SPEC-018 (sidecar), SPEC-019 (full shortcut map, M7) · ADR-003 (commands,
  `document_changed`, `notice`), ADR-004 (§3 `Marker`, markers in the snapshot, marker edits don't stop
  playback; §6 journal `marker_ops`) · PROMPT §2 (LOCKED: persistence), §3.3 (markers), §3.6 (layout,
  M = add marker)

## 1. Purpose

A voice-over editor uses markers as notes on the timeline. They flag a fluffed line while the talent
reads ("retake p. 12"), mark chapter starts, and mark the regions to send to the client. After a long
take, they also show each place where the recording was damaged (SPEC-002 dropouts). Markers must be:
- **instant to place**, including mid-playback and mid-take, exactly where the user heard the moment;
- **cheap to manage** in a list: rename, jump to, select, delete, even with thousands of them;
- **never in the way**: a marker edit never interrupts playback, and one Ctrl+Z undoes it;
- **durable**: they survive save, reopen, crashes, and a round trip through other editors via the WAV
  `cue ` chunk.

SPEC-004 defines what "undoable" and "doesn't stop playback" mean. SPEC-008 defines how audio edits move
markers. SPEC-006 draws them. This spec defines the marker model, every marker operation, and the
Markers panel.

## 2. Behavior / UX

### 2.1 The marker model

A marker is `{ id, pos_samples, len_samples, name, kind }` in document time (SPEC-000 §2.4).

| Field | Meaning | Rules |
|---|---|---|
| `id` | Stable identity (`MarkerId`, u64) | Unique within the document; never reused within a session (T-101 `allocate_marker_id`). Survives renames, moves and audio edits. Persisted in the sidecar (SPEC-018). |
| `pos_samples` | Start position | `0 ≤ pos ≤ L`, where L is the document length. A point marker at `L` (the end of the file) is legal, as SPEC-005 AC-7 already requires. |
| `len_samples` | 0 = **point** marker; > 0 = **region** marker | `pos + len ≤ L`. The exclusive end is `pos + len`, the same convention as the selection (SPEC-006 §2.2). |
| `name` | Free text | 1 … 1024 bytes of UTF-8 after normalization (§2.4). Several markers may share a name. |
| `kind` | `user` or `dropout` (plus preserved unknown kinds, SPEC-018 §2.7) | Set at creation and never changed by the user in v1. `dropout` markers are created only by the recorder (SPEC-002 §2.4). |

- **Point vs region** is derived from `len_samples`. It is not a separate field. PROMPT §3.3's "region"
  and Audition's "range marker" mean the same thing.
- **Canonical order.** The document's marker list is always sorted by `pos_samples`, then by `id`
  ascending. Ids grow with creation, and markers read from a WAV get ids in cue order. So this order
  equals SPEC-005 §2.9's "by position, then by cue order", and SPEC-008's "stable order at the same
  position". Several markers may share a position (SPEC-004 §2.2).
- **Type** (what the UI shows) is **Dropout** when `kind = dropout`, otherwise **Region** when
  `len > 0`, otherwise **Point**. Dropout markers are point markers in v1 (SPEC-002 places them "at the
  start of the gap").
- ⚠ **Structural delta, flagged (not silently resolved):** ADR-004 §3's `Marker` struct (and T-101's
  implementation) has no `kind`. This spec adds `kind: MarkerKind { User, Dropout, Other(Arc<str>) }`,
  defaulting to `User`, to the snapshot marker and to the journal's marker records. It is additive, and
  `Other` exists only so that unknown kinds from newer sidecars round-trip (SPEC-018 §2.7). This needs a
  one-line ADR-004 amendment.

### 2.2 Adding a marker

**Entry points:**
- **M**, verified in four independent sources: [killerkeys](https://www.killerkeys.com/adobe-audition-keyboard-shortcuts)
  ("Insert marker: M"), [tutorialtactic](https://tutorialtactic.com/blog/adobe-audition-shortcuts/),
  [pie-menu](https://www.pie-menu.com/shortcuts/adobe-audition) and
  [domestika](https://www.domestika.org/en/blog/7961-35-essential-shortcuts-for-adobe-audition).
  SPEC-002/004 already use it. Only unmodified `m`/`M` is bound; Shift+M stays unbound (§2.7).
- The **+** button in the Markers panel header.
- **Add Marker** in the waveform's right-click menu.

**Where the new marker goes:**

| Situation when invoked | Result |
|---|---|
| **Recording** | A point marker at the take position under the key press (SPEC-002 §2.2), committed with the take on Stop. |
| **Playing** (including looping), with or without a time selection | A point marker at the **heard position** under the key press (§4.3). |
| Stopped or paused, **non-empty selection** `[S, E)` | A **region** `pos = S`, `len = E − S`. |
| Stopped or paused, no selection (or an empty one, `S = E`) | A point marker at the cursor `c` (SPEC-008 §2.1). |
| Right-click menu → Add Marker, stopped or paused | A point marker at the right-clicked sample `sample(px)` (SPEC-006 §4.1). |
| Panel **+** button | Same as M. |

- Audition creates a range marker when M is pressed with a selection ("select the region that you want
  to convert to a range, then type M"; Adobe help, via search summary and
  [Larry Jordan](https://larryjordan.com/articles/adobe-audition-cc-using-markers/)). The official
  page returns 403.
- **Decided (autonomous, T-300): during playback or recording, M always adds a point marker, even when a
  selection exists.** While listening, M is the "flag this moment" key, and a selection left over from
  earlier must not turn the flag into a region the user never meant. Selection → region happens only
  when the transport is stopped or paused, where it is a deliberate act.

**Other rules:**
- **No auto-rename.** The new marker gets a default name (§2.3) and becomes the single **panel
  selection** without being *activated* (§2.8), so it neither seeks nor changes the time selection. The
  user can press `/` right away to rename it.
  - **Decided (autonomous, T-300):** opening a name editor automatically would steal keystrokes (Space,
    M) mid-playback. This is Audition's M then `/` flow.
- **Key repeat is ignored.** Holding M adds one marker (`KeyboardEvent.repeat` events are dropped).
  Pressing M twice in quick succession adds two markers, even at the same sample.
- **Focus.** Like every editor shortcut, M acts only when no text field or modal dialog has focus
  (SPEC-008 §2.11). In the panel's filter box or name editor, it types "m".
- **Limit.** Once the document holds `max_user_markers` = 10 000 markers, Add is disabled with the
  tooltip and notice "Marker limit reached (10 000)" (`notice.marker_limit`, command error
  `error.marker_limit`). See §2.12 for markers that don't count against it.
- **Undo.** One undo entry, `history.marker_add` ("Add Marker"). Playback doesn't stop (§2.9).
- **While importing** (SPEC-005 §2.3), marker commands are disabled like every other edit.

### 2.3 Default names

- **Format.** New user markers (points and regions alike) are named **"Marker NN"**, with N zero-padded
  to at least two digits: "Marker 01", "Marker 02", …, "Marker 99", "Marker 100". This follows Audition:
  "markers are given default names like 'Marker 01'"
  ([jjloomis Audition guide](https://jjloomis.gitbook.io/adobe-audition-basic-audio-editing/listening-and-logging/adding-range-markers-to-wav-file),
  search summary of Adobe help).
- **Numbering.** N = 1 + the largest number among existing markers whose name matches the locale's
  default pattern (en: `^Marker (\d{1,6})$`, matching "Marker 7" as well as "Marker 07"), or 1 if none
  match. Renamed markers don't count, and deleting the highest-numbered one frees its number.
  - **Decided (autonomous, T-300): max + 1, not "first gap".** Numbers then follow creation order, so the
    newest marker always has the highest number, even after some are deleted.
- **i18n.** The name is generated in the current locale when the marker is created (key
  `marker.default_name`, parameter `n`) and stored as plain text. It does not change when the locale
  changes.
- **Dropout markers** are named by SPEC-002 ("Dropout 12 ms", `marker.dropout`) and do not take part in
  numbering.
- ⚠ **Inconsistency, flagged:** SPEC-005 §2.9 gives an unnamed WAV cue the fallback name "Marker N"
  (unpadded, N = order by position), and its AC-7/AC-8 expect "Marker 1". This spec leaves SPEC-005's
  read fallback unchanged so that M2 work is not reopened. The numbering pattern above accepts both
  spellings, so a file with "Marker 1"…"Marker 7" continues with "Marker 08". SPEC-019/M7 may unify the
  padding.

### 2.4 Renaming

**Entry points:**
- **`/`** renames the single selected marker. Verified in three sources: killerkeys ("Rename selected
  marker: /"), tutorialtactic and pie-menu. The binding matches the produced character `key === "/"`,
  so it works on layouts where `/` needs Shift.
- **Double-click** the Name cell in the panel.
- **F2** while the panel has focus (the OS list convention).
- **Double-click a marker's flag** on the waveform, or choose **Rename** in the flag's right-click menu.

**Where the editor opens.** In the panel's Name cell when the panel is visible. When the panel is hidden,
or when renaming from a flag, a small popover editor opens anchored at the flag. Flag hit-testing takes
priority over SPEC-006 §2.9's double-click-selects-all, but only inside the flag hit box (§2.5).

**Editing:**
- **Enter** or a click elsewhere (blur) commits. **Esc** cancels.
- `/` with no selected marker, or with ≥ 2 selected, does nothing.

**Normalization**, applied in Rust on commit, the same rules SPEC-005 §4.6 uses when reading cue names:
- trim leading and trailing white space;
- strip control characters other than tab;
- cut to at most 1024 bytes at a character boundary.

**Rejections (no undo entry):**
- **Empty** after normalization: the rename is cancelled, the old name stays, and the field flashes an
  error outline for 1 s (`error.marker_name_empty` from a direct command).
- **Unchanged** after normalization: nothing happens.

**Other rules:**
- **Kind is kept.** Renaming a dropout marker keeps `kind = dropout`.
- **Undo.** One entry, `history.marker_rename` ("Rename Marker"). Allowed during playback. Disabled
  during recording (§2.11).

### 2.5 Moving and resizing

**On the waveform.**
- **Flags.** SPEC-006 §2.11 draws a flag at the top of each marker line. A region also gets an **end
  flag** (mirrored) at `pos + len`. The **flag hit box** is the flag's drawn area, widened to at least
  10 px (± 5 px around the line) and 12 px high at the top edge of the canvas. The marker line below the
  flag is **not** a hit target, so dragging on the waveform near a marker still makes a time selection.
- **Click vs drag.** Pressing a flag and moving the pointer ≥ 3 px starts a drag. Releasing before that
  is a **click**, which activates the marker (§2.8).
- **What the drag changes:**

  | Drag | Changes |
  |---|---|
  | A point marker's flag | `pos` |
  | A region's start flag | the start. The end stays, so `len` changes. |
  | A region's end flag | the end. `pos` stays. |
  | **Shift** + drag of either region flag | the whole region moves, keeping `len` |

- **Position.** The target sample is `sample(px)` (SPEC-006 §4.1), clamped so that `0 ≤ pos` and
  `pos + len ≤ L`.
  - A region's edges can't cross: a start drag stops at `end − 1`, and an end drag stops at `pos + 1`,
    so `len ≥ 1` throughout a drag.
  - A drag never turns a region into a point; typing Duration 0 does (§2.8).
- **Magnet.** Within **6 px** (SPEC-006's handle hit width), the dragged edge snaps exactly to the
  nearest of:
  - the cursor (when stopped or paused);
  - the selection start and end;
  - every other marker's start and end (the dragged marker's own other edge is excluded).

  The nearest target wins; at equal distance, the earlier sample wins. Holding **Alt** (Option on macOS)
  while dragging disables the magnet.
  - **Decided (autonomous, T-300):** markers are notes, not edit boundaries, so there is **no
    zero-crossing snap** for markers (SPEC-006 §2.10 applies to selection boundaries only). Snapping to
    the cursor and the selection is what makes "move this marker to here" precise.
  - Audition's global Snapping toggle (**S**, killerkeys/pie-menu) is not implemented in v1; S stays
    unbound for SPEC-019.
- **Auto-scroll.** While the pointer is beyond the left or right canvas edge during a drag, the view
  scrolls toward it at one viewport width per second.
- **Preview, cancel, commit:**
  - During the drag, only the UI draws the marker at its preview position. The document does not
    change, so there is no `rev` churn.
  - **Esc** cancels the drag, and nothing is committed.
  - **Release** commits one undo entry, `history.marker_move` ("Move Marker") for point drags and
    Shift-drags, or `history.marker_resize` ("Resize Marker") for region edge drags. Releasing at the
    original position commits nothing.
- **Playback and recording.** Dragging is allowed during playback and never affects the audio (§2.9).
  During recording, flags are not draggable.

**In the panel** (typed values; see §2.8 for the columns). The Start, End and Duration cells of a row
are editable by double-click or F2 + Tab. They accept the SPEC-008 §2.5 duration grammar:
- seconds (`2`, `2.5`);
- timecode `[[hh:]mm:]ss[.fff]`;
- an integer followed by `smp`.

Seconds convert with `round(t × rate)`, half away from zero.

| Edited cell | Effect | Undo label |
|---|---|---|
| Start | moves the marker to the new start, keeping `len` | `history.marker_move` |
| End (regions) | sets the exclusive end `pos + len`, keeping `pos` | `history.marker_resize` |
| Duration (regions) | sets `len`, keeping `pos`. **0 turns the region into a point**, consistent with SPEC-008 §2.4's collapse rule | `history.marker_resize` |

A value that would violate §2.1 (e.g. `pos + len > L`, or End ≤ Start) shows an error outline and
commits nothing.

### 2.6 Deleting

**Commands:**
- **Delete Selected Markers:** **Ctrl+0** (⌘0). Verified in three sources: killerkeys ("Delete selected
  marker: Ctrl + 0"), tutorialtactic and pie-menu. Also the panel's Delete button, and **Delete** in a
  flag's right-click menu (which deletes that flag's marker).
- **Delete All Markers:** **Ctrl+Alt+0** (⌘⌥0). Verified in three sources: killerkeys, tutorialtactic
  and pie-menu. Also in the panel menu.
- **Delete Filtered Markers (N)** in the panel menu, shown only while a panel filter is active (§2.8).
  It deletes exactly the rows the filter shows. Typical use: clearing all dropout markers after the
  repairs are done.

**The Delete key: focus rule** (extends SPEC-008 §2.11):

| Keyboard focus | Key | Effect |
|---|---|---|
| Markers panel list (a row focused, no cell editor open) | **Delete** (⌫ on macOS) | Deletes the **selected markers**. Does nothing if none are selected. Never touches audio. |
| A text field (panel filter, name/time cell editor, any dialog) | Delete / Backspace | Native text editing only |
| Editor (waveform or spectral pane) | **Delete** | SPEC-008 audio **Delete** of the time selection. Disabled without one. **Never deletes markers**, even when markers are selected in the panel. |
| Anywhere except text fields and modal dialogs | **Ctrl+0** | Deletes the selected markers |
| Anywhere except text fields and modal dialogs | **Ctrl+Alt+0** | Deletes all markers |

- **How focus moves.** Clicking a panel row gives the panel focus. Clicking the waveform, or a flag on
  it, gives the editor focus.
- ⚠ **Consequence, accepted:** clicking a region's flag activates it, and so time-selects its range
  (§2.8). The editor then has focus, and **Delete** deletes that audio (SPEC-008), not the marker. This
  is Audition's model as well. Ctrl+0 and the panel are the marker-delete paths, and Ctrl+Z restores
  the audio.
- **Decided (autonomous, T-300): no confirmation dialog for any marker delete, Delete All included.**
  Every delete is one undo entry. After a delete of more than one marker, the notice "Deleted 37
  markers" (`notice.markers_deleted`, parameter `count`) offers an **Undo** action.
- **Undo.** One entry per command, whatever the count: `history.marker_delete` ("Delete Marker" /
  "Delete Markers" by count). Allowed during playback. Disabled during recording.
- **After a delete,** the panel selection is empty. The time selection and playhead are unchanged.

### 2.7 Navigating

**Commands:**
- **Next marker:** **Ctrl+Alt+→** (⌘⌥→).
- **Previous marker:** **Ctrl+Alt+←** (⌘⌥←).
- **Verified** in three sources: tutorialtactic ("Move to next marker: Ctrl + Alt + Right Arrow"),
  pie-menu ("⌘ + ⌥ + →") and domestika ("Ctrl + Alt + Right Arrow → Move to next marker").
- Also available as panel menu items and View → Go to Next/Previous Marker.

⚠ **Contradictions, not silently resolved:**
- [Larry Jordan's Audition CC article](https://larryjordan.com/articles/adobe-audition-cc-using-markers/)
  lists **Shift+M** / **Option+M** for next/previous marker.
- killerkeys lists **Ctrl+←/→** as "Move CTI to previous/next".
- domestika lists **Ctrl+←/→** as "Set time indicator to previous/next marker/clip", while pie-menu
  lists **⌥←/→** for that same command.

Only the three-source Ctrl+Alt+arrow binding is used. Ctrl+←/→, Alt+←/→, Shift+M and Alt+M stay
**unbound**, listed for SPEC-019.

⚠ **Platform risk:** some Linux desktops (e.g. GNOME's legacy workspace switching) grab Ctrl+Alt+←/→
before the app sees them. The menu items remain; SPEC-019 re-checks this.

**Reference position p:**
- **stopped or paused:** the cursor;
- **playing:** the heard position at the key press (§4.3);
- **recording:** navigation is disabled, because seeking is disabled while recording (SPEC-002 §2.2).

**Target** (region **starts** only; region ends are not stops):
- **Next:** the marker with the smallest `pos > p`. When several share that position, the first in
  canonical order.
- **Previous, stopped or paused:** the marker with the largest `pos < p`.
- **Previous, playing:** the largest `pos < p − g`, with `g = 0.5 s`.
  - **Decided (autonomous, T-300):** without the grace period, a second press a moment after jumping
    back lands on the same marker again, because playback has moved a few ms past it. With it, repeated
    presses walk backwards. This is the media-player "previous track" convention.
- No target in that direction: a no-op. There is no wrap-around and no sound.

**Decided (autonomous, T-300):** region ends are not navigation stops, and the panel filter does not
restrict navigation. Navigation should be predictable, and a region's end is one click away (activating
the region selects it).

**Effect:**
- **Stopped or paused:** the cursor moves to the target's `pos`, and the time selection is **unchanged**.
- **Playing:** a **seek** to the target's `pos` (SPEC-003 §2.1: fade, reset at the new position,
  playback continues).
- In both cases the target becomes the single panel selection, scrolled into view. This is *not* an
  activation (§2.8), so a region's range is not selected.
- If the target is outside the viewport, the view scrolls horizontally to centre it. The zoom is
  unchanged.

### 2.8 The Markers panel

**Place.**
- The panel sits in the **left dock** (PROMPT §3.6 "markers + properties (left/bottom)"), above the
  Properties panel, with a draggable divider.
- It is toggled by View → Markers and is **visible by default**, 300 px wide.
- Its width and visibility are app settings (per user, not per document).
- **Decided (autonomous, T-300):** the left dock keeps the bottom dock free for meters and the analyzer
  (SPEC-007 §2.9), and a tall list fits a side column.

**Header:**
- **+** (Add Marker) and a Delete button;
- a filter text box;
- a type filter (**All / Points / Regions / Dropouts**);
- a panel menu (Delete All Markers, Delete Filtered Markers, Go to Next/Previous Marker);
- a count, "340 markers", or "12 of 340" while filtered.

**Columns:**

| Column | Content | Point markers |
|---|---|---|
| Name | `name`, ellipsized; full name in a tooltip | same |
| Start | `pos_samples` | same |
| End | exclusive end `pos + len` | "—" |
| Duration | `len_samples` | "—" |
| Type | Point / Region / Dropout, with a colour chip using `--wave-marker`, `--wave-marker-region` and the new `--wave-marker-dropout` token (SPEC-006 §2.12) | — |

- **Time format.** Start, End and Duration follow the time-ruler format (SPEC-006 §2.5):
  - timecode `[hh:]mm:ss.fff` (the hh group only for documents ≥ 1 h);
  - samples as integers;
  - seconds with **3 decimals**, fixed in the panel. The ruler's zoom-dependent precision would make the
    column jitter.

**Sorting:**
- Clicking a column header sorts by it; clicking again reverses. The **default is Start ascending**.
  Ties always fall back to canonical order.
- **Name** sorts with `Intl.Collator(locale, { numeric: true, sensitivity: "base" })`, so "Marker 2"
  sorts before "Marker 10" and "intro" ties with "Intro".
- **Duration** sorts points as 0. **Type** sorts Point < Region < Dropout.
- The sort column and direction are per-document view state, saved in the sidecar (SPEC-018).

**Filtering:**
- **Text.** Case-insensitive substring match on the name. Case folding uses `String.prototype.toLowerCase()`,
  locale-independent. The list updates ≤ 100 ms after the last keystroke.
- **Type:**
  - **Points** = user or unknown-kind point markers;
  - **Regions** = `len > 0`;
  - **Dropouts** = `kind = dropout`.
- The text filter is not persisted. The type filter is per-document view state (sidecar).

**Row selection and activation:**
- **Click** selects that one row and **activates** its marker:
  - **Point or dropout:** like clicking the waveform at `pos` (SPEC-006 §2.9). The time selection is
    cleared, and the cursor moves to `pos` (stopped) or playback seeks there (playing).
  - **Region:** the time selection becomes exactly `[pos, pos + len)`. The cursor moves to `pos`
    (stopped), or playback seeks to `pos` (playing).
- **↑ / ↓** move the single selection to the adjacent row and activate it. **Enter** activates the
  focused row.
- **Ctrl/⌘+click** toggles a row, and **Shift+click** / **Shift+↑↓** extend a range of rows in the
  current sort order. When **≥ 2** rows are selected, the time selection becomes their span
  `[min pos, max (pos + len))`. The cursor is unchanged, and there is no seek.
  - **Decided (autonomous, T-300):** this gives voice-over users "select between two markers" in one
    gesture. Audition needs Merge Markers for it (Adobe Community,
    [accepted answer](https://community.adobe.com/t5/audition/how-to-select-region-between-two-markers/m-p/10019374)).
    Sound Forge-style double-click-between-markers is not provided.
- **Esc** in the list clears the panel selection. The time selection is unchanged.
- **Clicking a flag** on the waveform is the same activation as clicking its row. The row is also
  scrolled into view.
- **Panel selection vs document state.** Panel selection is UI state, like the time selection
  (SPEC-004 §2.2): never undoable, never persisted. Markers that no longer exist after a change drop out
  of it.

**Live update and scale:**
- After any committed change (marker op, audio edit mapping, undo/redo, take commit), the panel shows
  the new list within **50 ms p95** of `document_changed`, at 10 000 markers.
- The list is **virtualized**: fixed 22 px rows, and only the visible rows plus 10 above and below exist
  in the DOM. Scrolling a 10 000-row list keeps SPEC-006's frame-time bound (AC-15).

### 2.9 Undo, playback and dirty state

| Operation | Undo entries | Label key | Stops playback |
|---|---|---|---|
| Add (M, +, context menu) | 1 | `history.marker_add` | no |
| Rename | 1 | `history.marker_rename` | no |
| Move (drag, Shift-drag, typed Start) | 1 | `history.marker_move` | no |
| Resize (region edge drag, typed End/Duration) | 1 | `history.marker_resize` | no |
| Delete selected / all / filtered (any count) | 1 | `history.marker_delete` | no |
| Undo or redo of any of the above | — | — | no |
| Markers added during a take (user and dropout) | part of the take's single "Record" entry (SPEC-002 AC-5) | — | n/a |
| Audio edits (SPEC-008/010) | the marker mapping is part of the audio op's entry | — | yes |

- **Marker-only entries.** They change `rev` but not `audio_rev` (SPEC-004 §2.1), so the waveform and
  spectrogram don't refetch. Playback output stays bit-identical to a run without the operation
  (SPEC-004 AC-6).
- **After undo or redo of a marker-only entry:**
  - the time selection and the playhead are **unchanged** (SPEC-008 §2.3's post-undo cursor rule applies
    only to audio entries);
  - the markers the entry touched that still exist become the panel selection, scrolled into view.
  - **Decided (autonomous, T-300):** the user sees what the undo changed without losing their place.
- **Dirty state.** A marker edit is an edit, so it sets `dirty` (SPEC-004 AC-8), and undoing back to the
  saved state clears it.
- **Stale positions.** Position-bearing commands (add with an explicit position, move, resize) carry the
  `audio_rev` the UI based the position on. If the audio changed in between, they are rejected with
  `error.document_changed` (SPEC-008 AC-18 pattern). Id-based commands (rename, delete) only require
  the ids to exist (`error.marker_not_found`).

### 2.10 Markers under audio edits
Audio edits map markers exactly as SPEC-008 §2.4/§4.2 define:
- markers never get deleted by an audio edit;
- a region shrunk to 0 becomes a point;
- ids, names and kinds never change.

Nothing here changes those rules.

### 2.11 During recording
- **Allowed:** Add (M, panel +). The marker lands at the take position (SPEC-002 §2.2, AC-15). Dropout
  markers are added by the recorder.
- **Pending markers.** Until Stop commits the take, the markers added during it are **pending**:
  - they are drawn live with a dashed line;
  - they are listed in the panel greyed, with Type "(recording)";
  - they can't be selected for editing.

  On Stop they become part of the take's single undo entry (SPEC-002 AC-5).
- **Disabled:** rename, move, resize, delete and navigation, for pending **and** existing markers, with
  "Not available while recording" (`error.not_while_recording`).
  - **Decided (autonomous, T-300):** undo is refused during recording (SPEC-004 §2.3). Allowing other
    marker edits would create entries the user can't undo until Stop, interleaved with a take that is
    still open.

### 2.12 Limits
- **`max_user_markers` = 10 000.** Add is refused at or above this count (§2.2).
- **Markers that don't count against it** are never refused below the hard cap:
  - markers read from files (SPEC-005 accepts ≤ 100 000 cue points);
  - markers read from the sidecar;
  - dropout markers.
- **`max_markers_total` = 100 000,** matching SPEC-005's `cue_max_points`.
  - A file or sidecar with more than 100 000 markers keeps the first 100 000 in canonical order and shows
    `notice.markers_truncated`.
  - A take that would push the total past it stops adding dropout markers. The dropout counter and the
    Stop notice still report every dropout.
- **Performance.** All panel and command ACs are specified at 10 000 markers. At 100 000 everything
  still works (virtualized list, O(n) marker ops), without the timing bounds.

### 2.13 Persistence and precedence

- **Journal.** Every marker op is journaled (ADR-004 §6 `marker_ops`, fdatasync before success), so
  crash recovery restores markers exactly (SPEC-004 AC-9), independently of any file.
- **Sidecar** (SPEC-018). It is **authoritative** for the full marker data: id, kind, name, position and
  length. It is written on every Save/Save As.
- **WAV `cue ` / `LIST adtl`** (SPEC-005 §2.9). Written on every Save/Save As to WAV, for interoperation
  with REAPER, Audacity and Audition. Kind is not representable, so a dropout marker is written as an
  ordinary cue named "Dropout 10 ms". FLAC stores no markers (SPEC-005 notice), and the sidecar keeps
  them.
- **On open, precedence** (T-306 implements it; refines SPEC-005 §2.9's "sidecar authoritative when the
  fingerprint matches"):

  | # | Situation | Markers used | Notice |
  |---|---|---|---|
  | 1 | Valid sidecar whose document identity matches (SPEC-018 §2.5), file is **not WAV** (FLAC) | sidecar | — |
  | 2 | Valid matching sidecar, WAV whose readable cue set **equals** the sidecar's cue projection (§4.5) | sidecar (ids, kinds) | — |
  | 3 | Valid matching sidecar, WAV whose readable cue set **differs** (another program changed the markers) | the **WAV's** markers. Each WAV marker equal in `(pos, len, name)` to a sidecar marker inherits that marker's kind. | `notice.open.markers_changed_externally` "Markers in ‹name› were changed by another program; using the file's markers" |
  | 4 | Valid matching sidecar, WAV whose `cue `/`adtl` is **malformed** (SPEC-005 §2.5) | sidecar | SPEC-005's `notice.open.markers_unreadable` |
  | 5 | No sidecar, or a sidecar that was ignored (mismatch, corrupt, too new: SPEC-018 §2.5) | WAV markers (SPEC-005 §2.9), all `kind = user`; none for FLAC or compressed files | SPEC-018's notice, if any |

  **Decided (autonomous, T-300), case 3.** If the user moved markers in another editor after saving,
  silently reverting to our sidecar's copy would undo their work. The cue set is the only marker data
  other programs can change, so a difference proves an external change.
- **Ids after open.** Sidecar ids are kept (cases 1, 2, 4). WAV markers get ids 1…n in canonical order
  (cases 3, 5). Either way, the next new id is max + 1.

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `max_user_markers` | Add-marker limit | markers | — | 10 000 | fixed | §2.12 |
| `max_markers_total` | Hard marker cap (files, sidecar, dropouts) | markers | — | 100 000 | fixed | = SPEC-005 `cue_max_points` |
| `marker_name_max_bytes` | Name length | bytes UTF-8 | 1 … 1024 | — | fixed | SPEC-005 §4.6 |
| `default_name_pattern` | Default name | — | — | "Marker NN" (≥ 2 digits) | — | `marker.default_name`, max + 1 numbering |
| `flag_hit_min_px` | Flag hit width | px | — | 10 (± 5 around the line), 12 high | fixed | §2.5 |
| `drag_threshold_px` | Click vs drag | px | — | 3 | fixed | §2.5 |
| `marker_magnet_px` | Drag magnet distance | px | — | 6 | fixed | = SPEC-006 `selection_handle_hit_px` |
| `drag_autoscroll_rate` | Auto-scroll beyond the canvas edge | viewports/s | — | 1 | fixed | §2.5 |
| `nav_prev_grace_s` | "Previous" grace while playing | s | — | 0.5 (0 when stopped) | fixed | §2.7 |
| `panel_width_px` | Markers panel default width | px | 200 … 600 | 300 | drag | app setting |
| `panel_row_px` | Row height | px | — | 22 | fixed | virtualization |
| `panel_overscan_rows` | Rows rendered beyond the viewport | rows | — | 10 each side | fixed | |
| `panel_update_budget_ms` | Change → panel updated (10 000 markers) | ms | — | ≤ 50 p95 | fixed | §2.8 |
| `filter_latency_ms` | Filter keystroke → list updated (10 000) | ms | — | ≤ 100 | fixed | |
| `marker_command_budget_ms` | Marker command → success, incl. journal `fdatasync` (SSD) | ms | — | ≤ 50 p95 | fixed | same as SPEC-008 |

## 4. Algorithm / implementation notes

### 4.1 Data and journal (`project`, T-303 on T-101/T-301)
- **Marker.** `Marker { id, pos_samples, len_samples, name: Arc<str>, kind: MarkerKind }` (§2.1 delta).
  The snapshot's `markers: Arc<[Marker]>` stays sorted canonically. A marker-only edit shares `pieces`
  (ADR-004 §3).
- **Marker ops.** T-101 already has `MarkerOp::{Add, Remove, …}`. T-303 makes sure the set covers
  `Add(Marker)`, `Remove(MarkerId)`, `Rename { id, name }` and `SetRange { id, pos, len }`.
  - A multi-delete is one `Edit` with several `Remove` ops.
  - Marker-only edits have empty `ops` and a `label_key` from §2.9.
  - Validation (§2.1 invariants, id existence, name normalization) happens before the journal append.
    On an invalid op, nothing is committed.
- **Cost.** Applying a marker edit rebuilds the marker array: O(n), ≈ 0.1 ms at 10 000. Allocating a new
  id uses the history's counter, which is persisted in checkpoints (T-101).

### 4.2 IPC (commands in `src-tauri`, logic in `project`/`engine`)
- **Commands:**
  - `marker_add { base_audio_rev, pos_samples, len_samples, source: "key" | "context" | "panel" }`. The
    UI resolves the position per §2.2. While recording, the engine routes the command to the pending
    take, and `base_audio_rev` is ignored.
  - `marker_rename { id, name }`
  - `marker_set_range { base_audio_rev, id, pos_samples, len_samples, label: "move" | "resize" }`
  - `marker_delete { ids }`
  - `marker_delete_all {}`
  - `markers_get() -> { rev, markers: MarkerDto[] }`, where
    `MarkerDto { id, pos_samples, len_samples, name, kind: "user" | "dropout" | "other" }`.
- **Results.** Mutating commands return `MarkerResult { rev, ids }` (the new or affected ids).
- **Errors** (`IpcError.key`):
  - `error.document_changed`
  - `error.marker_not_found`
  - `error.invalid_range`
  - `error.marker_limit`
  - `error.marker_name_empty`
  - `error.not_while_recording`
  - `error.document_busy` (a job is running, SPEC-008 §2.2)
- **Transport.** Markers are small structured data, so JSON is fine (CLAUDE.md's binary rule is for
  audio and peaks): 10 000 markers ≈ 0.8 MB. The UI refetches `markers_get` on every
  `document_changed` whose `rev` differs from its list's `rev`. An incremental `markers_changed` event is
  a permitted optimization if AC-15 needs it.
- **Navigation** needs no command. The UI computes the target from its list (binary search, §4.4) and
  issues `transport_seek` (playing) or the cursor move (stopped).

### 4.3 Position at the key press
- The UI takes `KeyboardEvent.timeStamp` (the event time, not the handler time) and converts it to the
  app clock through the clock-sync offset (ADR-003 §3).
- It then evaluates SPEC-003 §2.2's extrapolation (`displayed_position`) **at that time**, with the
  anchor current at that time, and clamps to `[0, L]` (wrapping inside the loop range when looping).
- Handler latency and IPC latency therefore don't bias the position. The bound is SPEC-004 AC-7's
  ±10 ms.
- Recording uses the same computation on the take position (SPEC-002 §2.2).

### 4.4 Navigation
- The canonical list is sorted by `pos`, so `partition_point(|m| m.pos <= p)` gives Next, and
  `partition_point(|m| m.pos < p − g) − 1` gives Previous. Both are O(log n).
- Among equal positions, canonical order picks the lowest id.

### 4.5 Cue projection (for §2.13)
- **Projection.** The cue projection of a marker list is the multiset of `(pos_samples, len_samples,
  name)` that SPEC-005 §2.9 would write, normalized as SPEC-005 reads it back (UTF-8 text cut at
  1024 bytes, control characters stripped).
- **Comparison.** Sort both projections by `(pos, len, name bytes)` and compare them element-wise.
- **No stored digest is needed.** The sidecar holds the full markers, and PowerVoice's cue writer is
  exact (SPEC-005 AC-7).

### 4.6 UI
- **Rendering.** Flags, end flags, dashed pending markers and drag previews are drawn by SPEC-006's
  renderer, using the same `px()` (SPEC-006 §4.1). The drag preview replaces the marker's committed
  position for that frame only.
- **Panel.** A virtual list in `ui/src/lib/markers/`:
  - fixed row height;
  - a sorted and filtered index array, recomputed off the `markers_get` result;
  - `Intl.Collator` created once per locale.
- **Keymap registry entries** (T-104), all no-ops while a text field or modal dialog has focus:
  - `marker.add` = M;
  - `marker.rename` = `/`;
  - `marker.delete_selected` = Ctrl+0 / ⌘0;
  - `marker.delete_all` = Ctrl+Alt+0 / ⌘⌥0;
  - `marker.next` = Ctrl+Alt+→ / ⌘⌥→;
  - `marker.prev` = Ctrl+Alt+← / ⌘⌥←.
- **Digit keys** in bindings match on `KeyboardEvent.code` (`Digit0`), so layouts where 0 needs Shift
  (AZERTY) still work. Letters and `/` match on `key`. Flagged for SPEC-019's keymap audit.
- **i18n keys:**
  - `markers.panel.*` (column titles, filter placeholder, type names, count);
  - `marker.default_name`;
  - `history.marker_*`;
  - `notice.marker_limit`, `notice.markers_deleted`, `notice.markers_truncated`,
    `notice.open.markers_changed_externally`;
  - `menu.view.markers`, `menu.view.next_marker`, `menu.view.prev_marker`.

## 5. Acceptance criteria

Unless stated otherwise, the document is 48 kHz mono, 10 s long (L = 480 000 samples), with no markers.
Times in the ACs are exact sample positions. "One undo entry" means the undo depth grows by exactly 1,
with the stated `label_key`.

- **AC-1 (model invariants).**
  - `marker_add` with `(pos 480 000, len 0)` succeeds: a point at L.
  - `(480 001, 0)` and `(470 000, 20 000)` return `error.invalid_range` and change nothing (`rev`
    unchanged).
  - A region set to `len = 0` through `marker_set_range` becomes a point: Type Point.
  - After 1 000 seeded random ops (add, move, resize, delete, cut, paste), the marker list is always
    sorted by `(pos, id)`, ids are unique, and every marker satisfies `pos + len ≤ L`.
- **AC-2 (add while stopped).**
  - With the cursor at 96 000 and no selection, M adds a point at exactly 96 000, named "Marker 01",
    `kind = user`, as one undo entry `history.marker_add`. `rev` increases, and `audio_rev` is unchanged.
  - With a selection [48 000, 96 000), M adds a region `pos 48 000`, `len 48 000`.
  - With an empty selection [200 000, 200 000), M adds a point at 200 000.
  - With the cursor at 480 000, M adds a point at 480 000.
  - Undo removes the marker, and redo restores it with the **same id**.
  - The new marker is the single panel selection, the time selection is unchanged, and no rename editor
    opens.
  - Holding M with 20 auto-repeat keydown events adds exactly 1 marker.
- **AC-3 (add during playback).** Given fake-backend playback of seeded white noise with a selection
  [10 000, 20 000) present, when synthetic M keydowns with `timeStamp`s T₁…T₂₀ are dispatched:
  - each resulting marker is a **point** within **±480 samples** (±10 ms) of the true heard position at
    Tᵢ, including across a loop wrap;
  - the time selection stays [10 000, 20 000);
  - the output is bit-identical to a control run without the keydowns, and the transport never stops.
- **AC-4 (recording).** Given a recording in progress:
  - M adds a pending marker, and after Stop it lies within ±10 ms of the take position at the press
    (SPEC-002 AC-15);
  - `marker_rename`, `marker_set_range`, `marker_delete` and `marker_delete_all` return
    `error.not_while_recording`, and Ctrl+Alt+→ does nothing;
  - undoing the take removes its markers, pending and dropout included.
- **AC-5 (default names).**
  - Three adds are named "Marker 01", "Marker 02", "Marker 03".
  - After deleting "Marker 03", the next is "Marker 03". After deleting "Marker 02" (with 01 and 03
    left), the next is "Marker 04".
  - After deleting all markers, the next is "Marker 01".
  - With existing names "Marker 7" (from a WAV) and "Intro", the next is "Marker 08".
  - With "Marker 99", the next is "Marker 100".
  - A marker renamed from "Marker 05" to "Intro" no longer counts.
  - Regions follow the same sequence. "Dropout 12 ms" names never count.
- **AC-6 (rename).** Given one selected marker "Marker 01":
  - `/` opens the editor on it. Typing "  Intro  " + Enter renames it to "Intro" as one entry
    `history.marker_rename`.
  - Esc, an empty or whitespace-only name, or an unchanged name adds no entry and keeps the old name.
  - A 2 000-byte name (multi-byte characters) is stored with ≤ 1024 bytes and cut at a character
    boundary. A name containing U+0007 is stored without it.
  - With 0 or 2 selected markers, `/` does nothing.
  - Double-clicking a flag opens the popover editor and does **not** select the whole document
    (SPEC-006 double-click).
  - Renaming a dropout marker keeps `kind = dropout`.
- **AC-7 (drag, magnet, clamps).** Given `startSample = 0`, `samplesPerPixel = 200`, a 1000 px canvas,
  a point marker at 50 000 (px 250), the cursor at 61 450 and a region [100 000, 110 000):
  - dragging the point's flag to px 306 lands it at **61 450** (the magnet: 1.25 px from the cursor);
  - with Alt held, it lands at **61 200**;
  - dragged to px 330, it lands at **66 000**;
  - dragging the region's end flag to px 480 clamps to `len = 1`, i.e. [100 000, 100 001);
  - dragging the region's end flag to px 560 gives [100 000, 112 000) as `history.marker_resize`;
  - Shift-dragging the region's start flag from px 500 to px 600 gives [120 000, 130 000) as
    `history.marker_move`;
  - pressing at px 250 and releasing at px 252 is a click: activation, no move entry;
  - Esc mid-drag leaves `rev` unchanged;
  - at equal magnet distance to two targets, the earlier sample wins;
  - no drag reads audio samples, because there is no zero-crossing snap.
- **AC-8 (typed edits in the panel).** Given a region [96 000, +4 800) with the time format timecode:
  - Start "0:03" → [144 000, +4 800);
  - then Duration "0.5" → len 24 000;
  - then End "150000 smp" → len 6 000;
  - then Duration "0" → a point at 144 000;
  - Start "0:11" (pos 528 000 > L) → error outline and no entry;
  - on a point marker, Start "1.5" → 72 000.
  Each successful edit is one entry, `move` or `resize` as in §2.5.
- **AC-9 (delete and focus rule).** In Vitest, with the keymap registry and a mocked document holding
  markers M1–M3 (M2 selected in the panel) and a time selection [10 000, 20 000):
  - Delete with the **panel** focused deletes M2 only (`marker_delete {ids:[M2]}`), with no audio
    command;
  - Delete with the **editor** focused issues SPEC-008's `edit_delete` for [10 000, 20 000) and no marker
    command;
  - Delete with the editor focused and no time selection issues nothing;
  - Delete in the filter box or a cell editor edits text only;
  - Ctrl+0 from either panel or editor issues `marker_delete {ids:[M2]}`;
  - deleting 3 selected markers is one entry;
  - after a delete, the panel selection is empty and the playhead is unchanged.
- **AC-10 (delete all / filtered).** Given 37 markers, 5 of them dropouts:
  - the Dropouts filter shows "5 of 37", and Delete Filtered Markers removes exactly those 5 as one
    entry;
  - Ctrl+Alt+0 then removes the remaining 32 as one entry, with the notice "Deleted 32 markers" and its
    Undo action, and no confirmation dialog;
  - two undos restore all 37 with identical ids, names, positions, lengths and kinds.
- **AC-11 (navigation).** Given markers A = point 48 000, B = point 144 000 (id 2), C = point 144 000
  (id 3), R = region [288 000, +96 000), D = point 480 000, and a time selection [10 000, 20 000).
  **Stopped, cursor 0:**
  - Next ×4 moves the cursor to 48 000 → 144 000 (B selected, not C) → 288 000 (R selected; 384 000 is
    not a stop) → 480 000;
  - a fifth Next is a no-op;
  - Previous from 480 000 → 288 000, and from 144 000 → 48 000; from 48 000 it is a no-op;
  - the time selection stays [10 000, 20 000) throughout, and activating is not triggered (R's range is
    not selected).

  **Playing:**
  - at heard position 172 800 (3.6 s), Previous seeks to 144 000;
  - at 153 600 (3.2 s), Previous seeks to 48 000 (grace 0.5 s);
  - Next seeks to 288 000, and playback continues (a SPEC-003 seek, not a stop);
  - a target outside the viewport is centred horizontally, with the zoom unchanged.
- **AC-12 (panel columns and sorting).** At 48 kHz, a region [96 000, +4 800) shows:
  - in timecode: Start "00:02.000", End "00:02.100", Duration "00:00.100", Type "Region";
  - in samples: "96000", "100800", "4800";
  - in seconds: "2.000", "2.100", "0.100".

  A point shows "—" for End and Duration, and a dropout shows Type "Dropout".
  - Names ["Marker 10", "Marker 2", "intro", "Intro"] sort by Name ascending as "intro"/"Intro" (by
    canonical order between them), "Marker 2", "Marker 10".
  - The default sort is Start ascending. A chosen sort survives save and reopen (SPEC-018 view state).
- **AC-13 (activation and multi-selection).**
  - Clicking region R's row sets the time selection to exactly [288 000, 384 000) and the cursor to
    288 000.
  - Clicking A's row clears the time selection and sets the cursor to 48 000.
  - Ctrl+clicking A then D selects both and sets the time selection to [48 000, 480 000). The cursor is
    unchanged, and there is no seek.
  - During fake-backend playback, clicking B's row seeks to 144 000 (a SPEC-003 seek).
  - Clicking R's flag on the waveform is identical to clicking its row.
  - ↑/↓ activate the adjacent row.
- **AC-14 (filter).** Given 10 000 markers with seeded names:
  - the text filter "take" shows exactly the markers whose lower-cased name contains "take" (same count
    and ids as a reference filter in the test);
  - the list updates ≤ 100 ms after the last keystroke;
  - the Points/Regions/Dropouts filters partition correctly;
  - the text filter is empty after reopen, and the type filter is restored.
- **AC-15 (scale and performance, reference hardware, release build).** Given 10 000 markers:
  - the panel DOM holds at most (visible rows + 20) row elements;
  - a scripted 10 s scroll sweep of the list has p50 frame time ≤ 16.7 ms, p99 ≤ 50 ms and at most 1
    frame > 50 ms (SPEC-006 AC-18 methodology);
  - from a committed marker op to the panel showing it takes ≤ 50 ms p95 over 100 seeded ops;
  - `marker_add`/`marker_rename`/`marker_set_range`/`marker_delete` each complete ≤ 50 ms p95,
    including the journal `fdatasync` (SSD, transport stopped);
  - at 100 000 markers (a WAV with 100 000 cues), open, panel scroll, navigation and delete-all all
    function, with no timing bound.
- **AC-16 (undo semantics and playback).** Given fake-backend playback and a seeded sequence of 200
  marker ops (add, rename, move, resize, delete, delete-all) with undos and redos mixed in:
  - the output is bit-identical to a control run without them (SPEC-004 AC-6);
  - each op adds exactly one entry with its §2.9 label, and the Edit menu reads "Undo Move Marker" after
    a move;
  - no undo or redo stops playback, changes the time selection or moves the playhead;
  - after an undo, the touched markers are the panel selection;
  - undoing everything restores the initial list exactly (ids, names, kinds, positions, lengths), and
    redoing everything restores the final list;
  - `dirty` follows SPEC-004 AC-8.
- **AC-17 (limits).**
  - With 10 000 markers, M is disabled: `marker_add` returns `error.marker_limit`, and the notice
    appears.
  - A WAV with 20 000 cue points opens with all 20 000.
  - A sidecar with 120 000 markers opens with the first 100 000 in canonical order and
    `notice.markers_truncated`.
  - A fake take producing 10 dropouts when the document already holds 99 995 markers adds 5 dropout
    markers, and the Stop notice reports 10 dropouts.
- **AC-18 [with T-306] (persistence precedence).** Given a WAV saved by PowerVoice with markers
  {"Intro" point 0, "Dropout 10 ms" dropout at 144 000, region "Take 2" [200 000, +4 800)} and its
  matching sidecar:
  - (a) reopening yields the three markers with their sidecar ids and the dropout kind (case 2);
  - (b) after a test rewrites the WAV's `cue `/`adtl` with "Take 2" moved to 210 000 (audio untouched),
    reopening yields the WAV's markers: "Take 2" at 210 000, "Dropout 10 ms" still `kind = dropout`
    (exact `(pos, len, name)` match), and `notice.open.markers_changed_externally` (case 3);
  - (c) with the `LIST adtl` truncated (malformed), the sidecar markers are used with
    `notice.open.markers_unreadable` (case 4);
  - (d) without the sidecar, the three markers are `kind = user` (case 5);
  - (e) a FLAC save and reopen yields the sidecar markers including kinds (case 1).
- **AC-19 (dropout markers).** After SPEC-002 AC-7's take, the dropout marker:
  - has `kind = dropout`, Type "Dropout", the `--wave-marker-dropout` colour and the Dropouts filter;
  - can be renamed, moved and deleted like any marker, and keeps its kind;
  - is saved to WAV as an ordinary cue named "Dropout 10 ms";
  - "Go to first" (SPEC-002 AC-7) selects it in the panel.
- **AC-20 (shortcut map and focus).** In Vitest with the keymap registry:
  - M, `/`, Ctrl+0, Ctrl+Alt+0, Ctrl+Alt+→ and Ctrl+Alt+← dispatch `marker.add`, `marker.rename`,
    `marker.delete_selected`, `marker.delete_all`, `marker.next` and `marker.prev` on Windows/Linux,
    with ⌘/⌥ equivalents on macOS;
  - Ctrl+0 also matches `code = Digit0` on an AZERTY layout event;
  - with focus in a text input or a modal dialog open, none of them dispatches;
  - Shift+M, Alt+M, Ctrl+←/→, Alt+←/→ and S dispatch nothing marker-related.

## 6. Test plan

| AC | Unit (`project`) | Integration (engine, fake backend) | Vitest (UI, mockIPC) | Manual smoke (owner, Linux) |
|---|---|---|---|---|
| AC-1 | invariant validation; seeded property test of ops + audio edits | — | — | — |
| AC-2 | add semantics per state; id reuse on redo | — | M routing by transport state/selection; no auto-rename; repeat ignored | press M at the cursor and over a selection |
| AC-3 | — | fake playback: marker positions vs true heard positions; output bit-identity | extrapolation at `event.timeStamp` (SPEC-003 AC-6 harness) | press M on words while listening |
| AC-4 | pending markers join the take edit | fake recording + refused commands | disabled states while recording | press M while recording |
| AC-5 | numbering function table | — | — | — |
| AC-6 | name normalization | — | `/`, F2, double-click cell and flag; Esc/Enter/blur | rename from the panel and from a flag |
| AC-7 | — | — | drag state machine with the geometry above; magnet, Alt, Shift, clamps, Esc | drag markers near the cursor and other markers |
| AC-8 | range validation | — | time-cell parsing and commit matrix | type new starts and durations |
| AC-9 | — | — | focus × key matrix | Delete in the panel vs the waveform |
| AC-10 | batch delete = one entry | — | filtered delete, notice with Undo | clear dropout markers after a take |
| AC-11 | Next/Previous target function (binary search, grace) | seek during fake playback | cursor/selection effects, view centring | walk a file with Ctrl+Alt+arrows (check the WM doesn't grab them) |
| AC-12 | — | — | cell formatting in 3 formats; collation sort | — |
| AC-13 | — | seek on activation during playback | activation and multi-selection rules | select between two markers |
| AC-14 | — | — | filter correctness and latency at 10 000 | — |
| AC-15 | marker op cost at 10 000 | command latency bench (`just bench`) | virtual list DOM count; frame-time sweep (ADR-009 watchdog pattern) | scroll a 10 000-marker list |
| AC-16 | seeded op/undo sequence equality | output bit-identity during marker ops | panel selection after undo | Ctrl+Z marker ops while playing |
| AC-17 | caps and truncation | fake take near the cap | disabled Add at the limit | — |
| AC-18 | cue projection compare; kind inheritance | open flows with sidecar/WAV variants (T-306) | notice keys | edit markers in REAPER/Audacity after a save, reopen |
| AC-19 | kind survives rename/move; WAV projection | SPEC-002 AC-7 take | dropout styling and filter | force dropouts (`stress`), inspect |
| AC-20 | — | — | keymap + focus matrix, AZERTY event | shortcuts in the running app |

Fixtures: seeded testkit noise; hand-built WAVs with cue/adtl variants (SPEC-005 fixtures); 10 000- and
100 000-marker lists and sidecars generated in test code. Nothing is committed.

## 7. Out of scope
- **Other marker types:** Audition's Subclip, CD Track and Cart Timer markers, marker descriptions and
  metadata, and per-marker colours.
- **Other operations:** merge markers, explicit convert point↔region commands (typing Duration 0 is the
  only conversion), and splitting or duplicating markers.
- **Snapping and batch work:** snap-to-markers for selection edges and Audition's global Snapping toggle
  (S); marker CSV import/export, export or batch processing by markers, and playlists.
- **Markers in other files:** markers in FLAC, MP3 or ID3 chapters, and `smpl` loops (SPEC-005 §2.10).
- **Clipboard:** copying or pasting markers with audio (SPEC-008 §7), and marker clipboards.
- **Shortcut conflicts:** resolving shortcut conflicts with OS window managers (SPEC-019).
