# SPEC-002 — Recording & monitoring

- **Status:** approved (owner, M0 checkpoint 2026-09-12)
- **Milestone:** M1 (T-106 recording, T-107 monitoring, T-108 meters/telemetry, T-109 UI)
- **Related:** SPEC-000 (glossary), SPEC-001 (devices, device loss), SPEC-003 (transport, playhead),
  SPEC-004 (undo, recovery), SPEC-012 (rack latency, bypass) · ADR-002 §1, §2, §6–§8 · ADR-003 (`VXTM`,
  `VXRP`) · ADR-004 §6, §7, §9 · ADR-008 §2, §5

## 1. Purpose
Recording is the first thing a voice-over user does, and the one step that can't be redone once the
talent has gone home. A voice-over user needs to:
- set a level before recording, and see when the input clipped;
- record without thinking about formats;
- never lose a take to a crash;
- find every place where the audio was damaged;
- hear themselves with a latency they can judge.

This spec covers **new-file recording** (M1) and the three monitoring modes. Recording at the cursor
(insert/overwrite), punch-in and latency calibration are M3 (T-304) and get their own spec.

## 2. Behavior / UX

### 2.1 Arming and the input meter
- **Prerequisite.** An input device and channel are selected (SPEC-001; the factory default is
  "None"). With no input device, the Arm and Record controls are disabled and show the hint "Choose
  an input device in Settings → Audio Devices".
- **Arm.** An **Input** toggle in the transport bar (next to Record) arms the input. Arming opens the
  input stream (ADR-002 §1) and starts the input meter. Record arms automatically.
  - While recording, the toggle is locked on.
  - The armed state is **not** persisted. PowerVoice always starts disarmed, so the OS microphone
    indicator is never lit just because the app is open.
- **Input meter** (meter bridge, visible while armed). It shows the selected input channel **before**
  any processing:
  - **Peak bar.** The sample peak since the previous telemetry frame (max-hold, `in_peak_dbfs` in
    `VXTM`), in dBFS. Instant attack; release 20 dB/s. A peak-hold tick holds for 1.5 s, then falls.
  - **RMS bar.** Unweighted RMS over a sliding 300 ms rectangular window, `20·log10(rms)` (the same
    convention as testkit and ACX, so a sine with peak −20 dBFS reads −23.0 dB), in `in_rms_dbfs`.
  - **Scale and readout** (H-112 amendment — see "Amendment 1" below). The scale runs from a
    selectable floor — −60 (factory default), −80 or −120 dBFS — to 0 dBFS, chosen from a control
    on the meter itself and persisted (`Settings.input_meter_floor`); digital silence (`-inf`)
    shows an empty bar. A numeric readout shows the highest peak since arming or since the last
    reset; clicking it resets it. The meter is vertical, matching the output meter's form
    (peak+RMS fill, peak-hold tick, colour zones, scale ticks) — the two sit side by side in the
    meter bridge.
- **Clip indicator.**
  - **Trigger.** It lights when any input sample has |x| ≥ 0.99990 (≈ −0.0009 dBFS). This catches
    integer full scale (16-bit +32767 → 0.999969; −32768 → −1.0) and float overs. The event is
    carried as the `IN_CLIP` flag of the next telemetry frame.
  - **Clip hold.** The lamp **latches** until the user clicks it, or until a new recording starts.
    Clearing at record start means a lit lamp after a take always means *that take* clipped.
  - **Counting.** During a take, clip events are counted. An event is a run of clipped samples; runs
    separated by < 10 ms count as one. On Stop, a non-zero count shows the notice "This take clipped
    N times".

### 2.2 New recording (M1)
- **Entry points.** The Record button, File → New Recording…, and the Record shortcut.
  - The Record shortcut is provisionally **Shift+R** (owner, M0 checkpoint: in Audition Shift+Space is
    Play from start, not Record — SPEC-003 §2.5). The final binding is set in SPEC-019.
- **Target document.**
  - If the open document is a new, empty, untitled document, Record records into it.
  - If the open document contains audio, M1 Record opens the **New Recording** dialog, which
    replaces the current document after the standard unsaved-changes prompt.
  - From M3, Record with a non-empty document records at the cursor (T-304).
- **New Recording dialog.**
  - **Sample rate** {44 100, 48 000, 88 200, 96 000} Hz and **bit depth** {16-bit, 24-bit, 32-bit
    float}.
  - Both are prefilled from Settings → Default format, whose factory value is **48 kHz / 24-bit**
    (PROMPT §2, LOCKED).
  - Mono only. The input channel comes from SPEC-001.
  - The chosen values become the document's sample rate and its default **save** format.
- **Capture is always lossless.**
  - The take is captured as 32-bit float at the document rate (ADR-004 §7), whatever the device's
    sample format. The bit depth applies only when the file is saved (M2: WAV writer, dither), so
    choosing 24-bit never degrades the working audio.
  - If the input device can't run at the document rate, the capture-writer resamples (ADR-002 §5).
    The record panel shows "Input runs at 44.1 kHz — resampled to 48 kHz".
- **Start.**
  - The take begins with the first sample whose capture time is at or after the moment the engine
    received the Record command. If the input had to be opened first, it begins with the first
    captured sample.
  - The recording state (red Record button, running time, `RECORDING` flag) is visible within
    100 ms of the press when armed, and within 600 ms when the stream has to be opened.
- **While recording.**
  - The waveform grows live (`VXRP`, 30 Hz) and the view follows the take end (SPEC-003 follow).
  - The record panel shows:
    - elapsed time `h:mm:ss.t`;
    - remaining disk time (§2.5);
    - a dropout counter (§2.4);
    - the clip lamp;
    - the input meter;
    - the monitoring latency (§2.7).
  - **Add marker** (`M`, verified in two independent sources: killerkeys.com, domestika.org; final map
    in SPEC-019) is allowed. The marker lands at the take position under the key press, extrapolated
    like the playhead (SPEC-003 §2.2), and is committed with the take.
  - Destructive edits, undo/redo, seeking, loop and device settings are disabled while recording.
    ADR-004 refuses destructive edits during recording. Their controls show "Not available while
    recording".
  - Pause-while-recording is not offered in v1 (§7).
- **Stop.** The Stop control and the Play/Pause key (Space, SPEC-003) both stop the recording. The
  take becomes **one undoable edit** labelled "Record":
  - It contains the audio, every marker added during the take and every dropout marker.
  - It is visible within 500 ms of Stop: final chunk committed, WAV patched and fsynced, journal
    record written.
  - The document becomes modified and stays untitled until saved.
  - Undo returns the document to the empty state, removing the take and all its markers. Redo
    restores them exactly.
  - The take WAV stays in the session as a backup until the session is cleaned up (SPEC-004 §2.8).
- **Length.** A take is limited only by disk space. Past the 4 GiB RIFF limit (≈ 6 h 12 min at
  48 kHz, 32-bit float) the capture file rolls over transparently (ADR-004 §7); it is still one take
  and one edit.
- **Quitting while recording** asks "Stop recording and quit?". Confirming commits the take, then
  shows the normal save prompt.

### 2.3 Crash safety
- **Process crash (including `kill -9`).** At most the audio captured in the last **250 ms** before
  the crash is lost. The recovered part is bit-identical to what was captured.
- **Power loss or OS crash.** At most the last **~1.5 s** is lost. The WAV header is patched and
  `fdatasync`ed every ~1 s (ADR-004 §6).
- **Recovery** happens at the next start through the SPEC-004 recovery dialog, which offers "Apply as
  recorded", "Open as new document" or "Discard take". Recovery takes the WAV as authoritative, and
  infers its length from the file size when the header is stale (ADR-004 §9).

### 2.4 What happens on an xrun during recording — **decision**
There are three distinct events, each handled differently.

| Event | Detected by | Effect on the take | User sees |
|---|---|---|---|
| **Input dropout**: the device or driver lost captured frames | A gap in the input stream's capture timestamps (§4.3), with the ADR-002 §7 callback-gap rule as fallback | **Recording continues.** The estimated number of lost frames (rounded, at most 2 s) is filled with digital silence, so every later sample stays at its correct time. A marker **"Dropout 12 ms"** (i18n key `marker.dropout`, duration in ms) is placed at the start of the gap. If the length can't be estimated, the marker says "Dropout (length unknown)" and nothing is filled. | A live amber dropout counter in the record panel. On Stop: "This take has N dropouts — markers were added", with **Go to first**. |
| **Input gap > 2 s** | Same | Handled as device loss (SPEC-001 §2.4): the take is finalized at the last good sample and kept. | The device-lost notice. |
| **Output underrun** during recording | ADR-002 §7 | None: the take comes from the dry input path. | The session xrun counter (`XRUN` telemetry flag). No marker. |
| **Capture-ring overflow**: the writer fell ≥ 10 s behind, e.g. a disk stall | Ring-full counter (ADR-002 §2) | **Recording stops.** The take is finalized at the last good sample, committed and kept (ADR-004 §7.4). | "Recording stopped: the disk could not keep up. 12:31 recorded and kept." |

**Why continue, fill and mark for input dropouts:**
1. **The performance matters more than the glitch.** Stopping on the first dropout loses everything
   after it, and the talent usually doesn't notice that recording stopped.
2. **Dropouts must be findable.** An ACX submission with an audible dropout is rejected, and finding
   a 10 ms hole by ear in a 60-minute take is impractical. A marker makes each one one click away.
3. **Filling keeps time.** Later audio stays where it was spoken. That matters for record-at-cursor
   and punch-in (M3), and the gap is visible in the waveform. The splice was already a
   discontinuity, so silence doesn't make it worse, and the user repairs it anyway (re-record,
   delete).
4. **One undo step.** The markers belong to the take's edit, so one undo removes everything the take
   introduced.

**Why not treat overflow the same way:** a ring overflow means the disk has not accepted data for
10 s. That condition persists, so continuing would produce a take riddled with holes.

**Rejected alternatives:**
- *Count only.* The damage can't be found.
- *Stop on every xrun.* It loses the performance.
- *Interpolate across the gap.* It hides damage, and spectral repair is out of scope.

### 2.5 Recording and disk space
- **Disk rate.** Recording writes two copies, the take WAV and the session chunks: 8 × rate bytes/s,
  which is 384 000 B/s (≈ 1.38 GB/h) at 48 kHz.
- **Remaining time.** The record panel shows **remaining recording time** =
  (free space on the session volume − 512 MiB) / (8 × rate). It turns amber below 10 min. Record
  start with less than 10 min remaining asks "Only 8 min of disk space left. Record anyway?".
- **Hard floor.** When free space falls below **512 MiB**, recording stops cleanly: the take is
  finalized and kept, with the notice "Disk almost full — recording stopped. 42:10 recorded and kept."
- **No housekeeping during recording.** SPEC-004's disk-budget housekeeping (compaction, dropping
  undo steps) never runs during recording.

### 2.6 Device loss during recording
This follows SPEC-001 §2.4:
- The take is finalized at the last good sample and committed as the normal undoable edit.
- The notice reads "Input device disconnected — recording stopped. 3:12 recorded and kept."
- The app does not crash, and recording does not resume automatically on replug.

**Output device lost while recording** (owner decision, M0 checkpoint; SPEC-001 §2.4 exception):
recording **continues**, because the take needs only the input. Monitoring stops (there is no output),
the SPEC-001 device-lost banner appears, and the take proceeds until the user stops it (AC-16).

### 2.7 Monitoring
- **Modes.** **Off / Dry / Through rack**; the factory default is **Off** (PROMPT §2, LOCKED). The
  selector is in the record panel and in Settings, and the last choice is saved as a preference.
- **When it is audible.** Monitoring is audible only while the input is **armed** or recording.
  Switching mode, arming or disarming fades over ≤ 10 ms, so there is no click.
- **Dry.** The input is added to the output **after** the rack, at unity gain.
- **Through rack.** The input is added to the rack input, together with any playback (ADR-002 §4), so
  the talent hears the processed voice.
- **Monitoring is never recorded.** The take is always the dry input, whatever the mode (ADR-002,
  ADR-008 §5).
- **Latency readout.** "Monitoring latency 23.7 ms" appears next to the selector. It is recomputed
  within 1 s of any change of buffer size, device, mode or rack. The formula is:
  `input latency + monitor buffer target F* + (rack latency, through-rack only) + output latency`
  (§4.4).
- **Warnings.**
  - **≥ 20 ms, amber:** "You may hear your voice delayed."
  - **≥ 40 ms, red:** also suggests a smaller buffer size, Dry mode, or removing high-latency
    modules. Bypassing doesn't help, because bypassed modules keep their latency. Noise reduction
    alone may add up to 50 ms (PROMPT §4).
  - The first time monitoring is enabled, a one-time hint appears: "Use headphones — monitoring
    through speakers can cause feedback."
- **Drift and rate mismatch.** Input and output may be different devices with different clocks, or
  even different nominal rates. The drift servo (ADR-002 §6) keeps monitoring continuous:
  - There are no periodic clicks.
  - The pitch correction never exceeds ±1000 ppm (≤ 1.7 cents).
  - Latency stays within ±1 ms of its target once converged.
  - A monitor underrun produces a short fade-out/fade-in and increments a monitor-dropout counter.
    Because the take is unaffected, it adds no marker.

> **OWNER DECISION OD-1 — Monitoring latency with sandboxed plugins (ADR-008 open question 1, MEMORY).**
> From M8, each external plugin in the rack adds one block of latency, B = the device period rounded up
> to a power of two (≤ 1024). That is ≈ 5.3 ms at 256 frames and 48 kHz, and it is audible only when
> monitoring through the rack.
> - **A.** Accept it. B is included in the latency readout and the warnings above.
> - **B.** Additionally offer a per-plugin synchronous "low-latency" mode, with no added latency. The
>   cost is that a slow plugin can xrun the whole engine.
> - **C.** Through-rack monitoring skips sandboxed plugins and runs built-ins only. This needs a second
>   chain, and the talent then hears something different from playback.
>
> **Recommended default, and what this spec uses until decided: A.** Revisit after the T-801
> measurements.

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `record_sample_rate_hz` | Recording sample rate | Hz | {44 100, 48 000, 88 200, 96 000} | 48 000 | list | per new file; Settings → Default format |
| `record_bit_depth` | Save bit depth of a new recording | — | {16, 24, 32f} | 24 | list | capture is always 32f |
| `monitor_mode` | Monitoring | enum | Off / Dry / Through rack | Off | list | persisted preference; audible only while armed or recording |
| `meter_input_floor_dbfs` | Input meter scale floor | dBFS | {−60, −80, −120} | −60 | list | H-112; persisted preference, UI-only (never read by the engine) |
| `meter_rms_window_ms` | Input RMS window | ms | — | 300 | fixed | rectangular, unweighted |
| `meter_peak_release_db_per_s` | Peak bar release | dB/s | — | 20 | fixed | UI ballistics |
| `meter_peak_hold_s` | Peak-hold tick | s | — | 1.5 | fixed | UI |
| `clip_threshold` | Clip threshold | linear | — | 0.99990 | fixed | ≈ −0.0009 dBFS |
| `clip_merge_ms` | Clip-event merge gap | ms | — | 10 | fixed | for the per-take count |
| `dropout_fill_max_s` | Maximum silence fill per dropout | s | — | 2 | fixed | larger gaps = device loss |
| `monitor_warn_ms` | Latency warning thresholds | ms | — | 20 (amber) / 40 (red) | fixed | |
| `disk_floor_mib` | Recording hard floor | MiB | — | 512 | fixed | free space on the session volume |
| `disk_warn_min` | Low-disk warning | min | — | 10 | fixed | remaining recording time |

## 4. Algorithm / implementation notes
- **4.1 Take pipeline.** See ADR-004 §7 and ADR-002 §1:
  - The input callback deinterleaves the selected channel and pushes it to the capture ring and the
    monitor ring.
  - The capture-writer appends to the take WAV (32f) and to the chunk store through a `ChunkWriter`.
  - The header is patched and `fdatasync`ed every ~1 s.
  - To meet the 250 ms crash bound, the writer drains the ring at least every 50 ms and hands the data
    straight to the OS (`write`), with no user-space batching beyond one drain.
- **4.2 Start/stop alignment.** Each input block's capture time is `app_ns(ts.capture)` (ADR-002 §8).
  The writer trims the first and last blocks, so the take covers capture times
  [t_record, t_stop) to ±1 sample.
- **4.3 Dropout detection.**
  - For consecutive input blocks k and k+1, the expected start of block k+1 is
    `capture_k + frames_k / rate`, and the gap is `g = capture_{k+1} − expected`.
  - A dropout is declared when `g ≥ max(0.5 × period, 1 ms)`. The lost frames are `round(g × rate)`.
  - The input callback records `(take sample index, lost frames)` in a small gap ring, and the
    capture-writer inserts the silence at that index.
  - Any backend whose capture timestamps are unreliable falls back to the ADR-002 §7 callback-gap rule
    with "length unknown". T-105 checks PipeWire/JACK timestamps (ADR-002 open question), and T-106
    calibrates the threshold against false positives on the owner's machine.
- **4.4 Monitoring latency.**
  - `input latency` = `callback − capture` of the input stream.
  - `output latency` = `playback − callback` of the output stream.
  - `F*` is the servo target fill: max input period + max output period + 1 ms (ADR-002 §6).
  - `rack latency` = the sum of slot latencies (SPEC-012 §2.5), in device samples.
  - Example: 256-frame periods at 48 kHz, 5 ms input and 7 ms output latency →
    5 + (5.33 + 5.33 + 1) + 7 ≈ 23.7 ms, which is amber.
- **4.5 Meters** are computed in Rust from the deinterleaved input, never in the UI. The UI only
  applies ballistics to the per-frame values.
- **4.6 RT constraints.** The dropout detector uses the one permitted `Instant` read per callback.
  Gap records, clip flags and meter values leave the callback through the RT event ring and atomics
  only (ADR-002 §2).

## 5. Acceptance criteria
- **AC-1 (input meter accuracy).** Given a fake input carrying a 997 Hz sine at −12.00 dBFS peak on
  the selected channel (48 kHz), when armed for 1 s, then every telemetry frame after the first
  300 ms reports `in_peak_dbfs` = −12.00 ± 0.05 dB and `in_rms_dbfs` = −15.01 ± 0.05 dB. With digital
  silence it reports `-inf` for both (JSON `null`).
- **AC-2 (clip indicator and hold).** Given an armed input at −40 dBFS:
  - when one sample of 0.99995 occurs, the `IN_CLIP` flag is set in the next telemetry frame and the
    lamp stays lit until clicked;
  - a single sample of 0.9998 does **not** light it;
  - the latch clears when a recording starts;
  - a take containing three clipped bursts 50 ms apart reports "clipped 3 times", and one containing
    two bursts 5 ms apart reports "clipped 1 time".
- **AC-3 (new recording, format and exactness).** Given the New Recording dialog with factory
  defaults, it shows 48 000 Hz / 24-bit. When a fake 48 kHz f32 input is recorded from command time
  t₀ to t₁, then:
  - the document rate is 48 000 Hz and its save format is 24-bit;
  - the take WAV is 32-bit float, mono, 48 000 Hz;
  - the take length is round((t₁ − t₀) × 48 000) ± 1 samples;
  - every take sample is bit-identical to the fake source at the same capture time.
- **AC-4 (input rate ≠ document rate).** Given a fake input device that supports only 44 100 Hz and a
  48 000 Hz document, when a 997 Hz sine at −20 dBFS is recorded for 10 s, then:
  - the take is at 48 000 Hz, and its length is round(10 × 48 000) ± 2 samples;
  - its measured frequency is 997 Hz ± 0.05 %;
  - its RMS level is −23.01 ± 0.1 dB (testkit);
  - the record panel shows the resampling notice.
- **AC-5 (take is one undoable edit).** Given a new document recorded for 5 s with 2 user markers and
  1 dropout, when Stop is pressed, then the take is visible within 500 ms and the undo stack grows by
  exactly one entry labelled "Record". Undo yields length 0 and 0 markers. Redo yields audio with the
  same FNV-1a hash and the same 3 markers at the same positions.
- **AC-6 (crash-safe take).** Given a recording of seeded PCG white noise, when the process is killed
  with `SIGKILL` after the fake backend has delivered K frames (K ≥ 5 s), then after restart:
  - the recovery dialog lists the session with a take;
  - "Apply as recorded" yields a take of length L with K − 0.25 × rate ≤ L ≤ K;
  - samples [0, L) are bit-identical to the source.
  Additionally, a take WAV truncated at 1 000 random byte offsets with a stale header recovers
  floor((size − header) / 4) samples each time, with no panic.
- **AC-7 (input dropout: continue, fill, mark).** Given a fake input that drops 480 frames (10 ms)
  at capture time 3.000 s during a 6 s recording at 48 kHz, then:
  - recording continues;
  - the take equals the source with samples [144 000, 144 480) replaced by 0.0, with ±1 sample at the
    edges, so every delivered sample sits at its source index ±1;
  - one marker "Dropout 10 ms" is at 144 000 ± 1;
  - the dropout counter reads 1;
  - on Stop, the notice with "Go to first" appears, and "Go to first" moves the cursor to that marker.
  With timestamps marked unreliable, the take is not padded and the marker reads "Dropout (length
  unknown)".
- **AC-8 (capture-ring overflow stops and keeps).** Given the capture-writer stalled by fault
  injection for 12 s during recording, when the ring overflows:
  - recording stops within one control tick of the overflow being detected;
  - the take is finalized with every sample up to the overflow bit-identical to the source, and is
    committed as one undoable edit;
  - the overflow notice appears;
  - the UI stays responsive, with no frame over 100 ms.
- **AC-9 (monitoring modes; never recorded).** Given same-clock fake devices at 48 kHz, a rack with a
  −6 dB Gain, and a 997 Hz −20 dBFS input, when armed with playback stopped:
  - **Off:** the output is digital silence (`-inf`).
  - **Dry:** the output tone RMS is −23.01 ± 0.05 dB, so the rack is not applied.
  - **Through rack:** it is −29.01 ± 0.05 dB.
  - Switching mode produces no sample step above the zipper criterion of SPEC-012 §4.3 with
    T_s = 10 ms.
  - Takes recorded in the three modes from the same source have identical FNV-1a hashes.
- **AC-10 (monitoring latency readout).** Given fake devices with 256-frame periods at 48 kHz, input
  latency 5 ms and output latency 7 ms, when an impulse is injected at the input after 10 s of
  convergence:
  - the measured delay from its capture time to its playback time equals the displayed readout within
    ±1 ms;
  - adding a test module that reports 480 samples of latency, in through-rack mode, raises both the
    readout and the measured delay by 10.0 ± 0.5 ms;
  - in Dry mode it raises neither.
- **AC-11 (drift and rate mismatch).** Given fake devices whose true rates differ by +200 ppm, then
  by −200 ppm, and in a third run 48 000 Hz in → 44 100 Hz out, when Dry monitoring runs for 10 min:
  - after the first 10 s there are 0 monitor underruns and 0 overruns;
  - monitoring latency stays within F* ± 1 ms;
  - a 997 Hz input tone measured against the output device's own clock is 997 Hz ± 20 ppm, averaged
    over the last 60 s.
- **AC-12 (latency warnings).** Given readouts of 19.9, 20.0, 39.9 and 40.0 ms, the record panel shows
  no warning, amber, amber and red respectively. The feedback hint appears only the first time
  monitoring is enabled.
- **AC-13 (disk space).** Given a fake free-space provider, the remaining-time display equals
  (free − 512 MiB) / (8 × rate) within ±1 %. When free space falls below 512 MiB during recording:
  - recording stops within 1 s;
  - the take up to that point is kept, bit-identical;
  - the disk notice appears;
  - compaction and undo dropping (SPEC-004) never run while recording.
- **AC-14 (input device lost during recording).** Given the fake backend raises `DEVICE_LOST` on the
  input stream mid-take, then:
  - the take is finalized at the last good sample and committed as one undoable edit;
  - the notice appears;
  - there is no panic, and recording doesn't resume when the device returns.
- **AC-15 (marker while recording).** Given a recording in progress, when Add marker is pressed at
  app time T, then after Stop the take contains a marker within ±10 ms of the take position captured
  at T (the SPEC-003 AC-6 bound), and undoing the take removes it.
- **AC-16 (output device lost while recording).** Given a recording in progress with Dry monitoring,
  when the fake backend raises `DEVICE_LOST` on the **output** stream only, then recording continues
  with no gap in the take (every sample bit-identical to the source), monitoring output stops, the
  device-lost banner appears within 2 s, and Stop later commits the whole take as one undoable edit.

## 6. Test plan

| AC | Unit | Integration (fake backend) | Manual smoke (owner, Arch/PipeWire) |
|---|---|---|---|
| AC-1 | meter math on synthetic buffers (vs testkit `peak_dbfs`/`rms_db`) | fake input tone → `VXTM` fields | talk into the mic, compare with `pw-top`/an external meter |
| AC-2 | clip detector + event merging | injected full-scale samples → flag, latch, count | clap near the mic, check the lamp latches |
| AC-3 | take-length trimming arithmetic | record fake source, compare hash and length | record 10 s, inspect the session take |
| AC-4 | capture-writer resampling (`dsp::resample`) | fake 44.1 kHz input device | only if a 44.1-only device is available |
| AC-5 | project: take edit = pieces + markers | record → undo → redo, hash compare | record, Ctrl+Z, Ctrl+Shift+Z |
| AC-6 | WAV-tail recovery on truncated files (1 000 offsets) | child process + `SIGKILL`, restart, recover | `kill -9` the app mid-take, restart, recover |
| AC-7 | gap estimation from synthetic timestamps | fake input with a dropped span | load the machine (`stress`) at 64-frame buffers, check markers |
| AC-8 | — | fault-injected stalled writer | — |
| AC-9 | monitor mix routing | three modes, level + hash checks | listen in all three modes |
| AC-10 | latency formula | impulse through fake devices | clap and listen; compare with the readout |
| AC-11 | PI controller step response | simulated ±200 ppm and 48 k→44.1 k runs, 10 min simulated | USB mic + laptop headphones for 10 min, no clicks |
| AC-12 | warning thresholds (Vitest) | — | change the buffer size, observe the warnings |
| AC-13 | remaining-time formula | fake free-space provider | — |
| AC-14 | device-lost state machine | `DEVICE_LOST` injected mid-take | unplug a USB mic mid-take |
| AC-15 | position extrapolation | marker command during a fake take | press M while recording |

Fixtures: testkit `sine`, `white` (seeded) and `impulse` generators. Fake-backend scripts live with
T-105/T-106/T-107, and no audio files are committed.

## 7. Out of scope
- Recording at the cursor (insert/overwrite), punch-in with pre/post-roll, latency calibration (M3,
  T-304).
- Pause during recording (v1). Input gain or level control (use the device or OS mixer). Stereo or
  multi-channel recording (PROMPT §3.8).
- Automatic repair of dropouts or clips.
- Saving and bit-depth conversion or dither (M2, WAV spec).
- The Record key binding (SPEC-019). Sandboxed-plugin monitoring implementation (M8).
- The output-device meter and analyzer (SPEC-003 / M2 specs).

**Open questions** — 1 and 2 resolved at the M0 checkpoint (recording continues on output loss, AC-16;
Record works with only an input device, SPEC-001 §2.3). Original questions:
1. Should recording continue when only the **output** device is lost? The take doesn't need it.
   SPEC-001 §2.4 currently stops, and this spec follows it for M1.
2. SPEC-001 §2.3 disables the transport without an output device. New-file recording needs only an
   input. Recommendation: keep Record enabled with an input device alone.
3. Should the per-take clip count also add markers, like dropouts? The recommendation is no for v1:
   clips are best found with the waveform/peak tools planned for M2.

## Amendment 1 — H-112 selectable input meter floor (2026-09-21, owner-requested)

The owner's words: "The output level bar is very nice and vertical and I can see the audio well
when playing. The input level should be the same way and shape — right now it is only a small
horizontal bar. It should definitely look like the Output level, but with options to change the
mic scale's minimum to −60, −80 or −120, so I can see the level of the mic input well."

This supersedes §2.1's original "the scale runs from −60 to 0 dBFS" wherever it appears, and the
inline bullet above is updated to match:

- **Form.** The input meter is now vertical, the same size and visual language as the output meter
  (peak+RMS fill, peak-hold tick, colour zones, scale ticks, `meter.peak`/`meter.rms` numeric
  readouts) — both share one implementation, `ui/src/lib/meters/VerticalMeter.svelte`. The two sit
  side by side in the meter bridge, the natural arrangement for setting a recording level. The
  input meter keeps its own Max readout (highest peak since arming or the last reset) and its
  click-to-reset, which the output meter has no equivalent of. The clip indicator stays where it
  already was (the transport bar, `RecordControls.svelte`) — the hold tick still tints on a latched
  clip, exactly like the output meter's own hold tick.
- **Selectable floor.** A control on the meter itself chooses the scale's minimum: −60 dBFS
  (factory default, unchanged), −80 or −120 dBFS. A quiet microphone or a room-tone check is
  invisible on a −60 floor; −120 shows the noise floor. The choice is persisted
  (`Settings.input_meter_floor`, `meter_input_floor_dbfs` in §3), UI-only — the engine still always
  sends `in_peak_dbfs`/`in_rms_dbfs` in dBFS regardless of what range the UI chooses to display.
  The scale's tick labels are fitted to stay legible (non-colliding) at every floor, the same
  collision-avoidance the output meter's scale already used (`meterScale.ts`).
- **The output meter is unchanged** — its own scale floor stays the fixed −60 dBFS from H-41/H-48;
  the owner likes it as-is, and this amendment only ever generalizes the *shared* implementation to
  support a floor parameter, never applies a selectable floor to the output meter itself.
