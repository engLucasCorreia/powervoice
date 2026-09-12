# SPEC-001 — Audio devices & I/O

- **Status:** draft
- **Milestone:** M1
- **Related:** SPEC-000 (architecture overview, glossary), SPEC-002 (recording & monitoring), SPEC-003
  (transport & playback), ADR-001 (crate graph — `engine` owns the backend), ADR-002 (threading &
  real-time rules — device-poll thread, output/input stream lifecycle, device-loss handling),
  ADR-003 (`devices_changed` event, `notice` event)

## 1. Purpose

VoxEdit records and plays back through one input device/channel and one output device at a time
(§3.1). Before any recording or playback can happen, the user has to be able to see what hardware is
available, pick it, and trust that VoxEdit will tell them — clearly and without crashing — when that
hardware misbehaves (unplugged mid-session, claimed by another app, wrong sample rate). This spec
defines that behavior: enumeration, selection, fallback rules, hot-plug, device loss/recovery, and
persistence. It does not define recording or playback mechanics themselves (SPEC-002, SPEC-003).

## 2. Behavior / UX

### 2.1 Settings surface

Device configuration lives in **Settings → Audio Devices** (PROMPT §3.6) and is also reachable from a
small device-status control in the transport bar (shows the current output device name; click opens
the same settings page). The panel has, top to bottom:

1. **Host** — a dropdown of audio backends available on this OS. Linux: **PipeWire** is the default
   when the `pipewire` cpal feature is compiled in and a PipeWire server is reachable (the owner's
   machine runs PipeWire 1.6.8); otherwise **ALSA** is the default. **JACK** is listed when its
   feature is compiled in and a server is reachable (MEMORY.md: cpal 0.18 needs the `pipewire`/`jack`
   features enabled; ALSA headers are always required). Windows: **WASAPI**.
   macOS: **CoreAudio**. Changing host re-enumerates devices for that host only.
2. **Input device** — dropdown of input-capable devices on the selected host, plus "None". Selecting
   "None" disarms recording/monitoring input entirely.
3. **Input channel** — dropdown of 1..N, N = the selected device's max input channels (cpal has no
   per-channel input API — VoxEdit opens the full device and deinterleaves; MEMORY.md). Disabled
   when input device is "None". Mono only: exactly one channel is captured.
4. **Output device** — dropdown of output-capable devices on the selected host.
5. **Sample rate** — dropdown of common rates (44100, 48000, 88200, 96000, 176400, 192000 Hz)
   intersected with what the *currently selected output device* reports supporting. Unsupported
   values are hidden, not shown disabled.
6. **Buffer size** — dropdown of common sizes (64, 128, 256, 512, 1024, 2048 frames) intersected with
   the output device's supported buffer-size range, plus "Auto" (device default), which is the
   factory default.
7. A **status line** per device (see §2.3) and a **Rescan** button (manual re-enumeration, in
   addition to automatic hot-plug polling).

### 2.2 Selection & fallback rules

- Enumeration lists every device the selected host currently reports. Devices are identified to the
  user by their host-reported name; internally VoxEdit keys persisted choices by a stable
  `(host, name)` pair, not by index (cpal indices are not stable across enumerations).
- If the user's chosen **sample rate** is not in the output device's supported set (e.g. after
  switching to a different device, or a device firmware that changed its supported list), VoxEdit
  falls back to that device's **default sample rate** and shows a one-line notice: "48000 Hz isn't
  supported by *Device X* — using 44100 Hz." The dropdown updates to reflect the fallback value; the
  user's original preference is not silently forgotten — see persistence (§2.5).
- If the chosen **buffer size** is outside the device's supported range, VoxEdit clamps to the
  nearest boundary (min or max) of that range and shows the same style of notice.
- If, after a fallback, the resulting stream still fails to open, the input or output is treated as
  "device lost" (§2.4) even though it was never lost — the messaging is the same because the user
  action is the same (pick another device/rate).
- The document's own sample rate (set when a new file is recorded or an existing file is opened) is
  independent of the device's rate; a mismatch is handled by resampling in the engine, not by this
  panel (SPEC-003 §2, ADR-002 §5).

### 2.3 Per-device/stream status shown in the UI

| State | What the user sees |
|---|---|
| No device selected | Greyed control, "No output device selected" placeholder, transport disabled |
| Selected, stream open, healthy | Device name, a small steady green dot |
| Selected, fallback applied (rate/buffer) | Device name, an amber dot, and the fallback notice (§2.2), dismissible, reappears only on the next fallback event |
| Selected, device lost | Device name struck through or dimmed, a red dot, persistent banner: "Output device disconnected — recording/playback stopped." with a manual "Rescan" action |
| Lost device back online (auto-recovered) | Banner briefly changes to "Output device reconnected." (auto-dismiss after ~4 s), dot returns to green |
| Hot-plugged device appearing for the first time | Silently added to the relevant dropdown(s); no notice unless it is the currently-configured-but-missing device reappearing |

### 2.4 Device-lost behavior

Device loss is detected two ways (ADR-002 §7): the backend's error callback sets an atomic flag
(immediate), and/or the ~1 s device-poll no longer reports the device (used to confirm it is gone,
not just erroring transiently, and to detect replug).

On loss of the **output** device or the **input** device while it is in use (recording or monitoring
open, SPEC-002):
1. The engine stops the transport (a short fade, not an abrupt sample drop) within one control tick.
2. If a recording was in progress, the in-progress take is finalized as a crash-safe file, not
   discarded (SPEC-002/ADR-004).
3. The app does not crash, freeze, or lose the open document. Every other panel keeps working.
4. The status banner and notice from §2.3 appear.
5. The engine keeps polling for the device; it does not require the user to reopen Settings.

On replug of the **same** device (matched by `(host, name)` — VoxEdit does not require the OS to
reuse the same numeric device ID):
1. The engine reopens the stream with the previously-configured rate/buffer (applying the fallback
   rules of §2.2 again if the replugged device's capabilities changed).
2. Playback/recording that was interrupted is **not** automatically resumed — the user presses
   Play/Record again. Auto-resuming audio right after a physical replug (e.g. of headphones) would
   be surprising and is out of scope for v1.
3. The reconnected notice (§2.3) is shown.

### 2.5 Persistence

Host, input device, input channel, output device, sample rate and buffer size are saved to the app
settings file as soon as they are successfully applied (not merely selected in the UI — a value that
triggered a fallback per §2.2 is saved as the user's *intent*, so VoxEdit retries it the next time
that device's capabilities might have changed). On startup, VoxEdit attempts to open the saved
devices; if a saved device is absent, it falls back to the host's reported default device for that
direction and shows: "*Device X* not found — using default output *Device Y*." The saved preference
is kept, not overwritten, so returning the original device later restores it (subject to §2.2
fallback rules at that time).

### 2.6 Platform notes (unverified)

This spec's enumeration and fallback rules are written to be backend-agnostic. Only the Linux path
(ALSA/PipeWire/JACK via cpal, on the product owner's Arch + PipeWire 1.6.8 + Hyprland machine) is
verified by manual smoke test for M1. **WASAPI (Windows) and CoreAudio (macOS) are described here but
unverified** — nobody has run this code on those platforms yet (PROMPT §"Test platform", MEMORY.md
risk). Track as a known risk until someone tests them.

## 3. Parameters

| id | name | unit | range | default | taper/step | notes |
|---|---|---|---|---|---|---|
| `host` | Audio host/backend | enum | platform-dependent: `alsa`\|`pipewire`\|`jack` (Linux), `wasapi` (Windows), `coreaudio` (macOS) | Linux: `pipewire` if available, else `alsa`; Windows: `wasapi`; macOS: `coreaudio` | n/a | re-enumerates devices on change |
| `input_device` | Input device | id/name string, or "None" | devices reported by `host` | "None" | n/a | keyed by `(host, name)`, not index |
| `input_channel` | Input channel | index | `1..=N` (N = device max input channels) | `1` | step 1 | disabled if `input_device` = "None" |
| `output_device` | Output device | id/name string | devices reported by `host` | host default output device | n/a | keyed by `(host, name)` |
| `sample_rate_hz` | Sample rate | Hz | {44100, 48000, 88200, 96000, 176400, 192000} ∩ device-supported | device default (typically 48000) | discrete list | independent of document rate |
| `buffer_size_frames` | Buffer size | frames | {64,128,256,512,1024,2048} ∩ device-supported, or "Auto" | "Auto" | discrete list | clamped to device range if out of bounds |

## 4. Algorithm / implementation notes

- Backend: `cpal` 0.18, confined to `engine` (ADR-001 §3). Input and output are **separate streams**,
  opened/closed independently; there is no combined full-duplex stream object (MEMORY.md). The
  output stream runs continuously once a device is configured (emits silence when idle, so Play never
  pays device start-up latency); the input stream opens on demand only when armed/monitoring/recording
  (ADR-002 §1), which is why an input device can show "healthy" in Settings without a live stream.
- Enumeration: a dedicated **device-poll** thread (ADR-002 §1), not the UI or the control thread,
  calls into cpal roughly every 1 s and posts *diffs* (added/removed device names) to the control
  thread, because ALSA enumeration can block for hundreds of ms. The Settings panel's device lists
  and the transport-bar status both read the control thread's current device-list snapshot; opening
  Settings does not itself trigger a fresh blocking enumeration (the Rescan button does, off the UI
  thread).
- Device loss detection: cpal error callbacks (which may run on any thread) only set bits in an
  `AtomicU32` (`DEVICE_LOST`, `BACKEND_ERROR`); the control thread's 16 ms tick checks these flags
  and drives the state machine in §2.4. This keeps the RT audio callback free of anything but the
  flag write (ADR-002 §2, §7).
- Sample-rate fallback for the *device* (this spec) is a different mechanism from the *document*
  sample-rate handling in ADR-002 §5 (opening the device at the document's rate, or resampling in the
  reader when the device can't run at it) — this spec's fallback only governs what the Settings panel
  offers/applies when the device's own supported set doesn't include the user's chosen value.
- `input_channel` selection: cpal has no per-channel input capture, so the input callback always
  receives the full interleaved device buffer and deinterleaves, keeping only the selected channel
  (MEMORY.md); changing the channel is therefore free (no stream reopen) and takes effect at the next
  callback.
- Persistence file: the app settings file managed by `src-tauri` (ADR-003 "settings-file plumbing").
  This spec does not fix its on-disk schema — only the user-visible save/restore/fallback behavior.

## 5. Acceptance criteria

- **AC-1 (enumeration).** Given the Settings → Audio Devices panel is opened, when the device-poll
  thread has completed at least one pass (≤ 1 s after engine start), then every currently connected
  input and output device for the selected host is listed, with no duplicate entries and no entries
  for devices that are not actually present.
- **AC-2 (input channel selection).** Given a 2-input-channel interface is selected with input
  channel = 2, when audio is present only on the interface's physical channel 2, then the live input
  meter (SPEC-002) reflects that signal and shows silence when audio is present only on channel 1,
  confirmed within one meter update interval (≤ 33 ms at the 30 Hz telemetry rate, ADR-003).
- **AC-3 (sample-rate fallback).** Given an output device whose supported rates do not include
  192000 Hz and the user selects/persists 192000 Hz, when VoxEdit opens that device, then it opens at
  the device's default supported rate instead, the Settings panel shows that rate, and a fallback
  notice naming both the requested and applied rate is shown within 1 s — VoxEdit does not fail to
  start audio.
- **AC-4 (buffer-size fallback).** Given a device whose supported buffer-size range is [128, 1024]
  frames and the user has 2048 frames selected, when VoxEdit opens that device, then the applied
  buffer size is clamped to 1024 frames and the same style of notice (AC-3) is shown.
- **AC-5 (hot-plug refresh).** Given the Settings panel is open (or closed — polling is
  background), when a USB audio device is physically connected or disconnected, then the relevant
  device dropdown(s) reflect the change within **≤ 2 s** of the physical event (device-poll interval
  ~1 s plus one control tick).
- **AC-6 (device-lost, output, mid-playback).** Given VoxEdit is playing back through output device D,
  when D is physically disconnected, then: transport stops within one control tick (≤ 16 ms after the
  loss is detected) with a fade rather than a sample-accurate discontinuity, the app does not panic or
  become unresponsive (other panels remain interactive), and the device-lost banner (§2.3) appears
  within 2 s.
- **AC-7 (auto-recovery on replug).** Continuing from AC-6, given device D is then physically
  reconnected, when the device-poll thread next reports it, then D becomes selectable/usable again
  (stream reopened with the previously-applied rate/buffer) within ≤ 2 s, a reconnected notice is
  shown, and playback does **not** auto-resume (the user must press Play again).
- **AC-8 (persistence across restart).** Given the user has configured host/input device/channel/
  output device/rate/buffer and closes VoxEdit, when VoxEdit is relaunched with the same devices still
  present, then all six values are restored exactly with no additional notices; when relaunched with
  the saved output device absent, then VoxEdit falls back to the host's default output device, shows
  the "not found — using default" notice (§2.5), and still retains the original saved preference (not
  overwritten by the fallback) for the next launch.

## 6. Test plan

| AC | Unit | Integration (fake backend) | Manual smoke (owner, Arch/PipeWire 1.6.8/Hyprland) |
|---|---|---|---|
| AC-1 | Diff/merge logic for device-list snapshots | Fake backend reports a fixed device set; assert Settings-panel model matches within one poll tick | Open Settings, confirm real ALSA/PipeWire devices listed |
| AC-2 | Deinterleave-and-select-channel function, given synthetic multi-channel buffers | Fake input backend with channel 1 = tone, channel 2 = silence (and vice versa); assert selected-channel meter output | Physical 2-in interface, tone into channel 2 only, confirm meter |
| AC-3 | Fallback-selection function given a synthetic supported-rate set | Fake backend advertises a limited rate set; assert applied rate + notice event | Select an unsupported rate for a real device if available; else defer to fake-backend coverage |
| AC-4 | Same fallback function for buffer-size clamping | Fake backend advertises a narrow buffer range; assert clamped value + notice | n/a (buffer ranges rarely restrictive on real hardware; fake backend is authoritative) |
| AC-5 | n/a (timing behavior, not pure logic) | Fake backend/device-poll driven by a simulated clock; inject add/remove events, assert UI-facing diff arrives within 2 simulated seconds | Physically unplug/replug a USB audio device, stopwatch the dropdown update |
| AC-6 | Device-lost state-machine transitions (unit-testable pure state machine) | Fake backend raises the lost-device error flag mid-playback; assert transport-stop event, no panic, banner event, within budget | Unplug the real output device (e.g. USB DAC) while VoxEdit plays; confirm no crash and the banner |
| AC-7 | Recovery state-machine transitions | Fake backend clears the lost flag and re-reports the device; assert reopen with prior config, no auto-resume | Replug the same device; confirm reconnect notice and that playback does not resume by itself |
| AC-8 | Settings load/save round-trip (serialize/deserialize), fallback-not-overwriting-preference logic | Engine startup against a fixture settings file with a missing device name; assert default-device fallback + notice + preference preserved on save | Restart the real app twice: once with all devices present, once with the output device unplugged |

## 7. Out of scope

- Recording, monitoring modes, and the pre-record meter's clip-hold behavior (SPEC-002).
- Transport mechanics, playhead, and document/device sample-rate resampling *of the playback path*
  (SPEC-003; this spec only defines the device-side rate/buffer selection and fallback).
- Punch-in pre-roll/post-roll (SPEC-002 / a future recording spec).
- The on-disk settings file schema (an engineering concern for the implementing ticket, not user
  behavior).
- WASAPI/CoreAudio verification (flagged in §2.6 as a risk, not resolved here).
