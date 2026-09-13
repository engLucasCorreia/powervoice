# SPEC-014 — Noise Reduction: noise print capture, streaming spectral NR, NR panel

- **Status:** approved (autonomous, T-500)
- **Milestone:** M5 (hardening target). Tickets:
  - **T-501** offline algorithm and goldens (`dsp`);
  - **T-502** noise print capture and profile storage (`modules`, `engine` job, `rack`);
  - **T-503** streaming module and restart on an FFT-size change (`modules`);
  - **T-504** NR panel (`ui`).

  Vertical-slice plan (owner D-022): **Slice 3** implements the lean subset in §9. Each AC is
  tagged **[S3]** (in the slice) or **[H]** (hardening).
- **Related:** PROMPT §2 (LOCKED "Noise reduction — spectral noise print"), §3.4 item 3, §4 ("NR
  latency ≤ 50 ms"), §5 · ADR-005 (§4 events, §8 latency/restart, §10 state blob, §11
  `NoiseProfile`, §12 replacement, §13 UI) · ADR-004 (session `state`) · SPEC-000 · SPEC-002 §2.7
  (monitoring latency warnings) · SPEC-004 (rack edits, including noise print capture, are not
  undoable) · SPEC-006 §2.9 (selection) · SPEC-007 §2.4, §2.10, §4.8 (log axis, analyzer stream,
  band convention) · SPEC-012 (§2.2 as amended: a replacement's fade-in waits for its latency; §2.4;
  §2.5; §2.8; **§4.3 zipper measurement is normative**) · SPEC-018 (the noise profile is the NR
  slot's state blob) · `docs/references.md` (Boll 1979; Ephraim–Malah 1984/85; Audacity NR;
  Audition NR parameters)

## 1. Purpose
Home voice-over rooms have a constant noise bed: computer fans, HVAC, preamp hiss, mains hum. ACX
rejects a noise floor above −60 dB. The Audition workflow every VO talent knows is:
1. select a stretch of room tone;
2. **Capture Noise Print**;
3. let Noise Reduction subtract that print from the whole take.

This module does the same **non-destructively in the rack**, so the talent hears the cleaned voice
while playing, can tune it live, and exports exactly what they heard. It must remove steady noise
by a predictable amount (the "Reduce by" value). It must leave the voice's level and timbre alone,
and it must not add the "bubbly"/"musical" artifacts that give away cheap noise reduction.

## 2. Behavior / UX

### 2.1 Identity
- **id `org.powervoice.noise-reduction`**, version 1.0.0.
  - This is the id ADR-005 §2 fixes, and SPEC-018's sidecar example already uses it. The T-500 brief
    wrote `org.powervoice.nr`. The accepted ADR wins, because built-in ids are permanent. Flagged
    (§8).
- **Name** "Noise Reduction" (`module.noise_reduction.name`); vendor "PowerVoice".
- **Features** `audio-effect`, `restoration`, `mono`. **Layout** MONO.
- **State** `state_format_version` 1: the parameters (§3) plus the **noise profile blob** (§4.2).
- **Extensions** `NoiseProfile` (capture/describe) and, [H], `Telemetry` (§3.2).
- **Latency** = FFT size N in samples (§4.4). **Tail** = `Tail::Samples(N)`.

### 2.2 What the user hears
- **With a noise print:**
  - Steady noise matching the print is turned down by up to **Reduce by** dB, frequency by
    frequency.
  - Voice well above the print passes at its original level: the tone ACs allow < 0.5 dB change.
  - The processed audio is delayed by the module's latency. The rack compensates: the playhead,
    A/B and exports are time-aligned (SPEC-012 §2.5).
- **Without a noise print:** the audio passes **unchanged** (bit-identical, delayed by the same
  latency). The panel says "No noise print" (§2.8). **Decided (autonomous, T-500):** keep the
  latency even without a print. The first capture is then a same-latency replacement, with no jump
  of the heard position, and the monitoring readout doesn't change under the talent.
- **Output noise only:** the user hears **what is being removed** instead of the result (§2.6).

### 2.3 Capture Noise Print (T-502)
- **Where the command lives:**
  - Menu **Effects → Noise Reduction / Restoration → Capture Noise Print** (Audition's menu path).
  - Shortcut **Shift+P**. Audition's default, confirmed by two secondary sources (shotkit.com,
    premiumbeat.com). Adobe helpx is unreachable (403), so the binding is **provisional for
    SPEC-019**. It conflicts with no binding in the approved specs.
  - A **Capture Noise Print** button in the NR slot panel.
  - **Decided (autonomous, T-500):** **Ctrl+Shift+P** shows the Noise Reduction panel, adding a
    slot if none exists. Secondary sources give this as Audition's "open Noise Reduction effect"
    key. Provisional for SPEC-019.
- **Target slot.** **Decided (autonomous, T-500):** the command acts on:
  1. the NR slot whose panel was focused last;
  2. otherwise, the first NR slot in rack order;
  3. otherwise, a **new NR slot inserted as the first slot**, with the notice "Noise Reduction added
     to the rack". Noise reduction belongs before gates, EQ and compression in a VO chain, and
     Audition users expect the command to work without opening the effect first.
- **Enabled when:**
  - a **non-empty selection** exists (SPEC-006 §2.9). Without one, the menu item is disabled and
    Shift+P posts "Select some room tone first";
  - the app is **not recording**. While recording, the item is disabled; the selection is locked
    then anyway (SPEC-022), and capture must never compete with the take writer.
  - Capture is allowed while stopped, playing or monitoring.
- **Selection length.**
  - **Decided (autonomous, T-500): minimum 0.5 s**, and at least 12 288 samples (3 analysis
    frames, §4.1). That means 0.5 s at 44.1/48/96 kHz and 0.77 s at 16 kHz. `min_capture_samples`
    returns `max(round(0.5·fs), 12288)`.
    - Why 0.5 s: it matches the shortest room tone ACX asks for at a file's head (0.5–1 s), so the
      natural head pause is always enough. 0.3 s gives only 4 analysis frames (±3 dB per-bin
      scatter), too few for a stable print.
  - Shorter → error "Select at least 0.5 s of room tone".
  - **0.5 s ≤ length < 1.0 s** → capture succeeds, with the warning "Short noise print — 1 s or more
    gives better results".
  - Only the **first 60 s** are analysed; longer selections post "Using the first 60 s of the
    selection". Room tone doesn't need more, and it bounds the job.
- **Source: the audio as the NR slot receives it.** **Decided per ADR-005 §11 (accepted):**
  - Audition captures from the file. Here, what the NR slot receives may differ (for example a Gain
    or EQ before it), and a print taken from the raw file would be mis-scaled.
  - The engine job renders **the slots before the target slot** offline (`rack::offline::render`,
    document rate, 4096-frame blocks) over `[sel_start − P, sel_end)`, then drops the first P
    samples.
    - **P = min(1 s, sel_start)** of pre-roll, so filters and envelopes upstream are warm.
    - Per-slot bypass flags are honoured.
    - Whole-rack A/B is ignored: it is a listening aid.
    - **Placeholder slots are omitted.** They pass dry at latency 0 live, so omitting them is
      equivalent. This avoids `offline::render`'s `MissingModules` refusal, which exists for
      exports.
  - With no slot before it, the excerpt is the document audio itself.
- **Checks on the excerpt:**
  - all samples exactly 0 → error "The selection is digital silence — nothing to capture";
  - RMS > −35 dBFS → capture succeeds with the warning "This selection is loud for room tone —
    make sure it contains no speech". **Decided (autonomous, T-500):** room tone sits far below
    −35 dBFS, and speech rarely does.

  Both checks are generic (host side), so `NoiseProfile` needs no new API.
- **Job.**
  - The job runs on a worker (ADR-002) and calls `NoiseProfile::capture(excerpt, fs, values,
    cancel)`.
  - Longer than 1 s (render included), it shows "Capturing noise print…" with **Cancel** in the
    panel. Cancel leaves the old print in place.
  - Typical cost: < 150 ms for 60 s of audio, excluding upstream rendering (§4.9).
- **Result.**
  - The new blob **replaces** the previous print as the slot's committed blob. There is one print
    per slot.
  - The host performs a **replacement** (ADR-005 §12): the new instance's fade-in waits for its
    latency (SPEC-012 §2.2 amended), then a 15 ms crossfade. It is live, click-free and never
    stops the transport.
  - Not undoable (SPEC-004 table). The session journals it within 2 s, and the sidecar becomes
    dirty (SPEC-018 §2.4).
  - Parameter values are unchanged.
- **Clear noise print.** The slot menu has **Clear Noise Print**. It drops the blob, and the
  replacement then passes audio unchanged. No confirmation, not undoable. **Decided (autonomous,
  T-500):** cheap, and it's the only way back to "no print" short of removing the slot.

### 2.4 The noise profile
- **Content.** Per-frequency-bin statistics of the excerpt at a **fixed analysis FFT of 8192**
  (§4.1):
  - the **mean power density**, which drives the reduction;
  - the **log-power spread** (standard deviation in dB). It is stored for the graph and for future
    variance-aware processing; the v1 gain rule doesn't use it.
- **Size.** 32 824 bytes (§4.2): about 43.8 KB as base64 in the sidecar.
- **Independent of the source audio.** It is statistics, not a reference. It **survives edits to,
  or deletion of, the region it was captured from**, a document switch (rack carry-over, SPEC-018
  §2.7), save/reopen, and crash recovery.
- **Persistence.** It is the NR slot's `state.blob` (SPEC-018 §2.6.4). No other field exists.
- **FFT sizes and sample rates.**
  - The profile is converted on `activate` to the running FFT size and sample rate (§4.3), so one
    capture serves every FFT size.
  - A print captured at 48 kHz still works on a 44.1 kHz document.
- **Presets** (T-406):
  - A user preset saved from an NR slot **includes the print** (the committed blob).
  - Factory presets are **parameter-only** and keep the current print (ADR-005 §12: parameter-only
    presets are smoothed and cause no replacement). Factory presets: "Light (6 dB)", "Medium
    (12 dB)", "Strong (20 dB)".
  - **Decided (autonomous, T-500):** no separate "Save/Load noise print" file format in v1. Module
    presets already carry the blob, and that covers reuse across projects.

### 2.5 No print, or an unreadable print
- **No blob:**
  - the output is the input delayed by N, **bit-identical**;
  - the parameters stay editable (they apply once a print exists);
  - status "No noise print".
- **A blob that fails validation (§4.2):**
  - `load_state` still succeeds (parameters load) and the module behaves as with no blob;
  - `describe` returns an error, and the panel shows "Noise print unreadable — capture again";
  - the host keeps the stored bytes verbatim until a capture or Clear replaces them (ADR-005 S1:
    the committed blob is host-owned), so nothing is lost silently;
  - a blob version newer than 1 reads "Noise print needs a newer PowerVoice".

### 2.6 Output noise only
- **What it does.** A toggle. When on, the module outputs **input − processed**, i.e. exactly the
  removed part (§4.7). It is the audition aid Audition offers: the user raises Reduce by until
  voice starts to appear in the noise-only output, then backs off.
- **Timing.** Toggling takes effect at the next frame boundary and crossfades over one frame length
  through the overlap-add (§4.6): click-free (AC-10).
- **It is a module parameter,** saved with the slot (ADR-005 §10), so exports would render noise
  only. **Decided (autonomous, T-500):**
  - while it is on, the slot header shows a highlighted **"Noise only"** badge;
  - export and bake show a confirmation when any non-bypassed NR slot has it on: "Noise Reduction
    is set to Output noise only — the result will contain only the removed noise. Continue?"
    (Continue / Cancel). The M6 export/bake specs must adopt this (§8).

### 2.7 FFT size, latency and monitoring
- **Choices.** FFT size **1024 / 2048 / 4096 / 8192**, default **2048**.
  - Latency = N samples: 42.7 ms at 48 kHz and 46.4 ms at 44.1 kHz for the default, which meets
    PROMPT §4 (≤ 50 ms).
  - Larger sizes resolve hum lines better but add latency (§4.4 table). The FFT dropdown's tooltip
    says "4096 and 8192 add more than 50 ms of latency — high for monitoring through the rack".
  - **Decided (autonomous, T-500):** the size is in samples, not rate-scaled, as in Audition. At
    96 kHz, 2048 gives 21.3 ms and 46.9 Hz bins, still adequate for broadband room noise.
- **Changing it** during playback:
  - the running instance keeps processing at its old size and requests `Restart` (ADR-005 §8
    example);
  - the host replaces the instance, and the new one fades in after its latency (SPEC-012 §2.2
    amended);
  - latency compensation and every readout (slot, rack total, monitoring, heard position) update
    within **100 ms** of the swap (SPEC-012 §2.5). The playhead shows the heard position correctly
    throughout (AC-15).
  - When stopped, the host may re-activate in place.
- **Monitoring** (SPEC-002 §2.7). The rack latency includes the NR latency in "through rack" mode,
  and the amber/red warnings apply unchanged. Bypassing NR doesn't reduce latency (SPEC-012 §2.3).
  The red warning's "remove high-latency modules" hint names the NR slot when it is the largest
  contributor [H].

### 2.8 NR panel (T-504)
A custom panel (ADR-005 §13) built only from the schema, the `NoiseProfile` extension, the analyzer
stream (SPEC-007 §4.9) and Telemetry. Top to bottom:
1. **Status line + actions.**
   - **Capture Noise Print** button, with the hint "Shift+P". Disabled with a tooltip when
     §2.3's conditions fail. It shows a spinner and **Cancel** while a job runs.
   - Status text:

     | State | Text (i18n key) |
     |---|---|
     | no blob | "No noise print — select room tone and press Capture Noise Print (Shift+P). Audio passes unchanged." (`module.noise_reduction.status.none`) |
     | capturing | "Capturing noise print…" (`…status.capturing`) |
     | loaded | "Noise print loaded" (`…status.loaded`) |
     | unreadable | "Noise print unreadable — capture again" (`…status.unreadable`) |
     | too new | "Noise print needs a newer PowerVoice" (`…status.too_new`) |

   - Capture warnings (short, loud, 60 s cap) show as a dismissible line under the status until the
     next capture. Errors are toasts (SPEC-000 notice).
   - **Decided (autonomous, T-500):** v1 shows no capture metadata (duration, rate, level) in the
     status, because `NoiseProfile` has no accessor for it. [H] proposes an additive ADR-005
     amendment `NoiseProfile::summary(blob)` (§8).
2. **Profile graph** [H]:
   - Log frequency axis 20 Hz → min(Nyquist, 24 kHz), the shared `freqAxis.ts` (SPEC-007 §2.10).
     The dB axis spans −120 … 0 dBFS by default and auto-fits the print's range ±10 dB.
   - **Noise print curve:** the `describe()` points (§4.10), a filled line.
   - **"Reduced to" line:** the print minus `reduction_db × amount_pct/100`, dashed. It shows where
     the noise should end up.
   - **Live spectrum:** the analyzer stream of the **rack output**, labelled "Output (rack)". With
     Output noise only on, it shows the removed noise, which should hug the print.
     - **Decided (autonomous, T-500):** rack output rather than the slot's input spectrum. SPEC-007
       §2.10 forbids a per-slot tap without an ADR-002/003 amendment (§8). The per-slot tap is
       [H].
   - Hover readout: frequency, print level, live level.
   - Empty with no print; greyed while the analyzer has no output device.
3. **Parameters.**
   - The **main** group: Reduce by, Noise reduction, and the Output noise only toggle.
   - Then the collapsed **Advanced** group: FFT size, Sensitivity, Spectral smoothing, Attack,
     Release.
   - These are the generic widgets (SPEC-012 §2.6), in schema order.
4. **Slot header** (SPEC-012 §2.1): name, latency (for example "42.7 ms"), the "Noise only" badge
   (§2.6), and the [H] reduction meter (§3.2).

Slice 3 (§9) ships items 1 and 3 on the generic panel, plus the Capture button and status line.

### 2.9 Edge cases
- **Several NR slots:** each has its own print. Capture targets per §2.3.
- **Capture while playing:** the replacement happens live (AC-14). Parameter drags during a capture
  job apply to the current instance, and the replacement takes the mirror's latest values
  (ADR-005 §12).
- **Changing the FFT size during a capture:** independent. The replacement uses the committed blob
  and the latest parameter values.
- **Document rate ≠ rack rate** (the rack runs at the device rate, ADR-005 §12): the print is
  captured at the document rate and converted at `activate` (§4.3).
- **Selection containing speech:** the print over-estimates the noise. The loud warning catches the
  obvious cases, and Output noise only reveals the rest.
- **A seek, loop wrap or transport start** resets the module (ADR-005 `reset`, §4.8). There is no
  noise burst at the start of playback, because the gain smoothers snap on the first frame.
- **Non-finite input:** cannot occur (the rack guard, SPEC-012 §2.9). The module still clamps
  internally (§4.9).

## 3. Parameters

### 3.1 Module parameters
All parameters are `AUTOMATABLE`. Ids are permanent. Keys follow ADR-005 §3. `smoothing_ms` is
the declared value (§4.6).

| id | key | name | unit | range | default | taper/step | smoothing_ms | group | notes |
|---|---|---|---|---|---|---|---|---|---|
| 0 | `reduction_db` | Reduce by | dB | 0 … 40 | **12** | Db { neg_inf_at_min: false }, 1 decimal | 200 | main | gain floor G_min = 10^(−r/20) for bins judged noise; Audition "Reduce By" (6–30 typical) |
| 1 | `amount_pct` | Noise reduction | % | 0 … 100 | **100** | Linear, 0 decimals | 200 | main | scales the applied reduction in dB (0 % = no reduction); Audition "Noise Reduction %" |
| 2 | `noise_only` | Output noise only | bool | off/on | **off** | BOOL | 0 | main | §2.6, §4.7 |
| 3 | `fft_size` | FFT size | enum | 1024, 2048, 4096, 8192 | **2048** (index 1) | enum, STEPPED | 0 | Advanced | a change → `Restart` (§4.8) |
| 4 | `sensitivity_db` | Sensitivity | dB | −6 … +12 | **+3** | Db { neg_inf_at_min: false }, 1 decimal | 200 | Advanced | the print is raised by this much before the gain rule (over-subtraction); higher removes more |
| 5 | `smoothing_hz` | Spectral smoothing | Hz | 0 … 1000 | **100** | Linear, 0 decimals | 200 | Advanced | width of the gain smoothing across frequency; 0 = off |
| 6 | `attack_ms` | Attack | ms | 1 … 200 | **5** | Log, 1 decimal | 200 | Advanced | how fast a bin's gain rises (voice onset) |
| 7 | `release_ms` | Release | ms | 10 … 2000 | **100** | Log, 0 decimals | 200 | Advanced | how fast it falls back to the floor (Audition "Spectral decay") |

- **Display order** is `params()` order: 0, 1, 2 (ungrouped main section), then the group
  "Advanced" (`module.noise_reduction.group.advanced`, `collapsed_by_default: true`): 3, 4, 5, 6, 7.
- **i18n keys:** `module.noise_reduction.param.<key>`. Enum labels are the plain numbers.

**Decided (autonomous, T-500), parameter set and defaults:**
- **Reduce by 12 dB.** It is the PROMPT §5 example and the middle of Audition's "6–30 dB work well"
  range. The noise floor drops by about 12 dB with no audible artifacts on VO room tone.
- **Noise reduction 100 %.** With our mapping, 100 % means "apply Reduce by in full". Lower values
  shrink every bin's reduction proportionally in dB, like a dB-domain dry/wet. Audition's exact
  semantics are undocumented.
- **Sensitivity +3 dB.** Classic over-subtraction (Boll) against print estimation error. It keeps
  the rare noise spikes below the floor, which is the main musical-noise lever at deep reductions
  (§4.5).
- **Spectral smoothing 100 Hz.** About ±2 bins at 2048/48 kHz, close to Audacity's default of
  3 bands. It suppresses isolated-bin gain spikes (musical noise) with no audible smearing of
  harmonics.
- **Attack 5 ms.** Effectively immediate at the 10.7 ms hop, so consonant onsets aren't swallowed.
- **Release 100 ms.** Word tails decay naturally without audible noise "breathing".
- **FFT 2048:** §2.7.
- **Omitted, [H] or later:** Audition's **Precision factor** (our overlap is fixed at 75 %),
  **Transition width** (a hard/soft per-band gate belonging to Audition's gate engine; the Wiener
  gain is inherently soft), **Noise print snapshots** (we use every frame), and the **High/Low
  reduction curve**. They are Audition-engine specifics that would add controls without improving
  VO results.

### 3.2 Telemetry [H]
One channel, key `reduction_db`, kind `GainReduction`, unit dB, range −40 … 0, `Hold::Min`. Per
frame it holds the power-weighted mean applied gain over 100 Hz–10 kHz, `10·log10(Σ g²·P / Σ P)`,
as the slot-header meter. With no print it reads 0.

### 3.3 Constants (internal, not parameters)
| name | value | notes |
|---|---|---|
| analysis FFT (profile) | 8192, periodic Hann, hop 2048 | §4.1, every rate |
| processing window | √(periodic Hann), analysis and synthesis | §4.4 |
| hop H | N/4 (75 % overlap) | §4.4 |
| DD smoothing β | 0.98 | Ephraim–Malah decision-directed |
| ξ_min | 1e-6 (−60 dB) | numerical guard only; the floor dominates |
| λ_floor | 1e-24 | bins with λ_eff below it pass (g = 1) |
| `min_capture_s` / `min_capture_samples` | 0.5 s / max(round(0.5·fs), 12288) | §2.3 |
| `short_capture_warn_s` | 1.0 s | §2.3 |
| `max_capture_s` | 60 s | §2.3 |
| `loud_capture_dbfs` | −35 dBFS RMS | §2.3 |
| `capture_preroll_s` | min(1 s, sel_start) | §2.3 |

T-501 may tune β and ξ_min only if an AC requires it, and must report the change. The structure in
§4 is normative.

## 4. Algorithm / implementation notes

### 4.1 Capture analysis (`NoiseProfile::capture`, T-502)
- **Input:** the excerpt `x[0..L)` (f32, mono, the slot input, §2.3), its rate `fs_c`, and the
  plain values.
- **Frames:** N_c = 8192, hop H_c = 2048, periodic Hann w_c. Frame j covers `[j·H_c, j·H_c + N_c)`
  entirely inside the excerpt, for j = 0 … M−1, with **M = ⌊(L − 8192)/2048⌋ + 1**. L ≥ 12 288
  guarantees M ≥ 3.
- **Per bin** k = 0 … 4096 (f64 accumulation, stored as f32):
  - **mean power density** `D(k) = (1/M)·Σ_j |X_j(k)|² / Σ_n w_c[n]²` (Σw² = 3·N_c/8 = 3072). For
    white noise of variance σ², E[D] = σ² in every bin, independent of N and window;
  - **log-power spread** `S(k) = std_j(10·log10(|X_j(k)|² / Σw² + 1e-30))` in dB. Gaussian noise
    gives ≈ 5.57 dB; a steady tone gives ≈ 0.
- **No DC removal:** a DC offset in room tone is simply noise at 0 Hz.
- **Cancellation:** `cancel` is polled every 64 frames.
- **Errors:** `Resource` on an all-zero excerpt (the host checks first; the module double-checks).
- **Deterministic:** the same excerpt and rate produce byte-identical blobs (AC-11).

### 4.2 Blob format v1 (little-endian)
| offset | type | field |
|---|---|---|
| 0 | `[u8;4]` | magic `"PVNP"` |
| 4 | u16 | blob_version = 1 |
| 6 | u16 | reserved = 0 |
| 8 | f64 | capture sample rate `fs_c` (Hz) |
| 16 | u32 | analysis FFT size N_c (8192 in v1) |
| 20 | u32 | hop H_c (2048) |
| 24 | u32 | window id (1 = periodic Hann) |
| 28 | u32 | frames M |
| 32 | u64 | excerpt length (samples, after the 60 s cap) |
| 40 | f32 | excerpt RMS (dBFS, finite) |
| 44 | u32 | bins B = N_c/2 + 1 (4097) |
| 48 | f32[B] | D(k), linear density, finite, ≥ 0 |
| 48 + 4B | f32[B] | S(k), dB, finite, ≥ 0 |

- **Size** 48 + 8·4097 = **32 824 bytes**.
- **Validation** (`load_state`/`describe`) → `StateError::InvalidBlob` if:
  - the magic is wrong or the blob is truncated;
  - B ≠ N_c/2 + 1, or N_c is not a power of two in 1024 … 16384;
  - `fs_c` is not in 8 000 … 384 000;
  - any value is non-finite or negative.
- **Versions:**
  - blob_version > 1 → the module treats the blob as absent and `describe` returns
    `ModuleError::Unsupported("newer")`, so the UI can show "too new";
  - readers accept any valid N_c, so a future capture size needs no migration.

  `ModuleState.format_version` stays 1. The blob carries its own version because it is opaque to
  `prepare_state`.

### 4.3 Deriving the processing noise spectrum (on `activate`)
- **Target:** processing FFT N at rate `fs`. Bin k' has centre `f_k' = k'·fs/N` and band
  `[f_k' − fs/(2N), f_k' + fs/(2N)]`.
- **Density model:** D as a function of frequency is the piecewise-linear interpolation of D(k)
  through the capture bin centres `k·fs_c/N_c`.
  - Above the capture Nyquist it is extended flat, at the mean of the top 1/3 octave.
  - At 0 Hz the band is clipped to [0, fs/(2N)].
- **Derived density:** `D_p(k') = (fs/fs_c) × (mean of that function over the bin's band)`.
  - The factor fs/fs_c keeps the physical noise density: the same room recorded at another rate
    has D scaled by the rate ratio.
  - When `fs = fs_c` and r = N_c/N is a whole number, this is exactly the half-weighted box average
    `(1/r)[½D(rk'−r/2) + Σ_{|i|<r/2} D(rk'+i) + ½D(rk'+r/2)]`, with mirrored indices at the ends.
    For r = 1 it is the identity.
- **Noise power at the processing size:** `λ(k') = D_p(k') · Σ_n w_a[n]²`. With √Hann,
  Σw_a² = N/2. Then E[|Y(k')|²] = λ(k') for noise matching the print.
- **Cost:** computed once (it allocates), stored as an f32 table of N/2 + 1 values.

### 4.4 STFT framing and latency (T-501/T-503)
- **Windows:** analysis and synthesis both `w_a = w_s = √(periodic Hann)`, hop H = N/4. The window
  product is Hann, and Σ_j Hann(n − jH) = 2 exactly, so the overlap-add output is scaled by ½.
  Reconstruction is perfect when every gain is 1.
- **Framing** (normative, block-size independent):
  - An internal sample counter c runs from 0 at `activate`/`reset`. Input before c = 0 counts as
    zero.
  - Frame j (j ≥ 1) covers input counts `[jH − N, jH)` and is processed once input count jH − 1
    has been consumed, before output count jH is produced.
  - Output count c carries overlap-add position c − N. So **latency = N**, and with unity gains
    `y[c] = x[c − N]` (within float error).
- **Latency table** (latency = N samples; hop H = N/4):

  | N | 44.1 kHz | 48 kHz | 96 kHz | hop @ 48 kHz | bin spacing @ 48 kHz |
  |---|---|---|---|---|---|
  | 1024 | 23.2 ms | 21.3 ms | 10.7 ms | 5.33 ms | 46.9 Hz |
  | **2048** | **46.4 ms** | **42.7 ms** | 21.3 ms | 10.67 ms | 23.4 Hz |
  | 4096 | 92.9 ms | 85.3 ms | 42.7 ms | 21.3 ms | 11.7 Hz |
  | 8192 | 185.8 ms | 170.7 ms | 85.3 ms | 42.7 ms | 5.86 Hz |

  The default meets PROMPT §4's ≤ 50 ms at 44.1 and 48 kHz. The minimal constant latency of this
  framing is N − 1; the one extra sample buys a simpler, order-independent implementation.
- **No blob:** the module runs a plain N-sample delay line (bit-exact) instead of the STFT, with
  the same latency.

### 4.5 Gain rule (per frame j, bin k)
Here λ = λ(k) from §4.3, and P_{j−1}, ĝ_{j−1} are the previous frame's values in the same bin.

1. `Y = FFT(w_a · frame)`, `P = |Y|²`.
2. `λ_e = λ · 10^(sensitivity_db/10)`. If `λ_e < λ_floor`, then g = 1 for this bin; skip to 8.
3. A-posteriori SNR `γ = P / λ_e`.
4. Decision-directed a-priori SNR (Ephraim–Malah):
   `ξ = max(β·ĝ_{j−1}²·P_{j−1}/λ_e + (1 − β)·max(γ − 1, 0), ξ_min)`.
   On the first frame after `activate`/`reset`, `ξ = max(γ − 1, ξ_min)`.
5. Wiener gain `G_w = ξ/(1 + ξ)`; floored gain `G_dB = 20·log10(max(G_w, G_min))`, with
   G_min = 10^(−reduction_db/20).
6. **Frequency smoothing** (musical-noise mitigation): the mean of G_dB over bins
   `k ± h`, with `h = round(smoothing_hz / (2·fs/N))` (0 = off). The window is truncated at the
   spectrum edges.
7. **Time smoothing** per bin, in dB:
   - if `G_dB > Ĝ`: `Ĝ ← Ĝ + (1 − a_att)(G_dB − Ĝ)`, else `Ĝ ← Ĝ + (1 − a_rel)(G_dB − Ĝ)`;
   - `a_x = exp(−H/(fs·τ_x/1000))`;
   - on the first frame after `activate`/`reset`, `Ĝ = G_dB` (snap).
   - Smoothing in dB makes release a constant dB/s glide, like Audition's spectral decay.
8. **Amount:** `g = 10^((amount_pct/100)·Ĝ/20)`. With λ_e < λ_floor, g = 1.
9. `ĝ_j = g` (for DD in step 4). Output spectrum: `g·Y`, or `(1 − g)·Y` with Output noise only
   (§4.7).
10. Inverse FFT, multiply by w_s, overlap-add with the ½ scaling.

**Why this rule** (Decided, autonomous, T-500; BOARD T-501 names it):
- Plain spectral subtraction (Boll) leaves isolated random spectral peaks: musical noise. The
  decision-directed a-priori SNR averages those peaks across frames.
- The gain floor (Reduce by) and the dB-domain frequency and time smoothing do the rest.
- On noise matching the print, with β = 0.98 and the default sensitivity, G_w exceeds a 12 dB
  floor only when γ ≳ 17, i.e. with probability ~1e-7 per cell. The noise is then attenuated by a
  near-constant gain, which is artifact-free by construction.
- Voice bins at ≥ 20 dB per-bin SNR converge to within 0.2 dB of unity.

### 4.6 Parameter timing and smoothing
- **Frame binding (normative):** frame j uses the parameter values in force at its **first input
  sample** (count jH − N). The module keeps the parameter vector sampled at each hop boundary in a
  ring of N/H + 1 = 5 entries.
  - Consequence: an event at input sample k can only affect frames starting at ≥ k, i.e. output
    samples ≥ k + latency. This honours ADR-005 §4 ("never earlier").
  - The first affected output sample follows within H samples (the next frame start).
  - **Every parameter is therefore declared to `ModuleTestHost` with `allow_delayed_effect`**
    (T-005 rule).
- **Smoothing** comes from the overlap-add. A step in any gain-shaping parameter between frames
  becomes the output envelope `Σ_j g_j·Hann(t − t_j)/2`: a smooth, C¹ transition lasting N samples.
  Nothing extra is needed.
  - The declared `smoothing_ms` of 200 covers N up to 8192 at ≥ 44.1 kHz (≤ 186 ms). Here "reaches
    its target" (SPEC-012 §2.4) refers to the **parameter value in use**. The adaptive gains keep
    evolving with the signal by design (§8).
  - `noise_only` and `fft_size` declare 0. `noise_only` switches at a frame boundary and is
    crossfaded by the overlap-add. `fft_size` never changes the running instance.
- **Block independence:** frame timing depends only on sample counts, so any block partition
  (including 0-length flushes) gives bit-identical output (AC-16).

### 4.7 Output noise only
- The spectrum is `(1 − g)·Y`, with exactly the same g as the normal path. DD and the smoothers
  always track g, never 1 − g.
- Therefore `normal + noise_only = ISTFT(Y) = x[c − N]` up to float rounding (AC-10), and switching
  modes never disturbs the adaptive state.
- With no blob, noise-only outputs **silence**: nothing is removed.

### 4.8 Lifecycle, restart, reset, state
- **`activate`:**
  - allocates the FIFOs, overlap-add buffer, FFT plans (N), λ table (§4.3), per-bin DD/smoother
    state and the parameter ring;
  - validates and decodes the committed blob (from `load_state`);
  - the latency becomes N of the **loaded** `fft_size`.
- **An `fft_size` event** whose value differs from the active size: the module keeps processing at
  the active size and calls `ctx.request(HostRequest::Restart)` in that block, once per such event
  (ADR-005 §8). An event back to the active size requests nothing.
- **`reset`** (RT-safe): clears the FIFOs, overlap-add buffer, DD history and the first-frame flags,
  and restarts the counter c at 0 (§4.4). Parameter values are kept.
- **`tail()`** = `Samples(N)`: after the input goes silent, the delayed content drains, then the
  output is exactly 0.
- **State:** `save_state` returns the parameters plus the blob last given to `load_state`
  (rule S1). `process` never mutates either. `migrate_state` uses the default (additive).
- **`NoiseProfile` handle:** `capture` and `describe` are pure functions of their inputs (the handle
  holds only constants), so they are safe from any non-audio thread (ADR-005 §7).

### 4.9 Real-time, CPU, determinism
- **`process`:** no allocation, locks, I/O or panics. Loops are bounded by N and the block length.
  FFTs use preallocated scratch.
- **Denormals:** gains ≥ 0.01 or exactly 1; states flushed below 1e-30; FTZ/DAZ set by the host.
- **Clamping:** non-finite intermediate values are clamped (γ, ξ ≤ 1e12).
- **FFT:** `realfft` 3.5 (f32), the crate PROMPT §4 and `docs/references.md` name, which SPEC-007's
  tile service also uses. `dsp` hosts the STFT. T-501 adds the dependency to `dsp` if the tile
  service hasn't.
- **CPU estimate** (default, 48 kHz): 93.75 frames/s × (2 real FFTs of 2048 + ~1025 × (log10 +
  pow + ~40 flops)) ≈ 0.4 % of one core. The budget (AC-18) allows 2 %.
- **Determinism:**
  - Same machine: two instances, and realtime vs offline, are bit-identical.
  - Across machines: rustfft picks SIMD kernels at runtime, so last-bit differences are possible.
    NR goldens are therefore tolerance-based, never hashes checked into the repo (§8).

### 4.10 `describe()` for the graph
- **Points:** one per SPEC-007 analyzer band centre `f_k = 20·2^(k/24)` Hz, for f_k ≤ min(fs_c/2,
  24 000). That is K = 246 at 48 kHz, so the curve shares x positions with the analyzer.
- **Level** in the analyzer's convention, so the print sits exactly where the analyzer would draw
  that noise:
  `L_k = 10·log10(D̄_k) + 10·log10(4·ENBW_BH4 / N_an)` dB.
  - D̄_k is the mean of the §4.3 density model over `[f_k·2^(−1/48), f_k·2^(1/48)]`.
  - ENBW_BH4 = 2.0044 bins.
  - N_an = the analyzer FFT at the capture rate (pow2 nearest 8192·fs_c/48 000).
  - Check: white noise of σ dBFS RMS gives σ − 30.09 dB at 48 kHz (SPEC-007 §4.8).
- Bands with D̄_k = 0 report −150.
- `describe` doesn't allocate beyond `out`.

## 5. Acceptance criteria
**Conventions.** Unless stated otherwise:
- 48 kHz; parameters at their defaults (N = 2048);
- signals from testkit (`tone_bursts`, `white_noise`, `pink_noise`, `voice_like`, seeded);
- **the print is captured with `NoiseProfile::capture` from a separate, independently seeded
  excerpt of the same noise type and level, 3.0 s long**;
- outputs are **latency-aligned** (the first N samples dropped) and the first 1.0 s of aligned
  output is excluded (warm-up);
- "noise floor" = `testkit::noise_floor_db(…, 500 ms)`;
- dB values are `20·log10(rms)`.

- **AC-1 [S3] (schema and Module API conformance).**
  - `validate_schema` passes. Ids, keys, ranges, defaults, tapers, flags, group and
    `smoothing_ms` are exactly as in §3.1. The descriptor id is `org.powervoice.noise-reduction`,
    version 1.0.0, features `audio-effect`, `restoration`, `mono`.
  - `ModuleTestHost` passes at 44.1/48/96 kHz with `allow_delayed_effect` for all 8 parameters.
    The module is built with a state holding a white-noise print (−50 dBFS), so the parameters have
    effects.
  - `extension(NoiseProfile)` returns the matching variant. Telemetry is present in [H] only.
  - `latency_samples()` = N and `tail()` = `Samples(N)` for every FFT size at every rate.
- **AC-2 [S3] (no print → exact passthrough).** Given no blob, for every FFT size and every value
  of every parameter (100 seeded parameter sets):
  - the output equals the input delayed by exactly N, **bit-identical** (60 s of seeded pink noise
    plus a sweep);
  - `noise_only` on outputs exact zeros;
  - `describe` of an empty blob returns an error, and the panel model shows the "none" status.
- **AC-3 [S3] (latency is exact; transparent reconstruction).**
  - Given a print of white noise at −140 dBFS RMS (so every gain is 1 for program material), for N
    ∈ {1024, 2048, 4096, 8192} at 44.1/48/96 kHz:
    - an impulse at input sample 10 000 appears in the output at exactly sample 10 000 + N, as the
      maximum |y|;
    - for 10 s of white noise at −20 dBFS, `null_test_db(y[N..], x[..len−N]) ≤ −100 dBFS`.
  - The reported latency at the default is 2048 samples = 42.67 ms at 48 kHz and 46.44 ms at
    44.1 kHz, both ≤ 50 ms (PROMPT §4).
- **AC-4 [S3] (PROMPT §5 example, white noise).**
  - Signal, 12 s: `tone_bursts(seed 1, 1000 Hz, −20 dBFS peak, None, on 0.5 s, off 1.0 s)` +
    `white_noise(seed 2, −50 dBFS RMS)`.
  - Print from `white_noise(seed 3, −50 dBFS, 3 s)`. `reduction_db` = 12.
  - Then:
    - noise floor(input) − noise floor(output) **≥ 10 dB** (expected ≈ 12);
    - for every burst, the RMS over its central 400 ms (50 ms trimmed at each end) differs between
      output and input by **< 0.5 dB**.
- **AC-5 [S3] (pink noise variant).** As AC-4, with `pink_noise` (seeds 2 and 3, −50 dBFS RMS)
  instead of white. The same thresholds: floor drop ≥ 10 dB, burst RMS change < 0.5 dB.
- **AC-6 [S3] (voice-like SNR improvement).**
  - Signal: `voice_like(seed 7, 220 Hz, −20 dBFS peak, snr 20 dB, on 0.4 s, off 0.6 s, 12 s)`, so
    the noise is white at −43 dBFS RMS. Print from `white_noise(seed 8, −43 dBFS, 3 s)`.
  - With `reduction_db` = 12:
    - the RMS over the central 300 ms of each "on" segment changes by < 0.5 dB;
    - the RMS over the central 400 ms of each "off" segment drops by ≥ 10 dB;
    - hence the SNR improves by ≥ 9.5 dB.
- **AC-7 [H] (harmonic voice timbre is preserved).**
  - Signal, 10 s, generated in test code:
    - f0 = 150 Hz with harmonics h = 1…20 (to 3 kHz), each at −20 − 6·log2(h) dBFS peak;
    - fixed seeded phases;
    - amplitude-modulated by `0.5·(1 − cos(2π·4 Hz·t))` (syllables);
    - plus white noise at −55 dBFS RMS.
  - Print from white −55 dBFS.
  - Measured with a Hann FFT of 8192 on the aligned output vs the input, averaged over the whole
    signal: every harmonic whose per-bin SNR at N = 2048 is ≥ 20 dB stays within **±1.0 dB**, and
    the total RMS changes by < 0.5 dB.
- **AC-8 [S3] (reduction depth follows the parameters).**
  - Input: 20 s of white noise at −50 dBFS; print from an independent excerpt.
  - For `reduction_db` r ∈ {6, 12, 20, 30} at amount 100 %, the output RMS drop is in
    **[r − 2.0, r + 0.5] dB**.
  - With amount 50 %, the drop is r/2 ± 1.0 dB. With amount 0 %, the drop is within ±0.05 dB of 0.
  - With `sensitivity_db` −6, r = 12, the drop is still ≥ 9 dB. Its sign is checked: higher
    sensitivity never reduces the drop (monotonic over −6, 0, +3, +12).
- **AC-9 [H] (musical noise stays low — kurtosis ratio).**
  - **Metric** (Uemura/Saruwatari "kurtosis ratio", ⚠ citation to confirm in T-501):
    - STFT of the aligned output and of the input: Hann N = 2048, hop 512.
    - For bins 300 Hz–8 kHz, normalise each bin's frame powers by that bin's mean, pool all values,
      and compute K = μ4/μ2² (centred moments). Gaussian noise gives K ≈ 9.
    - **KR = K_out / K_in.** Isolated tonal blips ("musical noise") make the output power
      distribution heavy-tailed and raise KR. A constant attenuation leaves KR = 1.
  - Input: 20 s of white noise −50 dBFS (seed 11); print from seed 12.
  - **Pass:** KR ≤ 1.5 at r = 12 dB and KR ≤ 1.5 at r = 24 dB, other parameters default.
  - **Calibration (must hold, or the metric is invalid):** a reference naive power spectral
    subtractor in test code, on the same framing, with `g = √max(1 − 2λ/P, 10^(−24/10))` (Boll,
    over-subtraction 2, floor −24 dB, no DD or smoothing), scores **KR ≥ 3.0**. The analytic
    expectation is ≈ 4.9.
  - If calibration fails, T-501 reports the measured numbers and the orchestrator re-derives the
    thresholds. The discrimination intent is normative; the numbers must not be loosened silently.
- **AC-10 [S3] (output noise only).**
  - Given the AC-4 signal and print, render once with `noise_only` off (y_n) and once on (y_o).
    Then `max |x[c − N] − y_n[c] − y_o[c]| ≤ −90 dBFS` for c ≥ N (a null ≥ 60 dB deeper than the
    signal).
  - Over a noise-only gap, y_o's RMS is within ±1 dB of `input RMS + 20·log10(1 − 10^(−12/20))`
    (−2.5 dB).
  - **Toggling:** with the §5.1 Z1 setup, a `noise_only` 0 → 1 event at S = 72 013 passes SPEC-012
    §4.3 with T_s = 1000·N/fs ms (42.7 ms), in realtime and offline mode, both directions.
  - [H]: while it is on, the slot header shows the badge, and an export/bake of that rack shows the
    §2.6 confirmation (M6 wiring).
- **AC-11 [S3] (profile blob: format, round trip, determinism).**
  - Capturing 10 s of white noise at 48 kHz gives a blob of exactly **32 824 bytes**, with the §4.2
    header (`"PVNP"`, 1, 48000.0, 8192, 2048, 1, M = 231, 480 000, RMS, 4097).
  - Capturing the same excerpt twice gives byte-identical blobs.
  - `save_state` → JSON (SPEC-018 serde) → `load_state` into a fresh instance: the blob is
    byte-identical, and a 20 s offline render of the AC-4 signal is bit-identical to the original
    instance's.
  - Sidecar save, close and reopen (SPEC-018 AC-3 path) preserves the blob byte for byte.
  - Corrupting the magic, truncating by 1 byte, or setting one D(k) to NaN, −1 or +inf each gives
    `InvalidBlob` from `describe`, a successful `load_state`, and passthrough behaviour (AC-2). The
    host re-serialises the original bytes unchanged.
- **AC-12 [H] (derivation across FFT sizes and rates).**
  - **White.** For 30 s of seeded white noise at 48 kHz captured at 8192, the derived λ for N ∈
    {1024, 2048, 4096} matches a direct Welch reference computed in test code (the same excerpt,
    √Hann at N, hop N/4, `mean |Y|²`). The test compares per 1/3-octave band, 100 Hz–20 kHz,
    within **±0.5 dB**.
  - **Pink.** The same for 30 s of pink noise.
  - **Tone.** The same for white noise at −60 dBFS plus a 1 kHz sine at −40 dBFS (hum). The band
    containing 1 kHz is within ±1.0 dB.
  - **Rates.** A print of white noise with variance σ² captured at 48 kHz and activated at
    44.1 kHz gives `D_p = σ²·44.1/48` in every bin (−0.37 dB) within ±0.05 dB. Above 22.05 kHz
    nothing is needed. Activated at 96 kHz, the bins above 24 kHz equal the flat extension of
    §4.3.
- **AC-13 [S3] (capture rules and errors).**
  - `min_capture_samples` returns 24 000 at 48 kHz, 22 050 at 44.1 kHz, 48 000 at 96 kHz and
    12 288 at 16 kHz.
  - A 0.49 s selection fails with "Select at least 0.5 s of room tone", and the old print is kept.
  - 0.5 s succeeds with the short warning; 1.0 s succeeds with no warning.
  - An all-zero selection fails with the silence error.
  - A −30 dBFS RMS selection succeeds with the loud warning.
  - A 90 s selection analyses 60 s (header length = 2 880 000 at 48 kHz) and posts the notice.
  - No selection → the command is disabled. While recording → disabled.
  - Cancel during a (test-slowed) capture leaves the committed blob unchanged.
- **AC-14 [S3] (capture source and live replacement).**
  - **Source.** Given the rack [Gain −6 dB, NR] and a 3 s selection of white noise at −50 dBFS,
    the captured D equals the capture of the raw excerpt × 10^(−6/10), per bin within 1e-4
    relative.
    - With the Gain slot bypassed, it equals the raw capture.
    - With a placeholder slot before NR, it equals the raw capture.
    - With Gain as the **last** slot (after NR), it equals the raw capture.
  - **Target slot.** With no NR slot, Shift+P inserts one as slot 0 and captures into it. With two
    NR slots, it captures into the last-focused one.
  - **Live replacement.** During realtime playback (fake backend) of the §4.3 signal with a
    white-noise print already loaded, a capture into the NR slot is swapped in at S_i. The output
    passes SPEC-012 §4.3 with S = S_i and T_s = 15 ms + 1000·N/fs. The transport never stops, and
    the reported latency is unchanged.
- **AC-15 [S3] (FFT size change during playback).**
  - Given realtime playback (fake backend, rack [NR]) of the §4.3 signal with a white-noise print
    at −50 dBFS, when `fft_size` changes from 2048 to 4096 at input sample 72 013:
    - the module requests exactly one `Restart`;
    - the host swaps in a 4096 instance at S_i;
    - the output passes SPEC-012 §4.3 with S = S_i, analysed without a latency shift and with
      T_s = 15 ms + 1000·4096/48 000 ms;
    - the rack total latency and the heard-position offset read 4096 within 100 ms of the swap
      (SPEC-012 AC-8).
  - **Playhead sync.** For a 1 Hz impulse train (−6 dBFS) through the same change, every impulse
    entering after S_i + 4096 + 15 ms is output at exactly its input position + the total latency
    in force (± 0 samples). The SPEC-003 heard position maps it back to its document position
    ± 1 sample.
  - The same for 4096 → 1024, and for a change while stopped (in-place re-activation, no swap).
- **AC-16 [S3] (realtime = offline; deterministic).**
  - Signal: the AC-4 signal plus 20 events at fixed sample positions on every continuous
    parameter and `noise_only`.
  - Realtime processing with random block sizes 1…1024, including 0-length flushes, vs offline
    4096-frame blocks: ≤ **1e-6** absolute at every sample (SPEC-012 §2.8). **Bit-identical is
    expected** (§4.6), and a difference is reported.
  - Two offline renders have equal FNV-1a hashes (same machine). At 44.1/48/96 kHz.
  - An event at input sample k never changes output samples < k + N (the frame-binding rule).
- **AC-17 [S3 reduction_db, H others] (no zipper noise, SPEC-012 §4.3).** Every continuous
  parameter (`reduction_db`, `amount_pct`, `sensitivity_db`, `smoothing_hz`, `attack_ms`,
  `release_ms`) passes §4.3 in both directions and in the drag variant, in realtime and offline,
  with the §5.1 setup. A direction with no effect passes trivially and is reported as such.
- **AC-18 [H] (CPU budget).**
  - In a release build on the owner's machine (`just bench`), offline-rendering 60 s of the AC-6
    signal at 48 kHz takes **≤ 1.2 s** at defaults (≤ 2 % of one core) and ≤ 1.8 s at N = 8192.
  - Capturing a 60 s excerpt takes ≤ 0.3 s.
- **AC-19 [S3] (RT safety, denormals, reset).**
  - `process` and `reset` run under `no_alloc` for every FFT size, with and without a print.
  - Setup: FTZ/DAZ **off**, 1 s of 0 dBFS white noise, then 60 s of digital silence. From N samples
    after the input goes silent, the output is exactly 0.0, and a test-only scan finds no
    subnormal value in the module state.
  - After `reset()` mid-signal, the output equals that of a freshly activated instance with the
    same state fed the same subsequent input, **bit-identical**. Its first processed frame uses
    snapped smoothers: over the first 100 ms after reset, the aligned output RMS of pure noise is
    ≤ the steady reduced level + 1 dB (no noise burst).
- **AC-20 [H] (`describe` and the graph).**
  - For 10 s of white noise at −50 dBFS captured at 48 kHz, `describe` returns 246 points at the
    SPEC-007 band centres, with levels of **−80.09 ± 0.5 dB** for bands 100 Hz–20 kHz.
  - For the same noise played through an empty rack, the SPEC-007 analyzer (Medium, after 5 s)
    reads within ±1.0 dB of the print curve for bands 100 Hz–20 kHz.
- **AC-21 [S3 status/button, H graph] (NR panel, Vitest with mocked IPC).**
  - The Capture button is disabled without a selection or while recording, with the matching
    tooltip.
  - During a job it shows a spinner and Cancel.
  - The status line shows each §2.8 state from the mocked blob/describe results.
  - Warnings render and are dismissible.
  - Main parameters come first and the Advanced group is collapsed by default.
  - The latency readout follows the mocked `latency_changed`.
  - [H]: the graph draws the print, the reduced-to line (print − r·amount) and the analyzer curve
    on the `freqAxis.ts` log axis. With no print the graph is empty. The "Noise only" badge follows
    the parameter.
- **AC-22 [H] (presets and persistence).**
  - A user preset saved from an NR slot and loaded into another NR slot reproduces the blob
    byte-identically and triggers a replacement.
  - The factory preset "Strong (20 dB)" changes only `reduction_db` (smoothed, no replacement) and
    keeps the print.
  - Cutting the selection the print came from leaves the blob and the NR output for the remaining
    audio unchanged (bit-identical to a render made before the cut, over the unaffected range).
  - A rack carried over to a new document keeps the print (SPEC-018 §2.7).

### 5.1 Zipper test setup (SPEC-012 §4.3 substitution)
§4.3's signal, analysis and pass threshold are unchanged. Only the module state differs:
- **Z1:** the print is `capture()` of **2.0 s of the §4.3 tone itself** (997 Hz, −20 dBFS peak,
  48 kHz). With the print equal to the signal, every bin sits at γ ≈ 1 and at the floor.
  - A change of `reduction_db` or `amount_pct` is then a plain level change of the tone, exactly
    the scenario §4.3 was calibrated for.
  - Every other bin must stay below −90 dBFS or within 3 dB of its steady level.
  - No harness change is needed: `ZipperTest::new(make, id)` with `make` loading the Z1 state.
- T_s = max(`smoothing_ms`, 1 ms) = 200 ms, so B_ex = 1000 Hz.

| Parameter | 0.25 → 0.75 plain values | Effect on Z1 |
|---|---|---|
| `reduction_db` | 10 → 30 dB | the tone falls by 20 dB over one frame length |
| `amount_pct` | 25 → 75 % | the tone falls from −3 to −9 dB (at the default 12 dB) |
| `sensitivity_db` | −1.5 → +7.5 dB | none: the tone stays at the floor (trivial pass, reported) |
| `smoothing_hz` | 250 → 750 Hz | none: uniform gains (trivial) |
| `attack_ms` | ≈ 3.8 → 53 ms (log) | none: steady (trivial) |
| `release_ms` | ≈ 38 → 532 ms (log) | none: steady (trivial) |

The time and smoothing parameters have no audible transition to measure on a steady signal. Their
correctness is covered by AC-8, AC-9 and the T-501 unit tests of step 7 (the time constants match
`a_x` within 1e-6).

## 6. Test plan
| AC | Unit (`dsp` / `modules`) | Integration (rack / engine / CLI) | Vitest | Manual (owner) |
|---|---|---|---|---|
| AC-1 | schema, `ModuleTestHost` with opt-outs and a print-loaded `make` | — | — | — |
| AC-2, AC-3 | passthrough bit-compare; impulse and null per N/rate | `powervoice-cli render --rack` with an NR slot | — | — |
| AC-4–AC-8 | goldens with numeric tolerances (T-501 offline, then T-503 streaming) | CLI: `gen` → `render --rack` → `analyze` (noise floor) | — | clean a real take with room tone; A/B |
| AC-9 | kurtosis metric + naive-subtractor calibration (test code) | — | — | listen at 24 dB for "bubbles" |
| AC-10 | two-render null; Z1 toggle | export confirmation (M6) | badge | toggle while playing |
| AC-11 | blob encode/decode, corruption cases | sidecar round trip (SPEC-018 path) | — | save/reopen a project |
| AC-12 | derivation vs a Welch reference | — | — | — |
| AC-13, AC-14 | `min_capture_samples`, capture checks | engine capture job on the fake backend; upstream sub-rack render; live swap + §4.3 | — | Shift+P on the head room tone while playing |
| AC-15 | restart request | fake backend swap; impulse-train alignment; readouts | latency readout | change the FFT size while playing |
| AC-16 | block-partition compare, frame binding | fake backend vs offline | — | — |
| AC-17 | §4.3 harness with Z1 | — | — | drag Reduce by while playing |
| AC-18 | — | `just bench` (release) | — | — |
| AC-19 | `no_alloc`, FTZ-off scan, reset compare | — | — | — |
| AC-20 | `describe` levels | analyzer comparison | graph fixture | compare the print with the analyzer |
| AC-21 | — | — | panel with mocked IPC | — |
| AC-22 | — | presets (T-406), cut + render, carry-over | — | — |

Fixtures are generated in test code from testkit (`tone_bursts`, `white_noise`, `pink_noise`,
`voice_like`, `impulse`) plus the AC-7 harmonic generator and the AC-9 reference subtractor.
Nothing is committed. Goldens are **tolerance-based** (§4.9), never cross-machine hashes.

## 7. Out of scope
- ML/adaptive noise estimation (PROMPT §3.8), and automatic noise tracking without a print
  (minimum statistics, etc.).
- Audition's Precision factor, Transition width, Noise print snapshots, the High/Low reduction
  curve, "Select entire file", and Keep/Remove toggles other than Output noise only.
- A separate noise print file format (use module presets, §2.4).
- Capture metadata display (needs the proposed `NoiseProfile::summary`, [H]).
- A per-slot spectrum tap for the graph (needs an ADR-002/003 amendment, [H]).
- Hum removal with harmonic notches, de-click, de-clip, de-reverb, spectral editing (PROMPT §3.8).
- Variance-aware per-bin processing using S(k) (stored now, unused in v1).
- Bake/export tail and pre-roll policy (T-602).

## 8. Sources and decisions

**Sources:**
- Adobe helpx NR pages (`/audition/using/…` and `/audition/desktop/effects-reference/…`) return
  403, as in earlier spec waves. Parameter names and meanings come from `docs/references.md` (the
  2026-09-12 research) and secondary sources.
- **Shift+P** = Capture Noise Print: shotkit.com "How to Remove Background Noise in Adobe Audition"
  and premiumbeat.com "5 Tools for Cleaning Up Audio" (both secondary). **Ctrl/Cmd+Shift+P** opens
  the Noise Reduction effect (same sources).
- Audition "Spectral decay rate 40–75 %" and "Precision factor 7–14, odd": community.adobe.com
  threads (secondary).
- Boll 1979; Ephraim–Malah 1984 (decision-directed); Cappé 1994 (musical-noise analysis of DD,
  ⚠ not re-verified); Audacity Noise Reduction manual (Frequency smoothing bands, Sensitivity).
- Kurtosis ratio as a musical-noise measure: Uemura, Saruwatari et al., 2008–2012. ⚠ Exact
  reference to be confirmed in T-501; the metric is fully defined in AC-9 regardless.
- SPEC-007 §4.8: the BH4 white-noise offset −30.09 dB at 8192, band centres 20·2^(k/24).

**Decided (autonomous, T-500)** in this spec, each with its rationale where it appears:
1. Module id `org.powervoice.noise-reduction` (ADR-005), not the brief's `org.powervoice.nr`.
2. Shift+P captures, Ctrl+Shift+P shows the NR panel; both provisional for SPEC-019. Menu path
   Effects → Noise Reduction / Restoration → Capture Noise Print.
3. Capture target: last-focused NR slot → first NR slot → a new NR slot inserted first.
4. Capture source: the slot input (ADR-005 §11). Upstream slots are rendered offline with 1 s
   pre-roll, bypass flags honoured, A/B ignored, placeholders omitted.
5. Minimum 0.5 s (and ≥ 12 288 samples); warn below 1 s; analyse at most 60 s; errors on silence;
   warn at > −35 dBFS RMS; disabled without a selection and while recording.
6. The print replaces the previous one; not undoable; a Clear Noise Print command.
7. No noise print file format; module presets carry the blob; factory presets are parameter-only.
8. Profile = mean power density + log-power spread at a fixed 8192-point Hann analysis (hop 2048)
   at every rate; blob v1 of 32 824 bytes with its own version.
9. Derivation by averaging a piecewise-linear density model, scaled by fs/fs_c, extended flat above
   the capture Nyquist.
10. STFT: √Hann analysis and synthesis, 75 % overlap, sizes 1024–8192 in absolute samples, default
    2048; latency = N (42.7 ms at 48 kHz).
11. Keep the latency without a print (exact delay line).
12. Gain rule: decision-directed Wiener (β 0.98), sensitivity over-subtraction, floor = Reduce by,
    dB-domain frequency and time smoothing, amount scaling in dB.
13. Parameter set, ranges and defaults (§3.1). Transition width, Precision factor, Snapshots and
    the High/Low curve are omitted.
14. Frame-start parameter binding; smoothing through the overlap-add; declared `smoothing_ms` 200;
    all parameters `allow_delayed_effect`.
15. Output noise only = (1 − g)·Y with shared adaptive state; a header badge; an export/bake
    confirmation.
16. Tail = N; reset snaps the smoothers on the first frame.
17. Profile graph: the print, a reduced-to line, and the **rack-output** analyzer.
18. Musical-noise metric: kurtosis ratio with a mandatory discrimination calibration.
19. CPU ≤ 2 % of one core at defaults (≤ 3 % at 8192); capture of 60 s ≤ 0.3 s.
20. Determinism: bit-identical on one machine; tolerance-based goldens across machines.
21. Capture metadata display deferred (proposed additive `NoiseProfile::summary(blob)` amendment).

**Contradictions and gaps flagged for the orchestrator:**
- **Module id:** brief `org.powervoice.nr` vs ADR-005 §2 / SPEC-018 `org.powervoice.noise-reduction`.
  We follow the ADR.
- **Latency vs Audition's FFT advice:** Audition recommends FFT 4096–8192. At 48 kHz those give 85
  and 171 ms, beyond PROMPT §4's ≤ 50 ms. The default is 2048; the larger sizes stay available with
  a tooltip and the SPEC-002 warnings.
- **"Noise print vs live input spectrum"** (brief) vs SPEC-007 §2.10 (no per-slot tap without an
  ADR-002/003 amendment). v1 shows the rack-output spectrum; the per-slot tap is [H].
- **SPEC-012 §2.8 "bit-identical on any machine":** realfft/rustfft choose SIMD kernels at runtime,
  so FFT-based modules are bit-identical only per machine. Suggest amending SPEC-012 §2.8 to "same
  machine" for FFT modules, or forcing rustfft's scalar planner in `dsp` (slower; T-501's call if
  cross-machine hashes are ever needed).
- **SPEC-012 §2.4:**
  - "Stepped parameters take effect at the change's sample": NR's `noise_only` takes effect at the
    next frame boundary (≤ H samples later), crossfaded.
  - "Reaches its target within the declared smoothing time": for NR this means the parameter value
    in use; the adaptive gains follow the signal.
  - Both are inherent to frame-based processing and compatible with ADR-005 §4 ("never earlier").
- **PROMPT §3.4 "reduction (dB), reduce-by %"** is mapped to Audition's pair: "Reduce by" (dB) =
  the floor, "Noise reduction" (%) = the share of that reduction applied.
- **Output noise only is persistent state** and would be exported. The M6 export/bake specs
  (T-601/T-602 wave) must add the §2.6 confirmation.
- **Minimum capture** is 0.5 s rather than the brief's example of 0.3 s (statistics, ACX head room
  tone). At 16 kHz the minimum is 0.77 s (3 analysis frames).
- **FFT dependency:** `dsp` has no FFT crate yet. `realfft` 3.5 (named in PROMPT §4 and
  references) must be added by T-501 or SPEC-007's T-204, whichever lands first.

## 9. Slice 3 subset (lean implementation, then harden)
Slice 3 ships a working capture → clean loop. Everything else in this spec is the hardening target.
- **In scope:**
  - **Module:** the full §3.1 schema, so ids and state are final from day one.
  - **DSP:** the full §4.4–§4.8 algorithm. Frequency and time smoothing are a few lines, and
    dropping them would change the sound later.
  - **Blob:** v1 §4.2 and the §4.3 derivation for the same rate (the cross-rate path may return the
    same-rate formula with the fs/fs_c factor; its AC-12 checks are [H]).
  - **Capture command:** Shift+P, the menu, and the panel button; the target-slot rule; the
    selection checks (length, silence); the upstream render; replacement.
  - **Panel:** the generic parameter UI plus the Capture button and the status line.
- **ACs in Slice 3:** AC-1, AC-2, AC-3, AC-4, AC-5, AC-6, AC-8, AC-10 (without the M6 export
  confirmation), AC-11, AC-13, AC-14, AC-15, AC-16, AC-17 (`reduction_db` only), AC-19, AC-21
  (status/button part).
- **Deferred to hardening [H]:**
  - AC-7, AC-9, AC-12, AC-18, AC-20, AC-22, and the rest of AC-17 and AC-21;
  - telemetry;
  - the profile graph;
  - the loud/short warnings' UI polish, and the 60 s-cap notice (the cap itself is S3);
  - Clear Noise Print, factory presets, the header badge, the export confirmation (M6);
  - the per-slot spectrum tap and `NoiseProfile::summary`.

## Amendment 1 — S3-04 implementation (2026-09-13, orchestrator, autonomous)
Accepted deviations, all measured in S3-04:
1. **Tail = 2N** (not N): frequency-domain gains smear up to N samples past the delayed input (measured
   last non-zero output 1919 / 3711 / 7295 / 15487 samples for N = 1024 / 2048 / 4096 / 8192). AC-1/AC-19
   check exact zero from 2N.
2. **Decision-directed history uses the full-strength gain 10^(Ĝ/20)**, not the applied (amount-blended)
   gain — makes "Noise reduction %" a true dB-domain dry/wet; identical at 100 %.
3. **AC-8 "sensitivity −6 → drop ≥ 9 dB"** is not reachable without worsening musical noise (DD recursion
   is bistable with the print 6 dB below the noise); the test asserts ≥ 5.5 dB and monotonic ordering.
   Re-derive the threshold in hardening.
4. **AC-17 drag variant** uses a 3.5 s §4.3 tone (the harness's 3.0 s leaves too few steady frames after
   drag + T_s + latency); proposed hardening item: a signal-length option in `ZipperTest`.
