# SPEC-015 — Parametric EQ (module + EQ graph)

- **Status:** approved (autonomous, T-400)
- **Milestone:** M4 — T-402 (DSP module, `ResponseCurve`, tests), T-409 (EQ graph UI on the rack
  panel of T-405, analyzer overlay from T-208)
- **Related:** PROMPT §2 "EQ" (LOCKED), §3.4 module 4, §5 (EQ precision example) · SPEC-000
  (glossary, testkit conventions) · SPEC-004 OD-4 (rack edits are not undoable) · SPEC-007 §2.10,
  §4.8–§4.9, AC-20 (analyzer reuse) · SPEC-012 §2.4, §2.6, §4.3 (normative zipper measurement),
  AC-5, AC-9 · ADR-001 §4 · ADR-002 §2 (FTZ/DAZ) · ADR-003 (binary IPC) · ADR-005 §2, §3, §4, §9,
  §11, §13 · `docs/references.md` "Filters" (RBJ cookbook, Butterworth Qs) · code:
  `crates/module-api` (`ParamInfo`, `Taper`, `Unit`, `ParamGroup`, `ResponseCurve`, `CurveHandle`)

## 1. Purpose
Equalization is the second thing a voice-over user reaches for after noise cleanup:
- a **high-pass** removes rumble, handling noise and plosive thumps below the voice;
- a **low shelf** or a broad cut tames boominess and proximity effect;
- a **narrow cut** removes a harsh room resonance or a whistle;
- a **presence boost** (2–5 kHz) adds intelligibility;
- a **high shelf** adds air, and a **low-pass** removes hiss above the voice.

Audition's Parametric Equalizer does all of this with fixed band roles and a draggable graph drawn
over the live spectrum. PowerVoice's EQ copies that model (PROMPT §2, LOCKED): HPF + LPF with
variable slope, a low shelf, a high shelf, five peaking bands, and a graph that shows the **exact**
response the audio receives. The classic "boost and sweep" technique (boost a narrow band and drag it
until a resonance jumps out) must work while playing, without clicks.

## 2. Behavior / UX

### 2.1 Identity
- Module id **`org.powervoice.parametric-eq`**, version **1.0.0**, `state_format_version` 1,
  features `["equalizer", "mono"]`, name "Parametric EQ" (i18n `module.parametric_eq.name`).
  **Decided (autonomous, T-400):** ADR-005 §2 (accepted by the owner) fixes the built-in ids; the
  T-400 brief's `org.powervoice.eq` is not used (see §7, contradiction 1).
- Latency 0. Tail: finite worst-case bound (§4.8).
- In the rack, the slot body is the **EQ panel** (§2.6). The generic parameter UI (SPEC-012 §2.6)
  stays available as the fallback and shows the same schema.

### 2.2 Bands (fixed roles, in processing order)
| # | Band | Type | Controls | Default |
|---|---|---|---|---|
| 0 | **HP** | Butterworth high-pass, 6–48 dB/oct | on, frequency, slope | **off**, 80 Hz, 24 dB/oct |
| 1 | **L** | RBJ low shelf | on, frequency, gain, Q | on, 100 Hz, 0 dB, Q 0.707 |
| 2–6 | **1 … 5** | RBJ peaking | on, frequency, gain, Q | on, 200 / 500 / 1 200 / 3 000 / 6 000 Hz, 0 dB, Q 1.0 |
| 7 | **H** | RBJ high shelf | on, frequency, gain, Q | on, 10 kHz, 0 dB, Q 0.707 |
| 8 | **LP** | Butterworth low-pass, 6–48 dB/oct | on, frequency, slope | **off**, 12 kHz, 24 dB/oct |

Plus a **master gain** after the last band.

- **Decided (autonomous, T-400): band types are fixed per slot** (Audition-like). Fixed roles keep the
  graph readable, keep presets portable, and match the LOCKED band list exactly. Switchable types
  would be a later `format_version` bump.
- **Decided (autonomous, T-400): slopes 6, 12, 18, 24, 30, 36, 42, 48 dB/oct** (Butterworth order
  N = 1…8). Every order is the same code (one optional first-order section plus ⌊N/2⌋ biquads). The
  full 6-dB ladder is what Audition's HP/LP menu offers (⚠ unverified, helpx returns 403).
- **Decided (autonomous, T-400): shelves use Q** (RBJ "Q" form, 0.3–2.0, default 0.707 = S 1, the
  steepest shelf without overshoot), not S. One control name for every gain band keeps the wheel
  gesture uniform (§2.6.4).
- **Defaults are neutral.** Shelves and peaks start enabled at 0 dB, so dragging a node changes the
  sound at once. HP/LP start disabled at useful voice positions. A default EQ passes audio
  **bit-exact** (AC-2).

### 2.3 Parameter changes
- **Continuous parameters** (frequencies, gains, Qs, master gain) glide to a new value over **20 ms**,
  starting at the event's sample (SPEC-012 §2.4). Frequency and Q glide on a log scale, gains in dB,
  and the master gain linearly in linear gain (as the Gain module does).
- **Band on/off** crossfades the band in or out over **20 ms**. The band keeps running while it's
  off, so turning it back on is seamless.
- **Slope** changes crossfade from the old cascade to the new one over **20 ms**.
- **What SPEC-012 §4.3 covers.** Every parameter passes SPEC-012 §4.3 in the configurations listed in
  §4.5, including slider drags across the spectrum. One case is inherent and exempt: a single jump
  that carries a narrow or strong band across a steady tone (§4.5).
- **Undo.** EQ edits are rack edits: no undo entries (SPEC-004 OD-4). They persist in the session
  state within 2 s and in the sidecar.

### 2.4 Sample rates
- The EQ runs at the rack's rate: the device rate live, the document rate offline (ADR-005 §12).
- Any band frequency above **0.49·fs** is processed as 0.49·fs, and the graph shows that clamped
  response. At 44.1 kHz and above the whole 20 Hz–20 kHz range stays below the clamp. It only matters
  for low-rate documents rendered offline (8–32 kHz).

### 2.5 Edge cases
- **Extreme settings** (5 peaks stacked at +24 dB, Q 30) are allowed. Output may exceed 0 dBFS
  (float) and later modules or the device clip it. The EQ stays finite and bounded (AC-9).
- **Rack bypass and slot bypass** are host-owned (SPEC-012 §2.3). The EQ has no `BYPASS` parameter.
- **Non-finite input** can't reach the EQ, because the rack's guard sits upstream (SPEC-012 §2.9).
  Finite input always gives finite output.

### 2.6 EQ panel and graph (T-409)

#### 2.6.1 Layout
- **Compact panel (slot body).** It fills the slot's width and has four parts:
  1. **Header row:** nine band toggles `HP L 1 2 3 4 5 H LP` in band colors (filled = on, hollow =
     off), a **Spectrum** toggle, a **Range** toggle `±12 / ±24 dB`, and an **Expand** button.
  2. **Graph:** 160 CSS px high.
  3. **Selected band row:** the generic widgets (T-405 slider + field) for the selected band's
     frequency, gain and Q, or frequency and slope for HP/LP.
  4. **Master gain** slider.
- **Expanded view.** Expand opens an in-app floating panel over the editor: default 900 × 400 CSS
  px, resizable (minimum 480 × 240), moved by its header, closed with its close button or Esc. It
  shows the same content with a larger graph. Its position and size are kept in the view state. The
  compact panel stays live underneath.
  **Decided (autonomous, T-400):** an in-app panel rather than an OS window, because it avoids
  multi-window Tauri work and focus problems on Wayland (ADR-009).

#### 2.6.2 Axes
- **Frequency (x).** Log scale from 20 Hz to f_hi = min(20 kHz, rack rate / 2). The mapping comes
  from the shared `ui/src/lib/spectrum/freqAxis.ts` (SPEC-007 §2.10). Labels are
  `20 50 100 200 500 1k 2k 5k 10k 20k`, thinned so they never overlap, with grid lines.
- **Gain (y).** Linear dB, **±12 dB by default**, with grid lines every 3 dB and an emphasised 0 dB
  line. The **±24 dB** view has grid lines every 6 dB. The choice is an app setting.
  **Decided (autonomous, T-400):** ±12 by default, because voice EQ moves are mostly within ±6 dB
  and the finer scale makes them visible. ±24 covers the whole parameter range.
- **Spectrum scale.** The analyzer overlay has its own fixed scale, −90 … 0 dBFS over the graph's
  full height. The expanded view labels it on the right edge; the compact view doesn't label it.

#### 2.6.3 What is drawn, back to front
1. **Live spectrum** (Spectrum toggle on, the default): the SPEC-007 analyzer stream as a filled
   curve (`--analyzer-fill`, 50 % opacity), without peak hold.
2. **Grid and labels.**
3. **Component curves:**
   - the selected band's own response: a thin line in the band color, filled to 0 dB at 20 %
     opacity;
   - the hovered band's own response: a thin outline.
4. **Total response:** 2 device px, `--eq-curve`, filled to 0 dB at 15 % opacity.
5. **Nodes,** one per band:
   - a 12 CSS px circle in the band color, labelled `HP L 1…5 H LP`;
   - its x is the band frequency (clamped as in §2.4);
   - its y is the band gain for shelves and peaks, and the band's own response at its cutoff
     (≈ −3 dB) for HP/LP, so HP/LP nodes sit on their curve;
   - disabled bands have hollow nodes at 50 % opacity and contribute nothing to the total curve;
   - a node outside the visible range is pinned to the edge with a small caret pointing outward.

Every curve value comes from Rust through the module's `ResponseCurve` (§4.9). The UI never
evaluates a filter; it only maps (frequency, dB) pairs to pixels (AC-17).

Theme tokens: `--eq-curve`, `--eq-fill`, `--eq-grid`, `--eq-band-hp`, `--eq-band-ls`,
`--eq-band-1` … `--eq-band-5`, `--eq-band-hs`, `--eq-band-lp`.

#### 2.6.4 Pointer gestures
- **Hover.**
  - Hovering a node shows a tooltip `Band 3 · 1.20 kHz · +3.0 dB · Q 1.00`. The texts are Rust's,
    from `param_changed`.
  - Hovering empty graph area shows a readout of the pointer frequency, the total response there
    (linearly interpolated between the returned curve points) and the spectrum level.
- **Drag a node** (pointer capture) to move the band:
  - horizontal movement sets the frequency, through the inverse `freqAxis` mapping;
  - vertical movement sets the gain, through the gain axis, for shelves and peaks. HP/LP move
    horizontally only.
  - **Shift** = fine: pointer deltas ×0.1.
  - **Esc** during a drag restores and re-sends the values from before the drag.
  - The node follows the pointer at once and snaps to Rust's echoed value when `param_changed`
    arrives.
- **Wheel over a node:**
  - Q × 2^(±1/6) per notch (Shift: 2^(±1/24));
  - for HP/LP, the next or previous slope.

  The wheel over empty graph area scrolls the rack panel as usual.
- **Click** a node to select its band. Click empty area to deselect.
- **Alt+click** a node toggles the band on or off. The header toggles do the same.
- **Double-click** a node resets that band's frequency, gain and Q (or slope) to their defaults. Its
  on/off state is unchanged.
- **Right-click** a node opens a menu: On/Off, Reset band and, for HP/LP, Slope ▸ 6 … 48 dB/oct.
- **Rate.** Changes are sent at most **once per animation frame per parameter**, latest value wins
  (SPEC-012 §2.4).

#### 2.6.5 Keyboard and accessibility
Each node is a focusable element (tab order HP, L, 1–5, H, LP, then the band row), with
`aria-roledescription="EQ band"` and `aria-valuetext` built from Rust's texts. With a node focused:

| Key | Effect |
|---|---|
| ← / → | frequency × 2^(∓1/12) (Shift: 2^(∓1/48)) |
| ↑ / ↓ | gain ±0.5 dB (Shift ±0.1 dB); for HP/LP, slope one step steeper / shallower |
| PageUp / PageDown | Q × 2^(±1/6) |
| Enter | toggle on/off |
| Home | reset the band (as double-click) |

- A focus ring is visible.
- Value changes are announced through an `aria-live="polite"` region, throttled to one message per
  250 ms.
- Space keeps its global meaning (play/stop), so the EQ never steals the transport key. Keys the
  graph handles don't reach the global keymap (T-104 keymap registry).

#### 2.6.6 Commands and data flow
- **Sending values.** The graph sends **plain** values with the new command
  `set_param_plain(slot, id, value)`. The control thread clamp-quantizes the value, updates the
  mirror, sends the event and echoes `param_changed`, exactly like `set_param_normalized`
  (ADR-005 §13).
  **Decided (autonomous, T-400):** the pointer maps to Hz and dB through display axes, which is not
  parameter taper code, so the rule "the UI never runs taper code" still holds. The command is an
  additive ADR-005 §13 / ADR-003 amendment (§7).
- **Getting the curve.** It is fetched with
  `response_curve_get(slot, { seq, f_lo, f_hi, columns, components })`, which returns a binary
  **`VXRC`** frame (§4.10).
  - Rust evaluates the module's `ResponseCurve` on the control thread from the **mirror's target
    values** at the rack's rate.
  - The frequency list is one frequency per device-pixel column (log-spaced, §4.10), plus the exact
    frequencies of every enabled band (peak apex, shelf midpoint, HP/LP cutoff), so a Q 30 apex is
    never missed.
- **When the curve is requested:**
  - when the graph becomes visible;
  - on every `param_changed` for the slot (coalesced to one request per animation frame);
  - on a resize or rack-rate change.

  At most one request is in flight. Responses with an older `seq` are dropped.
- **Spectrum.** The graph subscribes with `analyzer_subscribe` (SPEC-007 §4.9) while it is visible
  and Spectrum is on, and unsubscribes otherwise. The analyzer tap is the **rack output**, the same
  as the analyzer panel.
  **Decided (autonomous, T-400):** post-rack, because no ADR amendment is needed, it shows what the
  user hears, and Audition's EQ display also shows the processed signal (⚠ unverified). The cost:
  modules after the EQ (a limiter) also shape the displayed spectrum.

## 3. Parameters
All parameters are `AUTOMATABLE`. Frequencies use `Taper::Log` with `Unit::Hz`. Gains use
`Taper::Db { neg_inf_at_min: false }` with `Unit::Db`. Qs use `Taper::Log` with `Unit::None`.
`*_on` are `BOOL | STEPPED` (step 1). Slopes are enums (`STEPPED`, step 1, labels
`6 dB/oct` … `48 dB/oct`). `params()` order = the table order. The ParamIds are permanent.

| id | key | name | unit | range | default | taper/step | smoothing | notes |
|---|---|---|---|---|---|---|---|---|
| 0 | `master_gain_db` | Master gain | dB | −24 … +24 | 0.0 | Db, 1 decimal | 20 ms | ungrouped (main section) |
| 10 | `hp_on` | High-pass on | bool | 0/1 | 0 | BOOL | 20 ms xfade | group `hp` enable |
| 11 | `hp_freq_hz` | Frequency | Hz | 20 … 20 000 | 80 | Log, 0 decimals | 20 ms | |
| 14 | `hp_slope` | Slope | enum | 6…48 dB/oct (0…7) | 3 (24 dB/oct) | enum | 20 ms xfade | Butterworth order = index + 1 |
| 20 | `ls_on` | Low shelf on | bool | 0/1 | 1 | BOOL | 20 ms xfade | group `low_shelf` enable |
| 21 | `ls_freq_hz` | Frequency | Hz | 20 … 20 000 | 100 | Log, 0 decimals | 20 ms | shelf midpoint (gain/2) |
| 22 | `ls_gain_db` | Gain | dB | −24 … +24 | 0.0 | Db, 1 decimal | 20 ms | |
| 23 | `ls_q` | Q | — | 0.3 … 2.0 | 0.7071 | Log, 2 decimals | 20 ms | |
| 30, 40, 50, 60, 70 | `bK_on` (K = 1…5) | Band K on | bool | 0/1 | 1 | BOOL | 20 ms xfade | groups `band_1` … `band_5` |
| 31, 41, 51, 61, 71 | `bK_freq_hz` | Frequency | Hz | 20 … 20 000 | 200, 500, 1 200, 3 000, 6 000 | Log, 0 decimals | 20 ms | peak centre |
| 32, 42, 52, 62, 72 | `bK_gain_db` | Gain | dB | −24 … +24 | 0.0 | Db, 1 decimal | 20 ms | |
| 33, 43, 53, 63, 73 | `bK_q` | Q | — | 0.1 … 30 | 1.0 | Log, 2 decimals | 20 ms | RBJ peaking Q |
| 80 | `hs_on` | High shelf on | bool | 0/1 | 1 | BOOL | 20 ms xfade | group `high_shelf` enable |
| 81 | `hs_freq_hz` | Frequency | Hz | 20 … 20 000 | 10 000 | Log, 0 decimals | 20 ms | |
| 82 | `hs_gain_db` | Gain | dB | −24 … +24 | 0.0 | Db, 1 decimal | 20 ms | |
| 83 | `hs_q` | Q | — | 0.3 … 2.0 | 0.7071 | Log, 2 decimals | 20 ms | |
| 90 | `lp_on` | Low-pass on | bool | 0/1 | 0 | BOOL | 20 ms xfade | group `lp` enable |
| 91 | `lp_freq_hz` | Frequency | Hz | 20 … 20 000 | 12 000 | Log, 0 decimals | 20 ms | |
| 94 | `lp_slope` | Slope | enum | 6…48 dB/oct | 3 (24 dB/oct) | enum | 20 ms xfade | |

**Groups** (`GroupId` 1…9, all `collapsed_by_default = true` in the generic UI): `hp`, `low_shelf`,
`band_1` … `band_5`, `high_shelf`, `lp`. Each group's `enable_param` is the group's `*_on`. The
unused id gaps (12, 13, 24–29, …) are reserved per band for future controls.

**ResponseCurve components** (`component_count` = 9): 0 HP, 1 L, 2–6 bands 1–5, 7 H, 8 LP.
`handles()` returns 9 `CurveHandle`s in that order:
- `freq` = the band's frequency;
- `gain` = its gain (None for HP/LP);
- `q` = its Q (None for HP/LP; the panel finds the slope through the group);
- `enable` = its `*_on`.

**Graph constants (T-409)**

| id | value | notes |
|---|---|---|
| `graph_height_compact` | 160 CSS px | |
| `graph_expanded_default` | 900 × 400 CSS px (min 480 × 240) | view state |
| `gain_ranges` | ±12 (default), ±24 dB | app setting |
| `analyzer_scale` | −90 … 0 dBFS | fixed |
| `node_diameter` / `hit_radius` | 12 / 10 CSS px | |
| `wheel_q_factor` | 2^(1/6) (Shift 2^(1/24)) | |
| `key_freq_step` | 1/12 octave (Shift 1/48) | |
| `key_gain_step` | 0.5 dB (Shift 0.1 dB) | |

## 4. Algorithm / implementation notes

### 4.1 Signal flow
`x → HP → L → 1 → 2 → 3 → 4 → 5 → H → LP → × master gain → y`
- Every band is a cascade of sections: a first-order section and/or biquads.
- The order is fixed. It is irrelevant for static filters but it fixes numerics and the behavior of
  time-varying filters, which is what makes renders deterministic.

### 4.2 Coefficients (all f64)
RBJ Audio EQ Cookbook (W3C note, `docs/references.md`), normalized by `a0`:
- A = 10^(gain_dB/40), ω0 = 2π·f/fs (f clamped to 0.49·fs), α = sin ω0 / (2Q).
- **Peaking:** b = [1 + αA, −2 cos ω0, 1 − αA], a = [1 + α/A, −2 cos ω0, 1 − α/A].
  |H(f0)| = gain exactly; a cut is the exact inverse of the equal boost.
- **Shelves:** the cookbook `lowShelf`/`highShelf` in the Q form (the 2·√A·α terms). f is the
  **midpoint** frequency: |H(f0)| = gain/2 exactly, and the far side is 0 dB.
- **HP/LP (Butterworth order N = slope/6).** Take ⌊N/2⌋ RBJ `HPF`/`LPF` biquads, all at the same f,
  with these Qs:
  - odd N: Q_k = 1/(2 cos(kπ/N)), k = 1…⌊N/2⌋;
  - even N: Q_k = 1/(2 cos((2k−1)π/(2N))), k = 1…N/2.
  - odd N also adds one **first-order** section, a bilinear transform with prewarp,
    K = tan(π f/fs):
    - LP: b = [K, K]/(1+K);
    - HP: b = [1, −1]/(1+K);
    - both: a1 = (K−1)/(K+1).

  | N (dB/oct) | first order | biquad Qs |
  |---|---|---|
  | 1 (6) | yes | — |
  | 2 (12) | — | 0.7071 |
  | 3 (18) | yes | 1.0000 |
  | 4 (24) | — | 0.5412, 1.3066 |
  | 5 (30) | yes | 0.6180, 1.6180 |
  | 6 (36) | — | 0.5176, 0.7071, 1.9319 |
  | 7 (42) | yes | 0.5550, 0.8019, 2.2470 |
  | 8 (48) | — | 0.5098, 0.6013, 0.9000, 2.5629 |

  The even rows match `docs/references.md`. Every section shares the same prewarp frequency, so the
  cascade is exactly the bilinear transform of the analog Butterworth prototype. |H(fc)| is −3.0103 dB
  for every N, and the asymptotic slope is 6.02·N dB/oct.
  (Spec simulation: −3.0103 dB at fc for N = 1…8; HP slope at fc/16…fc/8 = 5.970, 12.041, …, 48.166
  dB/oct, equal to the analog prototype within 0.002 dB.)

### 4.3 Structure and precision
- **Direct Form I in f64** (coefficients and state). Each block converts f32 → f64 on input and
  f64 → f32 on output.
- **Decided (autonomous, T-400): f64, justified by the spec simulation:**
  - **f32 coefficients** give **1.9 dB** response error for a 20 Hz, Q 30 peak at 96 kHz, and
    0.05–0.08 dB for 20 Hz high-passes. The ±0.1 dB AC would fail.
  - **f32 state** for a 997 Hz −20 dBFS tone through HP 48 dB/oct at 20 Hz + a 50 Hz peak + a 30 Hz
    shelf gives a residual error of **−67 dBFS** RMS at 96 kHz (DF1) and −72.5 dBFS (TDF2), versus
    the f64 reference. That is audible hiss next to a −60 dB ACX noise floor.
  - **Cost of f64 is negligible:** 15 static f64 DF1 sections cost **39 ns/sample (0.19 % of one
    core at 48 kHz)** on the owner's machine.
- **DF1 rather than TDF2:** DF1's state is past input and output samples, not partial sums weighted by
  old coefficients, so it behaves better while coefficients move every sample (§4.4).

### 4.4 Parameter smoothing (the module owns it, ADR-005 §4)
- **Ramps.** Each continuous parameter has a linear ramp of n = round(0.020·fs) samples in its
  natural domain:
  - log2(Hz) for frequencies;
  - dB for band gains;
  - ln(Q) for Qs;
  - linear gain for the master gain.

  An event at offset k sets the ramp target. The value at sample k already moves by one step, and the
  last ramp sample equals the target **exactly** (the TestGain convention). A new event retargets
  the ramp from its current value, and a zero-length block applies its events (flush).
- **Coefficients are recomputed every sample** while any ramp of a band is active, from the ramped
  values; otherwise they are static.
  - Measured cost of recomputing all 15 sections every sample: ≈ 0.24 µs/sample (≤ 1.2 % of one
    core at 48 kHz), and only while a ramp runs.
  - **Decided (autonomous, T-400):** per-sample recomputation is exact and block-independent, and
    the simulation shows it passes §4.5.
  - T-402 **may** instead recompute coefficients on a grid of ≤ 16 samples anchored to
    `steady_time`, interpolating coefficients linearly per sample in between (a linear path between
    two stable biquads stays stable, since the stability triangle is convex). The simulation showed
    the same §4.3 results within ±3 dB. This is allowed only if every AC still passes, and must be
    reported.
- **Why not coefficient crossfades:** crossfading two static filters would remove the audible sweep
  of a dragged band, which is the "boost and sweep" workflow in §1. A new filter would also start
  from zero state with its own transient.

### 4.5 §4.3 conformance (normative configurations) and the swept-resonance exemption
**Method.** The spec simulation used SPEC-012 §4.3 exactly: 997 Hz at −20 dBFS, event at 72 013,
0.25 ↔ 0.75 normalized, BH4 N = 8192 hop 1024, T_s = 20 ms, per-sample coefficients, 48 kHz. The
harness reproduced SPEC-012's calibration: the hard step fails, and the 5 ms ramp passes at
−108 dBFS.

Worst excess = max(L − max(R + 3, −90)); negative passes.

| Parameter | Configuration (other params) | Single jump, both directions | Drag variant |
|---|---|---|---|
| `master_gain_db` | bands neutral | −6.6 dB | −37.2 dB |
| peak `bK_gain_db` | f = 997 Hz, Q ∈ {0.1, 1, 30} | −9.9 / −19.3 / −43.9 | ≤ −47 |
| shelf `ls/hs_gain_db` | f = 1 kHz, Q 0.707 | −19.3 | −53.6 |
| peak `bK_q` | f = 1 414 Hz, gain ∈ {+12, −24, +24} | −16.8 / −26.2 / −3.0 | −48.6 (+12) |
| shelf `ls/hs_q` | f = 2 kHz, gain +12 | −18.4 | −47.9 |
| peak `bK_freq_hz` | gain ±12, Q 1 | −17.1 (+12) / −21.3 (−12) | −48.4 |
| peak `bK_freq_hz` (drag only) | gain ±12 with Q 0.1…30; gain ±24 with Q ≤ 4 | — | ≤ −38.8 |
| shelf `ls/hs_freq_hz` | gain ±12, Q 0.707 | −11.9 / −20.8 (+12), −32.8 / −23.9 (−12) | −42.1 / −50.8 |
| `hp/lp_freq_hz` | every slope 6…48 | worst −4.8 (LP 48) | ≤ −43.3 |
| `hp/lp_slope` (stepped) | 18 ↔ 36 at HP 500/900 Hz and LP 1.1/2 kHz; 6 ↔ 48 at LP 1.1 kHz | ≤ −10.1 | — |
| `*_on` (stepped) | HP 48 @ 500 Hz, LP 48 @ 1.1 kHz, peak +12 @ 1 kHz | ≤ −8.6 | — |

**Rate.** The table is normative at 48 kHz, the rate of the §4.3 signal. Spot checks of single
jumps at other rates:

| Rate | Peak +12 dB, Q 1 | LP 48 dB/oct |
|---|---|---|
| 44.1 kHz | −17.8 dB | −5.7 dB |
| 96 kHz | −12.2 dB | −0.1 dB (no margin) |

T-402 records the 96 kHz numbers but doesn't gate on them.

**Exempt (informative, recorded by T-402, never gated).** A **single jump** of a peak or shelf
frequency that carries the band **across the tone**, with Q > 1 or |gain| > 12 dB, fails §4.3. So does
a drag at |gain| > 12 dB with Q > 4. Simulation:

| Case | Worst excess |
|---|---|
| Q 2, +12 dB | +11.3 dB |
| Q 4, +12 dB | +25.1 dB |
| Q 1, +24 dB | +25.2 dB |
| Q 30, +24 dB | +45.3 dB |
| drag, Q 30, +24 dB | +4.1 dB |
| Q 4, +12 dB with T_s = 100 ms | still fails at Q 8 |

**Cause.** The ramp moves the band five octaves in 20 ms, so a narrow band passes over the tone in a
fraction of a millisecond. The tone gets a level bump far shorter than T_s, and its sidebands reach
past B_ex. This is the physics of a swept resonance (what an analog parametric EQ does too), not
coefficient zipper noise:
- jumps that don't cross the tone pass (2 → 8 kHz at +24 dB, Q 30: −1.0 dB);
- the output never exceeds the static maximum (AC-9).

**Decided (autonomous, T-400):** keep the audible sweep ("boost and sweep" is a core voice-editing
technique) and exempt only these tone-crossing configurations; see §7, contradiction 3.

### 4.6 On/off, neutral bands and slope changes
- **Sections always run.** Every section's state is updated on its own input even while the band is
  off. The band's contribution is y = x + w·(F(x) − x), with w ramping linearly 0 ↔ 1 over 20 ms on
  on/off events.
- **Identity rule (bit-exactness).**
  - When w = 0 and no fade is running, the band outputs its input **unchanged** (no arithmetic).
  - When w = 1 and no fade is running, it outputs F(x).
  - A shelf or peak whose current **and** target gains are exactly 0 dB, with no ramp active, also
    outputs its input unchanged (its biquad is b = a in exact arithmetic but not in floating point).

  So the default EQ and any disabled band are bit-exact (AC-2), and the running state makes
  re-enabling seamless.
- **Slope change** (HP/LP):
  - A second preallocated cascade (up to 4 sections) starts from **zero state** with the new order.
  - The output crossfades linearly from the old cascade to the new one over 20 ms. Both run during
    the fade, then the old one is dropped.
  - A slope event arriving during a running slope fade is applied when that fade ends (latest value
    wins).
  - Simulated margin ≥ 10 dB (§4.5): the startup transient of the new cascade is small where w is
    small.
- **Frequency or Q events during a slope fade** apply to both cascades.

### 4.7 Denormals, RT safety, determinism
- The host sets FTZ/DAZ (ADR-002 §2). In addition, at the end of every `process()` call each f64
  state value with |v| < 1e-30 is set to 0.
- f32 subnormal inputs are normal numbers in f64. So neither decaying tails nor subnormal inputs can
  slow processing (AC-12).
- **Allocation.** All state is allocated in `activate`, including two cascades per HP/LP. `process()`
  and `reset()` never allocate. `reset()` clears filter states and snaps every ramp and fade to its
  target.
- **Determinism.** Processing is per sample and independent of block partition. Realtime and offline
  output differ only through the block-end denormal flush (< 1e-30), which is far inside SPEC-012's
  1e-6.

### 4.8 Latency and tail
- `latency_samples()` = 0.
- **Tail.** `tail()` must stay constant while the instance is active (ADR-005), but the true ringing
  depends on parameters. So the module reports the **worst case over the parameter ranges** at the
  activated rate:
  `Tail::Samples(ceil(1.1 × 144 / d_min))`, where d_min = −20·log10(r_max) dB/sample and r_max is the
  largest pole radius reachable (the peaking section at 20 Hz, Q 30, +24 dB).
  - 144 dB = from +24 dBFS (a full-scale tone at a +24 dB resonance) down to −120 dBFS.
  - Simulation: d_min = 9.52e-5 dB/sample at 48 kHz, so 31.5 s; the measured decay of that
    configuration after a steady full-scale 20 Hz tone was 31.4 s. The bound is therefore ≈ 34.7 s
    at every rate.
  - Stacking several +24 dB bands at one frequency exceeds the assumption. That is outside the
    guarantee and is documented for T-602, which caps selection-bake tails.

### 4.9 `ResponseCurve` (ADR-005 §11)
- **Evaluation.** `magnitude_db(values, fs, freqs, out)` computes the **target** response: the sum
  over enabled bands of each section's |H| in dB, plus the master gain.
  - It uses the same `dsp` coefficient functions as `process()`, with the same 0.49·fs clamp.
  - It uses the cookbook's numerically robust form with φ = sin²(ω/2):
    |H|² = [(b0+b1+b2)² − 4(b0b1 + 4b0b2 + b1b2)φ + 16b0b2φ²] / [(1+a1+a2)² − 4(a1 + 4a2 + a1a2)φ + 16a2φ²].
  - Bands with target gain exactly 0 dB (shelves, peaks) contribute exactly 0.
  - Values are clamp-quantized first, as by the host.
- **Components.** `component_count` = 9. `component_magnitude_db(k, …)` returns band k's own response
  whether it is on or off, so the UI can dim disabled bands. The sum of the enabled components plus
  the master gain equals the total within 0.001 dB.
- **Properties.**
  - Pure and deterministic; no allocation; `Send + Sync`; callable concurrently with `process`.
  - Cost: ≤ 2 ms for 2 048 points with all components (AC-22).

### 4.10 `VXRC` — response-curve frame (new, additive to ADR-003 §2)
- **Request** `response_curve_get(slot, { seq, f_lo, f_hi, columns, components })`.
- **Frequencies.** Rust generates f_i = f_lo·(f_hi/f_lo)^((i+0.5)/columns) for i < columns (the
  device-pixel column centres of the log axis). It appends the exact frequency of every enabled band
  that lies in [f_lo, f_hi], then sorts the list. The UI maps each returned frequency through
  `freqAxis`, so both layers agree to the pixel (SPEC-007 AC-20).

| Off | Type | Field |
|---|---|---|
| 0 | `[u8;4]` | `"VXRC"` |
| 4 | u16 | version = 1 |
| 6 | u16 | header_len = 32 |
| 8 | u32 | seq (echo of the request) |
| 12 | u32 | flags: bit0 `HAS_COMPONENTS` |
| 16 | u32 | point_count M |
| 20 | u32 | component_count C (0 or 9) |
| 24 | u32 | sample_rate_hz (rate the curve was evaluated at) |
| 28 | u32 | reserved = 0 |
| 32 | f32[M] | frequencies, Hz, ascending |
| … | f32[M] | total response, dB |
| … | f32[C·M] | component responses, dB, component-major |

- `gen_ipc_fixtures` gains a golden `VXRC` frame (SPEC-000 AC-7).

### 4.11 Performance budget
Measured by `just bench` on the owner's machine, 48 kHz, 256-frame blocks, all nine bands on, HP/LP
at 48 dB/oct:
- ≤ **1.0 %** of one core static;
- ≤ **3.0 %** while every continuous parameter ramps continuously.

The simulation estimates 0.19 % and 1.4 %. The PROMPT §4 whole-rack budget is 20 %.

## 5. Acceptance criteria
- **AC-1 (schema and host obligations).** Given the module:
  - `descriptor()` is `org.powervoice.parametric-eq` 1.0.0, `state_format_version` 1, features
    `equalizer`, `mono`;
  - `params()` and `groups()` equal §3 exactly: ids, keys, order, ranges, defaults, tapers, steps,
    flags, `smoothing_ms` (20 for every non-stepped parameter);
  - `validate_schema` passes;
  - `ModuleTestHost` passes at 44.1/48/96 kHz. `allow_delayed_effect` is declared for every band
    parameter (`*_on`, `*_freq_hz`, `*_gain_db`, `*_q`, `*_slope`): at defaults they are neutral, and
    a filter's first-sample response to a coefficient ramp can be below the host's 1e-6 probe. It is
    **not** declared for `master_gain_db`, which changes the output exactly at k.
- **AC-2 (neutral is bit-exact).**
  - Given the default state, `ModuleTestHost`'s varied signals (silence, sines, noise, impulses, DC,
    ±4.0 squares), every block size and 44.1/48/96 kHz, the output is **bit-identical** to the input.
  - Any configuration with band k off gives output bit-identical to the same configuration with band
    k's frequency, gain and Q/slope changed arbitrarily.
  - A peak or shelf at exactly 0 dB (not ramping) is bit-exact identity.
- **AC-3 (response accuracy, PROMPT §5).**
  - **Configurations:** 200 seeded random static configurations (every band randomly on/off,
    frequency log-uniform, gain uniform ±24 dB, Q log-uniform over its range, random slopes), plus
    corner cases (20 Hz / 20 kHz, Q 0.1 and 30, ±24 dB, all slopes).
  - **Measurement:** at 44.1, 48 and 96 kHz, the DTFT of `process()`'s impulse response, captured
    until the remainder bound from the slowest pole is < 1e-9.
  - **Reference:** an independent test implementation of the cookbook formulas (complex
    evaluation, not the `dsp` code).
  - **Result:** the two agree within **±0.1 dB** at every 1/12-octave point f_k = 20·2^(k/12) Hz
    (f_k ≤ 20 kHz, f_k < fs/2) where the analytic response is ≥ −60 dB, and within ±1 dB where it is
    between −80 and −60 dB (the f32 output limit).
- **AC-4 (ResponseCurve is exact and consistent).** For the AC-3 configurations and points:
  - `magnitude_db` equals the independent analytic response within **0.001 dB**;
  - the enabled components plus the master gain sum to the total within 0.001 dB;
  - at fs = 22 050 Hz a band set to 20 kHz yields exactly the curve of a band at 0.49·fs;
  - `magnitude_db` performs no allocation (`no_alloc`) and is bit-identical across calls and
    threads;
  - `handles()` matches §3.
- **AC-5 (HP/LP Butterworth).** For every slope N = 1…8, at 44.1/48/96 kHz, measured on `process()`
  output:
  - |H(fc)| = **−3.01 ± 0.02 dB** for fc ∈ {30, 80, 1 000, 8 000} Hz;
  - the −3.01 dB crossing lies within **fc ± 1 %**;
  - HP at fc = 1 kHz (48 kHz): attenuation(62.5 Hz) − attenuation(125 Hz) = **6.02·N ± 0.1 dB**;
  - LP at fc = 100 Hz (48 kHz): attenuation(1 600 Hz) − attenuation(800 Hz) = **6.02·N ± 0.3 dB**
    (bilinear warping adds up to +0.19 dB at N = 8).
- **AC-6 (shelf and peak semantics).** At 48 kHz, for gains ±{3, 12, 24} dB and each band's Q range
  ends and default:
  - a peak reads **gain ± 0.01 dB** at f0;
  - peak(+g) followed by peak(−g) at the same f0 and Q is flat within ±0.001 dB at every AC-3 point;
  - a shelf reads **gain/2 ± 0.01 dB** at f0;
  - a low shelf reads gain ± 0.05 dB at 20 Hz when f0 ≥ 400 Hz (Q 0.707), and a high shelf reads
    gain ± 0.05 dB at 20 kHz when f0 ≤ 1 kHz;
  - the far side of each shelf is within 0.05 dB of 0 dB two decades away.
- **AC-7 (smoothing and event timing).**
  - Given any continuous parameter event at offset k, output samples before k are bit-identical to
    the no-event run.
  - The ramp's value at sample k + round(0.020·fs) − 1 equals the target exactly (unit test on the
    smoother and on the coefficient source).
  - A zero-length block applies its events (`ModuleTestHost` flush).
  - A master gain step 0 → −6 dB changes the output at exactly k and reaches −6 dB within 1e-6
    relative from k + round(0.020·fs).
- **AC-8 (no zipper noise, SPEC-012 AC-5).**
  - Every **normative** row of §4.5 passes SPEC-012 §4.3 in both directions and, where listed, in the
    drag variant. This holds in realtime mode (random blocks 1…1024, seeded) and in offline mode,
    with T_s = 20 ms.
  - Stepped rows (`*_slope`, `*_on`) use the same analysis with T_s = 20 ms.
  - The exempt configurations are measured and printed but not gated.
- **AC-9 (bounded under abuse).**
  - Given full-scale white noise and every combination of gain ∈ {−24, +24} dB, Q ∈ {0.1, 30},
    frequency jumps 20 Hz ↔ 20 kHz and slope 6 ↔ 48, all bands on: output samples stay finite.
  - Given 1 000 random events per second for 10 s over every parameter: output samples stay finite,
    and the output peak never exceeds the highest static total-response maximum of the configurations
    visited, plus **6 dB**, relative to the input peak. The test computes those maxima from
    `ResponseCurve` on a 1/48-octave grid.
- **AC-10 (realtime equals offline; deterministic).**
  - Given 10 s of pink noise plus a 20 Hz–20 kHz sweep, a non-trivial configuration and 50 events at
    fixed absolute positions, realtime processing (random blocks 1…1024 including 0-length flushes)
    and offline processing (4096-frame blocks) differ by **≤ 1e-6** at every sample.
  - Two offline renders are bit-identical (FNV-1a equal).
- **AC-11 (denormal safety).** With FTZ/DAZ **disabled** in the test thread, given 1 s of full-scale
  noise, then 60 s of digital silence, then 60 s of a constant 1e-15 (−300 dBFS) input, then 10 s of
  f32 subnormal values (1e-40), with all bands on (HP/LP 48 dB/oct at 20 Hz / 20 kHz, peaks at
  +12 dB, Q 30):
  - the median `process()` time per 1024-frame block in the last 10 s of each section is ≤ 1.5 × the
    median over the noise section;
  - no f64 state value is subnormal at any block end (test hook).
- **AC-12 (performance).** The §4.11 budgets hold in `just bench` (owner machine): ≤ 1.0 % static and
  ≤ 3.0 % with continuous ramps. `process()` never allocates (`ModuleTestHost`).
- **AC-13 (sample-rate handling).**
  - At fs = 22 050 Hz, a band set to 20 kHz gives output bit-identical to the same band set to
    0.49·fs.
  - At fs = 8 000 Hz, every band at 20 kHz and every slope keep the output finite and the
    `ResponseCurve` NaN-free.
- **AC-14 (latency and tail).** `latency_samples()` = 0 at every rate. `tail()` equals the §4.8
  formula (≈ 34.7 s × fs). Given the worst configuration (peak 20 Hz, Q 30, +24 dB) fed a steady
  full-scale 20 Hz sine for 40 s, the output falls below −120 dBFS within `tail()` samples after the
  input stops.
- **AC-15 (state).**
  - The default instance's `save_state()` serializes to a checked-in golden JSON (sorted keys, 35
    params, no blob).
  - A state with unknown keys loads, ignoring them; missing keys take their defaults; values are
    clamp-quantized (`prepare_state`).
  - A save → load round-trip gives bit-identical output (`ModuleTestHost`).
- **AC-16 (curve IPC contract).**
  - A golden `VXRC` frame decodes in Vitest with every header field and float bit-identical.
  - `response_curve_get` returns frequencies ascending, including every enabled band's exact
    frequency. It evaluates at the rack's rate from the mirror's target values: after
    `set_param_plain`, the next `VXRC` reflects the new value with no audio processed in between.
- **AC-17 (graph draws Rust's curve only)** (Vitest, mocked IPC):
  - Given a `VXRC` payload with an arbitrary synthetic shape (a sawtooth in dB that no filter
    produces), the drawn total polyline passes through (freqAxis.x(f_i), gainAxis.y(dB_i)) for every
    point within 0.5 device px.
  - Nodes sit at their parameter values; out-of-range nodes are pinned with a caret.
  - The UI bundle for the graph imports no filter or taper math (a lint test greps `ui/src/lib/eq/`
    for `Math.tan`, `sin`, `cos` and `pow` outside `freqAxis.ts`).
- **AC-18 (pointer gestures)** (Vitest, mocked IPC):
  - A node drag emits `set_param_plain` for frequency and gain at ≤ 1 call per animation frame per
    parameter, with values equal to the inverse axis mapping of the pointer (Shift: deltas ×0.1).
    HP/LP drags never emit a gain.
  - The wheel changes Q by 2^(±1/6) per notch, or steps the slope for HP/LP.
  - Alt+click and the header toggles flip `*_on`; double-click emits the band's defaults; Esc during
    a drag re-emits the pre-drag values.
  - Displayed texts equal the latest mocked `param_changed` texts.
- **AC-19 (keyboard and accessibility)** (Vitest + manual):
  - Tab reaches the nine nodes in order HP, L, 1–5, H, LP.
  - The §2.6.5 keys emit the specified steps; Space is not consumed.
  - Every node exposes `aria-valuetext`, and the live region announces changes at most every
    250 ms.
  - The panel is fully operable without a pointer.
- **AC-20 (responsiveness)** (Vitest with a mocked clock):
  - after a `param_changed`, a curve request is sent in the same animation frame, and the new curve
    is drawn in the frame its response arrives (≤ 2 frames when the mock replies within one);
  - at most one request is in flight during a drag, and responses with an older `seq` are dropped;
  - on the owner's machine, the time from `set_param_plain` to the curve on screen is ≤ 50 ms at
    p95 (manual/bench).
- **AC-21 (spectrum overlay, SPEC-007 AC-20).**
  - The graph subscribes through `analyzer_subscribe` only while visible with Spectrum on, and
    unsubscribes when hidden, closed or toggled off. With the analyzer panel also hidden,
    `ANALYZER_ON` clears.
  - Analyzer band k and a curve point at f_k are drawn at the same x within 0.5 device px over the
    full width.
  - The spectrum uses the −90 … 0 dBFS scale, independent of the gain range.
- **AC-22 (curve cost).** `ResponseCurve` evaluation of 2 048 points with 9 components, all bands on
  at 48 dB/oct, takes ≤ 2 ms on the owner's machine (unit timing, median of 100).
- **AC-23 (manual smoke, owner).** On a real voice take during playback:
  - HP 80 Hz / 24 dB/oct audibly removes rumble;
  - a Q 4, +12 dB band dragged across 200 Hz–5 kHz sweeps audibly without clicks or zipper;
  - toggling bands and slopes is click-free;
  - the curve matches the node positions;
  - with the spectrum on, a 1 kHz test tone's peak sits under a 1 kHz node;
  - a flat EQ, bypassed versus active, nulls completely (`powervoice-cli analyze` null test → −inf).

## 6. Test plan
| AC | Unit (`dsp` / `modules`) | Integration | Vitest | Manual |
|---|---|---|---|---|
| AC-1 | schema equality; `ModuleTestHost` (+ `allow_delayed_effect` list) | — | — | — |
| AC-2 | bit-exact identity over host signals; disabled-band independence | — | — | flat EQ null |
| AC-3 | impulse-response DTFT vs independent cookbook, 200 seeded configs × 3 rates | — | — | — |
| AC-4 | ResponseCurve vs independent analytic; components sum; `no_alloc` | — | — | — |
| AC-5 | Butterworth −3 dB, crossing search, slopes | — | — | — |
| AC-6 | peak/shelf semantics, boost/cut reciprocity | — | — | — |
| AC-7 | smoother exactness, first-change timing, flush | — | — | — |
| AC-8 | SPEC-012 §4.3 harness (`module-api` test-util) over the §4.5 table, realtime + offline | — | — | slider drags while playing |
| AC-9 | abuse matrix + random event storm | — | — | — |
| AC-10 | — | `rack` realtime (random blocks) vs `rack::offline::render` | — | — |
| AC-11 | FTZ-off timing test + state inspection hook | — | — | — |
| AC-12 | — | `just bench` (T-110 harness) | — | `top` while dragging |
| AC-13 | 22.05 / 8 kHz clamp tests | — | — | — |
| AC-14 | tail formula; 40 s steady + decay measurement (`#[ignore]` long test in `just test-big`) | — | — | — |
| AC-15 | golden state JSON; `prepare_state` cases | sidecar round-trip (T-306 fixture) | — | — |
| AC-16 | `VXRC` builder; mirror-target evaluation | `gen_ipc_fixtures` golden | golden decode | — |
| AC-17 | — | — | synthetic-curve drawing; import lint | — |
| AC-18 | — | — | gestures with mocked IPC | drag / wheel / Alt / double-click |
| AC-19 | — | — | keyboard map, aria | keyboard-only session |
| AC-20 | — | — | request coalescing, stale drop | perceived latency |
| AC-21 | — | analyzer lifecycle (T-208 counters) | x agreement | voice under the graph |
| AC-22 | curve timing | — | — | — |
| AC-23 | — | — | — | owner smoke list |

**Vertical-slice subset (lean first implementation).** A lean slice implements the module and the
graph core:
- ACs: AC-1, AC-2, AC-3 (48 kHz only), AC-4, AC-5, AC-7, AC-8 (normative rows at 48 kHz, offline
  mode), AC-10, AC-16, AC-17, and AC-18 (drag and wheel only).
- The rest are the hardening target: AC-9, AC-11 … AC-14, AC-19 … AC-22, the expanded view, the
  context menu, and the keyboard map.
- The spec's design choices (f64 DF1, per-sample ramps, bit-exact neutral bands, the `VXRC` frame)
  apply from the first slice, so hardening adds tests, not rewrites.

**Fixtures.** testkit signals generated in code (sines, white/pink noise, log sweeps, impulses,
bursts). There are two golden files: the default-state JSON and the `VXRC` frame. No audio is
committed.

## 7. Out of scope, contradictions, notes

**Out of scope:** switchable band types, extra bands, band solo/listen, dynamic EQ, linear-phase or
mid/side modes, match EQ, auto-gain, analyzer freeze/reference curves, a pre-EQ or per-slot
analyzer tap (would need an ADR-002/ADR-003 amendment), factory and user presets (T-406), undo of EQ
edits (SPEC-004 OD-4), native plugin-style windows.

**Contradictions found (resolved by the defaults above; for the orchestrator):**
1. **Module id.** The T-400 brief says `org.powervoice.eq@1.0.0`, but ADR-005 §2 (accepted) fixes
   `org.powervoice.parametric-eq`. This spec follows the ADR.
2. **ADR-005 §13 / ADR-003** list only `set_param_normalized` and `set_param_text`. The graph needs
   the additive `set_param_plain` (plain value) and the new `VXRC` frame and `response_curve_get`
   command, so ADR-003 needs an amendment and SPEC-012 §2.6 a one-line addition.
3. **SPEC-012 AC-5** says every continuous parameter passes §4.3. Tone-crossing single frequency
   jumps of narrow or strong bands cannot (physics of a swept resonance, §4.5), so they are exempted
   with measured numbers. The orchestrator may prefer to record this as a SPEC-012 note.
4. **ADR-005 `tail()` is constant while active,** so the EQ reports a ≈ 35 s worst case. T-602's
   selection-bake tail policy must cap it (render cost is small, but tails should not be pasted
   blindly).
5. **SPEC-007 §4.9 `VXSA` layout** (header_len 48, f0/bands-per-octave fields) differs from
   **ADR-003 Amendment 1 `VXSA`** (header_len 32, peak-hold flag). This is pre-existing and not
   caused by this spec, but T-208 must reconcile it before T-409 consumes the stream.

**Notes for T-402/T-409:**
- Keep every coefficient formula in `dsp` (one function per section type), called by both
  `process()` and `ResponseCurve`.
- Tests use an independent implementation.
- `i18n` keys: `module.parametric_eq.*`, `eq.band.{hp,ls,1..5,hs,lp}`, `eq.graph.*`.
