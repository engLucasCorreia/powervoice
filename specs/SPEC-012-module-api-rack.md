# SPEC-012 — Module API & rack

- **Status:** approved (owner, M0 checkpoint 2026-09-12)
- **Milestone:** M1 (T-103: registry, chain swap, host bypass, dual-mono shim, parameter routing and
  coalescing, latency sum, Gain, offline render, `powervoice-cli render --rack`). M4 adds latency
  compensation (T-401), the rack panel and generic parameter UI (T-405) and presets (T-406). Each AC
  is tagged with its milestone.
- **Related:** SPEC-000 (glossary), SPEC-002 (through-rack monitoring latency), SPEC-003 (heard
  position), SPEC-004 OD-4 (rack edits and undo) · ADR-005 (structure) · ADR-001 §5 · ADR-002 §3 ·
  ADR-008 · code: `crates/module-api`, `crates/rack` (T-005, merged)

## 1. Purpose
The rack is where a voice-over user shapes their sound non-destructively, while hearing it. They
expect it to behave like a hardware chain:
- changes are heard at once, without clicks or zipper noise;
- bypass is an honest before/after;
- the latency they are warned about is the real one;
- the export sounds exactly like the preview.

This spec defines that behavior for every module behind the Module API (ADR-005): built-ins now,
installed modules and external plugins later.

## 2. Behavior / UX

### 2.1 Rack panel (UI in M4; the engine and CLI behavior is M1)
- **Place.** The rack is the right-hand panel (PROMPT §3.6).
- **Header.**
  - The whole-rack **A/B bypass** toggle.
  - The **total latency** readout, e.g. "Latency 12.7 ms (608 smp)", hidden when 0.
  - The rack-preset menu (T-406).
- **Slots.** Listed top to bottom in processing order, up to **16**. Each slot header shows:
  - a bypass (power) toggle;
  - the module name;
  - its latency when > 0;
  - a telemetry meter where the module offers one (e.g. gain reduction);
  - a menu (Presets, Remove);
  - a collapse arrow.

  The slot body is the generic parameter UI (§2.6) or a custom panel built only on the same schema
  and extensions (ADR-005 §13).
- **Add module.** It opens a menu of the modules in the **module registry**, grouped by feature (EQ,
  dynamics, restoration, utility, …). A new slot goes to the end; dragging a menu entry into the list
  places it at the drop position. At 16 slots the button is disabled.

### 2.2 Slot operations
- **Add, remove and reorder** (drag handle) are **live**: they apply during playback and monitoring,
  never stop the transport, and never create an undo entry (SPEC-004 OD-4 default).
  - The change is audible within **50 ms** of the command, plus the new module's own activation time
    for heavyweight modules, **plus its reported latency**: a new or replacement instance's fade-in
    waits until its output is valid (amended at T-103 review).
  - It arrives through a **15 ms crossfade**, so there are no clicks and no time jump.
- **Untouched slots keep their state.** A rack edit must not audibly restart the modules the user
  didn't touch: no envelope reset, no delay line emptied. Inserting a neutral module leaves the
  output unchanged (AC-1). T-103 is free to move unchanged instances into the new chain (the crate
  docs describe this) or to use any other mechanism that meets the AC. When a slot is reordered, the
  moved module may restart from its reset state behind the crossfade.
- **Empty rack.** Removing the last slot leaves an empty rack, which passes audio unchanged,
  bit-exact.
- **Failures at insertion.**
  - A module that fails to activate is not inserted: "Couldn't start ‹module›: ‹reason›".
  - A module with no mono or dual-mono-capable layout is refused: "‹module› has an unsupported
    channel layout". Stereo-only modules are wrapped in the dual-mono shim (ADR-005 §8) and behave
    like mono modules.
- **Missing modules.** A slot whose module is not installed (from a sidecar or preset) becomes a
  **placeholder**:
  - audio passes dry, latency 0;
  - the slot reads "Missing module ‹id@version›" or "‹id› requires a newer version";
  - its stored reference and state are written back verbatim on save (ADR-005 §2).
- **Persistence.** The rack is saved in the session within 2 s (SPEC-004) and in the sidecar (M3,
  T-306).

### 2.3 Bypass
- **Per-slot bypass.** The toggle crossfades over **15 ms** (linear, equal-gain) between the module's
  output and the slot input delayed by that module's latency.
  - The module **keeps processing**, so un-bypassing is seamless.
  - The reported latency does not change, and nothing shifts in time.
  - The bypass flag is part of the slot, so it is saved and honoured by offline renders (export,
    bake).
  - Modules with their own `BYPASS` parameter (external plugins) receive it instead, with the same
    user-visible result (ADR-005 §9).
- **Whole-rack A/B.** The toggle crossfades over 15 ms to the rack input delayed by the **total** rack
  latency, so the before/after comparison is time-aligned. It is a **listening aid**:
  - it affects playback and monitoring only;
  - it is reset to off when a document opens;
  - offline renders (export, bake, ACX) always render the rack, with per-slot bypass flags honoured;
  - while it is on, the rack header shows a highlighted "Rack bypassed (listening only)" badge.

### 2.4 Parameter changes and smoothing
- **Timing.** A continuous automatable parameter change starts at the change's sample and reaches its
  target within the module's declared smoothing time. This covers slider drags, wheel steps, typed
  values and parameter-only preset loads. It is **free of clicks and zipper noise** as measured in
  §4.3.
- **Stepped parameters** (switches, enums, FFT sizes) take effect at the change's sample. Where
  switching would click, the module's own spec requires a crossfade or a restart.
- **Changes needing a new instance** (a noise print, a preset with a state blob, a latency change)
  replace the instance behind a 15 ms crossfade (ADR-005 §12).
- **No lost values.**
  - The UI sends at most one change per animation frame while dragging; the latest value wins.
  - The rack never drops a change. If a block's event list is full, the rest are delivered at the
    start of the next block.
  - Several changes to the **same parameter at the same sample** are coalesced **in place**. The
    module sees one event, at the position of the first, carrying the latest value.
  - After any gesture, the value the module uses equals the last value the UI sent.
- **Values reported by a module** (read-only parameters, and changes made inside an external plugin's
  own GUI) reach the UI within 100 ms, through the same `param_changed` event. The T-005 rack
  currently drops them; T-103 adds the RT drain.

### 2.5 Latency reporting
- **Per slot.** Each slot shows its latency in ms when > 0, and in samples on hover.
- **Rack total.** The total is the **sum of all slot latencies, bypassed slots included**, since their
  dry path is latency-matched. Placeholders count as 0. It is shown in:
  - the rack header;
  - the monitoring readout (SPEC-002 §2.7);
  - the playhead's heard position (ADR-002 §8, SPEC-003).
- **Updates.** When a module's latency changes (e.g. a new FFT size forces a restart), every readout
  updates within **100 ms** of the replacement taking effect.
- **Compensation.** From M4 (T-401):
  - the playhead accounts for the latency;
  - A/B dry is delayed to match;
  - offline renders come out time-aligned with the input (§2.8). Offline alignment already applies in
    M1 through T-103's offline render.

### 2.6 Generic parameter UI (M4, T-405), derived only from the schema
- **Layout** (ADR-005 §13):
  - Ungrouped parameters come first, then groups in declaration order. One nesting level is rendered;
    deeper groups are flattened into "Parent / Child" titles.
  - A group with an enable parameter shows a header toggle. Its body is dimmed but stays editable
    when off.
  - Collapse state follows `collapsed_by_default`, and the user's changes are kept in the view state.
  - Modules with more than 32 visible parameters get a filter box.
- **Widgets from flags:**
  - hidden and bypass parameters are not shown (the slot's power toggle maps to bypass);
  - read-only parameters are readouts;
  - booleans are toggles;
  - enums are dropdowns;
  - stepped numbers are sliders with detents;
  - continuous parameters are an Audition-style horizontal slider plus a value field.
- **Interaction:**
  - dragging moves the **normalized** position through the parameter's taper; Shift = fine (×0.1);
  - the wheel moves one step (stepped) or 1 % of travel (continuous);
  - double-click resets to the default;
  - clicking the value opens a text field. Enter commits, and Rust parses the text. Unparseable text
    reverts to the previous value with an error outline and sends nothing. Esc cancels.
- **Displayed text** is always Rust's formatting, echoed in `param_changed`. The UI never formats or
  maps values itself, so displays are identical for built-ins and plugins.
- **Text rules** (locale-neutral, `.` decimal separator, `−` accepted as minus) are ADR-005 §3's table.
  For **stepped** parameters the number of decimals is **derived from the step grid**, which is the
  orchestrator decision T-005 implements as `display_decimals()`:
  - decimals = max(declared `decimals`, the decimals needed to write every legal value `min + k·step`
    exactly), capped at 6;
  - after a unit switch (Hz ≥ 1000 → kHz, ms ≥ 1000 → s): max(2, the grid's decimals in the new unit).

  So typed text always round-trips **exactly** for stepped, enum and boolean parameters. Examples: step
  0.5 with `decimals = 0` shows "2.5"; step 1 Hz shows "1.251 kHz". Continuous parameters round-trip
  within half a unit of the last shown digit.

### 2.7 Presets (M4, T-406)
Loading a parameter-only module preset behaves like typed values: smoothed, with no restart. A preset
with a state blob replaces the instance behind the 15 ms crossfade. The storage format belongs to
T-406.

### 2.8 Offline render (M1: T-103 and `powervoice-cli render --rack`; used by export, bake and ACX later)
- **Same code.** The offline render uses the same rack code as realtime, with its own module
  instances, in offline mode, with 4096-frame blocks (ADR-001 §5, ADR-002 §2).
- **Length and alignment.**
  - The output is **the same length as the input**, and **time-aligned**: output sample n corresponds
    to input sample n.
  - The total latency L is trimmed by feeding L samples of silence after the input end and discarding
    the first L output samples.
  - A whole-file render has no pre-roll and appends no tail. Tail and pre-roll policy for bakes and
    exports of selections belongs to T-602.
- **Deterministic.** Two renders of the same input and rack are bit-identical on any machine and with
  any thread count.
- **Equal to preview.** Realtime output with the same input and parameter changes at the same sample
  positions differs by at most **1e-6** absolute (≈ −120 dBFS), after aligning by the reported latency
  (ADR-001 §5).
- **CLI.** `powervoice-cli render --rack <rack.json> <in.wav> <out.wav>`:
  - the rack file uses the sidecar's slot schema (ADR-005 §10);
  - an unknown module id is an **error** (exit ≠ 0, listing the missing ids), because a render must
    never be silently dry;
  - the output WAV is 32-bit float at the input rate.
- **Module failure.** A module that fails during an offline render aborts it with an error naming the
  slot (ADR-008 §5).

### 2.9 Safety nets
- **Non-finite guard.** If a slot outputs any non-finite sample (NaN/±inf), the rack:
  - replaces the non-finite samples of that block with 0;
  - bypasses the slot (15 ms crossfade) and marks it **failed** ("‹module› produced invalid audio and
    was bypassed", with Restart / Remove);
  - never passes a non-finite sample to later slots or to the device.

  Offline renders abort with an error instead. This is a spec addition beyond ADR-005, which only
  requires finite output from valid modules: one bad module must not poison every downstream module's
  state or the user's ears.
- **Module errors** (adapters only; `ProcessStatus` is `#[non_exhaustive]`, and built-ins never
  return `Error`) get the same bypass-and-notice treatment (ADR-008 §5).

### 2.10 Module registry
- **Where it lives.** The registry is in the `rack` crate (T-103). It maps module id → factory; the
  Add menu and sidecar/preset resolution use it.
- **Registration.** The composition roots (`src-tauri`, `cli`) register the built-ins, and in M8 the
  plugin factories.
- **One version per id.** Only one version per id is registered, and ids are permanent. Resolution is
  by id; the stored version is informational (ADR-005 §2).

> **OWNER DECISION OD-1 — Are rack edits undoable?**
> This is decided together with SPEC-004 **OD-4**, and the options and rationale are there.
> **✅ Decided by the owner at the M0 checkpoint: rack edits are not in the document undo history in v1. Undoing a
> bake restores the pre-bake rack.**

> **OWNER DECISION OD-2 — Module id namespace (ADR-005 open question 1, ADR-006 open question 3).**
> Built-in ids (`org.powervoice.gain`, …) become permanent once the first sidecar is written (M3, T-306).
> - **A.** Keep `org.powervoice.*`, even if the product is renamed.
> - **B.** Switch to the final product's namespace before M3.
>
> **Owner decision (M0 checkpoint): rename the product now; built-in ids use the new product's
> namespace from M1 on (MEMORY D-012).** Superseded recommendation: A until the owner names the product; decide at the M2 checkpoint at the
> latest.**

## 3. Parameters
Rack behavior constants:

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `xfade_ms` | Bypass / swap / replacement crossfade | ms | 10–20 | 15 | fixed | linear, equal-gain (ADR-005 §9) |
| `max_slots` | Slots per rack | — | — | 16 | fixed | |
| `event_capacity` | Events per slot per block | events | — | 512 | fixed | overflow carries over |
| `queue_capacity` | Queued events per slot | events | — | 1024 | fixed | `RackOptions` |
| `rt_offline_tol` | Realtime vs offline difference | abs | — | 1e-6 | fixed | ADR-001 §5 |
| `edit_audible_ms` | Rack edit → audible | ms | — | ≤ 50 | fixed | plus heavy module activation |
| `latency_update_ms` | Latency readout update after a change | ms | — | ≤ 100 | fixed | |

Built-in **Gain** module (M1, `org.powervoice.gain@1.0.0`):

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `gain_db` | Gain | dB | −60 (= −∞) … +24 | 0.0 | Db { neg_inf_at_min }, continuous, 1 decimal | `smoothing_ms` = 20, linear ramp in linear gain; latency 0; no tail |

## 4. Algorithm / implementation notes

### 4.1 Status of the code (T-005, merged) vs this spec
- **Already present:** sample-accurate per-slot event routing across sub-blocks, carry-over of full
  lists, the latency sum, `display_decimals()`, and `ModuleTestHost`.
  - `ModuleTestHost` checks immediate effect by default. Parameters whose effect is deliberately
    delayed opt out with `allow_delayed_effect(id)`.
  - Its timing, flush and state checks are latency-aware (the first change is expected at
    `k + latency_samples()`) and run in offline mode.
- **T-103 adds:** the registry, chain swap/retire with the 15 ms crossfade, keeping untouched slots
  warm, host bypass and A/B, the dual-mono shim, placeholders, same-offset/same-id coalescing, the RT
  drain of module output events, the non-finite guard, offline render with latency trim, and the CLI.

### 4.2 Event coalescing
The rack's `push_event` checks whether the slot's pending queue already holds an event with the same
`(offset, id)` in the block being assembled. If so, it overwrites that event's value in place and
leaves its position unchanged; otherwise it appends. This keeps the ADR-005 §4 guarantee ("for the
same `(offset, id)`, the last one wins") without spending queue capacity. Carried-over events move
to offset 0 but are **not** merged with each other — every value is delivered (ADR-005 "delayed,
never lost", AC-7 bullet 3); only a new push coalesces with the last queued event of the same
`(offset, id)` (amended at T-103 review).

### 4.3 Zipper / click measurement (normative for AC-3, AC-4, AC-5 and SPEC-002 AC-9)
- **Signal.** A 997 Hz sine at −20 dBFS peak, 48 kHz, f32, 3.0 s, starting phase 0. The tone is
  non-synchronous with blocks and FFT bins, and sits at voice-over level. A module spec may substitute
  a different steady signal when the parameter has no effect on a sine, e.g. a gate threshold. It may
  not change the analysis or the threshold.
- **Change.**
  - One event at sample **S = 72 013**, deliberately not block-aligned. It moves the parameter from
    `from_normalized(0.25)` to `from_normalized(0.75)` in one run, and back in a second run.
  - The starting value is set by state before activation.
  - Runs are made in realtime mode (random block sizes 1…1024, seeded) and in offline mode.
  - For crossfades (bypass, swap, A/B), S is the sample at which the toggle takes effect.
- **Drag variant.** 60 events, 16.667 ms apart, moving linearly in normalized position from 0.25 to
  0.75.
- **Analysis.**
  - STFT with a 4-term Blackman-Harris window (a0 = 0.35875, a1 = 0.48829, a2 = 0.14128,
    a3 = 0.01168), N = 8192, hop 1024.
  - Bin level `L = 20·log10(|Y(k)| / (Σw/2))` dBFS, so a full-scale sine's peak bin reads 0 dBFS.
  - The output is shifted by the module's latency first.
- **Frames.**
  - T_s = max(declared `smoothing_ms`, 1 ms); for crossfades T_s = 15 ms.
  - **Steady frames:** the last 4 frames ending before S − 50 ms and the first 4 starting after
    S + T_s + 50 ms, or after the drag's last event + T_s + 50 ms.
  - **Reference** R(k) = the per-bin maximum over the steady frames.
  - **Transition frames:** every frame whose window overlaps [S, S + T_s), or the whole drag span.
- **Bins judged.** f ≤ 20 kHz and |f − 997 Hz| ≥ B_ex, where **B_ex = max(10 / T_s, 1000 Hz)**.
  Inside B_ex, the level change itself necessarily produces modulation sidebands; that is not zipper
  noise.
- **Pass** if, for every transition frame and every judged bin, **L ≤ max(R(k) + 3 dB, −90 dBFS)**.
  No component may rise more than 3 dB above the steady-state spectrum unless it stays below
  −90 dBFS. The report states the worst excess.
- **Calibration (this spec's own simulation, 997 Hz at −20 dBFS, 0 → −20 dB gain step):**

  | Transition | Worst level, 1–2 kHz from the tone | Worst level, 2–5 kHz from the tone | Worst level, 5–20 kHz from the tone | Verdict |
  |---|---|---|---|---|
  | Hard step | −70 dBFS | −75 dBFS | −82 dBFS | fails |
  | Linear ramp, 5 ms | −98 dBFS | −108 dBFS | −121 dBFS | passes (B_ex = 2 kHz) |
  | Linear ramp, 20 ms | −109 dBFS | −120 dBFS | −134 dBFS | passes (B_ex = 1 kHz) |
  | Linear ramp, 20 ms, updated every 64 samples | −77 dBFS | −80 dBFS | −87 dBFS | fails |
  | 15 ms linear crossfade, −12 → 0 dB | −108 dBFS | −119 dBFS | −133 dBFS | passes |

  Implication for module authors: linear ramps of ≥ 5 ms pass with margin, and block-rate parameter
  updates fail. A one-pole smoother with a 1 ms time constant (settling in ~5 ms) reached −81 dBFS at
  1–2 kHz, so fast exponential smoothers need a longer declared `smoothing_ms` or a different shape.
  `ModuleTestHost` gains test modules `TestHardStep` (must fail) and `TestStair64` (must fail) to prove
  the harness discriminates.

### 4.4 Offline render
- **Algorithm.** Build the chain from the `RackModel`. Activate it at the input rate with
  `max_block = 4096` in offline mode and set FTZ/DAZ. Stream the input, then L zeros. Drop the first L
  output samples.
- **Parameter events** in a render come from committed states, with real sample offsets when a job
  supplies automation (none in v1).

### 4.5 RT notes
- The coalescing search is bounded by the queue capacity.
- The non-finite check is one pass per slot per block.
- Both are allocation-free and run under `assert_no_alloc` in tests (ADR-002 §2).

## 5. Acceptance criteria
- **AC-1 [M1] (insert is live, seamless, untouched slots keep state).** Given realtime playback of
  seeded white noise at −20 dBFS through [TestDelay(480 samples)] on the fake backend:
  - When a Gain at 0 dB is inserted before it, then the output stays within 1e-6 of an uninterrupted
    run at every sample. A cold-restarted delay line would produce a 480-sample gap and fail this.
  - When a Gain at −6 dB is inserted instead, then from 65 ms after the command the RMS of every
    100 ms output window is −6.00 ± 0.01 dB relative to the same window of the uninterrupted run.
  - With the §4.3 signal, the transition passes §4.3 with T_s = 15 ms.
  - The transport never stops.
- **AC-2 [M1] (remove and reorder).**
  - Removing a −6 dB Gain brings every 100 ms RMS window to within ±0.01 dB of the uninterrupted
    no-Gain run from 65 ms after the command, and passes §4.3 (T_s = 15 ms).
  - Removing the last slot makes the output bit-identical to the input.
  - Reordering [Gain −6 dB, TestDelay 480] into [TestDelay 480, Gain −6 dB] passes §4.3. From 65 ms
    after the command, the output is within 1e-6 of the original order, and the total latency stays
    480.
- **AC-3 [M1] (per-slot bypass: click-free, no time jump).**
  - Given a Gain at −12 dB, toggling its bypass passes §4.3 (T_s = 15 ms) and the level change
    completes within 15 ± 1 ms.
  - Given a TestDelay(480) slot and white noise, toggling its bypass on and off keeps every output
    sample within 1e-6 of the input delayed by 480. The reported latency stays 480.
  - Offline renders of a rack with a bypassed slot equal renders without that slot, delayed by its
    latency, within 1e-6.
- **AC-4 [M1] (whole-rack A/B).** Given the rack [TestDelay 480, Gain −6 dB]:
  - Turning A/B on makes the output equal the input delayed by 480 within 1e-6, from 15 ms after the
    toggle. At no sample does the output deviate from the time-aligned crossfade of the two paths by
    more than 1e-6.
  - The transition passes §4.3.
  - An offline render made while A/B is on equals one made with it off, bit-identical.
  - The flag is off after reopening the document.
- **AC-5 [M1 Gain, every later built-in] (no zipper noise).** Every continuous automatable parameter
  of every built-in module passes §4.3 in both directions, and in the drag variant, in realtime and
  offline modes. The harness calibration holds: TestGain (5 ms linear) passes, and TestHardStep and
  TestStair64 fail.
- **AC-6 [M1] (event timing and smoothing).** Given a Gain step from 0 dB to −6 dB at block offset k:
  - output samples before k + latency are unchanged (bit-identical to the no-event run);
  - the output differs from sample k + latency onwards;
  - the applied gain equals the target, within 1e-6 relative, from k + latency +
    round(20 ms × rate) onwards.
  `ModuleTestHost` passes for Gain with no `allow_delayed_effect` opt-outs.
- **AC-7 [M1] (no lost or duplicated values).**
  - Given one block with pushes (100, g, −6), (100, g, −12), (100, h, 1), (100, g, −3), the module
    receives exactly [(100, g, −3), (100, h, 1)] in that order.
  - 10 000 pushes of (0, g, vᵢ) in one block never return `Full` and deliver one event with v₁₀₀₀₀.
  - 2 000 events at distinct offsets with capacity 512 all reach the module, in order, and the final
    `param_value` equals the last value.
- **AC-8 [M1 reporting, M4 UI] (latency reporting).**
  - Given [TestDelay 480, Gain, TestDelay 128 (bypassed), placeholder] at 48 kHz, the reported total
    is 608 samples. The UI reads "12.7 ms (608 smp)" and shows 480 and 128 on the respective slots.
  - When the first module requests a restart that changes its latency to 960, the total becomes 1088
    within 100 ms of the swap, as do the SPEC-002 monitoring readout and the heard-position offset.
- **AC-9 [M1] (realtime equals offline; deterministic).**
  - Given 10 s of pink noise followed by a 20 Hz–20 kHz sweep, the rack [Gain −6, TestDelay 480,
    Gain +3], static parameters, and also 20 parameter events at fixed absolute sample positions:
    realtime processing with random block sizes 1…1024 (including 0-length flushes), through the
    fake backend, and offline rendering differ by ≤ **1e-6** absolute at every sample after aligning
    by the reported latency.
  - Two offline renders are bit-identical (FNV-1a hash equal).
- **AC-10 [M1] (offline alignment and CLI).**
  - Given an impulse at sample 1000 of a 48 000-sample WAV and a rack [TestDelay 480], when rendered
    offline (engine job or `powervoice-cli render --rack`), then the output is 48 000 samples long with
    the impulse at exactly sample 1000.
  - Given a rack file naming an unknown module, the CLI exits with a non-zero status and lists the
    missing id.
  - The CLI's output for a Gain −6 dB rack on a −20 dBFS 997 Hz sine analyses (`powervoice-cli analyze`)
    to a peak of −26.00 ± 0.01 dBFS.
- **AC-11 [M1] (exact text round-trip for stepped parameters).** For each of the following synthetic
  stepped parameters, and every stepped parameter of every built-in, `text_to_value(value_to_text(v))`
  is bit-identical to `clamp_quantize(v)` for every legal value, or for 10 000 seeded legal values when
  there are more:
  - step 0.5, `decimals` 0: 2.5 displays as "2.5";
  - step 0.25, min −1;
  - step 1 Hz with values ≥ 1000: 1251 displays as "1.251 kHz";
  - step 0.1 ms with values ≥ 1000 ms.
  Continuous parameters round-trip within half a unit of the last displayed digit.
- **AC-12 [M4] (generic UI from schema).** Given a schema fixture with 40 parameters covering every
  flag, taper, unit, enum, group, nested group and enable parameter:
  - the rendered panel omits hidden and bypass parameters;
  - it renders read-only parameters as readouts, booleans as toggles, enums as dropdowns, stepped
    parameters as detented sliders and continuous ones as slider + field;
  - it follows declaration order, flattens nested groups as "Parent / Child", dims disabled groups and
    shows a filter box;
  - double-click sends the default;
  - invalid text sends nothing and restores the previous text;
  - every displayed value string equals the `text` of the latest (mocked) `param_changed` event.
- **AC-13 [M1] (missing module).** Given a rack description with an unknown id `com.acme.x@2.0.0`
  and state `{…}`:
  - the placeholder slot passes audio bit-exact, with latency 0;
  - the UI model carries the "Missing module com.acme.x@2.0.0" message;
  - re-serialising the rack yields the slot's JSON unchanged (JSON-equal).
- **AC-14 [M1] (non-finite guard).**
  - Given a test module that outputs NaN from sample 5000, in realtime every sample written to the
    fake device is finite. The slot is bypassed within 15 ms, marked failed, and a notice is posted.
    No later slot ever receives a non-finite sample.
  - The same rack rendered offline returns an error naming the slot.
- **AC-15 [M1] (module output events reach the UI).** Given a test module that reports a read-only
  parameter changing every 50 ms, the control-thread parameter mirror, and the resulting
  `param_changed` events, reflect each reported value within 100 ms. No report is lost while the RT
  output-event capacity is not exceeded.

## 6. Test plan

| AC | Unit (rack / module-api) | Integration (fake backend / CLI) | Manual smoke (owner) |
|---|---|---|---|
| AC-1 | chain swap keeps untouched instances warm | insert during fake playback, compare with an uninterrupted run | add Gain while playing a voice file |
| AC-2 | remove, reorder, empty rack | fake playback, level and §4.3 checks | drag slots while playing |
| AC-3 | bypass crossfade + delay line | §4.3 + time-alignment check | toggle bypass on a voice file |
| AC-4 | A/B dry delay | fake playback; offline render ignores A/B | A/B while playing |
| AC-5 | §4.3 harness in `module-api` test-util (+ calibration modules) | — | listen to slider drags |
| AC-6 | `ModuleTestHost` (offline, latency-aware) | — | — |
| AC-7 | `push_event` coalescing and overflow | — | — |
| AC-8 | latency sum incl. bypass/placeholder | restart swap updates readouts (T-401 for compensation) | check readouts (M4) |
| AC-9 | — | fake backend vs `rack::offline::render`, seeded | — |
| AC-10 | offline trim | `powervoice-cli gen impulse` → `render --rack` → `analyze` | — |
| AC-11 | exhaustive / seeded text round-trip | — | type values into fields (M4) |
| AC-12 | — | Vitest with a schema fixture and mocked IPC | inspect a built-in's panel (M4) |
| AC-13 | registry resolution → placeholder | sidecar round-trip (M3) | — |
| AC-14 | guard on a NaN test module | fake backend + offline job | — |
| AC-15 | RT drain ring | fake playback with a reporting module | watch a gain-reduction readout (M4) |

Test modules (in `module-api` test-util or `rack` tests):
- `TestGain` (exists);
- `TestDelay(n)` (latency n, exact delay);
- `TestHardStep` and `TestStair64` (calibration);
- `TestNaN`;
- `TestReporter` (READ_ONLY output events);
- `TestRestart` (requests a restart with a new latency).

Signals come from testkit (`sine`, `white`, `pink`, `log_sweep`, `impulse`); nothing is committed.

## 7. Out of scope
- The DSP and parameter sets of Noise Gate, Noise Reduction, EQ, Dynamics and the True-Peak Limiter
  (their own M4/M5 specs). Preset storage format (T-406).
- Bake/export pre-roll and tail policy (T-602). Parameter automation lanes (none in v1). Sidechain
  inputs.
- External plugins, scanning, the sandbox, installable modules (M8) and native plugin GUIs (M9),
  beyond the behavior they must share (§2.3, §2.4, §2.9).
- CPU budget and pausing bypassed modules (ADR-005 open question 2, T-704).

**Open questions**
1. ✅ Owner-confirmed at the M0 checkpoint: whole-rack A/B is a listening-only aid that offline renders ignore (§2.3). This spec decides that
   to prevent accidentally dry exports. The owner may prefer "export what you hear".
2. Should reordering require moved modules to keep their state as well? Currently they may reset
   behind the crossfade.
3. The 16-slot limit is a spec choice, pending the T-704 performance data.
