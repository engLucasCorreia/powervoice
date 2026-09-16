# SPEC-016 — Dynamics module (AutoGate, Expander, Compressor, Limiter) and its panel

- **Status:** approved (autonomous, T-400)
- **Milestone:** M4. Each AC carries its ticket:
  - **T-403** (Dynamics A): the shared dynamics engine in `dsp` (§4.2–§4.4, §4.6, §4.8–§4.10,
    §4.13), the compressor and limiter sections, the module shell with **every** parameter, Telemetry,
    and the `TransferCurve` extension in `module-api` (§4.11).
  - **T-407** (Dynamics B): the gate core with hysteresis and hold (§4.7), the AutoGate and Expander
    sections.
  - **T-410** (Dynamics UI): the Dynamics panel, transfer graph and gain-reduction meters, the
    `VXTC`/`VXMT` frames and commands (§4.12), and generic rendering of any module's `TransferCurve`
    and Telemetry (the Noise Gate's UI, SPEC-013 §2.7).
- **Related:** SPEC-013 (Noise Gate; reuses §4 of this spec, which is normative for it) · SPEC-012
  (rack; **§4.3 zipper measurement is normative**; §2.4 smoothing; §2.5 latency) · SPEC-015
  (parametric EQ; shares the plain-value parameter command, §4.12) · SPEC-017 (true-peak limiter;
  the delivery ceiling) · SPEC-004 OD-4 (rack edits are not undoable) · SPEC-000 (glossary, testkit
  conventions) · ADR-005 (§3 params, §4 events, §8 latency, §11 extensions; **§11 addition proposed
  in §4.11**) · ADR-003 (binary IPC; **two frames proposed in §4.12**) · ADR-002 §2 (RT contract,
  FTZ/DAZ) · ADR-001 §4/§5 · PROMPT §3.4 item 5, §5 · `docs/references.md` (Audition Dynamics)
- **Implementation status (H-51, doc/code sweep):** the module shipped early as a vertical slice
  (S3-02, `crates/modules/src/dynamics.rs`), ahead of and narrower than the T-403/T-407/T-410 plan
  above. **Live:** identity, the full permanent parameter schema, Compressor + makeup, Limiter, the
  shared detector/ballistics/ramps (§4.2–§4.6, §4.8–§4.10), Telemetry for the sections below.
  **Deferred (not T-403/T-407):**
  - **AutoGate and Expander** — their parameters (`autogate_*`, `expander_*`) exist in the schema
    but are flagged `HIDDEN` and have no effect on the signal (module doc comment: "Not yet
    available in this slice"); their telemetry channels (`TELEMETRY_GR_AUTOGATE`,
    `TELEMETRY_GR_EXPANDER`, `TELEMETRY_AUTOGATE_OPEN`) always read 0.
  - **Look-ahead** (`lookahead_ms`) — likewise `HIDDEN`, no effect; `latency_samples()` always
    returns 0, so §4.9's latency table and AC-10 do not hold yet.
  - **The `TransferCurve` extension** (§4.11) **does not exist** in `module-api`: `ExtensionId`
    and `Extension` (`crates/module-api/src/extension.rs`) have no such variant. Everything that
    depends on it — the transfer graph (§2.6), AC-14, and SPEC-013's UI — is therefore also not
    implemented.

  Every acceptance criterion, panel section and wire format below that covers AutoGate, Expander,
  look-ahead or `TransferCurve` describes the T-403/T-407/T-410 target, not current behaviour.

## 1. Purpose
A voice-over take swings between loud and soft words, has breaths and room tone between phrases,
and its occasional peaks must stay off the ceiling. Audition users solve this with one effect,
**Dynamics**, which has four switchable sections, an input/output transfer graph and a
gain-reduction meter. This spec defines:
- the PowerVoice Dynamics module;
- the **shared dynamics engine** (detector, gate core, gain computers, ballistics, parameter ramps,
  look-ahead), which the Noise Gate (SPEC-013) also uses. T-403, T-407 and T-408 share this code;
- the panel the user works in.

## 2. Behavior / UX

### 2.1 Identity
- **id** `org.powervoice.dynamics`, **version** 1.0.0. This is the id ADR-005 §2 lists.
- **Name** "Dynamics" (`module.dynamics.name`); vendor "PowerVoice".
- **Features** `audio-effect`, `compressor`, `expander`, `gate`, `limiter`, `mono`.
- **State** `state_format_version` 1, parameters only, no blob. **Layout** MONO.
- **Latency** = the look-ahead in samples (§4.9). **Tail** = `Tail::Samples(latency)`. The output
  after silent input is only the delayed input; gains never create signal from silence.

### 2.2 Signal flow and section order
- **Order: AutoGate → Expander → Compressor (+ makeup) → Limiter.**
  **Decided (autonomous, T-400).** The low-level sections come first, so their thresholds are read
  against the voice as recorded. Makeup lifts the level after compression, and the limiter comes last
  so it is the ceiling over everything, makeup included.
  - Audition's panel lists AutoGate, Compressor, Expander, Limiter. Its processing order is not
    documented in any source we could reach (§8).
  - With usual settings (gate and expander thresholds below the compressor threshold) the two orders
    give the same curve. The one difference: Audition's listed order would apply makeup before the
    expander.
  - The panel shows the sections in **processing** order, so what the user reads is what happens.
- **One detector, composed levels.** One level detector runs on the module input. Each section
  computes its gain from the level at **its own input**: the detector level plus the gains of the
  sections before it (§4.5).
  - For steady signals this is exactly the four sections in series.
  - It needs one look-ahead delay line, not four.
  - It makes the transfer graph exact (§4.11).
  - Per-section detectors on the intermediate signals would differ only while the upstream gains are
    moving.

### 2.3 Sections
Each section has a header **enable** toggle.
- **Off means inert.** A disabled section contributes exactly 0 dB, bit-identically, whatever its
  parameter values (AC-10).
- **Toggling** crossfades the section's contribution in or out over **20 ms** (§4.8). There are no
  clicks, even with a 0.1 ms attack.
- **Disabled sections keep running** (detector and ballistics stay warm), so turning one on is
  seamless.

**AutoGate.** Mutes everything below its threshold (pauses, breaths, room tone).
- The gate opens when the level reaches the threshold. It closes after **hold** once the level has
  fallen **3 dB** below the threshold (fixed hysteresis, §4.7).
- **Attack** is the duration of a raised-cosine fade-in. **Release** is the time constant of an
  exponential fade-out. Hold runs before release (as in Audition).
- **Closed = −∞** (full mute). **Decided (autonomous, T-400):** this matches Audition's AutoGate,
  which has no range control. For partial attenuation, a sidechain filter, adjustable hysteresis or
  pre-open timing, use the **Noise Gate** module (SPEC-013).
- AutoGate detection is always **peak** (§2.4).

**Expander.** Downward expansion below its threshold. Each dB the input falls below the threshold
lowers the output by **ratio** dB (2:1 → 2 dB).
- **Decided (autonomous, T-400):** the Expander has its **own attack and release**. Audition shows
  only threshold and ratio and does not document which time constants the expander uses.
  Borrowing another section's timing would make one section's sliders silently change another
  section's sound.
- The global **knee** applies.

**Compressor.** Threshold, ratio, attack, release, **makeup**.
- The global knee applies.
- Makeup belongs to this section and is inert while the section is off.
- **Makeup is manual only. Decided (autonomous, T-400):** no auto-makeup. Auto-makeup is a heuristic
  that fights the loudness normalization at the end of a voice-over workflow (PROMPT §3.3/§3.5), and
  Audition's Dynamics has only a manual makeup control.

**Limiter.** Ratio ∞ above its threshold, with attack and release.
- Always **peak** detection; it limits the **sample** peak.
- The ceiling is **guaranteed** (within 0.05 dB) only with look-ahead ≥ 7 × attack (AC-7, §4.6).
- Without look-ahead the threshold is reached in steady state, but fast transients overshoot by an
  amount set by the attack.
- Inter-sample (true) peaks are not controlled. Delivery ceilings belong to the **True-Peak Limiter**
  (SPEC-017), placed last in the rack.

### 2.4 Global settings
- **Detection: Peak / RMS** (default RMS). It applies to the **Expander and Compressor** only.
  - AutoGate and Limiter always use peak detection. **Decided (autonomous, T-400):** a gate must open
    on consonant onsets and a limiter must see peaks. RMS detection would open late and let peaks
    through.
  - RMS is the default because it tracks perceived loudness and reads naturally on the graph (Adobe's
    own guidance for Dynamics Processing, §8).
  - Levels are raw: a sine's RMS reads 3.01 dB below its peak (§4.3).
- **Knee** (dB width, 0 = hard). **Decided (autonomous, T-400):** Audition offers a soft-knee toggle.
  A width parameter covers both states (0 dB = hard), and the default 6 dB soft knee suits speech.
- **Look-ahead** (0–20 ms, default 0).
  - It delays the audio so every section reacts before the sound arrives: the AutoGate pre-opens
    on consonants and the limiter catches peaks.
  - It adds its value as latency, which the panel shows ("adds 5.0 ms latency").
  - Changing it restarts the module behind the rack's 15 ms crossfade (§4.9; SPEC-012 §2.4, §2.5).
  - Default 0, so inserting Dynamics adds no latency to monitoring.

### 2.5 State on insert (defaults)
Only the **Compressor** is on: −20 dBFS, 3:1, 10 ms / 100 ms, 0 dB makeup, RMS, 6 dB knee, no
look-ahead. AutoGate, Expander and Limiter are off. **Decided (autonomous, T-400):** a freshly
inserted Dynamics does something gentle and useful on speech, and no latency or muting surprises the
user. Audition's own defaults are unverified (§8).

### 2.6 Dynamics panel (T-410)
It is a custom panel (ADR-005 §13) built only on the schema, Telemetry and `TransferCurve`. Top to
bottom:
1. **Global row:** Detection dropdown, Knee slider, Look-ahead slider. When the look-ahead is > 0,
   a latency readout "adds 5.0 ms latency" appears next to it, from the slot's reported latency.
2. **Transfer graph** (square, 200–320 px, follows the panel width).
   - **Axes:** input **peak** level of a steady sine, −80 … +6 dBFS (x), against output peak level,
     same range (y).
   - **Grid** every 6 dB, labels every 12 dB, and a dashed 1:1 diagonal.
   - **Curve:** the total static curve from `TransferCurve` (§4.11). The rising branch is drawn solid.
     Where the falling branch differs (AutoGate hysteresis), it is drawn dashed.
   - **Threshold handles:** one small triangular handle on the x axis per **enabled** section, at
     `threshold + offset` (the offset is +3.01 dB for RMS-mode sections, §4.11). The label shows the
     parameter's own text (e.g. "−20.0 dB").
     - Dragging horizontally sets that threshold through `set_param_plain` (§4.12). Shift gives fine
       ×0.1 movement.
     - Double-click resets to the default.
     - The rest of the curve does not accept drags; ratio, knee and makeup are set with their
       controls.
   - **Operating point:** a dot at x = the `input_level_dbfs` telemetry and
     y = x + `gr_total_db` + effective makeup, updated every telemetry frame. It is hidden when the
     input level is ≤ −80 dBFS or no telemetry frame has arrived for 250 ms.
3. **Section panels**, in processing order: AutoGate, Expander, Compressor, Limiter.
   - **Header:** enable toggle, name, **gain-reduction meter** (the section's GR channel), collapse
     arrow. The AutoGate header also has an "open" lamp.
   - **Body:** the generic widgets (T-405) for the section's parameters.
   - **A disabled section** is dimmed but editable (ADR-005 §13). Its meter reads 0.
   - Off sections start collapsed (`collapsed_by_default`), and the user's collapse state is kept in
     the view state.
4. **Slot header** (SPEC-012 §2.1): the **total** GR meter (`gr_total_db`).

**Gain-reduction meters:**
- **Shape:** a horizontal bar growing leftwards from 0 dB. The scale runs 0 … −30 dB, with ticks at 0,
  −3, −6, −10, −20 and −30.
- **Readout:** one decimal ("−6.2 dB"). Values below −30 pin the bar but the readout keeps the value.
  The channel floor is −60, shown as "≤ −60 dB".
- **Ballistics:** each telemetry frame displays the received value directly. The module's
  `Hold::Min` already delivers the deepest reduction since the last frame, and the module's own
  attack/release are the ballistics. The UI adds none.
- **Stale:** with no frame for 250 ms (transport idle), meters fall to 0 and lamps go off.

**Curve refresh:**
- After a `param_changed` for this slot, the UI requests the curve again. Requests are coalesced to
  at most one per animation frame, and responses older than the newest request are dropped (by
  `seq`).
- The curve is therefore at most one animation frame plus one IPC round trip behind the controls.

**Generic fallback** (any module, built-ins and later plugins adapted to these extensions):
- a module exposing `TransferCurve` gets the same graph above its generic parameters;
- its Telemetry channels get the same widgets: `GainReduction` → GR meter, `Level` → level bar
  −60 … +6 dBFS, `Indicator` → lamp, `Value` → numeric readout;
- widgets sit in their group header, or in the slot header when the channel has no group (ADR-005
  §13).

This is the Noise Gate's whole UI (SPEC-013 §2.7).

**Strings** use i18n keys `module.dynamics.*` (names in the schema) and `ui.dynamics.*` (panel-only
text such as "adds {ms} ms latency").

### 2.7 Edge cases
- **Undo:** parameter edits and enables are rack edits, which are not undoable (SPEC-004 OD-4).
- **Seek, loop wrap, transport start:** the host calls `reset()` (§4.13). Every section restarts
  from 0 dB, and the AutoGate starts **closed**, so the first sound fades in over the attack.
- **Float input above 0 dBFS** is handled; levels are not clamped above. Levels below −150 dBFS,
  including digital silence, read −150 (§4.3).
- **Sample rates** 44.1–192 kHz: every time in ms is converted at the active rate (§4.2).
- **Latency changes:** the rack total and heard position update within 100 ms after a look-ahead
  change (SPEC-012 §2.5, AC-8).

## 3. Parameters
All parameters are `AUTOMATABLE`. "Smooth" is the declared `smoothing_ms`. "Delayed" means the
parameter is declared with `ModuleTestHost::allow_delayed_effect` (MEMORY T-005 note), because its
effect depends on the signal or state, or needs a restart.

**Global (ungrouped, shown first):**

| id | key | name | unit | range | default | taper / step | smooth | notes |
|---|---|---|---|---|---|---|---|---|
| 1 | `detection` | Detection | enum | Peak, RMS | RMS | enum | 20 | Expander + Compressor only; level crossfade (§4.3). Delayed |
| 2 | `knee_db` | Knee | dB | 0 … 20 | 6.0 | Db, 1 decimal | 20 | 0 = hard; Expander + Compressor. Delayed |
| 3 | `lookahead_ms` | Look-ahead | ms | 0 … 20 | 0 | Linear, step 1 | — | latency; change → Restart (§4.9). Delayed |

**Group 1 `autogate` "AutoGate"** (enable = 10; collapsed by default):

| id | key | name | unit | range | default | taper / step | smooth | notes |
|---|---|---|---|---|---|---|---|---|
| 10 | `autogate_enabled` | AutoGate | bool | off/on | off | BOOL | 20 | contribution crossfade. Delayed |
| 11 | `autogate_threshold_db` | Threshold | dBFS | −80 … 0 | −50.0 | Db, 1 decimal | 0 | opens at ≥ T, closes below T − 3 dB. Delayed |
| 12 | `autogate_attack_ms` | Attack | ms | 0.1 … 100 | 2.0 | Log, 1 decimal | 0 | raised-cosine duration. Delayed |
| 13 | `autogate_hold_ms` | Hold | ms | 0.1 … 1000 | 50.0 | Log, 1 decimal | 0 | range = Audition's (§8). Delayed |
| 14 | `autogate_release_ms` | Release | ms | 1 … 2000 | 100.0 | Log, 1 decimal | 0 | time constant τ. Delayed |

**Group 2 `expander` "Expander"** (enable = 20; collapsed by default):

| id | key | name | unit | range | default | taper / step | smooth | notes |
|---|---|---|---|---|---|---|---|---|
| 20 | `expander_enabled` | Expander | bool | off/on | off | BOOL | 20 | Delayed |
| 21 | `expander_threshold_db` | Threshold | dBFS | −80 … 0 | −45.0 | Db, 1 decimal | 20 | Delayed |
| 22 | `expander_ratio` | Ratio | ratio | 1 … 30 | 2.0 | Log, 1 decimal | 20 | ramped as R − 1 (§4.8). Delayed |
| 23 | `expander_attack_ms` | Attack | ms | 0.1 … 100 | 2.0 | Log, 1 decimal | 0 | τ, level rising. Delayed |
| 24 | `expander_release_ms` | Release | ms | 1 … 2000 | 100.0 | Log, 1 decimal | 0 | τ, level falling. Delayed |

**Group 3 `compressor` "Compressor"** (enable = 30; expanded by default):

| id | key | name | unit | range | default | taper / step | smooth | notes |
|---|---|---|---|---|---|---|---|---|
| 30 | `compressor_enabled` | Compressor | bool | off/on | **on** | BOOL | 20 | Delayed |
| 31 | `compressor_threshold_db` | Threshold | dBFS | −60 … 0 | −20.0 | Db, 1 decimal | 20 | range = Audition's (§8). Delayed |
| 32 | `compressor_ratio` | Ratio | ratio | 1 … 30 | 3.0 | Log, 1 decimal | 20 | range = Audition's; ramped as 1 − 1/R. Delayed |
| 33 | `compressor_attack_ms` | Attack | ms | 0.1 … 200 | 10.0 | Log, 1 decimal | 0 | τ. Delayed |
| 34 | `compressor_release_ms` | Release | ms | 1 … 2000 | 100.0 | Log, 1 decimal | 0 | τ. Delayed |
| 35 | `compressor_makeup_db` | Makeup | dB | 0 … 30 | 0.0 | Db, 1 decimal | 20 | the only parameter with **immediate** effect (no opt-out) |

**Group 4 `limiter` "Limiter"** (enable = 40; collapsed by default):

| id | key | name | unit | range | default | taper / step | smooth | notes |
|---|---|---|---|---|---|---|---|---|
| 40 | `limiter_enabled` | Limiter | bool | off/on | off | BOOL | 20 | Delayed |
| 41 | `limiter_threshold_db` | Threshold | dBFS | −30 … 0 | −1.0 | Db, 1 decimal | 20 | sample-peak ceiling. Delayed |
| 42 | `limiter_attack_ms` | Attack | ms | 0.1 … 50 | 1.0 | Log, 1 decimal | 0 | τ; ceiling guaranteed if look-ahead ≥ 7·attack. Delayed |
| 43 | `limiter_release_ms` | Release | ms | 1 … 2000 | 100.0 | Log, 1 decimal | 0 | τ. Delayed |

Ids are permanent (ADR-005 §3). The gaps are reserved for future section parameters. Every `Db`
taper has `neg_inf_at_min: false`.

**Telemetry** (`TelemetryCells`, §4.10):

| index | key | name | kind | unit | min … max | hold | group |
|---|---|---|---|---|---|---|---|
| 0 | `gr_total_db` | Gain reduction | GainReduction | Db | −60 … 0 | Min | none (slot header) |
| 1 | `gr_autogate_db` | AutoGate GR | GainReduction | Db | −60 … 0 | Min | autogate |
| 2 | `gr_expander_db` | Expander GR | GainReduction | Db | −60 … 0 | Min | expander |
| 3 | `gr_compressor_db` | Compressor GR | GainReduction | Db | −60 … 0 | Min | compressor |
| 4 | `gr_limiter_db` | Limiter GR | GainReduction | Db | −60 … 0 | Min | limiter |
| 5 | `input_level_dbfs` | Input level | Level | Dbfs | −100 … +6 | Max | none (graph dot; not a header meter in the custom panel) |
| 6 | `autogate_open` | Gate open | Indicator | None | 0 … 1 | Max | autogate |

**Engine constants** (shared with SPEC-013; changing one changes both modules):

| constant | value | notes |
|---|---|---|
| `PEAK_WINDOW_MS` | 5 | sliding-maximum window W_pk; ripple-free peak for tones ≥ 100 Hz (§4.3) |
| `RMS_WINDOW_MS` | 20 | rectangular mean-square window W_rms; ≤ ±0.17 dB level ripple for tones ≥ 120 Hz (§4.3) |
| `RAMP_MS` | 20 | linear parameter ramps and section/detection crossfades (§4.8) |
| `LEVEL_FLOOR_DBFS` | −150 | detector levels and composed levels are clamped to ≥ this |
| `GAIN_FLOOR_DB` | −120 | lowest gain of the dB-domain sections (Expander, Compressor, Limiter) |
| `AUTOGATE_HYSTERESIS_DB` | 3 | fixed AutoGate hysteresis |
| `SNAP_DB` / `SNAP_LIN` | 1e-6 dB / 1e-6 | smoothers snap onto their target within these distances (denormal safety, exact settling) |
| `SINE_PEAK_TO_RMS_DB` | 3.0103 | 20·log10(√2); graph offset of RMS-mode sections |
| `LOOKAHEAD_MAX_MS` | 20 | = `RMS_WINDOW_MS`, so every detector window still covers the output sample (§4.3) |

## 4. Algorithm / implementation notes (normative for SPEC-013 too)

### 4.1 Code layout
- **`dsp::dynamics`** holds pure, allocation-free-after-construction building blocks. They are used
  by both modules and by the `TransferCurve` evaluators:
  - `detector` (sliding peak, sliding RMS, delay line);
  - `curves` (static gain computers, §4.4);
  - `ballistics` (§4.6);
  - `gate` (the gate core, §4.7);
  - `ramp` (linear ramps, §4.8);
  - `sidechain` (the 4th-order Butterworth HPF used by SPEC-013).
- **`modules::dynamics`** and **`modules::noise_gate`** hold the Module API wrappers.
- ADR-001 §4 places filters, envelopes and smoothers in `dsp`.

### 4.2 Conventions
- **Per sample.** All processing is sample-by-sample state evolution. Nothing depends on where
  `process()` blocks begin or end, except the Telemetry writes (§4.10). Output is therefore
  **bit-identical for every block partition**, realtime or offline. This is stronger than SPEC-012's
  1e-6, and AC-16 tests it.
- **Precision.** Levels, gains, ramps and smoother states are `f64`. The audio path is
  `y = x_delayed · (g as f32)` in `f32`.
- **ms → samples.**
  - Windows and counts: `max(1, round(ms · fs / 1000))`; the look-ahead may be 0.
  - Time constants use the exact product `τ · fs` inside `α = exp(−1 / (τ · fs))`.
- **Transcendentals.** Per-sample `log10` and `10^x` may use fast approximations if the error is
  ≤ 0.001 dB and the code is deterministic. It is the same code in both modes.

### 4.3 Level detector
The input to the detector is the **sidechain signal** `s`: the module input, or for the Noise Gate
its high-passed input (SPEC-013 §4.1). Positions are in **output time** m (latency-aligned); `la` is
the look-ahead in samples.
- **Peak:** `L_pk[m] = 20·log10( max |s[i]| for i ∈ [m − W_pk + 1, m + la] )`.
  - The window reaches `la` samples ahead and W_pk = `PEAK_WINDOW_MS` behind, so it always covers
    the sample being output.
  - A tone's level is its exact peak, without ripple, when half its period is ≤ W_pk, i.e. for
    f ≥ 100 Hz. The detector holds the last peak for W_pk after a sound stops (the timing ACs account
    for this).
  - **Implementation:** an exact sliding maximum with bounded work per sample, e.g. a monotonic
    wedge (Lemire 2006) or van Herk/Gil-Werman. Its buffers are sized at `activate` for
    W_pk + la samples.
- **RMS:** `L_rms[m] = 10·log10( mean s[i]² for i ∈ [m + la − W_rms + 1, m + la] )`, with a
  rectangular window W_rms = `RMS_WINDOW_MS`.
  - Level ripple on a steady sine is ≤ ±0.013 dB at 997 Hz, ±0.17 dB at 120 Hz, and 0 at multiples of
    50 Hz.
  - **Implementation:** a running sum in f64 over a ring, **drift-free**. It is re-summed exactly from
    the ring every W_rms samples, counted from `activate`/`reset`, or uses an equivalent exact method.
    A window of digital silence must give exactly 0.
  - Because `la ≤ LOOKAHEAD_MAX_MS = RMS_WINDOW_MS`, the window always covers sample m.
- **Units — Decided (autonomous, T-400):** levels are raw dBFS. A sine at −20 dBFS peak reads −20.00
  in Peak mode and −23.01 in RMS mode, matching the testkit's `20·log10(rms)` convention
  (SPEC-000). No AES17 +3 dB offset is applied.
- **Floor:** levels below `LEVEL_FLOOR_DBFS` (−150), including −∞, become −150.
- **Detection-mode switch:** both detectors always run. The mode level is
  `L_mode = (1 − w)·L_pk + w·L_rms`, where the weight w ramps 0 ↔ 1 over `RAMP_MS` (§4.8). A switch
  therefore glides.

### 4.4 Static gain computers (dB)
x is the section's input level (§4.5). T, R and W are the ramped threshold, ratio and knee width.
The formulas follow Giannoulis, Massberg & Reiss 2012 (§8), eq. (4) and its mirror.

- **Compressor:**
  - `G = 0` if `2(x − T) < −W`;
  - `G = (1/R − 1)·(x − T + W/2)² / (2W)` if `2|x − T| ≤ W`;
  - `G = (1/R − 1)·(x − T)` otherwise.
- **Expander** (downward):
  - `G = 0` if `2(x − T) > W`;
  - `G = −(R − 1)·(x − T − W/2)² / (2W)` if `2|x − T| ≤ W`;
  - `G = (R − 1)·(x − T)` otherwise.
- **Limiter:** `G = min(0, T − x)` (hard, never a knee).
- **W < 1e-9 dB** means a hard knee: the middle branch is skipped.
- **Floor:** the Expander, Compressor and Limiter gains are clamped to ≥ `GAIN_FLOOR_DB` (−120 dB).
  This bounds how deep a smoother can go, so recovery from silence is prompt (e.g. an expander with
  2 ms attack is back within 1 dB of 0 in ≈ 10 ms).
- **Gate sections** (AutoGate, Noise Gate) are a state machine, not a pure function (§4.7). Their
  steady-state curves are in §4.11.
- These functions are the **single source** for `process()` and for `TransferCurve`.

### 4.5 Level-domain composition (Dynamics)
Per output sample m, in processing order, with `w_*` the section weights (§4.8) and all levels
clamped to ≥ −150 dBFS:

```
L_ag = L_pk                                         // AutoGate: always peak
g_ag_eff = 1 + w_ag · (g_ag − 1)                    // linear; g_ag from the gate core (§4.7)
G_ag = 20·log10(g_ag_eff)                           // −inf allowed
L_ex = L_mode + G_ag              → G_ex (ballistics §4.6 on the static target)   G_ex_eff = w_ex·G_ex
L_co = L_mode + G_ag + G_ex_eff   → G_co                                          G_co_eff = w_co·G_co
M_eff = w_co · makeup_db                            // makeup belongs to the compressor
L_li = L_pk + G_ag + G_ex_eff + G_co_eff + M_eff    → G_li   (always peak)        G_li_eff = w_li·G_li
g = g_ag_eff · 10^((G_ex_eff + G_co_eff + M_eff + G_li_eff) / 20)
y[m] = x[m − la] · (g as f32)
```

- **Current gains.** Upstream gains are the **current** (post-ballistics) values at the same sample,
  computed in order.
- **Exact passthrough.** When every weight is 0 and no ramp is running, `g` is exactly 1.0 and
  `y = x_delayed` bit-exactly. Implementations may special-case this; AC-11 checks the result.
- **Why compose in the level domain:** see §2.2. The static result equals the serial chain, and it
  needs one detector, one delay line and one exact curve.

### 4.6 Ballistics (Expander, Compressor, Limiter)
- **One-pole smoother on the section gain in dB, branching:**
  `G ← G_t + (G − G_t)·α`, with `α = α_A = exp(−1/(τ_A·fs))` when the target moves in the **attack
  direction**, else `α_R`.
  - Attack direction = the response to a **rising** level: `G_t < G` for Compressor and Limiter,
    `G_t > G` for Expander.
  - Snap: `|G − G_t| ≤ SNAP_DB` → `G = G_t`.
- **Time constant — Decided (autonomous, T-400):** `τ` is the time to **63.2 %** of a step in the dB
  gain. The 10–90 % time is 2.197 τ. This is the exact math of the one-pole used by the JAES tutorial
  (§8), and it makes the timing ACs exact. The UI shows the number as "Attack"/"Release" in ms,
  like Audition.
- **A time-constant change** takes effect from the event sample with the new α. `G` is not reset,
  so the gain stays continuous (no click by construction).
- **Limiter ceiling with look-ahead.**
  - The peak window (§4.3) sees a peak `la` samples before it is output. The target is at its
    lowest from then until the peak passes, so the gain only moves in the attack direction during
    that time.
  - At the peak the remaining error is at most `|G_needed| · e^(−la/τ_A)`. With `la ≥ 7·τ_A` and up
    to 20 dB of reduction, that is ≤ 0.02 dB (AC-7).
  - Upstream sections that change their gain inside the look-ahead window can add a transient error.
    The guarantee is stated for the limiter alone or with static upstream gains.

### 4.7 Gate core (AutoGate here; Noise Gate in SPEC-013)
Inputs per sample:
- level `L` (peak detector, §4.3);
- `T_open = threshold`;
- `T_close = threshold − hysteresis` (AutoGate: 3 dB fixed; Noise Gate: parameter);
- `H` = hold in samples;
- `A` = attack in samples (≥ 1);
- `τ_R` = release;
- floor `f` (AutoGate 0 = −∞; Noise Gate `10^(range/20)`, ramped, SPEC-013 §4.2).

```
state: phase ∈ {Closed, Opening, Open, Releasing}, hold counter h, gain g, ramp start g0, ramp pos p
per sample:
  if phase ∈ {Opening, Open}:                       // gate is open: judged against T_close
      if L ≥ T_close:      h = H                    // (re)load hold
      else if h > 0:       h -= 1
      else:                phase = Releasing
  else:                                             // Closed or Releasing: judged against T_open
      if L ≥ T_open:       phase = Opening; g0 = g; p = 0; h = H
  gain:
      Opening:   p += 1; g = g0 + (1 − g0)·(1 − cos(π·p/A))/2; if p ≥ A { g = 1; phase = Open }
      Open:      g = 1
      Releasing: g = f + (g − f)·α_R; if |g − f| ≤ SNAP_LIN { g = f; phase = Closed }
      Closed:    g = f        // follows f while the range ramps
```

- **Hysteresis without chatter.** An open gate is judged against the **lower** close threshold, a
  closed gate against the open threshold. A level wandering by less than the hysteresis around the
  threshold therefore causes at most one transition (SPEC-013 AC-4).
- **Hold** counts output samples after the last sample at or above `T_close`. The peak window adds
  its W_pk, so a sound ending at output sample e starts the release at ≈ e + W_pk + H (SPEC-013
  AC-5).
- **Shapes (click-free by construction).**
  - **Opening:** a raised-cosine ramp of exactly `attack` duration from the current gain. Its slope
    is continuous at both ends.
  - **Closing:** exponential in linear gain with time constant `τ_R`. Toward a 0 floor this is a
    **constant-dB-rate fade** of 8.69 dB per τ, which sounds like a natural room decay. Toward a
    finite floor it eases into the floor.
  - **Re-opening** during a release starts from the current gain, so there is never a jump.
  - **Decided (autonomous, T-400)**, rationale: an exponential opening has a slope step that splatters
    at fast attacks (SPEC-012 §4.3 calibration: a 1 ms one-pole reaches −81 dBFS), whereas a
    raised-cosine does not. An exponential release gives the familiar gate "tail".
- **Changes.** Changes to threshold, hysteresis, hold and time take effect at the event sample,
  inside this state machine. The gain itself stays continuous.
- **Reset / activate:** `phase = Closed`, `g = f`, `h = 0`; the detector windows are empty (level
  −150).
  - **Decided (autonomous, T-400):** after a seek the gate fades in over its attack rather than
    blasting a hold-and-release burst of room noise.
- **Open indicator** = `phase ∈ {Opening, Open}`.

### 4.8 Parameter ramps and crossfades
- **Linear ramps** of `RAMP_MS` (20 ms), per sample. The first ramped value applies **at the event
  sample k**, so there is no hidden delay. A new event mid-ramp starts a new ramp from the current
  value. Reset snaps ramps to their targets (ADR-005 §6).
- **Ramped domains:**
  - thresholds, knee and makeup in dB;
  - compressor ratio as the slope `1 − 1/R`;
  - expander ratio as `R − 1`;
  - Noise Gate range in **linear** gain (so −∞ is reachable);
  - section enable weights `w ∈ [0, 1]`;
  - the detection weight (§4.3).
- **Not ramped** (declared smoothing 0): attack, release, hold, AutoGate threshold, and the Noise Gate
  threshold, hysteresis and sidechain frequency. They act through smoothers or state machines, so
  the gain stays continuous anyway (§4.6, §4.7).
- **Why 20 ms:** a linear 20 ms ramp passes §4.3 with a wide margin (SPEC-012 calibration −109 dBFS),
  even when the attack is 0.1 ms and the gain follows the ramp directly. The Gain module uses the same
  time.
- **Stepped parameters with crossfades.** Enables and the detection mode are stepped parameters
  whose switches are crossfaded (SPEC-012 §2.4 lets a module spec require this). This spec requires
  them to pass §4.3 as well (AC-15).

### 4.9 Look-ahead and latency
- `la = round(lookahead_ms · fs / 1000)` samples. The audio delay line is exact (the delayed input
  is bit-identical).
- `latency_samples() = la`; `tail() = Tail::Samples(la)`.
- **A `lookahead_ms` event** does not change the running instance, because latency is fixed while
  active (ADR-005 §8). The module:
  - keeps processing with its current `la`;
  - calls `ctx.request(HostRequest::Restart)` once per changed value.

  The host then builds a replacement with the new value behind the 15 ms crossfade (ADR-005 §12,
  T-401). `param_value` reports the new target at once, and `save_state` stores it.
- **Detector windows** (§4.3) are sized from `la` at `activate`.

### 4.10 Telemetry
- Once per block with `frames > 0`, the module writes each channel **once** (the `TelemetryCells`
  single-writer rule):
  - `gr_*_db`: the minimum over the block of that section's per-sample **effective** gain in dB
    (`G_*_eff`; AutoGate `20·log10(g_ag_eff)`), clamped to [−60, 0];
  - `gr_total_db`: the minimum over the block of the per-sample sum of effective section gains,
    **excluding makeup** (makeup is gain, not reduction), clamped to [−60, 0];
  - `input_level_dbfs`: the maximum over the block of `L_pk`, clamped to [−100, +6];
  - `autogate_open`: 1 if the AutoGate's open indicator was set at any sample of the block and
    `w_ag > 0`, else 0.
- **Initial values:** at `activate` and `reset` the module writes GR 0, level −100 and lamp 0, so a
  meter never shows `TelemetryCells`' 0.0 start value as a 0 dBFS level.
- **`Hold::Min`/`Max`** keep short events between two UI reads (ADR-005 §11).

### 4.11 `TransferCurve` extension — proposed ADR-005 §11 addition (additive)
ADR-005's `ResponseCurve` is a frequency response. A dynamics module needs **level in → level out**,
so we propose a new extension instead of bending `ResponseCurve`. **Decided (autonomous, T-400)**;
flagged for an ADR-005 amendment, implemented in `module-api` by T-403.

```rust
// ExtensionId::TransferCurve => "org.powervoice.transfer-curve/1"  (also the CLAP custom-extension id, ADR-006)
// Extension::TransferCurve(Arc<dyn TransferCurve>); pub fn transfer_curve(m: &dyn Module) -> Option<Arc<dyn TransferCurve>>;
// ExtensionId::ALL gains the new id (ModuleTestHost::check_extensions covers it).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CurveBranch { Rising, Falling }

/// Static level transfer for the *target* parameter values, from the same `dsp` functions as
/// process(). Callable from any non-audio thread, concurrently with process(). Pure, deterministic,
/// no allocation beyond the output slices.
pub trait TransferCurve: Send + Sync {
    /// Settled output peak level (dBFS) of a steady 997 Hz sine whose input peak level is
    /// in_dbfs[i], for plain `values` in params() order. -inf allowed (full mute).
    /// Rising: the state reached from below/closed; Falling: from above/open (hysteresis).
    fn output_dbfs(&self, values: &[f64], branch: CurveBranch, in_dbfs: &[f64], out_dbfs: &mut [f64]);
    /// True when the Falling branch differs from Rising for these values.
    fn has_hysteresis(&self, _values: &[f64]) -> bool { false }
    /// Constant per module: individually drawable components (sections), in processing order.
    fn component_count(&self) -> usize { 0 }
    /// Group a component belongs to (for its name and colour).
    fn component_group(&self, _component: usize) -> Option<GroupId> { None }
    /// Effective gain in dB contributed by one component (0 when disabled), Rising branch.
    fn component_gain_db(&self, _component: usize, _values: &[f64], _in_dbfs: &[f64], _out_db: &mut [f64]) {}
    /// Draggable threshold handles.
    fn handles(&self) -> &[TransferHandle] { &[] }
    /// Graph x position = threshold + offset; offset = +3.0103 dB for RMS-detected sections, else 0.
    fn handle_offset_db(&self, _handle: usize, _values: &[f64]) -> f64 { 0.0 }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransferHandle { pub component: usize, pub threshold: ParamId, pub enable: Option<ParamId> }
```

**Semantics for Dynamics:**
- **Input.** The input is a steady **sine**. Peak-detected sections see `x = in`; RMS-detected
  sections see `x = in − 3.0103`. The detection mode used is the target value of `detection`, not
  the crossfade.
- **Composition.** The static composition of §4.5 is used with the settled gains, the weights at
  their targets, and makeup.
- **AutoGate branches:**
  - Rising: open (0 dB) iff `L_ag ≥ T`, else −∞;
  - Falling: open iff `L_ag ≥ T − 3`, else −∞.
- **Components:** 4 (AutoGate, Expander, Compressor, Limiter) with their groups. There are 4
  handles, one per section threshold, each with its enable.
- **Checking the curve.** `TransferCurve` must equal the measured steady-state output within
  ±0.1 dB (AC-17). That is the "graph shows what you hear" guarantee.

### 4.12 IPC (T-410) — proposed ADR-003 / ADR-005 §13 additions
All three are **Decided (autonomous, T-400)** and flagged for amendment.
- **`set_param_plain(slot, id, value)`**:
  - Why: graph handles produce plain values (dB), and ADR-005 §13 only has
    `set_param_normalized`/`set_param_text`.
  - The control thread `clamp_quantize`s the value, updates the mirror, sends the event and echoes
    `param_changed`, exactly like the other two commands.
  - Shared with the EQ graph handles (SPEC-015). Whichever ticket lands first adds it.
- **`module_transfer_curve(slot, seq, x_min_db, x_max_db, points ≤ 1024)`** → binary `Response`
  **`VXTC`**:
  - It is evaluated on the command thread from the slot's `TransferCurve` handle and the mirror's
    **target** values.
  - Errors: `no_extension` or `bad_request`.
  - Budget: ≤ 5 ms for 512 points.

  | Off | Type | Field |
  |---|---|---|
  | 0 | `[u8;4]` | `"VXTC"` |
  | 4 | u16 | version = 1 |
  | 6 | u16 | header_len = 40 |
  | 8 | u32 | seq (echo of the request) |
  | 12 | u32 | flags: bit0 `HAS_FALLING` |
  | 16 | f32 | x_min_db |
  | 20 | f32 | x_max_db |
  | 24 | u32 | points P (x_i = x_min + i·(x_max − x_min)/(P − 1)) |
  | 28 | u32 | components C |
  | 32 | u32 | handles K |
  | 36 | u32 | reserved = 0 |
  | 40 | f32[P] | Rising output dBFS (−inf allowed, never NaN) |
  | … | f32[P] | Falling output dBFS, only if `HAS_FALLING` |
  | … | f32[C·P] | component gains dB (Rising), component-major |
  | … | K × {u32 param_id, f32 x_dbfs, f32 offset_db, u32 flags (bit0 `ENABLED`)} | handles |

- **Module telemetry channel `module_telemetry_subscribe(channel)`** → **`VXMT`** frames at
  `TELEMETRY_RATE`. This is the frame ADR-003's table reserved as "later: module telemetry".
  - Frames are sent while ≥ 1 subscriber exists and ≥ 1 slot has Telemetry.
  - The meter publisher is the single reader of every handle (ADR-005 §11).
  - Channel descriptions (`TelemetryInfo`) travel once, with the rack-state DTOs (T-405).

  | Off | Type | Field |
  |---|---|---|
  | 0 | `[u8;4]` | `"VXMT"` |
  | 4 | u16 | version = 1 |
  | 6 | u16 | header_len = 32 |
  | 8 | u32 | seq |
  | 12 | u32 | flags = 0 (reserved) |
  | 16 | u64 | frame_time_ns (app clock) |
  | 24 | u32 | record count R |
  | 28 | u32 | reserved = 0 |
  | 32 | R × {u32 slot_uid, u16 count, u16 reserved, f32[count]} | values in `channels()` order |

  `slot_uid` is the rack's stable per-slot identifier used by the other rack commands (T-405).
- Both frames get golden fixtures in `gen_ipc_fixtures` (SPEC-000 AC-7).

### 4.13 Real-time, denormals, reset
- **Allocation** happens only in `activate`: detector rings, delay line, sidechain filter.
  `process`/`reset` run under `no_alloc` (ADR-002 §2).
- **Bounded work.** Loop bounds are the block length and the fixed window lengths; nothing is
  data-dependent beyond that.
- **Denormal safety without FTZ.** The snaps (`SNAP_DB`, `SNAP_LIN`) end every exponential approach
  exactly on its target. The RMS sum is exact (§4.3). Biquad states (SPEC-013) are flushed to 0 when
  `|v| < 1e-30`. The host's FTZ/DAZ guard is extra safety, not a requirement (AC-18).
- **`reset()`** does the following, and keeps parameter values:
  - clears the delay line and detector windows;
  - puts every section at 0 dB and the AutoGate `Closed` with g = 0;
  - snaps ramps to their targets;
  - writes the initial Telemetry values.
- **Non-finite input** cannot reach the module (the rack guard, SPEC-012 §2.9). For finite input
  the output is finite.

## 5. Acceptance criteria
Unless stated otherwise:
- tests run at 48 kHz with 997 Hz sines (testkit);
- the other sections are **off**;
- the look-ahead is 0;
- output levels are measured as sample peak or RMS over the last 500 ms of a 2 s steady segment;
- "bit-identical" means after latency alignment.

- **AC-1 [T-403] (schema and Module API conformance).**
  - `validate_schema` passes; ids, keys, groups and enable params are exactly as in §3; the
    descriptor is as in §2.1.
  - `ModuleTestHost` passes at 44.1/48/96 kHz, with `allow_delayed_effect` declared for every
    parameter except `compressor_makeup_db`. Makeup's first effect is exactly at `k + latency`.
  - The state round-trip is bit-identical, including a non-zero look-ahead.
- **AC-2 [T-403] (compressor static curve, Peak).**
  - Setup: input peak levels −60 … 0 dBFS in 1 dB steps; every combination of T ∈ {−40, −20, −6},
    R ∈ {1.5, 4, 30}, W ∈ {0, 6} dB; attack 10 ms, release 100 ms, makeup 0.
  - The output level equals `in + G_comp(in)` (§4.4) within **±0.1 dB**. PROMPT §5 requires ±0.5 dB;
    the design is exact for steady tones, so the tighter bound catches implementation errors.
  - With makeup 6 dB, every output rises by 6.00 ± 0.01 dB.
- **AC-3 [T-403] (compressor static curve, RMS).**
  - With the AC-2 grid in RMS mode, the output equals `in + G_comp(in − 3.0103)` within ±0.1 dB.
  - At **120 Hz** (worst-case window ripple) it is within ±0.5 dB.
- **AC-4 [T-407] (expander static curve).**
  - Setup: input peak levels −90 … 0 dBFS in 1 dB steps; T ∈ {−50, −30}; R ∈ {2, 4, 30};
    W ∈ {0, 6}; Peak and RMS.
  - The output equals `in + max(G_exp(x), −120)` within ±0.1 dB wherever the expected output is
    ≥ −120 dBFS (x per mode).
  - Levels ≥ T + W/2 pass at unity gain within ±0.01 dB.
- **AC-5 [T-407] (AutoGate static behaviour and hysteresis).** Setup: T = −40.
  - 300 ms bursts at −39.5 dBFS open the gate: after the attack, output = input within ±0.01 dB and
    `autogate_open` = 1.
  - Bursts at −40.5 dBFS never open it: the output is exactly 0.0 from reset, and the lamp stays 0.
  - After an opening burst at −30 dBFS, a steady tone at −42.5 dBFS (T − 3 + 0.5) keeps it open for
    10 s.
  - A steady tone at −43.5 dBFS closes it after hold, and the output becomes exactly 0.0 within
    14 τ_R + 1 sample of the release start.
  - Repeated for T ∈ {−60, −20}.
- **AC-6 [T-403] (limiter steady state).**
  - For T ∈ {−12, −6, −1} and steady input peaks T + {1, 6, 20} dB, the output sample peak is
    T ± 0.05 dB.
  - Inputs at T − 1 dB pass within ±0.01 dB.
  - This holds in both detection modes, because the limiter is always peak (§2.4).
- **AC-7 [T-403] (limiter ceiling with look-ahead).**
  - Setup: limiter alone, T = −1 dBFS, attack 1 ms, look-ahead 7 ms (and also attack 2 ms with
    look-ahead 14 ms); stress set:
    - 997 Hz bursts starting at a positive peak, 20 dB over;
    - single-sample impulses at +6 dBFS;
    - a full-scale 100 Hz square wave;
    - seeded pink noise at −6 dBFS RMS;
    - a staircase of +2 dB steps every 3 ms.
  - The output sample peak is ≤ T + 0.05 dB everywhere.
  - Without look-ahead no ceiling is claimed; the test records the overshoot for the report.
- **AC-8 [T-403 Compressor/Limiter, T-407 Expander] (time constants).**
  - Setup: Peak mode, knee 0, DC steps (exact per-sample gain `y/x`).
  - **Compressor** (T −20, R 4, step −40 → −10 dBFS, target −7.5 dB): for τ_A ∈ {0.5, 10, 100} ms,
    the gain first reaches 63.2 % of the target at τ_A.
  - **Compressor release** (step −10 → −40): for τ_R ∈ {10, 100, 1000} ms, the gain recovers 63.2 %
    at W_pk + τ_R after the step.
  - **Expander** (T −40, R 2): step −60 → −30 (attack) and back (release).
  - **Limiter** (T −10): step −20 → −4 and back.
  - Tolerance: **±2 % or ±1 sample**, whichever is larger. This is tighter than the ±10 % of the
    T-400 brief because the shapes are exact; the ±10 % bound is implied.
- **AC-9 [T-407] (AutoGate shapes and hold).** Setup: DC bursts at −20 dBFS, T −40.
  - **Opening:** the gain follows `g0 + (1 − g0)(1 − cos(πp/A))/2` within 1e-6 and is exactly 1.0 at
    p = A, for attack ∈ {0.1, 2, 20, 100} ms.
  - **Release** starts at output sample e + W_pk + H (e = last burst sample) ± 1 sample, for
    hold ∈ {0.1, 10, 50, 500} ms. It is one-pole with 63.2 % at τ_R ± 2 %.
  - With 997 Hz bursts instead of DC, the release start is within ±1 ms.
- **AC-10 [T-403, T-407 for its sections] (disabled sections are inert).**
  - For each section X, with X disabled, the output is **bit-identical** to the reference run for 100
    seeded random value sets of X's parameters.
    - Reference: X disabled, X's parameters at their defaults, the other sections at seeded random
      settings.
    - Signal: seeded pink noise with 200 ms bursts every 700 ms at −6 dBFS peak over a −50 dBFS
      floor.
  - Checked in realtime (random blocks 1…1024) and offline.
- **AC-11 [T-403] (all sections off = passthrough).**
  - With every section disabled (any other values), the output is bit-identical to the input
    delayed by `la`, for look-ahead ∈ {0, 5, 20} ms.
  - Checked on the AC-10 signal and on full-scale white noise.
- **AC-12 [T-403] (section and detection crossfades).** Setup: DC signal.
  - Toggling any section's enable at sample S moves its effective gain linearly from the old to the
    new value, reaching it at S + round(20 ms·fs) ± 1 sample. The ballistics are held static, so the
    ramp is visible alone.
  - A detection-mode switch glides the mode level over the same 20 ms.
- **AC-13 [T-403] (look-ahead, latency, restart).**
  - For look-ahead 0 … 20 ms (every step) at 44.1/48/96 kHz, `latency_samples()` =
    round(la·fs/1000) and `tail()` = `Samples(latency)`.
  - A `lookahead_ms` event makes the instance call `request(Restart)` exactly once. Its output stays
    bit-identical to a run without the event.
  - With look-ahead 5 ms and compressor attack 0.5 ms, a DC step from −40 to −10 dBFS is output with
    ≥ 99.99 % of its target reduction already applied (in dB) at its first sample (latency-aligned).
  - In the rack, the SPEC-012 AC-8 readouts update within 100 ms of the replacement.
- **AC-14 [T-403, T-407] (telemetry accuracy).**
  - For every steady case of AC-2 … AC-6, each `gr_*_db` equals that section's measured
    contribution within **±0.1 dB**. The contribution is the output level minus the input level with
    only that section on, minus makeup.
  - `gr_total_db` = Σ sections ± 0.1 dB, and `input_level_dbfs` = input peak ± 0.1 dB.
  - `autogate_open` is 1 exactly while the gate is open.
  - A single 5 ms dip to −12 dB GR between two reads 16.7 ms apart is reported as ≤ −11.9 dB
    (`Hold::Min`).
  - After `activate` and `reset` the channels read 0 / −100 / 0.
- **AC-15 [T-403, T-407] (no zipper noise, SPEC-012 §4.3).**
  - Every parameter in the §5.1 table passes §4.3 in both directions and in the drag variant, in
    realtime and offline, with the signal and setup listed there.
  - Enables and detection are judged with T_s = 20 ms.
  - The report states the worst excess per parameter.
- **AC-16 [T-403] (realtime = offline, bit-identical; deterministic).**
  - Signal: 20 s of seeded pink noise, a 20 Hz–20 kHz sweep and the AC-10 burst signal.
  - Settings: all sections on, RMS, look-ahead 5 ms, plus 30 parameter events at fixed positions
    (including every enable and one detection switch).
  - Realtime processing with random block sizes 1…1024, including 0-length flushes, is
    **bit-identical** to offline 4096-frame processing, at 44.1, 48 and 96 kHz.
  - Two offline renders have equal FNV-1a hashes.
- **AC-17 [T-403 total/compressor/limiter, T-407 AutoGate/expander] (`TransferCurve` equals what
  you hear).**
  - For 30 seeded random parameter sets (random enables, both detection modes), `output_dbfs` at
    input levels −80 … +6 dBFS in 1 dB steps equals the measured settled output peak of a steady
    997 Hz sine within ±0.1 dB, wherever both are ≥ −100 dBFS.
  - It is −∞ exactly where the measured output is exactly 0.0.
  - The Falling branch is checked by approaching each level from +6 dBFS.
  - The component gains sum to `output − input`.
  - Calls run allocation-free under `no_alloc`. `handles()` and `handle_offset_db` match §4.11.
- **AC-18 [T-403, T-407] (denormals and silence).**
  - Setup: FTZ/DAZ **off**; all sections on; 1 s of 0 dBFS white noise, then 120 s of digital
    silence.
  - From `la` samples after the input goes silent, every output sample is exactly 0.0.
  - A test-only accessor shows no subnormal `f32`/`f64` in the module state at the end.
- **AC-19 [T-403] (CPU budget).** In a release build on the owner's machine (`just bench`),
  offline-rendering 60 s of 48 kHz pink noise:
  - with all sections on, RMS and look-ahead 10 ms, takes ≤ **0.6 s** (≤ 1 % of one core);
  - with the default settings, ≤ 0.3 s.

  This keeps the PROMPT §4 full-rack budget (< 20 % of one core) comfortable.
- **AC-20 [T-410] (panel structure, Vitest with a schema fixture).**
  - The panel shows the global row, then the graph, then sections in the order AutoGate, Expander,
    Compressor, Limiter.
  - Header toggles are bound to ids 10/20/30/40. Disabled bodies are dimmed but editable.
    `collapsed_by_default` is honoured.
  - Section bodies use the T-405 generic widgets.
  - The look-ahead readout appears only when latency > 0.
  - No user-facing literal strings: every string comes from an i18n key.
- **AC-21 [T-410] (transfer graph, Vitest with mocked `VXTC`).**
  - Curve vertices sit at the axis-mapped positions within 0.5 px. The dashed Falling branch is
    drawn only with `HAS_FALLING` and only where it differs.
  - Handles sit at `x_dbfs` within 0.5 px.
  - Dragging the compressor handle from x(−20) to x(−26) sends `set_param_plain(slot, 31, v)` with
    v = −26 ± one pixel's dB span, at most once per animation frame. Shift reduces the movement
    ×0.1. Double-click sends the default.
  - A `param_changed` for the slot triggers one new request, and a response with an older `seq` is
    ignored.
  - The operating-point dot is at (level, level + total + makeup) within 0.5 px and is hidden per
    §2.6.
- **AC-22 [T-410] (meters and generic fallback, Vitest with mocked `VXMT`).**
  - A frame with compressor GR −6.0 puts the compressor header bar's end at the −6 dB scale position
    within 0.5 px, with the text "−6.0 dB". −45 pins the bar and reads "−45.0 dB".
  - After 250 ms without a frame every meter reads 0 and every lamp is off.
  - The slot header shows `gr_total_db`, and the AutoGate lamp follows channel 6.
  - A generic-panel fixture with the Noise Gate's schema, `TransferCurve` and telemetry (SPEC-013
    §3) renders the graph above the parameters, a lamp and a GR meter in the slot header, and a
    level bar in the Sidechain group header.
- **AC-23 [T-410] (IPC contract).**
  - Golden `VXTC` (with and without `HAS_FALLING`) and `VXMT` frames decode in Vitest with every
    field bit-identical (SPEC-000 AC-7).
  - `module_transfer_curve` returns within 5 ms for 512 points (Rust integration test) and returns
    `no_extension` for a Gain slot.
  - `set_param_plain` clamp-quantizes and echoes `param_changed`.
- **AC-24 [T-410] (manual smoke, owner machine).** On a real voice take playing through Dynamics:
  - dragging each threshold handle changes the sound without clicks;
  - the GR meters move with syllables;
  - the operating-point dot rides the curve;
  - enabling the look-ahead updates the rack latency readout.

### 5.1 Zipper test setups (SPEC-012 §4.3 substitutions)
§4.3's analysis and pass threshold are unchanged. Only the signal and the other parameter values
change (SPEC-012 §4.3 allows this when a parameter has no effect on the standard sine). All signals
are at 48 kHz.
- The **AM tone** is a 997 Hz carrier with a sinusoidal envelope at 46.875 Hz (period exactly 1024
  samples = the STFT hop, so every frame sees the same pattern), swinging between the two stated peak
  levels in dB.
- The **keyed tone** is a 997 Hz tone at −20 dBFS, 1024 samples on / 1024 off, with 2 ms
  raised-cosine edges. Each 8192-sample frame holds exactly 4 periods.

| Parameter(s) | Signal | Setup (others default unless stated) | 0.25 → 0.75 plain values |
|---|---|---|---|
| `compressor_threshold_db` | standard 997 Hz sine, −20 dBFS | compressor on | −45 → −15 dBFS |
| `compressor_ratio` | standard sine | T −40 | 2.34 → 12.8 |
| `compressor_makeup_db` | standard sine | T −40 | 7.5 → 22.5 dB |
| `knee_db` | standard sine | T −23, R 4 (knee centred on the RMS level) | 5 → 15 dB |
| `compressor_attack_ms`, `compressor_release_ms` | AM tone −32 … −20 dBFS | T −30, R 4 | 0.67 → 30 ms; 6.7 → 299 ms |
| `expander_threshold_db` | sine −30 dBFS | expander on, compressor off | −60 → −20 dBFS |
| `expander_ratio` | sine −30 dBFS | expander on, T −20 | 2.34 → 12.8 |
| `expander_attack_ms`, `expander_release_ms` | AM tone −32 … −20 dBFS | expander on, T −26, R 4 | 0.56 → 17.8 ms; 6.7 → 299 ms |
| `autogate_threshold_db` | sine −40 dBFS | AutoGate on, compressor off | −60 → −20 dBFS (open ↔ closed) |
| `autogate_attack_ms`, `autogate_release_ms` | keyed tone | AutoGate on, T −40, hold 0.1 ms | 0.56 → 17.8 ms; 6.7 → 299 ms |
| `autogate_hold_ms` | keyed tone | AutoGate on, T −40, release 5 ms | 1 → 100 ms |
| `limiter_threshold_db` | sine −10 dBFS | limiter on, compressor off | −22.5 → −7.5 dBFS |
| `limiter_attack_ms`, `limiter_release_ms` | AM tone −10 … −2 dBFS | limiter on, compressor off, T −6 | 0.47 → 10.6 ms; 6.7 → 299 ms |
| `autogate_enabled`, `expander_enabled`, `compressor_enabled`, `limiter_enabled` | standard sine −20 dBFS | each section toggled **alone** (all other sections off): AutoGate T −10 (mutes); expander T −10 R 4; compressor T −40 R 4; limiter T −30. Toggled off → on and on → off | T_s = 20 ms |
| `detection` | standard sine −20 dBFS | compressor T −40, R 4 | Peak ↔ RMS, T_s = 20 ms |

`lookahead_ms` is stepped and changes latency through a replacement. Its crossfade is covered by
SPEC-012 AC-8/§2.4, not here.

## 6. Test plan
| AC | Unit (`dsp`, `modules`) | Integration (rack / engine / CLI) | Vitest | Manual |
|---|---|---|---|---|
| AC-1 | `validate_schema`, `ModuleTestHost` with opt-outs | — | — | — |
| AC-2–AC-4 | static-curve sweeps on synthetic sines (testkit), both modes | `powervoice-cli render --rack` + `analyze` spot check of one point | — | — |
| AC-5, AC-9 | gate core over DC and tone bursts | — | — | listen to the AutoGate on a take |
| AC-6, AC-7 | limiter steady and stress set | — | — | — |
| AC-8 | DC-step timing probes | — | — | — |
| AC-10, AC-11 | seeded random-parameter bit-compare; passthrough | offline render hash | — | — |
| AC-12, AC-13 | crossfade ramps; latency table; restart request | T-401 replacement updates readouts | — | toggle look-ahead while playing |
| AC-14 | telemetry vs measured contributions; Hold semantics | fake backend: `VXMT` values reach the channel | — | watch the meters |
| AC-15 | §4.3 harness (`module-api` test-util) with the §5.1 table | — | — | drag sliders on a take |
| AC-16 | block-partition bit-compare | fake backend vs offline render | — | — |
| AC-17 | `TransferCurve` vs measured sines | — | — | — |
| AC-18 | FTZ-off run + subnormal scan (test accessor) | — | — | — |
| AC-19 | — | `just bench` (release) | — | — |
| AC-20–AC-22 | — | — | panel, graph, meters with mocked IPC and a schema fixture | inspect the panel |
| AC-23 | `gen_ipc_fixtures` golden frames | command timing test | golden decode | — |
| AC-24 | — | — | — | owner smoke on a voice take |

Signals come from testkit (`sine`, `white`, `pink`, `log_sweep`) plus three small generators added by
T-403: DC steps, the AM tone and the keyed tone. Nothing is committed.

## 7. Out of scope
- External or sidechain key inputs, sidechain filters in Dynamics (the Noise Gate has one),
  stereo linking (mono editor).
- Multiband dynamics, upward compression and expansion, parallel (dry/wet) mix, auto-makeup,
  program-dependent release, oversampling.
- True-peak limiting (SPEC-017).
- Factory presets. Built-in presets can be added through `ModuleFactory::presets` with T-406's
  storage; none are defined here.
- Per-section look-ahead; look-ahead above 20 ms.

## 8. Sources and decisions

**Sources.** Adobe helpx returned 403 to our tools, so Adobe statements come through search
snippets and third-party quotes (secondary).
- Adobe, *Amplitude and compression effects* (https://helpx.adobe.com/audition/using/amplitude-compression-effects.html):
  - Dynamics = AutoGate, Compressor, Expander, Limiter;
  - compressor threshold −60 … 0 dB, ratio 1:1 … 30:1 — secondary, moderate-high confidence;
  - Dynamics Processing offers Peak/RMS; "RMS is reflected precisely in the graph" — secondary.
- AutoGate hold 0.1 … 1000 ms; Expander has threshold and ratio only (Premiere's Dynamics, same
  engine): https://www.tumblr.com/dbpremiereaudio/95044390198/3-dynamics-autogate ·
  https://www.tumblr.com/dbpremiereaudio/95031781028/2-dynamics-expander — secondary, moderate.
- Soft-knee option in Audition Dynamics:
  https://community.adobe.com/t5/audition-discussions/classic-soft-knee-and-breaths/td-p/9927940 —
  secondary.
- **Unverified:** Audition's section processing order, its default enables and values, its AutoGate
  closed attenuation, the range of its look-ahead, and which time constants its Expander uses.
- D. Giannoulis, M. Massberg, J. D. Reiss, "Digital Dynamic Range Compressor Design — A Tutorial
  and Analysis", *J. Audio Eng. Soc.* 60(6), 2012: gain computer with soft knee, log-domain
  branching smoothing, time-constant definition.
- D. Lemire, "Streaming Maximum-Minimum Filter Using No More than Three Comparisons per Element",
  *Nordic J. Computing* 13(4), 2006; M. van Herk (1992) / J. Gil & M. Werman (1993): exact sliding
  maximum.

**Decided (autonomous, T-400)** in this spec:
- processing order AutoGate → Expander → Compressor → Limiter, shown in that order;
- one detector with level-domain composition;
- AutoGate closed = −∞ with fixed 3 dB hysteresis;
- Expander has its own attack and release;
- manual makeup only;
- knee as a width (default 6 dB);
- detection Peak/RMS for Expander and Compressor only, default RMS, raw RMS dB;
- look-ahead 0–20 ms in 1 ms steps, default 0, applied through a restart;
- defaults per §2.5 and §3;
- τ = 63.2 % time constants;
- raised-cosine gate opening and exponential closing;
- 20 ms ramps and crossfades;
- engine constants per §3;
- the `TransferCurve` extension;
- `set_param_plain`, `VXTC` and `VXMT`;
- bit-identical block-partition determinism;
- CPU budgets.

**Flagged for the orchestrator:**
- **ADR-005** §11 needs the `TransferCurve` extension; §13 needs `set_param_plain` and the generic
  `TransferCurve`/Telemetry rendering.
- **ADR-003** gains `VXTC` and `VXMT`.
- **PROMPT §3.4** lists the Expander with threshold and ratio only; this spec adds attack and release
  (an addition, not a change).
- **Timing tolerances.** The timing ACs are tighter than the T-400 brief (±2 % against ±10 %) and the
  static-curve ACs are tighter than PROMPT §5 (±0.1 dB against ±0.5 dB). Both are compatible.
