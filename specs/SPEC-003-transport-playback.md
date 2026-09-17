# SPEC-003 — Transport & playback

- **Status:** approved (owner, M0 checkpoint 2026-09-12)
- **Milestone:** M1
- **Related:** SPEC-000 (architecture overview, glossary — document time vs. device time, heard
  position), SPEC-001 (audio devices & I/O — device/rate selection this spec builds on), SPEC-002
  (recording & monitoring — record is a transport-adjacent control but specified there), SPEC-004
  (document & undo — destructive edits stop playback), ADR-002 (threading & real-time rules §4, §5,
  §8 — output callback, reader/packets, transport clock, resampling), ADR-003 (IPC data paths §3 —
  playhead extrapolation & clock sync, `VXTM` telemetry frame), ADR-004 (document storage — destructive
  edits stop playback)

## 1. Purpose

Playback is how a voice-over editor spends most of its time: listening back to a take, checking an
edit, previewing the rack. This spec defines the transport's user-visible behavior — starting fast,
tracking a trustworthy playhead, looping a selection, and behaving predictably when the document and
device disagree on sample rate — so it's fast enough and honest enough to trust while editing.

## 2. Behavior / UX

### 2.1 Transport model

PowerVoice's transport bar (PROMPT §3.6, top of the app shell) exposes:

- **Play/Pause** (single toggle control): while stopped, starts playback from the current playhead
  position; while playing, pauses — playback halts and the playhead stays exactly where it stopped,
  ready to resume from there on the next press. This mirrors Adobe Audition's actual default
  behavior, where Space is a single Play/Stop toggle that, by default, does **not** return the
  playhead to the start on stop (that legacy behavior is an opt-in preference — see §2.5).
- **Stop** (owner decision, M0 checkpoint): halts playback (~5 ms fade) and **returns the playhead to
  where playback last started** — the position Play (or Play from start) was pressed at, like
  Audition's "return CTI to start position on stop". **Pause** (Space while playing) halts and keeps the
  playhead where it stopped. Stop while already stopped does nothing. Stops initiated by the engine
  (destructive edit, device loss) behave like **Pause**: the playhead stays at the heard position.
- **Play from start** (Shift+Space, owner-confirmed Audition behavior): starts playback from the start
  of the current time selection, or from sample 0 when there is no selection.
- **Return to Start**: seeks the playhead to document sample 0. If playback was in progress, it
  continues playing from 0 (a seek, not a stop); if stopped, the playhead simply moves.
- **Loop**: toggles looping. When on and a time selection exists (at least the minimum length,
  §3), playback loops between the selection's start and end; when on with no selection, or one
  shorter than the minimum, playback loops the **whole document** (H-80 — supersedes the "no
  selection: inert" reading below and in §3/AC-4). Turning loop off while a looped playback is in
  progress lets the current pass finish and then stop advancing past the old loop end normally
  (does not cut off mid-loop).
- **Playhead follow**: while playing, the waveform view scrolls to keep the playhead visible (a
  settings/view toggle can disable this so the user can read a static region while audio plays
  underneath — out of scope to fully specify the toggle UI here, but the default is "on").

### 2.2 Playhead display and the extrapolation contract

The playhead the user sees is not sample-accurate telemetry redrawn 30 times a second — it is
**extrapolated** between telemetry frames, per ADR-003 §3:

- The engine reports, at 30 Hz over the `telemetry` channel (`VXTM`, ADR-003), an anchor:
  `(playhead_sample, playhead_time_ns, rate)` — the *heard* position (post-rack-latency, post-output-
  latency position, ADR-002 §8), the app-clock time that position was heard at, and the current
  playback rate in document samples/second (0 when stopped).
- Every animation frame, the UI computes
  `displayed_position = playhead_sample + (now_app_ns − playhead_time_ns) × rate / 1e9`, clamped to
  the document length and wrapped inside the loop range when looping.
- `now_app_ns` uses the UI's own clock-sync offset against the engine's app clock (ADR-003 §3:
  median-RTT sample of `clock_now_ns`, refreshed every 30 s).
- When a new anchor arrives that differs from the current prediction by less than 20 ms of audio, the
  UI **slews** the visible position to the new anchor over 100 ms (no visible jump); a larger
  difference (a seek, a loop wrap, a resume after pause) **jumps** immediately.
- Markers or edits made during playback are timestamped at the extrapolated position at key-press
  time, then land in document time (ADR-002 §8) — this spec only owns the *display* contract; where a
  marker actually lands is SPEC-004's concern.

### 2.3 Start latency

Pressing Play must feel instant. The output stream is already running (silence) whenever an output
device is configured (ADR-002 §1), so Play only has to wake the reader and start pulling packets —
budgeted at wake (< 1 ms) + a small pre-buffer (~20 ms) + one device period + output latency, which
ADR-002 §5 fixes at **under 50 ms** total. This applies equally to a fresh Play from a stopped state
and to resuming from Pause.

### 2.4 Device/document sample-rate mismatch

The document has its own sample rate (set at recording or import); the output device has whatever
rate is configured (SPEC-001). PowerVoice always opens the device at the document's rate when the device
supports it — the common case, so most playback needs no resampling. When the device cannot run at
the document's rate (SPEC-001 §2.2's fallback already applied, or the document rate simply isn't in
the device's supported set), the **reader** resamples document → device rate with `rubato::Fft`
(synchronous, fixed-ratio, allocation-free) before packets reach the rack (ADR-002 §5). This is
transparent to the user other than:
- The rack then runs at device rate rather than document rate in that fallback case — module
  parameters are rate-agnostic by contract, so audible behavior should be equivalent within normal DSP
  tolerances, but preview can differ from an export rendered at the document's native rate. This is a
  known, documented consequence (ADR-002 "Consequences"), not a bug.
- No user-visible dialog or delay is expected; the resampler is primed and its `output_delay()` is
  discarded so the displayed playhead (§2.2) stays exact.

### 2.5 Transport keyboard shortcuts

PowerVoice's shortcuts are meant to be Audition-compatible (PROMPT §2 "Shortcuts", locked) and not
remappable in v1. `docs/references.md` flags the exact bindings as **⚠ unverified**; this section
records what was verified via web research for this ticket, with sources, and marks the rest
**unverified** rather than guessing, per this ticket's instructions.

| Command | Shortcut | Status | Source |
|---|---|---|---|
| Play/Pause (start/stop playback in place) | **Space** | **Verified** | Multiple independent sources agree Space is Audition's Start/Stop-playback toggle: [Complete Adobe Audition Keyboard Shortcuts](https://tutorialtactic.com/blog/adobe-audition-shortcuts/) ("Start/stop playback... Spacebar"), [Adobe Audition shortcuts (pie-menu mirror)](https://www.pie-menu.com/shortcuts/adobe-audition) ("Start/stop playback uses the spacebar"), [35 Essential Shortcuts for Adobe Audition (Domestika)](https://www.domestika.org/en/blog/7961-35-essential-shortcuts-for-adobe-audition), [UW–Madison Audition manual mirror](https://sts.doit.wisc.edu/manuals/audition/), [Adobe Audition shortcut keys list](https://techguruplus.com/adobe-audition-shortcut-keys-list/). The official page (`helpx.adobe.com/audition/desktop/keyboard-shortcuts/default-keyboard-shortcuts.html`) returned HTTP 403 to automated fetch and could not be read directly, consistent with `docs/references.md`'s existing note that this page 403s. |
| Return to Start (seek playhead to 0) | **Home** | **Verified** | [Complete Adobe Audition Keyboard Shortcuts](https://tutorialtactic.com/blog/adobe-audition-shortcuts/) ("Return to Beginning: Home key"); [Adobe Audition shortcuts (pie-menu mirror)](https://www.pie-menu.com/shortcuts/adobe-audition) ("Set time indicator to beginning uses the Home key"). Two independent sources agree. |
| "Return playhead to start on stop" legacy preference (context only — see open question below; **not** itself a transport action PowerVoice necessarily implements) | Shift+X (toggles the *preference*, not playback) | Verified as an Audition preference toggle, **not evaluated for PowerVoice adoption** | [Adobe Audition shortcuts (pie-menu mirror)](https://www.pie-menu.com/shortcuts/adobe-audition) ("Toggle Preference for return cti to start position uses ⇧+x") |
| Play from start | **Shift+Space** | **Owner-confirmed** (M0 checkpoint) | The owner's Audition muscle memory; also the only web source found (tutorialtactic). |
| Record | **Shift+R** (provisional) | **Provisional** (M0 checkpoint) | Shift+Space is not Record in Audition (owner). Final binding in SPEC-019 (M7 shortcut audit). |
| Loop Playback toggle | **Unverified** | **Unverified** | No source (official or mirror) documents a default binding for toggling loop playback. `docs/references.md` does not cover it either. |
| Pause as a distinct action from Play | n/a — PowerVoice models Play/Pause as one toggle (§2.1), matching Audition's actual single Space toggle | — | See Play/Pause row above |

**Resolved at the M0 checkpoint:** the owner confirms Shift+Space = Play from start in Audition, so
PROMPT §3.6's "Shift+Space record" was wrong; Record is provisionally Shift+R. Original research note,
kept for the record — **⚠ Contradiction:** PROMPT §3.6 states "Shift+Space record" as part of
its illustrative shortcut list, and this ticket's "must cover" line repeats "Space, Shift+Space —
verify bindings." The one source this research found that mentions Shift+Space at all —
[Complete Adobe Audition Keyboard Shortcuts](https://tutorialtactic.com/blog/adobe-audition-shortcuts/)
— states "Play from Start: Shift + Spacebar," **not** Record, and no other source corroborates either
claim. The official Adobe page could not be fetched (403) to settle it directly. **This spec does not
assign Shift+Space to Record or to Play-from-Start.** It is left unverified/unassigned pending owner
confirmation or a working fetch of the official page; SPEC-002 (recording) should not assume
Shift+Space = Record without the same caveat, since recording's shortcut is equally unconfirmed.

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `loop_enabled` | Loop toggle | bool | on/off | off | n/a | loops the whole document with no (long-enough) time selection (H-80); inert only with no document |
| `loop_start_sample` / `loop_end_sample` | Loop region | doc samples (`u64`) | `0..=len_samples` | current selection | n/a | set from the current time selection when loop is enabled |
| `playhead_follow` | Playhead-follow view toggle | bool | on/off | on | n/a | view-only; does not affect audio |
| `telemetry_rate_hz` | Playhead/meter update rate | Hz | {30, 60} | 60 | n/a | ADR-003 §1; 60 Hz measured free on WebKitGTK (ADR-009 §3); 30 Hz kept as a Settings option |

## 4. Algorithm / implementation notes

- **Start.** Control bumps a play epoch, sends `Start{epoch, pos}` to the reader and `Play{epoch}` to
  the output callback; the callback discards any stale-epoch packets and only starts audibly once
  ≥ 20 ms of the current epoch is buffered, with a ~5 ms fade-in (ADR-002 §5). This, not a UI-level
  timer, is what the < 50 ms budget in §2.3 measures against.
- **Pause/Stop.** Both apply a ~5 ms fade-out and stop the reader from advancing; the playhead
  (`heard_pos`) simply stops updating at its last value. No `reset()` is called on the rack for a
  plain pause/stop (only seeks and loop wraps reset the rack, ADR-002 §5/§4, because the module chain
  state — e.g. a compressor's envelope — should carry over across a resume).
- **Return to Start / seek.** A seek while playing is fade-out → discard stale packets → `reset()` the
  rack at the new position → fade-in (ADR-002 §5). A seek while stopped just relocates the playhead;
  no audio state changes.
- **Loop wrap.** The reader emits a short packet up to the loop end, then a `DISCONTINUITY`-flagged
  packet at the loop start; the output callback calls `reset()` on the rack at that boundary (ADR-002
  §4, §5). This is why a loop wrap is a "jump" (§2.2), not a "slew," in the playhead display.
- **Transport clock.** The output callback is the transport clock (ADR-002 §8): `heard_pos = p_in −
  round(L_rack · r_doc / r_dev)`, clamped to at least the play start; `heard_time_ns` comes from
  cpal's own `playback` timestamp mapped into the app clock. The playhead is therefore never a UI-side
  guess about buffering — it is derived from the same numbers the audio hardware reports.
- **Resampling in the mismatch case (§2.4).** `rubato::Fft`, primed, `output_delay()` frames
  discarded so `doc_pos` in each packet stays exact (ADR-002 §5); the resampler lives in `dsp`, used
  from the reader (ADR-001 §3 rule: `rubato` confined to `dsp`).
- **Destructive edits stop playback.** Per ADR-004, a destructive edit (SPEC-004) stops the transport
  (fade, wait for acknowledgement) before committing, then hands the reader the new snapshot. This
  spec's Stop/Pause behavior (§2.1/§4) is the same mechanism triggered by a different caller; the UI
  should not surprise the user with an edit silently continuing playback of stale audio.

## 5. Acceptance criteria

- **AC-1 (start latency).** Given the output stream is already open (idle, emitting silence) and the
  document is at rest, when the user presses Play, then audible, correct-position output reaches the
  device buffer in **under 50 ms** measured from the engine receiving the Play command to the first
  non-silent rendered sub-block being written to the device buffer.
- **AC-2 (pause/resume preserves position).** Given playback is running and reaches document sample
  P, when the user presses Play/Pause, then playback halts, the displayed and reported playhead stay
  at P (± one telemetry frame of extrapolation, i.e. ≤ 33 ms of audio at 30 Hz, converging to exact on
  the next telemetry frame), and pressing Play/Pause again resumes from P with the same < 50 ms budget
  as AC-1.
- **AC-3 (return to start).** Given the playhead is anywhere in the document, when Return to Start is
  invoked (button or, once confirmed, its shortcut), then the playhead is set to sample 0 exactly; if
  playback was in progress it continues playing from 0 without a stop/start gap perceptible as a
  separate Play press (i.e., it behaves as one seek, per §4).
- **AC-4 (loop selection).** *(Amendment 1 supersedes this AC's rack-reset clause: the seam is
  seamless with no rack reset — H-37/A-023. Amendment 3 (H-80) adds the whole-document loop when
  there is no selection (or too short a one); this AC's selection case is otherwise unchanged.)*
  Given a time selection
  [S, E) and loop enabled with an empty (unity)
  rack, when playback runs for at least 3 loop passes, then the rendered output is **sample-exact**
  equal (max abs difference ≤ 1e-6) to the concatenation source[S..E) ‖ source[S..E) ‖ … — no
  dropped, duplicated or gap samples at any seam, and the sample after E − 1 is S. (Any click at the
  seam is then inherent to the content, as in Audition's hard-cut loop; PowerVoice adds no artifact of
  its own.) With a non-empty rack, the rack is `reset()` exactly at each seam (§4), verified by a
  fake module that records reset positions. Looping repeats until Stop or loop is disabled.
- **AC-5 (playhead follow).** Given `playhead_follow` is on and the view is zoomed such that the full
  document is not visible, when playback's displayed position approaches the edge of the visible
  waveform region, then the view scrolls to keep the playhead visible at all times during playback,
  with no visible waveform tearing/flicker (i.e., the scroll and the playhead redraw happen in the
  same animation frame).
- **AC-6 (playhead extrapolation accuracy).** Given steady playback at a constant rate with no
  discontinuity, when the displayed position is sampled at an arbitrary instant between two telemetry
  frames, then it is within **± 10 ms** of the true heard position at that instant (bound dominated by
  clock-sync offset error and playback-rate stability, not by the 33 ms telemetry period, because of
  the extrapolation formula in §2.2).
- **AC-7 (device/document rate mismatch).** Given a document at 48 kHz and an output device that only
  supports 44.1 kHz (SPEC-001 fallback already applied), when the document is played, then: no error
  is shown, the resampled output's dominant frequency for a 1 kHz test tone is within ± 0.05 % of
  1 kHz (i.e., the resample ratio is exact, not approximate), and the reported/displayed playhead
  position tracks document position with the same accuracy as AC-6 (the resampler's fixed output
  delay is compensated, not left as drift).
- **AC-8 (destructive edit stops playback).** Given playback is running, when a destructive edit
  commits (SPEC-004), then playback stops (fade, per ADR-004) before the new snapshot is handed to the
  reader — the user never hears audio from a snapshot that no longer matches the document.
- **AC-9 (Stop returns to the play start; Pause keeps position).** Given playback started (Play) at
  document sample P₀ and now at P > P₀, when Stop is pressed, then playback halts with a ≤ 6 ms fade and
  the playhead is set to P₀ exactly; when Pause is pressed instead, it stays at P (AC-2). Given Play from
  start with a selection [S, E), playback starts at S (± 0 samples) and Stop returns to S; without a
  selection it starts at 0. An engine-initiated stop (AC-8, SPEC-001 device loss) leaves the playhead at
  the heard position.

## 6. Test plan

| AC | Unit | Integration (fake backend) | Manual smoke (owner, Arch/PipeWire 1.6.8/Hyprland) |
|---|---|---|---|
| AC-1 | Start-budget arithmetic (pre-buffer + period + latency) as a pure calculation over synthetic device-period values | Fake backend with synthetic callback timing; drive Play and assert first non-silent sub-block timestamp − command timestamp < 50 ms, including randomized callback sizes (per ADR-002 `assert_no_alloc` harness) | Play a file, eyeball/measure via a loopback or by ear that Play feels instant; not a substitute for the integration timing test |
| AC-2 | Playhead-hold logic when a `Pause` command is applied mid-stream | Fake backend: play, pause at a known sample, assert reported `heard_pos` unchanged across ticks, resume, re-check < 50 ms | Manually pause/resume a playing file, confirm no audible glitch or position jump |
| AC-3 | Seek-while-playing vs seek-while-stopped branch logic | Fake backend: seek to 0 mid-playback, assert continuous audio (no full stop/start) and exact `doc_pos` | Press Return to Start during playback, confirm it feels like a jump, not a restart |
| AC-4 | Loop-wrap packet emission (short packet + `DISCONTINUITY` at boundary) as a pure function of loop region | Fake backend renders ≥ 3 passes of a loop over a ramp/noise source with an empty rack; assert output == repeated source[S..E) within 1e-6 (testkit null test); with a reset-recording fake module, assert `reset()` positions == seam positions | Loop a short selection on real hardware; confirm the loop repeats seamlessly and only content-inherent clicks are heard |
| AC-5 | n/a (UI/view behavior, not engine logic) | Vitest: simulated telemetry stream drives the waveform view component; assert scroll-to-playhead invariant holds every animation frame | Play a long file zoomed in, confirm the view visibly tracks the playhead with no tearing |
| AC-6 | Extrapolation formula (`§2.2`) unit-tested against synthetic anchors + a fake now-clock, checked against a ground-truth linear position function | Fake backend + simulated clock-sync offset/jitter; assert predicted vs. true position error stays within ± 10 ms across a played interval | n/a (sub-frame timing not practically eyeballed); rely on the integration test |
| AC-7 | Resample-ratio exactness (`rubato::Fft` wrapper) against a synthetic 1 kHz tone, measured with `testkit` frequency estimation | Fake backend configured with a device rate ≠ document rate; assert tone frequency and playhead accuracy (reuses AC-6's harness) | Force a rate mismatch via Settings (SPEC-001) on a real device if one is available that doesn't support the document's rate; otherwise this AC is covered by the integration test only |
| AC-8 | n/a (cross-spec sequencing, not a pure function here) | Fake backend: trigger a destructive edit mid-playback (stub from SPEC-004's test surface), assert transport-stop event precedes the snapshot swap | Perform a destructive edit (e.g. trim) while a file plays, confirm playback stops before the edit visibly applies |
| AC-9 | Transport state machine: Stop → play-start position, Pause → hold, engine stop → hold; Play from start with/without selection | Fake backend: play from P₀, stop at P, assert heard position = P₀; pause variant = P | Play, Stop (jumps back), Play, Space (stays); Shift+Space with and without a selection |

## 7. Out of scope

- Recording, punch-in, and the Record control's shortcut (SPEC-002) — flagged above as sharing the
  same Shift+Space verification problem.
- Monitoring modes and monitoring latency/drift correction (SPEC-002; this spec's playback path is the
  same rack input as monitoring but the monitoring-specific behavior is specified there).
- Marker placement/navigation and their shortcuts (SPEC-004 / a markers spec).
- Undo/redo shortcuts (Ctrl+Z / Shift+Z, PROMPT §3.6) — SPEC-004.
- The full Audition shortcut map beyond the transport commands named in this ticket (owner review at
  M7 per PROMPT §7).
- Whether PowerVoice adopts Audition's "return playhead to start on stop" legacy preference (Shift+X) —
  noted in §2.5 as verified Audition behavior but not decided for PowerVoice; open question below.

---

**Open questions for the owner** — resolved at the M0 checkpoint: (1) Stop returns the playhead to
the play start (§2.1, AC-9); (2) Shift+Space = Play from start, Record provisionally Shift+R; (3) the
loop-toggle key is assigned in SPEC-019. Original questions:
1. Should PowerVoice implement Audition's "return playhead to start on stop" preference (default off,
   toggle verified as Shift+X in Audition)? This spec currently assumes PowerVoice always leaves the
   playhead in place on Pause/Stop (Audition's modern default), with no such preference in v1.
2. Record's default shortcut is unverified and contradicts PROMPT §3.6's "Shift+Space record" claim
   (§2.5). Needs either a working fetch of the official Adobe page or an explicit owner decision to
   keep/drop that binding.
3. Loop Playback's default shortcut is unverified (no source found any binding at all) — assign a
   PowerVoice-specific key, or leave loop toggle mouse/menu-only for v1?


## Amendment 1 — H-37 loop playback, as implemented (2026-09-15, autonomous)

Conservative, Audition-like readings of what §2.1/§3/§4/AC-4 left open or contradicted:

- **Loop region = the live time selection.** While Loop is on, the loop region is the current
  selection; the UI syncs every selection change to the engine (`transport_set_selection`, which
  also feeds Play from start — the engine's selection was a never-called stub before). A change
  during playback applies from the reader's read position (≤ ~200 ms ahead of what is heard). A
  selection shorter than **10 ms** is inert, like no selection. Clearing the selection during looped
  playback lets playback continue normally past the old loop end.
- **Where looping applies.** Playback wraps when it reaches the loop end from before it: a Play
  before S plays into the loop, a Play at or after E plays on to the document end without looping.
  Turning Loop **on** mid-play wraps at E when the read position is still before E.
- **Loop off mid-play** (§2.1's "lets the current pass finish and then stop advancing past the old
  loop end"): the pass being heard finishes, then playback **stops at the old loop end** with Pause
  semantics (the playhead stays at E, like the document end; the end is reported once the rack has
  drained, T-401). If the reader already queued the next pass, the output callback ends at that
  pass's first packet. Re-enabling Loop within the reader's read-ahead of the end may not cancel
  the stop.
- **The seam** is a hard cut with no crossfade (AC-4) and is **seamless**: no fade, no epoch
  change, and — **superseding §4 "Loop wrap" and AC-4's reset clause — no rack reset**. The rack
  processes the looped stream continuously, so tails (reverb, delay lines, lookahead) run into the
  next pass, as in Audition. The reader flags the first packet of a pass `LOOP_WRAP` (not
  `DISCONTINUITY`). AC-4's last sentence now reads: *With a non-empty rack, the rack is not reset at
  a seam: through an exact delay module the output is the looped stream delayed by the module's
  latency across every seam.*
- **Heard position (§4 "Transport clock").** `heard_pos = p_in − L_rack` is mapped through a small
  preallocated history of the rack input's recent starts and loop wraps, so for one rack latency
  after a wrap the playhead reads the end of the previous pass, not the play start. The "clamped to
  at least the play start" rule applies to the current play/seek start only.
- **Display (§2.2).** The extrapolated position wraps inside the engine's effective loop range
  (`TransportState.loop_range`) once it passes the loop end, so the playhead jumps back to S at the
  seam by itself and the post-wrap anchor agrees with the prediction (§4's "a loop wrap is a jump"
  is superseded: at most a slew). Telemetry sets `VXTM` `LOOPING` while playing a loop.
- **Shortcut (§2.5 "Loop Playback toggle" row, and open question 3):** **Ctrl/⌘+L**, Audition's
  default per the secondary sources checked (killerkeys, Prism Multimedia); the official Adobe page
  still refuses automated fetches. The toolbar's Loop button (after Play from start) shows the
  pressed state; while Loop is on without a usable selection its tooltip says to select a range.
- **Device-rate mismatch (§2.4).** The reader resamples the looped stream without resetting the
  resampler at the seam, so the resampled output stays continuous; packet positions follow the
  exact input-time of each device frame, so the playhead does not drift over passes.
- **Export and bake** render the document range and never loop.

## Amendment 2 — H-46 rack pre-roll on Play and seek (2026-09-15, autonomous; A-026)

Decided by A-026 ("pre-roll the rack on Play/seek so rack latency isn't added to the playback
start"); what it left open is read the conservative, Audition-like way. Supersedes the rack part
of §2.3's budget, §4 "Start" and "Return to Start / seek", and §4 "Transport clock"'s clamp window;
the engine side is ADR-002 Amendment 4, the rack side SPEC-012 §2.5.2.

- **Pre-roll.** Every start — Play, Play from start, a seek or Return to Start while playing, a
  record operation's playback run (SPEC-022 §4.4) — pre-rolls the rack, so its latency L (the
  installed chain's total, SPEC-012 §2.5) is not added to the start:
  - **warm-up:** the reader starts L samples (at the document rate, rounded up) before the play
    position; the rack gets that audio first, and it is never heard. Near the document start there
    is less of it (nothing before sample 0: the rack then starts from its reset state, i.e. from
    silence, as before). Only a start that resets the rack uses it; a resume at the pause position
    (no reset, §4) keeps the rack's own state and drops the warm-up.
  - **look-ahead:** the rack then gets L samples from the play position ahead of time, so the play
    position is the next sample out of the rack.
  - Both run off the device timeline, faster than real time, with the rack's output discarded. The
    heard output starts **exactly at the play position**, with the ~5 ms fade-in applied **after**
    the rack (the rack sees the unfaded audio, as in an export). Nothing before the play position
    is heard, and the meters only see what is heard.
- **Start time (§2.3, AC-1).** The pre-roll runs inside the output callback at up to 32× real time
  per callback (the voice rack's ~2 × 2.3k samples fit in one 256-frame callback, or three 64-frame
  ones), so a start takes what it takes with an empty rack, plus at most a few callbacks for a very
  long latency or a tiny buffer. If the pre-roll outruns the reader (L beyond ~90 ms), it resumes
  in a later callback: the device gets silence and the rack is not advanced in between, never fed
  silence inside the stream. AC-1's < 50 ms therefore holds with any realistic rack; the
  `playback_start` bench asserts the voice rack within 5 ms of the empty rack.
- **Seek while playing** (§2.1, §4; also a Play during a Pause/Stop fade-out): **re-pre-roll, no
  crossfade.** The heard output fades out (~5 ms) **at once, after the rack** — not after the rack
  has played out its latency — then the rack is reset, pre-rolled at the target, and the target
  fades in. What the rack still held of the old position (≤ L) is dropped under the fade-out, as
  any seek drops the rest of the old position. A Play after a Pause/Stop whose tail is still
  coming out of the rack (≤ L after its fade-out) still waits for that tail: nothing heard is cut
  (T-401).
- **Commands during a pre-roll** (Play, seek, Stop before the first sample is heard) abandon it and
  reset the rack (it holds only unheard audio); Stop then reports the play position, as a Stop
  before the start always did.
- **Document end inside the look-ahead** (a start less than L before the end): the rest of the
  look-ahead is silence, so the remaining audio is heard at once; the end is reported once its
  last sample is heard (SPEC-012 §2.5.1).
- **Loop** (Amendment 1): the pre-roll never wraps. A Play at or after the loop end whose warm-up
  reaches back across it plays on; a Play inside the loop (or from its start) warms up with the real
  audio before the play position, also before the loop start. Wraps inside the look-ahead are
  seamless as before.
- **Through-rack monitoring** (SPEC-002 §2.7): while the live input flows through the rack, a start
  is as before — no reset, no pre-roll, the fades before the rack, the rack latency added — because
  a reset or a burst would cut the talent's monitored voice.
- **Punch-in and record runs** (SPEC-022): the run's own pre-roll (§2.2: playback from `S − pre`) is
  unchanged; the rack pre-roll comes before it and only makes the run heard L sooner. The take's
  alignment uses the heard time of `at` (SPEC-022 §2.13), which stays exact.
- **Heard position** (§4 "Transport clock", ADR-002 §8): same formula, `heard_pos = p_in − L_rack`,
  with rack-input frames counting the pre-roll, so the first heard sample reads the play position
  and the playhead is exact from the first frame. The window where the playhead sat "clamped to the
  play start" for L after a start is gone. The telemetry anchor of a callback in which playback
  starts part-way through is stamped with that frame's heard time.
- **Export and bake** (offline, SPEC-012 §2.8.1) are unchanged.

## Amendment 3 — H-80 loop the whole document with no selection (2026-09-16, owner-requested)

The owner reported that with nothing selected, turning Loop on and pressing Play ran to the
document end and stopped, "even though loop playback is active" — exactly §2.1/§3/AC-4's "inert
with no time selection" reading (H-37, Amendment 1). Confirmed as designed, not a defect; the
owner asked for it to loop the whole file instead, as Adobe Audition's Loop Playback does. This
amendment supersedes that reading everywhere it appears (§2.1's "Loop" bullet, the §3 table's
`loop_enabled` note, and AC-4's introduction) and updates §2.5's "Loop Playback toggle" tooltip via
the UI story below. Nothing else about H-37/Amendment 1's loop mechanics changes: the seam stays
seamless with no rack reset, the reader/output packet mechanism (§4) is unaware of *why* the
effective region is what it is, and loop-off's "finish the pass, then stop at the old end" is
unchanged.

- **Effective loop region.** While Loop is on: the current time selection, when it is at least the
  minimum length (10 ms, unchanged); otherwise **the whole document**, `[0, len)` — covering both
  "no selection" and "a selection shorter than the minimum" the same way, so the Loop toggle's
  on-state always means "this is actually looping" (never silently inert with a document open).
  `None` (no effective region) now only means the toggle is off, or there is no document at all.
  **Choice, and why:** the alternative (a notice explaining a too-short selection is inert) was
  rejected — it would leave the toggle lit while nothing loops, contradicting the "on-state always
  means actually looping" requirement, and a silent, visible fallback needs no separate UI: the
  loop overlay (brace strip, SPEC-006 §2.12) already shows the *effective* region, so covering the
  whole waveform *is* the notice.
  - **Toolbar tooltip** (§2.5's "Loop Playback toggle" row): `transport.loop_inert` ("open a file
    to loop") now shows only with no document loaded; every other case shows the plain
    `transport.loop` label, since Loop is never inert-with-lit-toggle once a document is open.
- **Switching the region live.** Making a selection while looping the whole document, or clearing
  one while looping a selection, switches the effective region the same way any selection edit
  already does during looped playback (Amendment 1: "applies from the reader's read position,"
  no epoch change, no rack reset) — no stop, no glitch, seamless either direction. The existing
  "where looping applies" rule (Amendment 1) covers the edge case where the read position has
  already passed the new region's end when the switch lands: playback then continues to the
  document end without looping into the (now behind it) region, exactly as a fresh Play at or
  after the loop end would — the switch is not a special case, it reuses the same rule.
- **`loop_region()`'s signature is unchanged** (`crates/engine/src/transport.rs`): only its
  fallback changed, from `None` to `Some((0, len))`, when the selection is missing or too short
  (and the document is non-empty).
