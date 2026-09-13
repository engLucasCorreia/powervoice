# SPEC-017 — True-peak limiter

- **Status:** approved (autonomous, T-400)
- **Milestone:** M4 — T-404 (module, testkit true-peak reference, tests). Rack latency compensation
  and Restart handling come from T-401. The gain-reduction meter UI is T-410 (generic Telemetry
  meter).
- **Related:** PROMPT §2 "Loudness" (LOCKED), §3.4 module 6, §3.5, §5 (TP example) · SPEC-000
  (testkit conventions) · SPEC-004 OD-4 · SPEC-011 (ACX check, M6) · SPEC-012 §2.4, §2.5, §4.3,
  AC-5, AC-8, AC-9 · SPEC-016 (Dynamics' own limiter section is a different, sample-peak tool) ·
  ADR-002 §2 · ADR-005 §3, §4, §8, §11, §12 · `docs/references.md` (BS.1770 true peak) · MEMORY
  (ebur128 true-peak caveat)

## 1. Purpose
Delivery specs are written in **true peak** (dBTP): EBU R128 allows −1 dBTP, streaming and podcast
platforms ask for −1 or −2 dBTP, and lossy encoders (MP3 export, M6) overshoot on intersample peaks.
Sample-peak normalizing leaves peaks *between* samples that can exceed 0 dBFS after
digital-to-analog conversion or encoding.

The True-Peak Limiter is the last module in a voice-over rack. The user sets a ceiling (default
−1.0 dBTP) and pushes the level with Input gain, and the module guarantees the output's true peak
never exceeds the ceiling. It is transparent when nothing needs limiting and doesn't pump or
distort audibly at its defaults. Audition's equivalent is the Hard Limiter.

## 2. Behavior / UX

### 2.1 Identity
- Module id **`org.powervoice.true-peak-limiter`**, version **1.0.0**, `state_format_version` 1,
  features `["limiter", "mastering", "mono"]`, name "True-Peak Limiter" (i18n
  `module.true_peak_limiter.name`).
- **Decided (autonomous, T-400):** ADR-005 §2 fixes the id; the brief's `org.powervoice.tplimiter`
  is not used (§7).
- The UI is the generic parameter UI (SPEC-012 §2.6) plus the gain-reduction meter in the slot
  header (Telemetry, T-410). There is no custom panel.

### 2.2 Controls
Four parameters (§3), in Audition Hard Limiter terms:

| Control | Audition name | Default | Notes |
|---|---|---|---|
| **Input gain** | Input Boost | 0 dB | pushes the level into the ceiling |
| **Ceiling** | Maximum Amplitude | **−1.0 dBTP** | PROMPT §3.4 |
| **Release** | Release Time | 100 ms | |
| **Look-ahead** | Look-Ahead Time | **5 ms** | |

- Audition's own defaults (from memory, ⚠ unverified, helpx returns 403) are Maximum Amplitude
  −0.1 dB, Input Boost 0 dB, Look-Ahead 7 ms and Release 100 ms.
- **Decided (autonomous, T-400):**
  - **Look-ahead 5 ms** keeps through-rack monitoring latency at 5.3 ms, and the §5 distortion
    bounds hold at 5 ms.
  - **Release 100 ms** follows Audition.
  - **No "link channels"** (mono) and **no auto-release** in v1, because a fixed, documented
    release keeps renders deterministic and testable. Auto-release is out of scope.

### 2.3 What the user can rely on
- **Ceiling guarantee.** The output's true peak never exceeds the ceiling by more than 0.1 dB, and its
  sample peak never exceeds the ceiling (§5 AC-3). This holds at every setting, including heavy
  input gain and during ceiling changes: each audio sample is limited against the ceiling in
  force when that sample is output.
- **Transparency.** Material whose true peak stays below the ceiling passes **bit-exact**, only
  delayed by the latency (AC-8).
- **Release.** After a peak, the gain recovers at **12 dB per release time** (Audition's definition:
  "time to rebound 12 dB", ⚠ unverified). It starts once the look-ahead window has passed the last
  peak, and it returns exactly to unity.
- **Attack.** Gain reduction starts one look-ahead window before a peak and follows a smooth
  S-curve, so peaks are caught before they happen and there is no overshoot or click.
- **Latency.** Latency = look-ahead + 16 samples of detector delay, e.g. 256 samples (5.33 ms) at
  48 kHz. It is reported to the rack, shown on the slot, and compensated (SPEC-012 §2.5).
  - Changing the look-ahead changes the latency, so the rack replaces the instance behind a 15 ms
    crossfade (ADR-005 §12). Readouts update within 100 ms (SPEC-012 AC-8).
- **Meter.** The slot header shows gain reduction in dB (0 … −24 dB scale), with the deepest
  reduction since the previous frame.

### 2.4 Edge cases
- **Content above 20 kHz** (at 44.1/48 kHz) has no well-defined true peak: every meter depends on
  its own reconstruction filter. The guarantee is stated for content ≤ 20 kHz, which covers voice
  and every stress signal in §5. Above that, the detector's roll-off (0.4535·fs) is the documented
  limit.
- **Float input above 0 dBFS**, e.g. after an EQ boost, is fine. Input gain up to +24 dB on a
  +6 dBFS input still meets the ceiling.
- **Silence** in gives exact silence out; the gain is exactly 1.
- **Undo.** Rack edits: none (SPEC-004 OD-4).

## 3. Parameters
All parameters are `AUTOMATABLE`. ParamIds are permanent.

| id | key | name | unit | range | default | taper/step | smoothing | notes |
|---|---|---|---|---|---|---|---|---|
| 0 | `input_gain_db` | Input gain | dB | −12 … +24 | 0.0 | Db { neg_inf_at_min: false }, 1 decimal | **100 ms**, linear in linear gain | 20 ms fails §4.3 under limiting (§4.4) |
| 1 | `ceiling_dbtp` | Ceiling | dBTP | −12.0 … 0.0 | −1.0 | Db, `Unit::Dbtp`, 1 decimal | 20 ms, linear in dB | travels with the audio (§4.3) |
| 2 | `release_ms` | Release | ms | 10 … 1000 | 100 | Log, 0 decimals | 0 (a rate, no discontinuity) | 12 dB per release time |
| 3 | `lookahead_ms` | Look-ahead | ms | 1.0 … 10.0 | 5.0 | Linear, step 0.5 (STEPPED), 1 decimal | 0 | change ⇒ `HostRequest::Restart` |

**Telemetry** (`TelemetryCells`, one write per `process()` call):

| index | key | kind | unit | min / max | hold |
|---|---|---|---|---|---|
| 0 | `gain_reduction_db` | `GainReduction` | Db | −24 / 0 | `Min` |

**Derived constants:**
- L = round(lookahead_ms·fs/1000), rounded up to even.
- D = 16 (detector delay).
- latency = L + D.
- `Tail::Samples(0)`: once the latency is flushed, silence in gives silence out.

## 4. Algorithm / implementation notes

### 4.1 Signal flow (f64 inside, f32 at the edges)
```
x ─► × g_in[n] ─► u ─┬──────────────── delay L + D ─────────────────► × G ─► y
                     └► 4× TP detector (D) ─► r ─► min-hold (L+1) ─► release ─► 2× moving avg ─► G
```

### 4.2 True-peak detection (4×, BS.1770 Annex 2 style, plus parabolic refinement)
- **Interpolator.** A 4× polyphase FIR with 32 taps per phase (128 in total): a Kaiser-windowed sinc
  with cut-off fs/2 and a window half-width of 16 input samples. β = 4.60, from the Kaiser formula
  for the transition 0.4535·fs … 0.5465·fs, ≈ 50 dB.
  - Phase 0 reduces to the input sample itself, so sample peaks are always seen exactly.
  - The group delay is D = 16 samples.
  - Coefficients are computed once in `activate` (f64) and applied in f32 or f64 (T-404's choice),
    provided AC-3 holds.
- **Interval peak.** For each interval between input samples n−1 and n, the peak is q = max |v| over
  its five 4× points (both ends included). At every local maximum of |v|, it also takes the vertex
  of the parabola through that point and its two neighbours, accepted when the vertex lies within
  ±1 step.
- **Decided (autonomous, T-400): parabolic refinement.** Plain 4× points under-read a
  near-20 kHz sine by up to 0.56 dB (cos(π·0.4535/4)). The spec simulation measured output
  overshoot against the 16× reference (§4.6) for three designs:

  | Design | Overshoot (band-limited stress set) |
  |---|---|
  | plain 4× | +0.21 … +0.26 dB |
  | **4× + parabola** | **+0.027 dB** (44.1 kHz), **+0.020 dB** (48 kHz) |
  | 8× + parabola | +0.011 dB |

  4× keeps PROMPT §3.4's "4× oversampled detection". The refinement costs a few operations per local
  maximum.
- **Required gain.** r = min(1, c/q) applies to **both** endpoints of the interval, where c is the
  linear ceiling attached to the audio (§4.3).

### 4.3 Gain computer
1. **Look-ahead hold.** h[n] = min r over the last L+1 values. Use a monotonic deque in a ring
   preallocated in `activate` for the largest L at that rate; O(1) amortized, RT-safe.
2. **Release.** e[n] = min(h[n], min(1, e[n−1]·k_r)), with k_r = 10^(12 / (20 · release_s · fs)):
   a linear-in-dB recovery of 12 dB per release time, clamped exactly at 1.0.
3. **Attack smoothing.** G = two cascaded moving averages of length L/2 + 1 each.
   - Their combined support is L+1 samples and lies inside the hold window, so every value averaged
     into the gain of a peak sample is ≤ that peak's requirement. **Output ≤ ceiling holds by
     construction** (on the detector's estimate), with an S-shaped attack spanning the look-ahead.
   - Running sums must not drift. Keep a count of the non-unity values in each window (or re-sum
     exactly every window length), so that a window of all 1.0 yields **exactly** 1.0. That is what
     makes AC-8 bit-exact.
4. **Output.** y[n] = G[n] · u[n − L − D], rounded once to f32.
5. **Telemetry.** Once per `process()` call, write 20·log10(min G over the block) (0.0 when
   G ≡ 1).

### 4.4 Parameter timing and smoothing (latency-aware, ADR-005 §4, `ModuleTestHost`)
- **The rule.** A look-ahead limiter would react to a change *before* the audio carrying it reaches
  the output. To keep the rule "an event at offset k changes nothing before output sample
  k + latency", the module delays events internally:
  - **`input_gain_db` and `ceiling_dbtp`** are applied to the input sample at k + **L**. From there
    they are per-sample signals that travel with the audio: g_in multiplies u, and c is attached to
    each sample and reaches the r computation D samples later. The gain computer can then start
    preparing for the new values at output sample exactly k + latency, and every sample is judged
    against its own ceiling. So the ceiling guarantee also holds *during* ceiling ramps.
  - **`release_ms`** is applied to the gain computer at k + L + D.
  - **`lookahead_ms`**: a new value different from the active one calls
    `ctx.request(HostRequest::Restart)` in the same `process()` call. The live instance keeps its old
    look-ahead, and the replacement is activated with the new value (T-401).
- **Pending events** wait in a queue preallocated in `activate` (capacity 1 024). On overflow the
  newest value overwrites the last queued entry of the same parameter (latest wins; values are
  never lost).
- **`reset()`** applies pending events at once, snaps ramps to their targets, and clears the delay
  line, detector, hold, release and averages (G = 1).
- **Ramps.**
  - Ceiling: 20 ms, linear in dB.
  - Input gain: **100 ms**, linear in linear gain; the first ramp sample already moves, and the
    last equals the target (the TestGain convention).
  - **Decided (autonomous, T-400): 100 ms for input gain.** The spec simulation measured the input
    gain step (normalized 0.25 → 0.75 = −3 → +15 dB) on a tone that is being limited:

    | Ramp length | Worst excess |
    |---|---|
    | 20 ms | fails by up to +8.9 dB |
    | 50 ms | fails by +2.0 dB |
    | 100 ms | passes, −3.5 dB worst |

    The cause is the per-peak gain staircase while the level rises fast. That is legitimate limiter
    behavior, but it's audible as grit on a fast boost. A 100 ms glide on an input boost is
    unobtrusive.
- **`ModuleTestHost`** declares `allow_delayed_effect` for all four parameters: input gain first
  shows at k + L + latency when nothing is limited, ceiling and release act like thresholds or
  rates, and look-ahead changes only through replacement.

### 4.5 RT, precision, cost
- All buffers are sized in `activate` for the maximum look-ahead at that rate (≤ 961 hold entries
  and a delay line of ≤ 976 samples at 96 kHz). `process()` never allocates. Internal state is f64.
- Denormals: G is ≥ its minimum and exactly 1 at rest, and the FIR state is f32 input data, so
  there is no decaying recursion. The host's FTZ/DAZ covers subnormal inputs.
- Deterministic and block-independent: realtime equals offline (AC-15).
- **Budget:** ≤ **1.5 %** of one core at 48 kHz in `just bench`. Estimate: 128 MAC plus O(1) hold and
  averages per sample, ≈ 0.5 %.

### 4.6 testkit true-peak measurement (added by T-404)
- **`true_peak_reference_dbtp(samples, rate)`** is the normative meter for this spec.
  - 16× polyphase Kaiser-windowed sinc, 128 taps per phase (half-width 64 input samples), designed
    for 150 dB (β = 15.6), plus the same parabolic refinement at every local maximum.
  - Input is zero-padded; test signals start and end with ≥ 5 ms fades and 50 ms of silence, so
    truncation doesn't create Gibbs peaks.
  - Accuracy (spec simulation): **≤ 0.0014 dB** error on sines from 997 Hz to 20 kHz at random
    phases, at 44.1 and 48 kHz.
  - Test-only cost is ≈ 2 k MAC/sample.
- **`true_peak_4x_dbtp`** is the plain 4× points of the same interpolator: PROMPT §5's literal
  "4×-oversampled peak".
- **`ebur128`** (`precision-true-peak`, 48-tap Hann-windowed 4× at < 96 kHz, 2× at 96 kHz) is kept
  as a cross-check with its measured error. On faded sines it reads −0.20 … +0.105 dB (+0.104 at fs/4,
  45°). On limited band-limited noise it reads **up to +0.20 dB high at 44.1 kHz** and +0.10 dB at
  48 kHz. So MEMORY's +0.10 dB caveat is the fs/4 case only, not the worst case.

## 5. Acceptance criteria
**Stress set S** (generated in code; all content ≤ 20 kHz, with 5 ms fades and 50 ms silence
padding; 44.1, 48 and 96 kHz unless stated):
- the fs/4 pattern `+a +a −a −a` at a = 1.0 (true peak +3.01 dBFS);
- 1 s sines at 997 Hz, 5 kHz, 10 kHz, fs/4, 15 kHz, 18 kHz and 19.9 kHz, at 0 dBFS, with random
  phases;
- a 5 s log sweep from 20 Hz to 20 kHz at 0 dBFS;
- 3 s of white noise at −6 dBFS RMS, low-passed at 19.5 kHz (Kaiser FIR, 110 dB);
- band-limited clicks (19.5 kHz windowed-sinc pulses at random sub-sample positions), sparse (40)
  and dense (2 000), peaks 0.5–1.0;
- 997 Hz tone bursts with abrupt on/offsets, then band-limited;
- testkit `voice_like` at −6 dBFS peak;
- pink noise at −14 dBFS RMS.

Each is run with ceilings {−12, −3, −1, −0.1} dBTP × input gains {0, +6, +12, +24} dB, at default
release and look-ahead, and with look-ahead 1 ms and 10 ms and release 10 ms and 1 000 ms at
ceiling −1.

- **AC-1 (schema, identity, host obligations).**
  - The descriptor, `params()` and telemetry channels equal §2.1/§3 exactly; `validate_schema`
    passes.
  - `ModuleTestHost` passes at 44.1/48/96 kHz, with `allow_delayed_effect` for all four parameters
    (§4.4).
  - `extension(Telemetry)` returns `TelemetryCells`; `extension(ResponseCurve)` is None.
- **AC-2 (latency is exact).**
  - For fs ∈ {44.1, 48, 96} kHz and look-ahead ∈ {1, 5, 10} ms, `latency_samples()` = L + D as in
    §3 (48 kHz / 5 ms → 256).
  - An impulse of 0.25 at input sample 1 000 (below every ceiling ≥ −12 dBTP) appears bit-exact at
    output sample 1 000 + latency, with zeros elsewhere.
  - An offline render through the rack keeps it at sample 1 000 (SPEC-012 AC-10).
- **AC-3 (ceiling guarantee — normative, PROMPT §5).** For every run of set S:
  - `true_peak_reference_dbtp(output)` ≤ **ceiling + 0.10 dB** (simulation worst: +0.027 dB);
  - `true_peak_4x_dbtp(output)` ≤ ceiling + 0.10 dB (the literal PROMPT §5 reading);
  - the sample peak is ≤ ceiling (exactly, allowing 1 f32 ULP).
- **AC-4 (ebur128 cross-check).** At 44.1 and 48 kHz, for every run of set S,
  `measure::loudness(..).true_peak_dbtp` of the output is ≤ **ceiling + 0.25 dB**. That is the
  0.10 dB guarantee plus ebur128's measured over-read of up to +0.20 dB (simulation worst +0.20).
  The test also prints ebur128's reading next to the reference, so drift in either shows up.
- **AC-5 (reference meter accuracy).** `true_peak_reference_dbtp` reads the analytic peak within
  **±0.005 dB** at 44.1/48/96 kHz for 16 phases each of sines from 20 Hz to 0.45·fs (faded and
  padded). The testkit fs/4, 45° case (sample peak −6.00 dBFS) reads −2.990 ± 0.005 dBTP.
- **AC-6 (no over-limiting).** Given a steady sine (997 Hz, 5 kHz, 10 kHz, 15 kHz and 19.9 kHz,
  0 dBFS, 48 kHz) with ceiling −1.0 dBTP, the steady output true peak is **−1.00 ± 0.05 dBTP**
  (simulation ±0.008) and telemetry reads −1.0 ± 0.1 dB. For a −6 dBFS 997 Hz sine with +6 dB input
  gain, the output true peak is −1.00 ± 0.02 dBTP.
- **AC-7 (attack happens before the peak).**
  - Given digital silence followed by a 997 Hz burst at +6 dBFS (ceiling −1), no output sample
    exceeds the ceiling (AC-3), and G first drops below 1 exactly L samples of output time before
    the burst's first over-ceiling interval.
  - The gain trajectory from 1 down to the required value is monotonic and spans L + 1 samples.
- **AC-8 (transparency, bit-exact).** Input gain 0 dB, ceiling −1:
  - Given pink noise at −20 dBFS RMS or `voice_like` at −6 dBFS peak (both with reference true peak
    ≤ −1.1 dBTP), the output equals the input delayed by the latency **bit-identically**, and
    telemetry reads 0.0.
  - After the AC-10 release episode, once G has returned to exactly 1.0, the output is again
    bit-identical to the delayed input.
- **AC-9 (distortion).** THD+N = 10·log10(residual power / fundamental power) after a least-squares
  fit of the fundamental and DC over 0.5–1.9 s of the output. With defaults and ceiling −1:
  - a 997 Hz and a 1 000 Hz sine at −6 dBFS with +6 dB input gain (≈ 1 dB GR), at 44.1/48/96 kHz:
    THD+N ≤ **−80 dB** (simulation worst −88.8 dB, at 44.1 kHz / 1 kHz);
  - the same with +18 dB input gain: ≤ −80 dB;
  - 80 Hz with +6 dB: ≤ **−55 dB** (simulation −65.5 dB).

  Informative (printed, not gated): 50 Hz −44 dB; 100 Hz with look-ahead 1 ms and release 10 ms
  −28 dB. The generic UI's parameter descriptions note that look-ahead shorter than half the period
  of the lowest fundamental distorts bass.
- **AC-10 (release timing).** Given 997 Hz at 0 dBFS with +12 dB input gain (GR ≈ 13 dB) for 1 s,
  then the same tone at −40 dBFS, at 48 kHz and look-ahead 5 ms:
  - the applied gain rises from −12.5 dB to −0.5 dB in **release ± 2 %** for release 100 ms and
    1 000 ms (simulation 100.00 / 1 000.00 ms), and in 10.0–12.0 ms for release 10 ms (simulation
    10.83 ms);
  - it reaches exactly 1.0 no later than release/12 + look-ahead after crossing −0.5 dB;
  - it never rises before the look-ahead window has passed the last over-ceiling peak.
- **AC-11 (no zipper noise, SPEC-012 AC-5, §4.3 analysis unchanged).**
  - **`ceiling_dbtp`** (T_s 20 ms). SPEC-012's −20 dBFS tone lies below the whole 0.25–0.75 span
    (−9 … −3 dBTP), so the substitute signal is a 997 Hz sine at **0 dBFS**. Both directions and the
    drag variant pass (simulation −39.3 / −46.5 / −46.3 dB).
  - **`input_gain_db`** (T_s 100 ms) passes with the standard −20 dBFS tone, both directions and the
    drag variant (simulation −23.1 dB). It also passes with the limiting-active substitutes at
    −6 dBFS and 0 dBFS (simulation −9.4 and −3.5 dB; drag −32 dB).
  - **`release_ms`** runs formally; it has no effect on a steady limited tone.
  - **`lookahead_ms`** goes through the host replacement crossfade, which passes §4.3 with
    T_s = 15 ms in the T-401 rack integration test.
  - Realtime (random blocks) and offline modes.
- **AC-12 (parameter timing and ceiling changes).**
  - For every parameter, an event at offset k leaves output samples before k + latency bit-identical
    to the no-event run.
  - Given a 0 dBFS 997 Hz sine and a ceiling event −1 → −6 dBTP at k, output samples from
    k + L + round(0.020·fs) + latency on have a reference true peak ≤ −5.9 dBTP.
  - Every output sample during the ramp stays ≤ its own attached ceiling + 0.10 dB.
- **AC-13 (look-ahead change).**
  - A `lookahead_ms` event with a new value requests `HostRequest::Restart` in the same call, and
    the live instance's output is unchanged by it.
  - A fresh instance loaded with the new value reports the new L + D.
  - With T-401, the rack's latency readout and heard-position offset update within 100 ms
    (SPEC-012 AC-8).
- **AC-14 (telemetry).**
  - After each block, `read(0)` returns the minimum applied gain in dB since the previous read,
    within 0.01 dB.
  - Two blocks with minima −3 and −1 dB, read once, give −3.
  - It is exactly 0.0 while transparent, and never positive or NaN.
- **AC-15 (realtime equals offline; deterministic; RT-safe).**
  - Given voice_like + pink noise with 30 events at fixed positions, realtime processing (random
    blocks 1…1024 including 0-length flushes) and offline processing differ by **≤ 1e-6**.
  - Two offline renders are bit-identical.
  - `process()` and `reset()` never allocate (`ModuleTestHost`).
- **AC-16 (robustness).** With FTZ/DAZ **off**:
  - ±4.0 squares with +24 dB input gain meet AC-3 with finite output;
  - 60 s of f32 subnormal input (1e-40) and of silence are processed with median block time ≤ 1.5 ×
    that of noise, and give exact zeros out.
- **AC-17 (performance).** ≤ 1.5 % of one core at 48 kHz, 256-frame blocks, in `just bench` on the
  owner's machine.
- **AC-18 (manual smoke, owner).**
  - A voice take pushed with +8 dB input gain into −1.0 dBTP: `powervoice-cli analyze` reports true
    peak ≤ −0.75 dBTP (ebur128) with no audible pumping or grit at defaults.
  - The gain-reduction meter moves only on peaks.
  - Changing the look-ahead during playback is click-free, and the latency readout follows.
  - A flat passage below the ceiling nulls against the bypassed slot.

## 6. Test plan
| AC | Unit (`dsp` / `modules` / `testkit`) | Integration | Manual |
|---|---|---|---|
| AC-1 | schema equality; `ModuleTestHost` | — | — |
| AC-2 | latency table; impulse alignment | offline render alignment (`rack`) | latency readout |
| AC-3 | set S × settings vs `true_peak_reference_dbtp` / `_4x` (long matrix under `just test-big`; a 48 kHz subset in `just test`) | — | — |
| AC-4 | set S vs `measure::loudness` (44.1/48 kHz) | — | `powervoice-cli analyze` |
| AC-5 | reference meter vs analytic sines | — | — |
| AC-6 | steady sines | — | — |
| AC-7 | burst onset gain trajectory (test hook exposing G) | — | — |
| AC-8 | bit-exact delayed identity; return to unity | — | null vs bypass |
| AC-9 | THD+N fit | — | listen at defaults |
| AC-10 | release crossings | — | — |
| AC-11 | SPEC-012 §4.3 harness with substitutes | replacement crossfade (T-401) | ceiling/input drags while playing |
| AC-12 | pre-latency bit-identity; ceiling ramp | — | — |
| AC-13 | Restart request; new latency | rack restart + readouts (T-401) | look-ahead change while playing |
| AC-14 | TelemetryCells min-hold | meter publisher (T-410) | GR meter |
| AC-15 | `ModuleTestHost` | fake backend vs `rack::offline::render` | — |
| AC-16 | FTZ-off timing; ±4 squares | — | — |
| AC-17 | — | `just bench` | — |
| AC-18 | — | — | owner smoke list |

**Vertical-slice subset (lean first implementation).** A lean slice implements:
- the §4.2–§4.3 algorithm as specified (detector, hold, release, averages, exact unity);
- ACs: AC-1, AC-2, AC-3 (48 kHz, ceiling −1, input gains 0/+12), AC-6, AC-8, AC-10 (100 ms), AC-11
  (ceiling and input gain, offline), AC-15.

If T-401's Restart path is not yet in the slice, `lookahead_ms` may temporarily be flagged `HIDDEN`
and fixed at 5 ms (schema otherwise unchanged; report it). The rest is the hardening target.

**Fixtures.** All signals are generated in code (testkit generators plus the band-limiting FIR).
Nothing is committed.

## 7. Out of scope, contradictions, notes

**Out of scope:** auto-release or program-dependent release, stereo link, limiting at the
oversampled rate (up/down-sampling the audio path), soft-clip or saturation stages, dither, loudness
targeting (LUFS normalize, M6), sidechain, codec-aware ceilings, and a custom panel or
gain-reduction history graph.

**Contradictions and flags (resolved by the defaults above):**
1. **Module id.** The brief's `org.powervoice.tplimiter` conflicts with ADR-005 §2's
   `org.powervoice.true-peak-limiter`; this spec uses the ADR's id.
2. **PROMPT §5** ("no 4×-oversampled peak > −0.9 dBTP" at −1 dBTP) can't be verified with ebur128:
   its own 4× interpolator over-reads by up to +0.20 dB on band-limited noise at 44.1 kHz. So:
   - the normative check is the 16× testkit reference plus the literal plain-4× reading, both at
     +0.10 dB (AC-3);
   - the ebur128 check uses +0.25 dB (AC-4);
   - MEMORY's "+0.10 dB near fs/4" should be widened to "−0.2 … +0.2 dB depending on content".
3. **PROMPT §3.4** "4× oversampled detection" is kept, plus parabolic peak refinement. Plain 4×
   would overshoot by up to +0.26 dB on the reference.
4. **Event timing.** ADR-005 §4 says a change starts affecting output at sample k. For this module
   (latency L + D) every effect starts at k + latency or later, by design (§4.4). That is consistent
   with `ModuleTestHost`'s latency-aware check, and all four parameters use `allow_delayed_effect`.
5. **Smoothing.** Input gain smoothing is 100 ms, not the 20 ms other gain controls use, with
   measured justification (§4.4).
6. **Look-ahead** as a parameter relies on T-401's Restart handling (ADR-005 §8/§12), like NR's FFT
   size.

## Amendment 1 — S3-05 implementation (2026-09-13, orchestrator, autonomous)
- **Input-gain timing:** input gain changes apply at input sample **k + L + D** (look-ahead L plus the
  detector's D = 16-sample read-ahead), not k + L — otherwise the output gain would change before
  k + latency, breaking this spec's own "no output change before event + latency" rule. Ceiling stays
  at k + L; release at k + L + D.
- **Interval endpoint coverage:** with hold L+1 and averaging over L+1, the second endpoint of an
  interval misses only the last averaging tap (weight 1/(L/2+1)²). The sample-peak bound is exact; the
  true-peak bound is measured at ceiling + 0.011 dB worst (limit + 0.10 dB). Accepted for v1; an exact
  construction costs one more sample of latency — tracked as backlog H-03.

## Amendment 2 — H-03 hardening (2026-09-14, autonomous)
- **Exact interval-endpoint coverage (supersedes Amendment 1's measured bound).** Input sample n
  bounds two intervals, (n − 1, n) and (n, n + 1). The gain computer is fed the per-sample
  requirement ρ[n] = min(r(n − 1, n), r(n, n + 1)), known one sample after the second interval's
  detector reading. Every output sample's gain is then ≤ the requirement of both intervals it
  bounds, by construction (on the detector's estimate: what remains is the 4× + parabola
  detector's own under-read, §4.2).
- **Latency = L + D + 1** (§3 derived constants, §4.1 "delay L + D + 1", AC-2): 61/239/459 samples
  at 44.1 kHz, 65/257/497 at 48 kHz, 113/497/977 at 96 kHz for look-ahead 1/5/10 ms; the §4.5
  delay line is ≤ 977 samples at 96 kHz.
- **Event timing (§4.4, Amendment 1):** the ceiling is attached to the input sample at k + L + 1;
  input gain and release apply at k + latency (= k + L + D + 1). Nothing changes before output
  sample k + latency.
- **AC-7:** G first drops exactly L samples before the first endpoint of the burst's first
  over-ceiling interval.
- **Measured (H-03):** full AC-3/AC-4 matrix (`just test-big`, 1 440 runs): reference true peak
  worst +0.0245 dB over the ceiling at 44.1 kHz, +0.0167 dB at 48 kHz, +0.0028 dB at 96 kHz; plain
  4× +0.014 dB; sample peak exact; ebur128 +0.138 dB. AC-16 (60 s, FTZ/DAZ off): subnormal and
  silence blocks take 0.76× the noise block time. AC-17 (`just bench`): 0.23–0.27 % of one core at
  48 kHz, 256-frame blocks.
- **Gain-reduction meter (§2.3, T-410's generic part):** module telemetry channel descriptions
  travel with the rack state (`RackSlotDto.telemetry`); values arrive in `VXMT` frames (SPEC-016
  §4.12 layout) via `module_telemetry_subscribe`, read by the control thread at the telemetry rate.
  The slot header renders every `GainReduction` channel with `group: None` as a meter on the
  channel's own scale (limiter 0 … −24 dB), so Dynamics' total GR and the Noise Gate's gain show
  there too.
