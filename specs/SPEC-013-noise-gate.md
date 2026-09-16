# SPEC-013 — Noise Gate module

- **Status:** approved (autonomous, T-400)
- **Milestone:** M4.
  - **T-408** builds the module. It reuses the shared dynamics engine that T-403 and T-407 build for
    SPEC-016 (detector, gate core, ramps, look-ahead, `TransferCurve`) and adds the sidechain HPF and
    the range floor.
  - **UI:** the generic panel (T-405), plus the generic `TransferCurve` graph and Telemetry widgets
    from T-410 (SPEC-016 §2.6).
- **Related:** **SPEC-016 §4 (the shared engine — normative for this module)** · SPEC-012 (rack;
  **§4.3 zipper measurement is normative**; §2.4 smoothing; §2.5 latency) · SPEC-004 OD-4 (rack edits
  are not undoable) · SPEC-000 · ADR-005 (§3, §8, §11; the `TransferCurve` addition proposed in
  SPEC-016 §4.11) · ADR-002 §2 · PROMPT §3.4 item 2 · `docs/references.md`
- **Implementation status (H-51, doc/code sweep):** the module shipped early as a vertical slice
  (S3-02, `crates/modules/src/noise_gate.rs`), ahead of and narrower than the T-408 plan above.
  **Live:** identity, range, hysteresis, hold, the sidechain high-pass, the gate core (§4.7 below,
  shared with SPEC-016). **Deferred, not T-408:** **look-ahead** — `lookahead_ms` exists in the
  permanent parameter schema but is flagged `HIDDEN` and has no effect; `latency_samples()` always
  returns 0 (module doc comment: "Not yet available in this slice"). **Deferred, blocked on
  SPEC-016's own T-403:** the `TransferCurve` extension referenced throughout this spec (§2.7 UI,
  §4.3, §4.11) **does not exist** in `module-api` (`ExtensionId`/`Extension` have no such variant) —
  see SPEC-016's own status note. Every acceptance criterion, UI section and wire format below that
  depends on look-ahead or `TransferCurve` describes the T-408/T-410 target, not current behaviour.
  (H-58 made SPEC-016's own look-ahead live — `vox_dsp::dynamics::detector::DelayLine` and the
  `W_pk + la` peak window are ready for T-408 to reuse here.)

## 1. Purpose
Between phrases a voice-over take carries room tone, computer fans, traffic rumble and breaths. A
noise gate turns the audio down while the talent is silent and gets out of the way the moment they
speak. Audition itself only offers the AutoGate inside Dynamics (§8). This module is the
full-control gate:
- **range** instead of a hard mute, so room tone stays natural;
- adjustable **hysteresis** and **hold** against chatter on soft word endings;
- a **sidechain high-pass** so rumble can't open it;
- optional **look-ahead** so consonant onsets aren't clipped.

## 2. Behavior / UX

### 2.1 Identity
- **id `org.powervoice.noise-gate`**, version 1.0.0.
  - This is the id ADR-005 §2 fixes (accepted). The T-400 brief wrote `org.powervoice.gate`; the ADR
    wins because built-in ids are permanent. Flagged (§8).
- **Name** "Noise Gate" (`module.noise_gate.name`); vendor "PowerVoice".
- **Features** `audio-effect`, `gate`, `mono`.
- **State** `state_format_version` 1, parameters only. **Layout** MONO.
- **Latency** = look-ahead in samples. **Tail** = `Tail::Samples(latency)` (SPEC-016 §4.9).

### 2.2 What the user hears
- **Open** (someone speaking): the audio passes **unchanged**. The gain is exactly 1.0, so the
  samples are bit-identical to the input.
- **Closing:** when the voice falls below the close threshold, the gate waits **hold**, then fades
  down to **range** with the **release** time constant. This is an exponential fade: toward −∞ a
  constant 8.69 dB per time constant, like a room decay.
- **Opening:** when the level reaches the **threshold**, the gate fades up to unity along a smooth
  raised-cosine curve lasting exactly **attack**. Re-opening during a release starts from wherever
  the gain is, so there are never jumps.
- **Detection** is by peak level (SPEC-016 §4.3, 5 ms sliding peak), after the optional sidechain
  high-pass. **Decided (autonomous, T-400):** peak only, because a gate must key on onsets and RMS
  would open late. Detection never alters the audio path.

### 2.3 Threshold and hysteresis
- **Threshold** is the **opening** threshold. The **close** threshold is **threshold − hysteresis**.
  **Decided (autonomous, T-400):** the user sets the level at which speech must open the gate, and
  hysteresis extends how far a decaying word may fall before the gate starts to close.
- **Example:** threshold −40 dBFS, hysteresis 6 dB. The gate opens at ≥ −40 and starts its hold
  once the level is below −46.
- **No chatter.** An open gate is judged only against the close threshold, and a closed gate only
  against the open threshold. A level wobbling by less than the hysteresis around the threshold
  causes at most one transition. Hold bridges short dips larger than that (SPEC-016 §4.7).
- **Hysteresis 0** makes both thresholds the same. Hold then becomes the only protection against
  chatter.

### 2.4 Range
- **Range** is the gain applied while fully closed, from **0 dB** (gate does nothing) down to **−∞**
  (full mute; the slider's minimum shows "−inf dB", ADR-005 `Taper::Db { neg_inf_at_min }`).
- **0 dB:** the output equals the input bit-exactly at all times (AC-8).
- **−∞:** after the release has run (≤ 14 time constants) the output is **exact digital silence**.
- **Default −30 dB. Decided (autonomous, T-400):**
  - it pushes a typical −60 dBFS booth noise floor to −90 dBFS, far below the ACX ≤ −60 dB
    requirement;
  - it keeps a trace of room tone, so pauses don't sound like dropouts;
  - fully muted pauses are commonly discouraged for audiobooks (not verified as a written ACX rule).
- **Changing range** glides over 20 ms (SPEC-016 §4.8).

### 2.5 Sidechain high-pass
- A **24 dB/oct Butterworth high-pass** on the detection signal only. It is **on by default at
  100 Hz**; the frequency ranges 20 Hz–2 kHz.
- **Why:** HVAC, traffic and mic-handling rumble below ~60 Hz is often louder than the hiss the gate
  is meant to catch, and would hold the gate open. The voice's energy lies well above it.
- **Rejection:** at 100 Hz the filter attenuates 40 Hz by 31.8 dB. A 40 Hz rumble at −20 dBFS
  therefore reaches the detector at −51.8 dBFS, well under the default −40 dBFS threshold (AC-9).
- **Decided (autonomous, T-400), slope and default:**
  - a 12 dB/oct filter would pass that rumble at −36 dBFS, opening the gate;
  - the steeper slope costs nothing, because the filter is not in the audio path;
  - "on by default" follows because PROMPT calls the filter optional, and an off-by-default rumble
    guard would not help the users who need it.
- **Changing** the frequency or the switch affects only the detector. At worst it causes one gate
  transition (§4.1).

### 2.6 Look-ahead
- **0–20 ms** in 1 ms steps, default 0.
- It delays the audio so the gate starts opening before the sound arrives. With look-ahead ≥ attack,
  a word's first sample passes at full gain (AC-10).
- It adds its value as latency, which the rack shows and compensates (SPEC-012 §2.5).
- A change restarts the module behind the rack's 15 ms crossfade (SPEC-016 §4.9).
- **Default 0** keeps through-rack monitoring latency-free.

### 2.7 UI
- **Generic parameter panel** (T-405): the main section, then the **Sidechain** group, whose header
  toggle is `sc_hpf_enabled` and whose body holds the frequency.
- **Generic extensions** (T-410, SPEC-016 §2.6):
  - a **transfer graph** above the parameters, drawn from `TransferCurve`: solid rising branch, and
    the falling branch dashed between the close and open thresholds (the hysteresis loop);
  - the **Gate open** lamp and the **gain meter** in the slot header;
  - the **sidechain level** bar in the Sidechain group header.
- There is no custom panel.
- **Strings** use i18n keys `module.noise_gate.*`.

### 2.8 Edge cases
- **Undo:** parameter changes are rack edits and are not undoable (SPEC-004 OD-4).
- **`reset()`** (seek, loop wrap, transport start): the gate starts **closed** at the range gain, the
  detector and delay line are empty, and ramps snap to their targets. The first sound fades in over
  the attack; there is no burst of room noise (SPEC-016 §4.7).
- **Float input above 0 dBFS** passes when open. Levels below −150 dBFS read −150.
- **Sample rates** 44.1–192 kHz: times and the HPF are computed at the active rate. The HPF
  frequency is limited to 0.45·fs internally (a no-op at supported rates, where 2 kHz ≪ Nyquist).

## 3. Parameters
All parameters are `AUTOMATABLE`. "Smooth" is the declared `smoothing_ms`. **Every** parameter is
declared with `ModuleTestHost::allow_delayed_effect`, because each one's effect depends on the gate
state or the signal, affects only the sidechain, or needs a restart (MEMORY T-005 note).

| id | key | name | unit | range | default | taper / step | smooth | notes |
|---|---|---|---|---|---|---|---|---|
| 1 | `threshold_db` | Threshold | dBFS | −80 … 0 | −40.0 | Db, 1 decimal | 0 | opening threshold |
| 2 | `hysteresis_db` | Hysteresis | dB | 0 … 20 | 6.0 | Db, 1 decimal | 0 | close threshold = threshold − hysteresis |
| 3 | `attack_ms` | Attack | ms | 0.1 … 100 | 2.0 | Log, 1 decimal | 0 | raised-cosine opening duration |
| 4 | `hold_ms` | Hold | ms | 0.1 … 1000 | 50.0 | Log, 1 decimal | 0 | Audition AutoGate's range (SPEC-016 §8) |
| 5 | `release_ms` | Release | ms | 1 … 2000 | 100.0 | Log, 1 decimal | 0 | exponential time constant τ (63.2 %) |
| 6 | `range_db` | Range | dB | −100 (= −∞) … 0 | −30.0 | Db { neg_inf_at_min: true }, 1 decimal | 20 | closed gain; ramped in linear gain |
| 7 | `sc_hpf_enabled` | Sidechain HPF | bool | off/on | on | BOOL | 0 | enable of group `sidechain` |
| 8 | `sc_hpf_hz` | Frequency | Hz | 20 … 2000 | 100 | Log, 0 decimals | 0 | 24 dB/oct Butterworth; detection only |
| 9 | `lookahead_ms` | Look-ahead | ms | 0 … 20 | 0 | Linear, step 1 | — | latency; change → Restart |

- **Group 1** `sidechain` "Sidechain" (enable = 7; not collapsed). Parameters 1–6 and 9 are
  ungrouped, in the order above.
- **Decided (autonomous, T-400), defaults:**
  - threshold −40 dBFS: between a booth noise floor around −60 and speech peaks around −12;
  - hysteresis 6 dB: soft word endings stay open;
  - attack 2 ms: fast enough for plosives, and click-free with the raised cosine;
  - hold 50 ms;
  - release 100 ms;
  - range −30 dB (§2.4);
  - HPF on at 100 Hz (§2.5);
  - look-ahead 0 (§2.6).

  Audacity's gate ships with −12 dB level reduction and attack ≥ 1 ms (§8). We chose a deeper range
  because a voice-over gate targets room noise, not bleed.
- **Ranges** for threshold, attack and release match SPEC-016's AutoGate so presets and habits
  transfer between the two.

**Telemetry** (`TelemetryCells`; written once per non-empty block; initial values written at
`activate`/`reset`, SPEC-016 §4.10):

| index | key | name | kind | unit | min … max | hold | group | value |
|---|---|---|---|---|---|---|---|---|
| 0 | `gate_open` | Gate open | Indicator | None | 0 … 1 | Max | none (slot header) | 1 if the gate was open (Opening/Open) at any sample of the block |
| 1 | `gain_db` | Gate gain | GainReduction | Db | −100 … 0 | Min | none (slot header) | 20·log10 of the block's minimum gate gain; −∞ → −100 |
| 2 | `sidechain_level_dbfs` | Sidechain level | Level | Dbfs | −100 … +6 | Max | sidechain | the block's maximum detector level (after the HPF) |

Initial values are 0, 20·log10(f) clamped (the gate starts closed), and −100.

**Constants** are shared with SPEC-016 §3 (`PEAK_WINDOW_MS` 5, `RAMP_MS` 20, `LEVEL_FLOOR_DBFS`
−150, `SNAP_LIN` 1e-6, `LOOKAHEAD_MAX_MS` 20). The sidechain filter is two cascaded biquads with
Q = 0.5412 and 1.3066 (`docs/references.md`, 4th-order Butterworth).

## 4. Algorithm / implementation notes
The detector, gate core, ramps, look-ahead, telemetry pattern, determinism and denormal rules are
**SPEC-016 §4.2, §4.3, §4.7, §4.8, §4.9, §4.10 and §4.13**, and are not repeated here. Only the
gate-specific parts follow.

### 4.1 Sidechain
- **Signal:** `s = HPF(x)` when `sc_hpf_enabled`, else `s = x`. The HPF runs on the **undelayed**
  input, so look-ahead applies to it like the detector (SPEC-016 §4.3). The audio path never passes
  through the HPF.
- **Filter:** two transposed-direct-form-II biquads in `f64`. Both are high-pass sections at the same
  corner frequency f_c, with Q₁ = 0.5412 and Q₂ = 1.3066. Coefficients follow the RBJ cookbook
  (bilinear transform with prewarping; `docs/references.md`).
  - Magnitude = 4th-order Butterworth: `|H(f)| = (f/f_c)⁴ / √(1 + (f/f_c)⁸)` in the analog
    prototype. Bilinear warping makes the discrete response differ slightly near Nyquist, which is
    irrelevant at f_c ≤ 2 kHz.
- **Frequency changes** recompute the coefficients at the event sample, keeping the filter state.
  - **Decided (autonomous, T-400):** no coefficient smoothing. The filter only feeds the detector,
    so a coefficient step cannot click in the audio. Its detector transient is absorbed by the
    hysteresis and hold, and at worst causes one gate transition.
- **Enable switch:** changes the detector input at the event sample (same reasoning). The filter
  keeps running while disabled, so re-enabling is warm.
- **Denormals:** states are flushed to 0 when `|v| < 1e-30` (SPEC-016 §4.13).

### 4.2 Gate core and output
- **Level:** `L_pk` from SPEC-016 §4.3 on the sidechain signal `s`.
- **Gate state** comes from the SPEC-016 §4.7 core, with:
  - `T_open = threshold_db`;
  - `T_close = threshold_db − hysteresis_db`;
  - `H = round(hold_ms·fs/1000)`;
  - `A = max(1, round(attack_ms·fs/1000))`;
  - `τ_R = release_ms`;
  - floor `f` = the current value of the **range ramp**. The ramp runs in linear gain from the old
    `10^(range/20)` to the new one over 20 ms, and `range_db` at its minimum (−100) means f = 0.
- **While closed** (`Closed` phase) the gain follows f, so a range change is heard as a 20 ms glide.
  While releasing, the exponential approaches the moving f.
- **Output:** `y[m] = x[m − la] · (g as f32)`. With the gate open, g = 1.0 exactly, so the output
  is bit-identical to the input. With range 0 dB, f = 1, so g is 1.0 at all times.

### 4.3 `TransferCurve` (SPEC-016 §4.11)
- **Branches** (steady 997 Hz sine of input peak level `in`):
  - Rising: `out = in` if `in ≥ T_open`, else `in + 20·log10(f_target)` (−∞ when the range is −∞);
  - Falling: the same with `T_close` in place of `T_open`.
- **`has_hysteresis`** = `hysteresis_db > 0`.
- **One component** (the gate gain). **One handle** (`threshold_db`, offset 0, no enable).
- **The sidechain HPF is ignored by the curve.** The curve describes a 997 Hz tone.
  - The HPF passes 997 Hz within 0.01 dB for f_c ≤ 460 Hz, which includes the 100 Hz default.
  - Above that the curve is optimistic: the detector sees 997 Hz 0.11 dB lower at 632 Hz and 24 dB
    lower at 2 kHz.
  - The curve also does not model hold or time.

### 4.4 Real-time
- `activate` allocates the delay line and the peak window (W_pk + la).
- `process`/`reset` do no allocation, and their loops are bounded by the block and window lengths.
- **Cost:** one sliding maximum, two biquads, one `log10` per sample (or a comparison in the linear
  domain, see below), and one gain multiply.
- **Linear-domain comparison.** Implementations may compare `max|s|` against
  `10^(T/20)` in the linear domain instead of taking the log per sample, if the decisions are
  identical. The telemetry level is then computed once per block from the block maximum.

## 5. Acceptance criteria
Unless stated otherwise:
- tests run at 48 kHz with 997 Hz tones (testkit);
- the sidechain HPF is **off** (so the tests isolate the gate core) and the look-ahead is 0;
- other parameters are at their defaults;
- the gain is measured per sample as `y/x` on DC or on tone peaks;
- "bit-identical" means after latency alignment.

- **AC-1 [T-408] (schema and Module API conformance).**
  - `validate_schema` passes, and ids, keys, group, taper and flags are exactly as in §3.
  - `ModuleTestHost` passes at 44.1/48/96 kHz with `allow_delayed_effect` for all nine parameters.
  - `range_db` text round-trips, with "-inf dB" at the minimum.
  - The state round-trip is bit-identical, including a non-zero look-ahead.
- **AC-2 [T-408] (open threshold exact).** For threshold T ∈ {−60, −40, −20} dBFS and 300 ms tone
  bursts separated by 1 s of silence:
  - bursts at **T + 0.5 dB** open the gate every time: `gate_open` = 1, and after the attack
    output = input bit-identical;
  - bursts at **T − 0.5 dB** never open it: the output equals input × f for the whole run, and
    `gate_open` stays 0.
- **AC-3 [T-408] (close threshold = threshold − hysteresis).** For hysteresis ∈ {3, 6, 12} dB with
  T = −40:
  - after a 100 ms opening burst at −30 dBFS, a steady tone at **T − hysteresis + 0.5 dB** keeps the
    gate open for 10 s (gain exactly 1.0 throughout);
  - a steady tone at **T − hysteresis − 0.5 dB** starts the release after the hold (AC-5 timing).
  - With hysteresis 0, the open and close decisions both flip at T ± 0.5 dB.
- **AC-4 [T-408] (no chatter).**
  - Signal: a tone whose level alternates between **T + 0.5 dB and T − 0.5 dB every 10 ms**, with
    1 ms raised-cosine level transitions, for 10 s. Settings: hysteresis 3 dB, hold 0.1 ms.
  - Result: exactly **1** transition (the first opening) and **0** closings. After the first attack
    the gain is exactly 1.0 to the end.
  - **Discrimination check:** the same signal with hysteresis 0 and hold 0.1 ms produces
    ≥ **200** transitions, which proves the test can fail.
- **AC-5 [T-408] (hold time).** For hold ∈ {0.1, 10, 50, 500} ms:
  - with DC bursts at −20 dBFS (T −40), the release starts at output sample **e + W_pk + H ± 1
    sample**, where e is the last burst sample, W_pk = round(5 ms·fs) and H = round(hold·fs/1000);
  - with 997 Hz bursts it starts within **±1 ms** of the same point.
- **AC-6 [T-408] (attack shape and duration).** For attack ∈ {0.1, 2, 20, 100} ms:
  - opening from the closed state (g0 = f) and from a mid-release gain (g0 ≈ 0.5), the per-sample
    gain equals `g0 + (1 − g0)(1 − cos(πp/A))/2` within 1e-6;
  - it is exactly 1.0 from p = A on;
  - no sample-to-sample gain step exceeds `(1 − g0)·π/(2A)`, i.e. there is no discontinuity.
- **AC-7 [T-408] (release time constant and shape).** For release ∈ {5, 100, 1000} ms and range
  ∈ {−30 dB, −∞}:
  - the gain after the release start follows `f + (1 − f)·exp(−n/(τ_R·fs))` within 1e-6 until the
    snap;
  - the 63.2 % point is at τ_R ± max(2 %, 1 sample). The T-400 brief allowed ±10 %.
- **AC-8 [T-408] (range).**
  - For range ∈ {−6, −30, −60, −99.9} dB, the closed attenuation (output RMS / input RMS of a
    steady −60 dBFS tone, measured after 16 τ_R) equals the range within **±0.2 dB**. The design is
    exact after the snap, within ±0.001 dB.
  - With range −∞ the output is **exactly 0.0** from ≤ 14 τ_R + 1 sample after the release start.
  - With range **0 dB**, the output is bit-identical to the input for 60 s of the AC-4 signal plus
    seeded pink noise, for every value of the other parameters (100 seeded sets).
- **AC-9 [T-408] (sidechain HPF).** With the HPF on at 100 Hz (default), threshold −40:
  - a **40 Hz sine at −20 dBFS** for 10 s **never opens** the gate: `gate_open` = 0, and the
    output = input × f;
  - with the HPF off, the same signal opens it;
  - a 997 Hz tone makes the same open/close decisions as in AC-2, within ±0.5 dB;
  - with the gate open, a 40 Hz + 997 Hz mix passes bit-identical, which proves the audio path
    bypasses the filter;
  - `sidechain_level_dbfs` for sines at 25, 40, 63, 100 and 1000 Hz equals the input peak plus the
    4th-order Butterworth magnitude within ±0.5 dB.
- **AC-10 [T-408] (look-ahead latency and pre-open).**
  - For look-ahead ∈ {0, 1, 5, 20} ms at 44.1/48/96 kHz, `latency_samples()` = round(la·fs/1000)
    and `tail()` = `Samples(latency)`.
  - A `lookahead_ms` event triggers exactly one `request(Restart)`, and the output stays
    bit-identical to a run without the event.
  - With look-ahead 5 ms and attack 2 ms, a 997 Hz −20 dBFS burst after 1 s of silence is output
    **bit-identical** to the input from its first sample. With look-ahead 0, its first 2 ms follow
    the attack shape.
  - In the rack, the SPEC-012 AC-8 readouts update within 100 ms of the replacement.
- **AC-11 [T-408] (no zipper noise, SPEC-012 §4.3).**
  - Every continuous parameter (threshold, hysteresis, attack, hold, release, range, sidechain
    frequency) passes §4.3 in both directions and in the drag variant, in realtime and offline,
    with the §5.1 setups.
  - A direction in which the parameter has no effect on its setup passes trivially and is reported
    as such.
- **AC-12 [T-408] (realtime = offline, bit-identical; deterministic).**
  - Signal: 20 s of seeded pink noise at −50 dBFS with 997 Hz and speech-like pink bursts at
    −12 dBFS.
  - Settings: HPF on, look-ahead 5 ms, and 20 events at fixed positions on every continuous
    parameter and the HPF switch.
  - Realtime processing with random block sizes 1…1024, including 0-length flushes, is
    **bit-identical** to offline 4096-frame processing, at 44.1/48/96 kHz.
  - Two offline renders have equal FNV-1a hashes.
- **AC-13 [T-408] (telemetry).**
  - `gate_open` reads 1 exactly for the blocks in which the gate was open. A single 30 ms opening
    between two reads 16.7 ms apart is reported (`Hold::Max`).
  - `gain_db` equals 20·log10 of the block's minimum gain within ±0.01 dB (−100 for −∞).
  - `sidechain_level_dbfs` equals the block's maximum detector level within ±0.1 dB.
  - After `activate` and `reset`, the channels read 0, 20·log10(f) (clamped) and −100.
- **AC-14 [T-408] (`TransferCurve`).**
  - For 20 seeded parameter sets (HPF off, or on with f_c ≤ 460 Hz, §4.3), `output_dbfs` Rising and
    Falling at input levels −80 … +6 dBFS in 1 dB steps equals the measured settled output peak of a
    steady 997 Hz sine within ±0.1 dB. The
    Falling branch is approached from +6 dBFS, and both are −∞ where the output is exactly 0.0.
  - The branches differ only in [T − hysteresis, T).
  - `has_hysteresis`, the component and the handle are as in §4.3. Calls are allocation-free.
- **AC-15 [T-408] (denormals and silence).**
  - Setup: FTZ/DAZ **off**, HPF on, range −∞; 1 s of 0 dBFS white noise, then 120 s of digital
    silence.
  - From `la` samples after the input goes silent to the end, the output is exactly 0.0.
  - The HPF states are exactly 0, and a test-only accessor finds no subnormal value in the module
    state.
- **AC-16 [T-408] (reset).** After `reset()` mid-signal:
  - the gate is `Closed` at g = f, and the telemetry reads its initial values;
  - the output from then on is bit-identical to a freshly activated instance with the same parameter
    values fed the same subsequent input.
- **AC-17 [T-408] (CPU budget).** In a release build on the owner's machine (`just bench`),
  offline-rendering 60 s of 48 kHz pink noise with the HPF on and look-ahead 5 ms takes **≤ 0.3 s**
  (≤ 0.5 % of one core).

### 5.1 Zipper test setups (SPEC-012 §4.3 substitutions)
§4.3's analysis and pass threshold are unchanged. Only the signal and the other parameter values
change, as SPEC-012 §4.3 allows when a parameter has no effect on the standard sine (gate thresholds
and times). All signals are at 48 kHz, with the HPF on at 100 Hz unless stated.
- The **keyed tone** is a 997 Hz tone at −20 dBFS, 1024 samples on / 1024 off, with 2 ms
  raised-cosine edges. Each 8192-sample STFT frame holds exactly 4 periods, so every frame sees the
  same pattern.
- T_s follows §4.3: max(`smoothing_ms`, 1 ms). That is 1 ms for the parameters declared 0, and 20 ms
  for `range_db`.

| Parameter | Signal | Setup (others default unless stated) | 0.25 → 0.75 plain values | Effect |
|---|---|---|---|---|
| `threshold_db` | steady sine −40 dBFS | — | −60 → −20 dBFS | open → closed (release); reverse: closed → open (attack) |
| `hysteresis_db` | 100 ms at −30 dBFS (opens), then steady −50 dBFS; the burst lies outside the analysed frames | hold 50 ms | 5 → 15 dB | 0.75 → 0.25 closes the gate; 0.25 → 0.75 has no effect (trivial pass) |
| `attack_ms` | keyed tone | hold 0.1 ms | 0.56 → 17.8 ms | opening shape changes every cycle |
| `release_ms` | keyed tone | hold 0.1 ms | 6.7 → 299 ms | closing depth changes every cycle |
| `hold_ms` | keyed tone | release 5 ms | 1 → 100 ms | closes every cycle ↔ stays open |
| `range_db` | steady sine −50 dBFS (gate closed) | — | −75 → −25 dB | closed gain glides over 20 ms |
| `sc_hpf_hz` | steady sine −20 dBFS (gate open) | — | 63 → 632 Hz | detection only; the gate must stay open, and the audio must stay bit-identical |

`sc_hpf_enabled` (bool) and `lookahead_ms` (stepped, replaced through a restart, covered by SPEC-012)
are not continuous and are outside AC-5 of SPEC-012.

## 6. Test plan
| AC | Unit (`dsp`, `modules`) | Integration (rack / CLI) | Vitest | Manual (owner) |
|---|---|---|---|---|
| AC-1 | `validate_schema`, `ModuleTestHost` with opt-outs | — | — | — |
| AC-2–AC-4 | tone-burst and dither-level signals; transition counter (state probe + telemetry) | — | — | gate a take with pauses; listen for chatter on word endings |
| AC-5–AC-7 | DC and tone bursts; per-sample gain traces | — | — | — |
| AC-8 | range sweep; 0 dB bit-compare over seeded parameter sets | — | — | compare −30 dB against −∞ on room tone |
| AC-9 | HPF magnitude and rumble signal | — | — | play a take with HVAC rumble |
| AC-10 | latency table; restart request; pre-open bit-compare | T-401 replacement updates readouts | — | toggle look-ahead while playing |
| AC-11 | §4.3 harness with the §5.1 setups | — | — | drag threshold and range on a take |
| AC-12 | block-partition bit-compare | fake backend vs offline render | — | — |
| AC-13 | telemetry vs internal state | `VXMT` path (T-410) | generic lamp/meter (SPEC-016 AC-22) | watch the lamp follow speech |
| AC-14 | `TransferCurve` vs measured sines | — | graph fixture (SPEC-016 AC-22) | — |
| AC-15, AC-16 | FTZ-off run + subnormal scan; reset compare | — | — | — |
| AC-17 | — | `just bench` (release) | — | — |

Signals come from testkit plus the SPEC-016 test generators (DC steps, keyed tone), and one more
generator: the level-dither tone of AC-4. Nothing is committed.

## 7. Out of scope
- External key input and sidechain listen/monitor.
- Sidechain low-pass or band-pass; "ducking" (inverted gate).
- Frequency-dependent (multiband) gating; spectral gating (that is noise reduction, M5).
- RMS detection for the gate.
- Look-ahead above 20 ms.
- Factory presets (T-406 may add them).

## 8. Sources and decisions

**Sources:**
- No standalone Noise Gate exists in current Audition; the AutoGate section of Dynamics serves that
  role (several secondary sources; Adobe helpx was unreachable, 403).
- AutoGate hold 0.1–1000 ms (secondary, see SPEC-016 §8).
- Audacity Noise Gate manual (https://manual.audacityteam.org/man/noise_gate.html, primary):
  level reduction down to −100 dB with a default of −12 dB, attack 1–1000 ms, a "gate frequencies
  above" option.
- ReaGate (https://wiki.cockos.com/wiki/index.php/ReaGate): hysteresis, pre-open (look-ahead) and
  sidechain HP/LP filters exist; exact numbers are unverified.
- FabFilter Pro-G (https://www.fabfilter.com/help/pro-g/using/timecontrols): range and hysteresis
  exist; defaults unverified.
- Butterworth Q values: `docs/references.md`.

**Decided (autonomous, T-400)** in this spec:
- threshold = opening threshold, close = threshold − hysteresis;
- peak-only detection (shared 5 ms sliding peak);
- raised-cosine attack duration and exponential release time constant (shared with SPEC-016);
- the gate starts closed after a reset;
- range −100 = −∞, default −30 dB, ramped in linear gain;
- sidechain HPF 24 dB/oct, on by default at 100 Hz, 20 Hz–2 kHz, unsmoothed coefficient changes;
- look-ahead 0–20 ms in 1 ms steps, default 0;
- defaults per §3;
- telemetry channels per §3;
- bit-identical block-partition determinism;
- CPU budget ≤ 0.5 % of one core.

**Contradictions flagged for the orchestrator:**
- **Module id:** T-400 brief `org.powervoice.gate` against ADR-005 §2 `org.powervoice.noise-gate`.
  We follow the accepted ADR.
- **Sidechain HPF default:** PROMPT §3.4 calls it "optional". It is still optional (it has a
  switch), but on by default.
- **Time tolerances:** the timing ACs use ±2 % instead of the brief's ±10 % (tighter, compatible).
