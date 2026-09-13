# SPEC-022 — Record at cursor (insert/overwrite), punch-in, latency calibration

- **Status:** approved (autonomous, T-300)
- **Milestone:** M3 (T-304). Depends on T-301 (undo/redo, journal, recovery), the M1 recording and
  monitoring engine (T-106, T-107) and the M2 selection model (SPEC-006, T-206).
- **Related:** SPEC-000 (glossary: document time, heard position, app clock, take, dropout, fake
  backend), SPEC-001 (device loss), SPEC-002 (new-file recording, dropouts, monitoring, disk rules;
  this spec extends it), SPEC-003 (transport, start latency, heard position, Stop semantics),
  SPEC-004 (undo, refused while recording, recovery), SPEC-006 (selection, edit cursor), SPEC-008
  (Replace primitive, marker mapping, no micro-fades, post-undo cursor rule), SPEC-012 (rack latency),
  SPEC-019 (shortcut map, M7) · ADR-002 §1, §2, §4, §5, §7, §8 · ADR-003 (commands, events, `VXRP`)
  · ADR-004 §3, §6, §7, §9 · PROMPT §2 (LOCKED), §3.1, §3.8 · MEMORY D-014, D-017, D-018, D-020, A-002

## 1. Purpose

Voice-over work is rarely one clean pass. A narrator flubs a word in the middle of a chapter, a
client asks for one line read differently, or a session continues the next day at the end of
yesterday's file. The user needs to:
- **add** new material at any point without destroying what is there (Insert);
- **replace** from a point onward, as audiobook "punch-and-roll" does (Overwrite);
- **re-record exactly one selected region**, hearing a few seconds of lead-in first so the new read
  matches the old one in cadence, tone and energy (Punch-in);
- trust that a word spoken in time with the playback lands in time on the timeline. That needs
  **latency compensation**, and a **calibration** the user can run once per device setup.

SPEC-002 covers recording into a new or empty document, and its §1 defers these three features to
this spec. Everything in SPEC-002 that is not changed here (arming, input meter, clip lamp, crash
safety, dropout fill-and-mark, disk rules, monitoring modes) applies unchanged.

## 2. Behavior / UX

### 2.1 Terms and notation

- The document is `A`, with length `L` samples at the document rate `r`. The **cursor** `c` is the
  playhead position while stopped (SPEC-006 §2.9). The selection `[S, E)` has an exclusive end, as in
  SPEC-008 §2.1. "No selection" means `null` or empty (`S == E`), as in SPEC-008 §2.2.
- A **record operation** runs from the Record press to its end. It has up to three **phases**:
  1. **pre-roll**: playback before the record point;
  2. **recording**: the **record window**, the part that enters the document;
  3. **post-roll**: playback after a punch.
- The **record point** (`at`) is the document position where the new audio starts: `c` (or `S`) for
  cursor recordings, `S` for a punch.
- The **take** is the capture file of the whole operation (ADR-004 §7). `take[k]` is its k-th sample
  at the document rate. The document receives only the record window `take[k_start, k_end)`.
- **Aligned start.** The record window opens at the capture sample that corresponds to the moment
  `at` was **heard**, after latency compensation (§2.13). Punch-ins and pre-rolled cursor recordings
  use an aligned start.
- **Free start.** The take begins with the first sample captured at or after the moment the engine
  received Record (SPEC-002 §2.2). Cursor recordings without pre-roll use a free start, because no
  playback precedes them, so there is nothing to align to.
- `‖` means concatenation. `T[q]` means the take sample aligned to document position `q`, that is
  `take[k_start + (q − at)]`.

### 2.2 What Record does (resolution rule)

Record (button, Shift+R, §2.15) resolves to exactly one operation. The first matching row wins:

| # | Condition when Record is pressed | Operation |
|---|---|---|
| 1 | document empty (`L = 0`) | **new recording** into it: SPEC-002 §2.2 unchanged (mode, selection and pre-roll are ignored; there is nothing to play or replace) |
| 2 | non-empty selection **and** `punch_on_selection` on | **Punch-in** over `[S, E)` (§2.6) |
| 3 | non-empty selection **and** `punch_on_selection` off | record at `S` in the current `record_mode`; the selection is cleared when recording starts |
| 4 | no selection | record at the cursor `c` in the current `record_mode` (§2.4, §2.5) |

- **Decided (autonomous, T-300): a non-empty selection means punch-in by default.** Recording into a
  selection is the natural reading of "re-record this", and SPEC-008 already makes pasted or inserted
  audio the selection (A-002). "Insert 2 s of silence, then Shift+R" therefore records a pickup into
  the gap. Users who keep stray selections can switch `punch_on_selection` off.
- **Rule 3 follows SPEC-008 §2.1** (Insert silence with a selection inserts at `S`), so every command
  that uses "a position" treats a selection the same way.
- **Record while playing.** The transport first stops. This is an engine-initiated stop, so it
  behaves like Pause and the cursor becomes the heard position (SPEC-003 §2.1). Then the operation
  starts exactly as from the stopped state.
  - **Decided (autonomous, T-300):** on-the-fly punch, where playback continues seamlessly into
    recording, is deferred to v1.x. One start path keeps results deterministic, and pre-roll gives
    the talent the same lead-in.
- **Refused.**
  - While a record operation or a document job runs: `error.not_while_recording` or
    `error.document_busy` (SPEC-008 §4.3). The Record button is locked on while recording, as in
    SPEC-002.
  - When the UI's `base_rev` is stale: `error.document_changed`.
  - A **punch** without an output device: `error.punch_needs_output`, notice "Punch-in needs an
    output device for pre-roll and post-roll." Cursor recordings still work with only an input
    device (D-020), but with a free start: the pre-roll is skipped and the record panel shows
    "Pre-roll needs an output device".

### 2.3 Record panel controls

The SPEC-002 record panel gains a **Punch & pre-roll** section. The same values appear in Settings →
Recording. Right-clicking the Record button also opens a menu with the mode items, as in Audition,
where right-click on Record toggles Overwrite, Insert and Punch and Roll mode.

| Control | Values | Default |
|---|---|---|
| **Mode** (segmented) | Insert / Overwrite | **Insert** |
| **Punch-in on selection** | on / off | **on** |
| **Pre-roll** | 0.0 – 20.0 s | **5.0 s** |
| **Post-roll** | 0.0 – 20.0 s | **1.0 s** |
| **Pre-roll when recording at the cursor** ("punch and roll") | on / off | **off** |
| **Hear original while re-recording** | on / off | **off** |
| **Punch crossfade** | 0 – 50 ms | **10 ms** |
| **Recording offset** readout | "Offset +3.00 ms (calibrated)" / "Not calibrated" link | — |

- All values are **app preferences** in the settings file (T-104), not document state. The controls
  are locked while a record operation runs.
- **Decided (autonomous, T-300): the default mode is Insert.** It never destroys audio. Audition
  offers Overwrite and Insert in its Waveform Editor (help-page summary and secondary sources); which
  one Audition uses by default is unverified.
- **Decided (autonomous, T-300): pre-roll defaults to 5.0 s, range 0–20 s.** This matches Audition's
  Punch and Roll pre-roll default of 5 s (Preferences → Recording → Punch and Roll Recording; Adobe
  help search summary, travisbaldree.com, Mike Murphy). The narrator hears a whole preceding sentence,
  which is what matching a read needs.
- **Decided (autonomous, T-300): post-roll defaults to 1.0 s, range 0–20 s.** One second lets the
  engineer hear the join without waiting. No Audition equivalent was found.
- **Decided (autonomous, T-300): pre-roll at the cursor is off by default.** In Audition, Punch and
  Roll is a separate mode, and Shift+R at the cursor starts at once. With it on, Overwrite plus
  pre-roll is audiobook punch-and-roll.
- **Decided (autonomous, T-300): "Hear original" is off by default.**
  - Hearing the old read under the new one causes flamming and doubling in the talent's headphones,
    and bleed into the mic when monitoring through speakers.
  - The option exists for guide-track re-reads, where the talent matches the timing of the original
    line, and for the loopback alignment check (§2.14, AC-20).
  - It applies to Punch and Overwrite only. In Insert, the audio after `c` is not being replaced, so
    it is never played during the take.

### 2.4 Insert at the cursor

- **Result.** `A' = A[0,c) ‖ W ‖ A[c,L)` with `W = take[k_start, k_start + n)` and `L' = L + n`.
  - Audio after `c` shifts right by `n`.
  - Nothing is faded. Consistent with SPEC-008 §2.8, the take is spliced in exactly. It starts and
    ends in the talent's own pre- and post-phrase silence, and no old material overlaps it.
- **Start.** Free start, or aligned start at `c` when "Pre-roll when recording at the cursor" is on.
  Pre-roll then plays `[c − pre, c)` (§2.6 padding rule).
- **End.** Only Stop, input loss, disk floor or ring overflow end it (SPEC-002 §2.4–§2.6). Length is
  limited only by disk.
- **Special cases.**
  - `c = L` appends.
  - `c = 0` prepends.
  - Insert at `L` and Overwrite at `L` give identical documents.

### 2.5 Overwrite from the cursor

- **Result.** `A'[q] = A[q]` for `q < c`, `T[q]` for `c ≤ q < c + n`, and `A[q]` for `q ≥ c + n`,
  with `L' = max(L, c + n)`.
  - It replaces as the take grows, and extends the document when the take runs past its end.
  - Its boundaries get the punch crossfade (§2.8): a fade-in at `c` when `c < L`, and a fade-out at
    `c + n` when `c + n < L`. Both lie **inside** the new range.
- **Start and end.** Same as Insert (§2.4).
- **Decided (autonomous, T-300): Overwrite is an open-ended punch.** It gets the same crossfades,
  "hear original" option and marker rule as a punch. Two recordings of the same room meet mid-stream
  at both of its boundaries.

### 2.6 Punch-in over a selection

The talent hears the lead-in, speaks during the selection, and then hears the join.

1. **Pre-roll.**
   - Playback runs from `P₀ = S − pre` to `S`, through the rack, as normal playback.
   - If `P₀ < 0`, the missing part is digital silence, so **the pre-roll always lasts exactly `pre`
     seconds** and the talent always gets the full lead-in time.
   - If the input stream first has to open (SPEC-002 §2.2, ≤ 600 ms), the pre-roll starts once the
     input delivers its first block.
   - **Decided (autonomous, T-300):** pad with silence rather than shortening the pre-roll. A punch
     near the start of a file must not come as a surprise.
2. **Recording.**
   - The record window opens with an **aligned start** at `S` (§2.13).
   - It captures exactly `E − S` samples and **ends automatically at `E`**. The punch replaces exactly
     the selection and never changes the document length.
   - **Decided (autonomous, T-300)**, following Audition, whose record-within-a-time-selection stops
     at the selection end (secondary source; the official page returns 403).
3. **Post-roll.**
   - Playback of `A[E, min(E + post, L))` follows.
   - If `E = L`, or post-roll is 0, the operation ends at `E`.
   - Then the transport stops (≈ 5 ms fade) and the edit commits (§2.11).

- **The take keeps the whole pass**, pre-roll and post-roll included, in `takes/take-NNNN.wav`, as a
  session backup. If the talent started early, it is still there. Only the window enters the
  document.
- **Re-punch.** After a punch the selection is still `[S, E)` (§2.10), so Shift+R again re-punches the
  same region. Audition's separate "Punch Again" command is not needed and is listed for SPEC-019
  (§7).
- **Short selections** are allowed down to 1 sample. The crossfades shrink (§2.8).

### 2.7 What the talent hears

| Phase | Document playback (through the rack) | Monitoring |
|---|---|---|
| Pre-roll | `A[P₀, at)` (silence where `q < 0`), with a 5 ms fade-out ending at `at` | per `monitor_mode` (SPEC-002 §2.7): audible because the input is armed |
| Recording, Insert | silence | per `monitor_mode` |
| Recording, Overwrite / Punch | silence, or `A[q]` while `q < L` if **Hear original** is on | per `monitor_mode` |
| Post-roll (punch) | `A[E, E + post)`, with a 5 ms fade-in at `E` | per `monitor_mode` |

- **Muting.** The reader emits silence packets (ADR-002 §4 left this choice to T-304). Their
  positions keep advancing, also past `L`, so the playhead tracks the growing take. The rack is not
  reset at these boundaries: effect tails simply decay.
- **Listening fades only.** The 5 ms fades are applied to what is heard, never to the document.
- **Monitoring is unchanged across phases.**
  - **Decided (autonomous, T-300):** there is no automatic input-monitoring switch (as with Pro Tools
    "auto input"). SPEC-002's rule "audible while armed or recording" stays the single rule.
  - With the default Off, the talent hears only playback, which suits interfaces with direct
    monitoring.
- **Monitoring is never recorded** (SPEC-002 §2.7). The take is always the dry input.

### 2.8 Punch crossfades

- **Where.** At the punch-in boundary `S` and the punch-out boundary (`E`, or the stop point `p`).
  Overwrite uses the same fades at `c` and `c + n` (§2.5). Insert has none.
- **Shape.** Equal-power (sine/cosine, §4.2). Old and new are two independent performances of
  similar room tone, which are uncorrelated. For uncorrelated signals an equal-power fade keeps the
  level constant; a linear fade would dip by 3 dB in the middle.
- **Length.** `punch_xfade_ms`, default **10 ms** (480 samples at 48 kHz), range 0–50 ms.
  - Each fade is shortened to at most half the new range, so the two never overlap.
  - With 0 ms the edit is a pure splice.
- **Decided (autonomous, T-300): fades lie entirely inside the replaced range.** Audio outside
  `[S, E)` is **bit-identical** after a punch; only `[S, E)` changes. That is the guarantee "what is
  selected is what changes" (SPEC-008 §1), and it needs no captured material beyond the window.
  - Rejected: a fade centred on the boundary, which would modify ≤ 5 ms of unselected audio for no
    audible gain.
  - Rejected: no fade (the SPEC-008 §2.8 policy). A punch joins two different recordings mid-phrase,
    so a hard cut there clicks far more often than a cut within one recording. The fade is part of
    the punch, not a user fade feature (PROMPT §3.8 fades stay out of scope).

### 2.9 Markers

- **Insert** maps existing markers with SPEC-008 §4.2's insertion rule: markers at or after `c`
  shift right by `n`, and region ends follow `map_end`.
- **Overwrite and Punch preserve the timeline**, so existing markers **do not move** (identity), like
  Silence in SPEC-008 §2.4.
  - This applies also inside the replaced range: a marker at 6 s stays at 6 s and now points at the
    new read of that moment.
  - **Decided (autonomous, T-300):** markers are time annotations such as "retake line 12". Moving
    them to `S` (SPEC-008's replace rule) would pile them up for no reason. No marker is ever deleted
    (A-002).
- **Markers added during the operation** (`M`, SPEC-002 §2.2):
  - **pre-roll and post-roll:** at the heard position under the key press (SPEC-004 §2.2), on the
    existing audio;
  - **recording:** at `at + k`, where `k` is the take offset under the key press, extrapolated as in
    SPEC-003 §2.2. In Insert mode this is inside the new audio.
  - All of them, and every dropout marker (§2.12), belong to the operation's single edit.
- **Cancelled operation** (§2.10): markers added during it are committed as one ordinary "Add Marker"
  entry each, as if they had been added during playback, so they are never lost.

### 2.10 Stopping and ending

The Stop control and Space (SPEC-002 §2.2) end the operation in any phase.

| Ended by | During pre-roll | During recording, at heard position `p` | During post-roll |
|---|---|---|---|
| **Stop / Space** | **cancelled**: no edit, no undo entry, document unchanged (`rev` and hash equal) | **partial**: Punch replaces `[S, p)` only, with the punch-out fade ending at `p`, and `A[p, L)` is untouched. Insert/Overwrite: the take ends at `p` (`n = p − at`) | punch complete: identical to letting the post-roll run out |
| **Automatic end at `E`** (punch) | — | the post-roll starts | — |
| **Post-roll runs out** | — | — | commit |
| **Input device lost** | cancelled, notice `notice.punch_cancelled` | partial at the last good sample (SPEC-002 §2.6), notice "Input device disconnected — punch-in stopped at 0:12.345; the recorded part was kept." | complete |
| **Output device lost** | cancelled: the talent can no longer hear the lead-in | **continues**: the window was fixed when it opened (§4.3), so the take is unaffected (D-017). A punch ends at `E` with no post-roll | complete, and the post-roll ends |
| **Capture-ring overflow / disk floor** | cancelled | partial at the last good sample, SPEC-002 notices | complete |

- **What `p` is.** `p` is the document position being heard when the engine receives Stop, clamped
  to `[at, E]` for a punch. If `p ≤ at`, the operation counts as cancelled.
- **Decided (autonomous, T-300): an early stop keeps a partial punch.** Old audio after the stop point
  is preserved, which is the least destructive reading. One Ctrl+Z restores everything.
- **Decided (autonomous, T-300): the output lost during pre-roll cancels.** D-017 keeps a take alive
  when the output goes, but in pre-roll no take exists yet, and a punch without the lead-in is not
  what the user asked for.
- **Cancellation.** The take file is deleted and journaled as cancelled (§4.6). The panel shows
  "Punch-in cancelled — nothing changed" for 4 s when the cancel was not user-initiated.

**Selection and playhead afterwards** (committed, partial or cancelled):

| Operation | Selection after | Playhead after |
|---|---|---|
| Punch-in | `[S, E)` (unchanged) | `S` |
| Insert / Overwrite | none | `at + n` (the end of the new audio) |

- **Decided (autonomous, T-300).**
  - After a punch, Shift+Space (SPEC-003) plays the region again and Shift+R re-punches it.
  - After an Insert/Overwrite take, the next Shift+R continues right where the talent stopped. Keeping
    `[at, at + n)` selected (the A-002 paste rule) would turn that next Shift+R into a punch over the
    take just recorded.
  - This refines D-018 (Stop returns to the play start) and SPEC-003's "engine-initiated stop =
    Pause" for record operations. SPEC-003 does not cover these.
- **Undo** of any of these edits follows SPEC-008 §2.3: the selection is cleared and the playhead
  goes to `at`.

### 2.11 Undo, history and refusals

- **One edit.** Each committed operation is **one undoable edit**. It holds the audio change, the
  markers added during it and its dropout markers. It stops nothing, because the transport has
  already stopped by the commit (ADR-004 §3 sequence).
  - Labels: `history.record` "Record" for Insert and Overwrite, and `history.punch` "Punch-in" for a
    punch.
  - Undo restores exactly the previous audio (hash) and marker list. Redo restores exactly the result.
- **Commit time.** The edit is visible within 500 ms of the operation's end: the post-roll end or
  Stop (SPEC-002 §2.2).
- **Refused.** During the whole operation, pre-roll and post-roll included, the RECORDING state holds
  (SPEC-004 AC-15):
  - audio edits, undo and redo return `error.not_while_recording`, and so does calibration;
  - Add marker is allowed;
  - selection gestures on the waveform are ignored, so the punch range cannot be confused with a new
    selection while it runs;
  - loop playback is inactive.
- **Panel display.** A phase label with a countdown: "Pre-roll 3.2 s", then "Punch-in 0:01.3 /
  0:03.0" (Insert/Overwrite: "Recording 0:01.3"), then "Post-roll".
- **Waveform display.** The live take (`VXRP`, §4.5) is drawn at `at`. In Insert mode the existing
  waveform after `at` is drawn shifted right by the current take length. In Overwrite and Punch the
  take is drawn over the old audio in the record colour.

### 2.12 Dropouts, clips, disk and crash recovery

- **Dropouts.** SPEC-002 §2.4 fill-and-mark applies during the record window.
  - The silence fill keeps every later sample at its correct document position, so alignment
    survives a dropout.
  - The marker "Dropout N ms" lands at the document position of the gap start.
  - Dropouts in pre-roll or post-roll are filled in the take file, which keeps the index mapping
    exact, but get no marker, because that audio never enters the document.
  - A dropout of unknown length (unreliable timestamps) gets the "length unknown" marker and no fill,
    as in SPEC-002. The rest of the window is then early by the lost length, and the marker says where.
- **Clips.** The clip count ("This take clipped N times", SPEC-002 §2.1) covers the record window
  only.
- **Disk.** SPEC-002 §2.5 applies to the whole operation. Pre-roll and post-roll are also written to
  the take.
- **Crash during the recording phase.** Recovery (SPEC-004 §2.7) shows "A punch-in was in progress
  (0:01.8 of 0:03.0 recovered)" (or "A recording at 1:02:03 was in progress …").
  - **Apply as recorded** (default) applies exactly the edit that Stop at the recovered end would
    have made: a partial punch, or an Insert/Overwrite of the recovered length. Its alignment equals
    the live run's.
  - **Open as new document** opens the window part only.
  - **Discard take.**
- **Crash during pre-roll.** Nothing of the window was captured, so there is nothing to apply. The
  take file is discarded at recovery and not listed as a take.

### 2.13 Latency compensation

**What must hold.** A sound the talent makes at the instant they hear document position `q` lands
at `q`.

**Automatic part.** Recording alignment rests on cpal's own timestamps (ADR-002 §8):
- The heard time of `q` comes from the output stream (`playback` timestamp, rack latency included in
  the heard position).
- The capture time of each input sample comes from the input stream (`capture` timestamp).
- Both are mapped to the app clock.
- Aligning the two covers input latency, output latency, the rack's latency, and document ≠ device
  rate resampling, with no user action.

**Recording offset (residual).**
- Converters, USB transfer and driver safety buffers add delay that drivers often fail to report.
  The **recording offset** `δ` (ms, signed) corrects what the timestamps miss.
- `δ > 0` means recorded audio would land **late** by `δ` and is moved earlier.
- It comes from calibration (§2.14) or is typed by the user, in ms or as `N smp`, which is converted
  at the device rate. Range −500 … +500 ms.
- With `δ = 0` and ideal timestamps, alignment is already exact.

**Per device setup.**
- Offsets are stored in settings keyed by **(host, input device, output device, device sample
  rate)**. Each entry stores the value, its source (`calibrated` with date and confidence, or
  `manual`) and the buffer size it was measured at.
- A setup with no entry uses `δ = 0`, and the record panel shows "Not calibrated", linked to the
  wizard.
- If the current buffer size differs from the stored one, the value is still applied, with the amber
  hint "Calibrated at 256 frames, now 512 — recalibrate for best accuracy".
- **Decided (autonomous, T-300).**
  - With reliable timestamps the residual is mostly converter delay, which does not depend on buffer
    size, so an exact buffer-size key would throw away good calibrations.
  - The sample rate is in the key because converter filter delays are in samples.

**Unreliable timestamps.** If T-105/T-106 find a backend's timestamps unreliable:
- the automatic part falls back to estimates: capture ≈ callback − one input period, playback ≈
  callback + one output period;
- the panel shows "This audio system doesn't report latency — calibrate for accurate punch-ins";
- calibration then measures the whole error.

**Where the offset applies.** Only to **aligned starts**. Free starts and new recordings (SPEC-002)
have nothing to align to; SPEC-002 AC-3 stays valid.

**Readout.** The record panel and Settings show "Recording offset +3.00 ms (144 smp) · calibrated
13 Sep 2026".

### 2.14 Loopback calibration wizard

Settings → Audio Devices → Recording offset → **Calibrate…** (also reached from the "Not calibrated"
link). It needs an input and an output device. It is refused while recording and stops playback. No
document is needed, and none is touched.

1. **Connect.** "Connect a cable from an output of your interface to the selected input (most
   accurate), or hold the microphone within 5 cm of a speaker or headphone cup. A test sweep will
   play — take headphones off your ears and set a moderate volume."
   - Monitoring is forced **Off** for the run (it would add a second path and can feed back) and is
     restored afterwards.
   - The input is armed automatically.
2. **Measure.**
   - Five logarithmic sine sweeps play (§4.7), about 8.5 s in total, with a progress bar and the live
     input meter.
   - The sweeps are injected **after the rack**, so rack contents never affect the result.
3. **Result.** Two outcomes:
   - **Accepted:** "Measured offset +3.00 ms (144 samples at 48 kHz). Confidence: 5 of 5 measurements
     agree." It also shows the recorded peak level. The buttons are Apply, Retry and Cancel.
   - **Rejected** (confidence below 0.8): "Couldn't detect the test signal reliably (2 of 5
     measurements agree). Check the cable or move the microphone closer, then retry." Apply is
     disabled, and the stored offset is never changed by a rejected run.
   - **Warnings** on accepted runs:
     - *clipping* (any input sample ≥ 0.99990): "The input clipped — lower the output volume";
     - *weak* (median peak-to-sidelobe ratio < 3): "The signal was weak — the result may be less
       reliable".
4. **Verify** (after Apply, optional). The same measurement runs again with the new offset applied.
   It reports the remaining error, "Residual after compensation: 0.02 ms", which should be within
   ±1 sample.

- **Decided (autonomous, T-300): the test signal is an exponential sine sweep, analysed with
  band-limited GCC-PHAT.**
  - A sweep puts its energy into every frequency in turn, so the SNR is high at a comfortable level.
  - Harmonic distortion from a small speaker lands at other lags (Farina).
  - PHAT weighting makes the correlation peak sharp even through a coloured speaker→mic path.
  - Rejected: an **impulse**, which has little energy and is a loud click; **MLS**, which is fragile
    under speaker non-linearity and any clock drift across its period.
- **Acoustic path.** The mic-to-speaker distance adds ≈ 2.9 ms per metre; the 5 cm instruction limits
  it to ≈ 0.15 ms. A loopback cable measures exactly what alignment needs.

### 2.15 Shortcuts

| Command | Key | Status |
|---|---|---|
| Record (resolves per §2.2) | **Shift+R** | provisional (D-014); final in SPEC-019 |
| Stop an operation in any phase | Space, or the Stop control | SPEC-002 |
| Record mode, pre/post-roll, punch toggles | — | no default key; panel, Record-button context menu and Settings |
| Punch Again (Audition) | — | not implemented; Shift+R re-punches (§2.6); listed for SPEC-019 |

Keys act when the editor has focus and no text field or modal dialog does (SPEC-008 §2.11).

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `record_mode` | Record mode | enum | Insert / Overwrite | Insert | list | preference |
| `punch_on_selection` | Punch-in on selection | bool | on/off | on | — | §2.2 rule 2 |
| `preroll_s` | Pre-roll | s | 0.0 … 20.0 | 5.0 | 0.1 | always its full length (silence-padded) |
| `postroll_s` | Post-roll | s | 0.0 … 20.0 | 1.0 | 0.1 | truncated at `L` |
| `preroll_at_cursor` | Pre-roll when recording at the cursor | bool | on/off | off | — | aligned start for Insert/Overwrite |
| `hear_original` | Hear original while re-recording | bool | on/off | off | — | Punch, Overwrite |
| `punch_xfade_ms` | Punch crossfade | ms | 0 … 50 | 10 | 1 | equal-power, inside the new range; Punch and Overwrite |
| `listen_fade_ms` | Playback fade at the mute boundaries | ms | — | 5 | fixed | listening only |
| `record_offset_ms` | Recording offset (residual) | ms | −500.00 … +500.00 | 0 (uncalibrated) | 0.01 display; f64 stored | per (host, input, output, device rate) |
| `calib_sweep` | Sweep | — | — | 100 Hz → min(12 kHz, 0.45·rate), 1.000 s, −12 dBFS peak, 10 ms raised-cosine fades | fixed | §4.7 |
| `calib_reps` / `calib_spacing_s` | Repetitions / spacing | — / s | — | 5 / 1.6 | fixed | ≈ 8.5 s total |
| `calib_lag_window_ms` | Lag search window | ms | — | −50 … +500 | fixed | |
| `calib_psr_min` | Valid-repetition peak-to-sidelobe ratio | ratio | — | 2.0 (6.0 dB) | fixed | sidelobes outside ±1 ms |
| `calib_agree_samples` | Agreement tolerance | samples | — | ±2 | fixed | around the median |
| `calib_min_confidence` | Acceptance threshold | fraction | — | 0.8 (4 of 5) | fixed | |
| `calib_weak_psr` | Weak-signal warning | ratio | — | 3.0 | fixed | median over valid repetitions |
| `capture_history_s` | Capture-writer look-back for `k_start` | s | — | 2 | fixed | §4.5 |

## 4. Algorithm / implementation notes

### 4.1 Edits as `Replace` ops (SPEC-008 §4.1)

| Operation | Ops | Marker mapping |
|---|---|---|
| Insert at `c`, `n` samples | `Replace{c, 0, W}` | SPEC-008 §4.2 (insertion), plus the added markers |
| Overwrite at `c`, `n` samples | `Replace{c, min(n, L − c), [F_in?] ‖ W_mid ‖ [F_out?]}` | **identity**, plus the added markers |
| Punch `[S, E′)`, `E′ = E` or `p` | `Replace{S, E′ − S, [F_in] ‖ W_mid ‖ [F_out]}` | **identity**, plus the added markers |

- **Pieces.** `W` / `W_mid` are pieces referencing the take's chunks, which the capture-writer
  committed during the window. `F_in` and `F_out` are small new chunks holding the rendered crossfade
  samples (§4.2), written through a `ChunkWriter` at commit, at most 2 × 2 400 samples at 48 kHz.
  Everything else is a pure splice.
- **Marker mapping.** Identity mapping cannot be derived from a `Replace` with `remove_len > 0`
  (SPEC-008 §4.2 would move inner markers to `at`). The edit must therefore carry explicit
  `marker_ops` (ADR-004 §6) that keep positions, or a mapping flag. T-304 picks one, and the choice
  applies to journal replay too.
- **Commit sequence** (ADR-004 §3):
  1. transport already stopped;
  2. commit the final take chunk and the fade chunks;
  3. journal `chunks` + `edit {label_key, ops, marker_ops}` + `fdatasync`;
  4. swap the `Arc`;
  5. hand the reader the new snapshot.

### 4.2 Crossfade

For a boundary of length `X′` starting at document position `b`, fading from `F` to `G`
(both document-aligned):

```
for i in 0 .. X′:
    θ_i = (π/2) · (i + 0.5) / X′
    A′[b + i] = f32( cos θ_i · F[b + i] + sin θ_i · G[b + i] )   // computed in f64, rounded once
```

- **Punch-in at `S`:** `F = A`, `G = T`, `b = S`.
- **Punch-out at `E′`:** `F = T`, `G = A`, `b = E′ − X′`.
- **Punch lengths:** `X′ = min(X, ⌊(E′ − S)/2⌋)` at both ends, where `X = round(punch_xfade_ms ·
  r / 1000)`.
- **Overwrite:** the fade-in has `X_s = min(X, L − c, ⌊n/2⌋)` at `c` (only when `c < L`). The fade-out
  has `X_e = min(X, ⌊n/2⌋)` at `c + n − X_e` (only when `c + n < L`).
- **Properties.**
  - `cos²θ + sin²θ = 1` for every `i`, so uncorrelated inputs of equal power keep their power.
  - Identical (fully correlated) inputs peak at `√2` (+3.01 dB) at the midpoint. That is inherent to
    equal-power fades, and ACs check it as a property, not as an error.
  - The `+0.5` offset keeps both gains strictly inside (0, 1) and makes in/out symmetric.

### 4.3 Alignment (aligned start)

- **Heard time of `at`.** When the output callback renders the block whose heard range contains `at`,
  it posts `RtEvent::WindowHeard { t_at_ns }`:
  `t_at_ns = heard_time_ns + (at − heard_pos) · 1e9 / r`.
  - `heard_pos` and `heard_time_ns` are those of that block (ADR-002 §8).
  - Heard positions advance at `r` document samples per second whatever the device rate.
  - The rack latency is already inside `heard_pos`.
- **Capture index of `at`.** Each input block `b` has a take index `k_b` (after resampling, with the
  resampler delay compensated as in SPEC-002 §2.2) and a capture time `c_b = app_ns(ts.capture)`. The
  capture-writer computes
  `k_start = k_b + round((t_at_ns + δ_ns − c_b) · r / 1e9)`,
  using the input block whose capture span contains `t_at_ns + δ_ns` (or the nearest one), where
  `δ_ns = round(record_offset_ms · 1e6)`.
- **Window.** It is `[k_start, k_start + (E − S))` for a punch, and `[k_start, k_stop)` for cursor
  recordings.
  - `k_stop` corresponds to the heard stop position `p`: `k_stop = k_start + (p − at)`.
  - The window is fixed in take samples once `k_start` is known. Later output loss or clock drift
    cannot move it.
- **Clock drift.** With separate input and output devices whose clocks differ by `ε` ppm, the window
  is aligned exactly at `at`. The misalignment then grows by at most `ε · (q − at)` samples: 50 ppm
  over a 5 s punch is 0.25 ms. The take is not resampled to the output clock; capture stays lossless.
- **Free start.** `k_start` is the first sample captured at or after the Record command
  (SPEC-002 §4.2); `δ` is not applied.

### 4.4 Reader during an operation

- **Start.** The run starts at the virtual position `P₀ = at − pre` (or `at` without pre-roll).
- **Silence packets.** Packets with positions `< 0` are silence.
- **Record range.** Positions in `[at, E)` (punch) or `≥ at` (cursor) are silence, unless Hear
  original is on and the position is `< L`. Positions `≥ L` are always silence.
- **Fades.** The reader applies the 5 ms linear fade-out ending at `at` and the fade-in starting at
  `E` to the packet samples, so the rack sees faded audio.
- **Run length.** The run ends at `min(E + post, max(E, L))` for a punch, and at Stop for cursor
  recordings.
- **Transport state.** No `DISCONTINUITY` packet and no rack reset occur inside a run. The epoch and
  play start are set as for Play (SPEC-003 §4); the playhead after the run is set by §2.10, not by
  D-018.

### 4.5 Capture-writer

- **Take file.** For aligned operations the take WAV starts with the operation and ends with it,
  holding the whole pass.
- **Look-back.** `k_start` is usually known before its samples arrive, but may already lie up to
  input latency + max offset (≤ ~0.6 s) in the past. The writer therefore keeps the last
  `capture_history_s` = 2 s of samples in a preallocated buffer and starts the `ChunkWriter` at
  `k_start` from there.
- **Live peaks.** `VXRP` frames describe the window only (take index `k_start` onwards), with
  `take_len_samples` = window samples so far. The UI places them at `at`, from the `record_phase`
  event (§4.9). The ADR-003 `VXRP` layout is unchanged.
- **Dropout filling** (SPEC-002 §4.3) inserts silence into the take indices, so `k_b` stays
  consistent with capture times.

### 4.6 Journal (needs an ADR-004 amendment)

ADR-004 §6 already has `take_begin {take, mode: new|insert|overwrite|punch, at, len}`. This spec
adds:
- `take_begin` also carries `xfade_samples`, `offset_ns` and `aligned: bool`. For a punch, `len` is
  `E − S`; otherwise it is 0.
- **`take_window {take, k_start}`** is appended and `fdatasync`ed when the record window opens. Free
  starts write `k_start = 0` together with `take_begin`. Recovery needs it to reproduce the live
  alignment exactly (§2.12, AC-15).
- **`take_cancel {take}`**, for a cancelled operation. It lets recovery classify the session as clean
  and GC delete the take file.
- A `take_begin` without `take_window` means a crash during pre-roll: nothing is applicable, and the
  take is discarded at recovery.

### 4.7 Calibration algorithm

- **Sweep.** `x(t) = a · sin(2π f₀ K (e^{t/K} − 1))`, with `K = T / ln(f₁/f₀)`, `f₀ = 100 Hz`,
  `f₁ = min(12 kHz, 0.45 · rate)`, `T = 1.000 s` and `a = 10^(−12/20)`. Raised-cosine fades of 10 ms
  apply at both ends. Five copies play at 1.6 s spacing, injected post-rack by the output callback
  from a preallocated buffer.
- **Reference time.** Repetition `i` is heard at `t_i`, reported like `WindowHeard`.
- **Recording.** The capture is taken with the automatic alignment and `δ = 0`, so capture index 0
  of repetition `i` corresponds to heard time `t_i`. The measured lag is therefore the residual.
- **Per repetition.**
  1. Take the window `rec[τ_min, n + τ_max)` with `τ ∈ [−50 ms, +500 ms]`.
  2. Compute `R = FFT(window) · conj(FFT(sweep))`, with FFT size = the next power of two ≥ window
     length + sweep length (`realfft` in `dsp`).
  3. PHAT: `R/|R|` for bins in `[f₀, f₁]` with `|R| > 1e-20`, and 0 elsewhere.
  4. The inverse FFT gives the correlation. The peak `τ̂` is found by parabolic interpolation.
  5. PSR = peak / highest value outside ±1 ms of it.
- **Decision.**
  - A repetition is **valid** if PSR ≥ 2.0.
  - The estimate is the median of the valid `τ̂`.
  - Confidence = (valid repetitions within ±2 samples of the median) / 5.
  - Accept if confidence ≥ 0.8.
  - All-zero input makes every repetition invalid ("no signal").
- **Result.** `record_offset_ms = median τ̂ / rate · 1000`, stored as f64 without rounding and
  displayed with 2 decimals.
- **Verify** repeats this with `δ` applied; the reported residual is the new median.
- **Reference simulation** (T-300, std-only Rust with the parameters above), for the implementer's
  margins:
  - **Flat loopback**, −20 dB gain, −40 dBFS RMS white noise, delays of 240/831/2 400/5 923/9 600/−144
    samples at 48 kHz and 1 000 at 44.1 kHz: every error was ≤ 0.01 sample, PSR 53–84, confidence 1.0.
  - **Band-limited path** (2nd-order Butterworth HPF 200 Hz + LPF 5 kHz): +2.08 samples (0.043 ms).
  - **Echo** −10 dB at +3 ms: PSR ≈ 4.4, and the direct path was chosen.
  - **Noise only** (−20 dBFS): PSR 1.0–1.2, rejected.
  - **Loopback gain −50 dB:** PSR 2.0–2.6, confidence 0.8, marginal. At −60 dB it was rejected.
- **RT.** The sweep buffer and the correlation buffers are allocated before the run, on the control
  thread. The analysis runs on a worker.

### 4.8 Fake-backend requirements (T-304 extends the T-105 harness)

- **Reported latencies.** The input (`callback − capture`) and output (`playback − callback`)
  latencies are configurable. A separate hidden **residual** can be added that the timestamps do not
  report.
- **Input source kinds** (added to the existing signals):
  - `Loopback { gain_db, filter: none | bandpass(200 Hz, 5 kHz), echo: none | (delay_ms, gain_db) }`:
    a frame whose true DAC time is `t` arrives with true capture time `t`, and its **reported**
    capture time is `t + residual`.
  - `AlignedTalent(X)`: the input frame truly captured when document position `q` is truly heard
    carries `X[q]`, and 0 outside a run. It models a perfectly timed performer.
  - Additive seeded white noise at a given RMS dBFS; muting of chosen time ranges.
- **Clocks.** Same clock, or a ±ppm offset between the input and output devices.
- **Output capture.** The rendered output buffer is captured for assertions, and faults are injected
  (`DEVICE_LOST` per stream, dropped frames, unreliable-timestamp flag) as in SPEC-002.

### 4.9 IPC (shape only; names are T-304's choice)

- **Start.** `record_start { base_rev, selection: {start, end} | null }` resolves per §2.2 from
  engine-side settings.
  - It returns `RecordStarted { take_id, op: new | insert | overwrite | punch, at_samples,
    end_samples | null, preroll_samples, postroll_samples, aligned }`.
  - Errors: those of §2.2.
- **Event `record_phase { take_id, phase: preroll | recording | postroll | committing, doc_pos_samples,
  app_ns }`**, sent at each phase change. The UI uses it for the countdown and for placing `VXRP`.
- **Event `record_finished { take_id, outcome: committed(EditResult) | cancelled(reason) }`.**
  `EditResult` is SPEC-008's (selection and playhead per §2.10).
- **Calibration.** `calibration_run { verify }` is a job with `job_progress`.
  - It ends with `calibration_result { offset_ms, offset_samples, device_rate_hz, confidence,
    reps_agreeing, psr_median, peak_dbfs, clipped, accepted, reason }`.
  - Applying uses `record_offset_set { key, offset_ms, source }`.
  - The record settings go through `settings_set` (T-104).
- **i18n keys.**
  - `record.mode.*`, `record.punch_on_selection`, `record.preroll`, `record.postroll`,
    `record.preroll_at_cursor`, `record.hear_original`, `record.xfade`, `record.phase.*`,
    `record.offset.*`;
  - `calibration.*`, `history.record`, `history.punch`;
  - `notice.punch_cancelled`, `notice.punch_partial_input_lost`;
  - `error.punch_needs_output`.
- These add two events to the ADR-003 §1 list.

### 4.10 Real-time constraints

- `WindowHeard` is a position comparison inside the existing per-block `heard_pos` computation, with
  no extra time read. It leaves through the RT event ring.
- Muting and fades happen on the reader thread.
- The calibration sweep is a preallocated slice mixed post-rack. Its state is a read index and a
  flag, set by an `AudioCmd`.
- There is no allocation, lock or I/O in the callbacks (ADR-002 §2). SPEC-000 AC-2's scripted session
  gains a punch-in and a calibration run.

## 5. Acceptance criteria

**Default fixture**, unless stated otherwise:
- **Document.** 48 kHz, `A` = 20.000 s (960 000 samples) of seeded white noise at −20 dBFS RMS
  (seed 1). Point markers at 2, 6, 10 and 20 s (the last at `L`), and a region [6.5 s, 7.5 s).
- **Fake backend.** One clock, 256-frame periods with random callback sizes, reported input latency
  5 ms (240) and output latency 7 ms (336), no residual.
- **Engine state.** Empty rack, monitoring Off, `record_offset_ms = 0`.
- **Talent.** Seeded white noise `X` (seed 2, −20 dBFS RMS).
- **Settings.** Pre-roll 5.0 s, post-roll 1.0 s, crossfade 10 ms (`X = 480`).

Sample comparisons use testkit (`fnv1a_hash`, `null_test_db`, `peak_dbfs`, `rms_db`). "Fade formula"
means §4.2 within 1e-6 absolute.

- **AC-1 (resolution rule).** Given every combination of: document empty / non-empty; selection
  `null` / empty `[S, S)` / non-empty; `punch_on_selection` on / off; mode Insert / Overwrite; output
  device present / absent.
  - When Record is issued, the resolved operation equals the §2.2 table (new / punch / at `S` /
    at `c`).
  - Punch without an output device returns `error.punch_needs_output` and changes nothing.
  - Record during playback first stops the transport. The operation's `at` equals the heard position
    at the stop (±1 sample, rule 4).
  - Checked by an engine table test and by Vitest with mockIPC.
- **AC-2 (insert, free start).** Given the cursor at `c` = 480 000 and a fake input source recorded
  from command time t₀ to Stop at t₁ ≈ t₀ + 3 s, where `n = round((t₁ − t₀) × 48 000) ± 1`:
  - `A′ = A[0,c) ‖ W ‖ A[c,L)` exactly, with `W` bit-identical to the source captured over
    [t₀, t₁) (SPEC-002 AC-3), and `L′ = L + n`.
  - `W`'s first and last samples are unmodified (no fades).
  - Markers: 2 and 6 s are unchanged; 10 s → 480 000 + n; 20 s → 960 000 + n; the region is
    unchanged.
  - Selection none, playhead `c + n`, one undo entry "Record".
  - The same holds for `c = 0` and `c = L`.
  - Insert at `L` and Overwrite at `L` of the same replayed take give hash-equal documents.
- **AC-3 (overwrite inside the document).** Given mode Overwrite, `c` = 240 000 (5 s) and a take of
  `n` ≈ 144 000 (free start):
  - `A′[0,c)` and `A′[c+n, L)` are bit-identical to `A`;
  - `A′[c+480, c+n−480)` is bit-identical to `W[480, n−480)`;
  - both boundaries match the fade formula (`F=A, G=W` at `c`; `F=W, G=A` at `c+n−480`);
  - `L′ = L`;
  - every marker and the region are at their original positions (identity), including the 6 s marker
    inside the overwritten range;
  - one undo entry "Record".
  Variant: with `preroll_at_cursor` on and `AlignedTalent(X)`, `A′[c+480, c+n−480)` is bit-identical
  to `X[c+480, c+n−480)`.
- **AC-4 (overwrite past the end).** Given `c` = 864 000 (18 s) and a take of `n` ≈ 192 000 (4 s):
  - `L′ = c + n`;
  - `A′[0,c)` is bit-identical to `A`;
  - `[c, c+480)` matches the fade formula;
  - `A′[c+480, c+n)` is bit-identical to `W[480, n)`, with no end fade;
  - the 20 s marker stays at 960 000.
  With `c = L` there is no fade at all (`X_s = 0`).
- **AC-5 (punch result, exact).** Given the selection [240 000, 384 000) (5–8 s) and
  `AlignedTalent(X)`, when the punch runs to completion:
  - `A′[0, 240 000)` and `A′[384 000, L)` are bit-identical to `A` (hashes equal);
  - `A′[240 480, 383 520)` is bit-identical to `X` over the same range (shift 0);
  - `[240 000, 240 480)` and `[383 520, 384 000)` match the fade formula;
  - `L′ = L` and markers are unchanged;
  - there is exactly one undo entry, "Punch-in";
  - the selection is [240 000, 384 000) and the playhead is 240 000;
  - the take WAV holds the whole pass: at least `pre + (E − S) + post` = 432 000 samples and at
    most 432 000 + 0.1 × 48 000 (start and stop latency).
- **AC-6 (alignment and compensation).** Given `Loopback { gain 0 dB }`, Hear original on, and the
  selection [240 000, 384 000):
  - with true latencies equal to the reported ones, the punched interior `A′[240 480, 383 520)` is
    bit-identical to `A` there;
  - adding to the rack a pure-delay test module reporting 480 samples of latency gives the same
    result;
  - with an unreported residual of 144 samples (3.000 ms) and δ = 0, the interior equals `A` shifted
    late by exactly 144 (`A′[q] = A[q − 144]`);
  - with δ = +3.000 ms it is bit-identical to `A` again, and entering "144 smp" gives the same δ.
  With `AlignedTalent(X)` on devices whose clocks differ by +200 ppm and then −200 ppm:
  - the shift at `S + 480` is 0 ± 1 sample;
  - at `E − 480` it is at most 200 × 10⁻⁶ × (E − S) + 1 samples (≤ 30 samples).
- **AC-7 (crossfade shape and level).**
  - With `A` = DC 0.5 and `AlignedTalent(0)`, a punch gives `A′[S+i] = 0.5·cos θ_i` and
    `A′[E−480+j] = 0.5·sin θ_j`. With `A` = 0 and talent DC 0.5, it gives `0.5·sin θ_i` and
    `0.5·cos θ_j`. All within 1e-6.
  - With `A` = DC 0.5, Loopback gain 0 dB and Hear original (correlated), the fade samples equal
    `0.5·(cos θ + sin θ)`: minimum ≥ 0.5 (no dip) and maximum 0.5·√2 (+3.01 dB) ± 1e-6.
  - With independent seeded white noise at −20 dBFS RMS as old and new:
    - at 10 ms, the power pooled over the fade regions of 20 seeded punches is −20.00 ± 0.25 dB;
    - at 50 ms, each single fade is −20.0 ± 0.5 dB.
  - With 0 ms, `A′ = A[0,S) ‖ X[S,E) ‖ A[E,L)` exactly.
  - With `E − S` = 500 samples, each fade is 250 samples long.
- **AC-8 (pre-roll, post-roll timing and what is heard).** Given pre-roll 2.0 s, post-roll 1.0 s,
  the selection [240 000, 384 000) and a rack with no latency, the captured output from the first
  frame of the run is, within 1e-6:
  - 96 000 frames equal to `A[144 000, 240 000)` with the last 240 frames × a linear fade-out;
  - then 144 000 frames of 0.0;
  - then 48 000 frames equal to `A[384 000, 432 000)` with the first 240 × a linear fade-in and the
    last 240 × the stop fade;
  - then silence.
  Also:
  - With `S` = 48 000 (1 s), the first 48 000 frames are 0.0 (padding), followed by
    `A[0, 48 000)`.
  - With `E = L`, the run ends at `E` with no post-roll.
  - With Hear original on, the 144 000 frames equal `A[240 000, 384 000)`.
  - With Dry monitoring and a −20 dBFS 997 Hz input, the window's output RMS is −23.01 ± 0.05 dB
    (SPEC-002 AC-9).
  - The first pre-roll frame is audible < 50 ms after the command (SPEC-003 AC-1).
  - `record_phase` events `recording` and `postroll` arrive within one telemetry frame (≤ 17 ms) of
    the heard times of `S` and `E`.
  - The panel counts down "Pre-roll 2.0 s … 0.1 s" (Vitest).
- **AC-9 (stopping in each phase).** For the AC-5 punch:
  - **Stop during pre-roll:** `rev`, `audio_rev`, the hash and the undo depth are unchanged. The
    journal has `take_cancel` and no `edit`, the take file is deleted, the selection is
    [240 000, 384 000) and the playhead is 240 000.
  - **Stop during recording at reported `p`:**
    - `A′[0,S)` and `A′[p, L)` are bit-identical to `A`;
    - `A′[S+480, p−480)` equals `X`;
    - both fades match the formula with `E′ = p`;
    - `L′ = L`, with one undo entry "Punch-in";
    - `p` equals the heard position at the Stop command ±1 sample.
  - **Stop during post-roll:** the result is hash-equal to an uninterrupted run with the same talent.
  - **Stop with `p ≤ S`** (at the window's first sample) counts as cancelled.
- **AC-10 (markers).**
  - For the punch, Overwrite at 5 s and Overwrite past the end, the 2/6/10/20 s markers and the
    region keep their exact positions.
  - For Insert at 10 s, markers and region ends map by SPEC-008 §4.2.
  - Add marker during a punch lands:
    - in pre-roll, at the heard position;
    - in the window, at `S + k` (the take offset under the key press);
    - in post-roll, at the heard position.
    Each is within ±10 ms (SPEC-003 AC-6), and all are in the "Punch-in" edit.
  - In Insert mode, a marker added at take offset `k` lands at `c + k` ± 10 ms.
  - If the punch is cancelled after a pre-roll marker was added, that marker persists as its own
    "Add Marker" undo entry.
  - Undo of any of these operations restores the original marker list exactly (ids, names,
    positions).
- **AC-11 (undo/redo exactness).** For each of Insert, Overwrite inside, Overwrite past the end,
  full punch and partial punch:
  - undo restores the pre-operation hash, length and marker list exactly, and redo restores the
    post-operation ones;
  - the Edit menu reads "Undo Record" or "Undo Punch-in";
  - after undo the selection is `null` and the playhead is at `at` (SPEC-008 §2.3);
  - a seeded sequence of 20 mixed operations, undone and redone completely, matches the start and
    end hashes.
- **AC-12 (refused while an operation runs).** In each of pre-roll, recording and post-roll:
  - audio-edit commands, undo, redo and `calibration_run` return `error.not_while_recording` and
    change nothing;
  - a second `record_start` returns `error.not_while_recording`;
  - Add marker succeeds;
  - the record-panel controls are disabled, and waveform selection gestures leave the selection
    unchanged (Vitest).
- **AC-13 (dropouts during a punch).** Given `AlignedTalent(X)` and a dropped span of 480 frames at the
  capture time of `S` + 48 000:
  - the window stays aligned: `A′[q] = X[q]` for every sample after the gap (±1);
  - `A′[S+48 000, S+48 480)` is 0.0 (±1 at the edges);
  - one marker "Dropout 10 ms" is at `S` + 48 000 ± 1, inside the edit.
  A dropout during pre-roll adds no marker, and the window is still exactly aligned. With
  timestamps flagged unreliable, the marker reads "Dropout (length unknown)" and nothing is filled.
- **AC-14 (device loss during a punch).**
  - **Input** `DEVICE_LOST`:
    - in pre-roll: cancelled, the document is unchanged, and the notice appears;
    - at window position `p`: a partial punch to the last good sample, hash-equal to Stop at the same
      `p` with the same data, with the notice;
    - in post-roll: the full punch.
  - **Output** `DEVICE_LOST`:
    - in pre-roll: cancelled;
    - during recording: the take continues bit-exactly and the window ends at `E` with no post-roll;
      the result is hash-equal to the no-loss run.
  - No panic, and nothing resumes on replug (SPEC-001).
- **AC-15 (crash during an operation).**
  - `SIGKILL` after `K` window samples (K ≥ 1 s) of a punch:
    - the recovery dialog shows the interrupted punch;
    - "Apply as recorded" yields a partial punch to `L_rec`, with `K − 0.25 × rate ≤ L_rec ≤ K`;
    - `A′[S+480, S+L_rec−480)` is bit-identical to the live run's captured window, so alignment is
      preserved via `take_window`;
    - it adds one "Punch-in" entry.
  - `SIGKILL` during pre-roll: recovery lists no take for it, and the document state equals the
    state before the operation.
- **AC-16 (calibration accuracy).** Given `Loopback { gain −20 dB }` with seeded white noise at
  −40 dBFS RMS added at the input, reported latencies 0/0, and unreported residuals of 240, 831,
  2 400, 5 923 and 9 600 samples at 48 kHz (5 to 200 ms) and 1 000 at 44.1 kHz, three seeds each:
  - the result is accepted with confidence ≥ 0.8;
  - `offset_samples` equals the residual ± 1 sample.
  With reported latencies of 10 ms in and 10 ms out and a true round trip of 17 ms, the offset is
  −144 ± 1 samples. With the band-limited filter plus a −10 dB echo at +3 ms, the offset is within
  ±0.25 ms of the residual and accepted.
- **AC-17 (calibration rejection).**
  - Rejected, with the stored offset unchanged: digital-silence input (reason "no signal",
    confidence 0); white noise only at −20 dBFS (confidence < 0.8); loopback with 3 of 5 repetitions
    muted (confidence 0.4); loopback gain −60 dB under −40 dBFS noise.
  - Accepted with the correct value: 1 of 5 repetitions muted (0.8); loopback gain −40 dB.
  - A loopback that clips produces the clipping warning.
- **AC-18 (calibration scope and persistence).**
  - Apply stores the offset under (host, input, output, device rate).
  - Selecting another output device switches to that key's value, or to 0 with "Not calibrated".
  - A restart restores the values.
  - A different buffer size keeps the value and shows the recalibrate hint.
  - Manual entry: "150 smp" at 48 kHz stores 3.125 ms; ±600 ms is clamped to ±500 ms.
  - A calibration run leaves the document's `rev` unchanged and restores the monitoring mode.
  - A −6 dB Gain module in the rack changes neither the measured offset nor the recorded peak level
    (post-rack injection).
  - Verify after Apply reports a residual ≤ 1 sample.
- **AC-19 (shortcuts and focus).** In Vitest with the keymap registry:
  - Shift+R dispatches `record_start` with the current selection;
  - Space during any phase dispatches Stop;
  - the mode and pre/post-roll controls have no key binding;
  - with focus in a text input or a modal dialog open, Shift+R dispatches nothing.
- **AC-20 (real hardware, manual smoke).** On the owner's machine (Arch/PipeWire), with a loopback
  cable from an interface output to its input:
  - calibration is accepted, and Verify, run 5 times, reports |residual| ≤ 1 ms each time;
  - with Hear original on and a document containing a 1 kHz click every 0.5 s
    (`powervoice-cli gen`), a punch over [2 s, 6 s) yields recorded clicks within ±1 ms of the
    originals (checked by cross-correlating the punched region with `A`);
  - a mic-near-speaker calibration agrees with the cable result within ±1 ms;
  - a USB mic plus laptop headphones (separate clocks) punch shows no audible flam at `S`.
- **AC-21 (commit cost and RT safety).**
  - The committed edit becomes visible ≤ 500 ms after the operation ends (SPEC-002 AC-5).
  - Apart from the take's own chunks, a punch writes only the fade chunks (≤ 2 × `X` samples) and
    reads only `A[S, S+X)` and `A[E′−X, E′)` from the store.
  - The SPEC-000 AC-2 scripted session, extended with a punch-in, an overwrite and a calibration run,
    reports 0 allocations and 0 deallocations in the callbacks.

## 6. Test plan

| AC | Unit | Integration (fake backend) | Vitest (UI, mockIPC) | Manual smoke (owner, Linux) |
|---|---|---|---|---|
| AC-1 | resolution function (engine) | record during playback → stop → start | resolution + error notices | Shift+R with/without selection |
| AC-2 | `project`: insert Replace + marker mapping | free-start insert, hash vs source | playhead/selection from `EditResult` | record a pickup at a pause |
| AC-3 | overwrite Replace, fade rendering | overwrite inside, aligned variant | — | punch-and-roll in a chapter |
| AC-4 | overwrite length/extension rules | overwrite past the end | — | continue a file past its end |
| AC-5 | punch Replace composition | `AlignedTalent` punch | countdown, selection kept | re-read one line |
| AC-6 | alignment math (`k_start`) on synthetic timestamps | loopback + Hear original; delay module; residual; ±200 ppm | — | covered by AC-20 |
| AC-7 | crossfade gains, DC and noise cases (`dsp`/`project`) | correlated loopback case | — | listen to punch joins in room tone |
| AC-8 | reader packet muting/fades as a pure function | captured output timeline, phase events | countdown display | hear pre-roll and post-roll |
| AC-9 | phase state machine | Stop in each phase | — | stop early during a punch |
| AC-10 | identity mapping + added markers | markers in each phase | markers redraw | press M during a punch |
| AC-11 | seeded op sequence | undo/redo through the engine | "Undo Punch-in" text | Ctrl+Z / Ctrl+Shift+Z |
| AC-12 | refusal API | commands during each phase | disabled controls, selection lock | try Ctrl+Z during pre-roll |
| AC-13 | gap fill in take indices | dropped span during a window | — | load the machine at 64-frame buffers |
| AC-14 | loss state machine | `DEVICE_LOST` per stream and phase | notices | unplug a USB mic mid-punch |
| AC-15 | journal `take_window` replay | child `SIGKILL`, recover, compare | recovery dialog text | `kill -9` mid-punch |
| AC-16 | sweep + GCC-PHAT on synthetic buffers (`dsp`) | loopback runs through the engine | result screen | cable calibration |
| AC-17 | validity/confidence rules | rejection runs | rejected state, Apply disabled | unplug the cable and retry |
| AC-18 | settings key/fallback logic | device switch, restart, post-rack injection | readout and hints | change buffer size |
| AC-19 | — | — | keymap + focus | shortcuts in the running app |
| AC-20 | — | — | — | loopback cable, mic near speaker, USB mic |
| AC-21 | store I/O counters | commit timing; extended `no_alloc` session | — | — |

- **Fixtures.** Seeded testkit signals (`white_noise`, `sine`, DC constants, a click train via
  `powervoice-cli gen`) and the fake-backend sources of §4.8. Nothing is committed.
- **Calibration unit tests** run the algorithm on synthetic arrays without the engine. Their numbers
  must reproduce the §4.7 reference margins.

## 7. Out of scope

- **Recording variants:**
  - on-the-fly punch while playback continues (v1.x);
  - Audition's "Punch Again" command and take layering/comping (Audition stacks punch takes);
  - loop or cycle recording;
  - count-in or metronome.
- **Monitoring and alignment:**
  - automatic input-monitoring switching (Pro Tools "auto input");
  - drift-correcting a take to the output clock;
  - automatic latency measurement without a loopback;
  - video or ADR sync.
- **Editing:**
  - editing a punch's crossfade after the fact;
  - a user-facing fade tool (PROMPT §3.8);
  - fitting a recording of a different length into a selection.
- Stereo recording (PROMPT §2) and pause during recording (SPEC-002 §7).
- The final Record key and any Punch Again or mode keys (SPEC-019).

### Cross-document notes (reported, not silently resolved)

1. **SPEC-008 §2.8** decides "no fade or crossfade at any edit boundary". This spec adds crossfades
   to Punch and Overwrite boundaries.
   - SPEC-008 §7 excludes these operations, so there is no formal conflict.
   - The difference is deliberate (§2.8): two recordings join, and audio outside the selection stays
     bit-exact.
   - Insert follows SPEC-008 (no fades).
2. **D-018 / SPEC-003 §2.1** say Stop returns to the play start and engine-initiated stops keep the
   heard position. For record operations, §2.10 sets the playhead explicitly instead (`S` after a
   punch, `at + n` after Insert/Overwrite) in every ending, including device loss.
3. **D-017 / SPEC-002 §2.6** say that output loss while recording continues the recording. §2.10
   refines this: output loss during **pre-roll** cancels the punch, because no take exists yet.
4. **ADR-004 §6** needs an amendment:
   - `take_begin` gains `xfade_samples`, `offset_ns` and `aligned`;
   - new records `take_window {take, k_start}` and `take_cancel {take}`;
   - `edit` records must be able to carry an identity marker mapping for a `Replace` with
     `remove_len > 0` (§4.1, §4.6).
   T-301 builds the journal and should reserve these.
5. **SPEC-008 §4.2** moves markers inside a replaced range to its start. Overwrite and Punch use the
   identity mapping instead (§2.9), like SPEC-008's Silence.
6. **SPEC-002 §2.2** says "From M3, Record with a non-empty document records at the cursor". This
   spec refines it: a non-empty selection means punch-in by default (§2.2).
7. **ADR-002 §8** says "minus a user calibration offset". The sign convention is fixed here (§2.13:
   δ > 0 = recording arrives late and is moved earlier), and the offset applies only to aligned
   starts.
8. **ADR-003 §1** gains the events `record_phase` and `record_finished`.
9. **SPEC-006 §2.9:** selection gestures are ignored during a record operation (§2.11). This is a
   new UI rule for T-206/T-304.
10. **Audition parity, partly unverified.**
    - Overwrite/Insert modes via right-click on Record and the 5 s Punch and Roll pre-roll are
      supported by the Adobe help search summary and VO community sources.
    - Audition's default record mode, "record within a selection stops at the selection end", and any
      post-roll preference are unverified (helpx pages return 403 to automated fetches, as noted in
      SPEC-003/006/008).
    - The PROMPT §3.1 wording "configurable pre-roll/post-roll" is met by the two parameters.
