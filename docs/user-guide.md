# PowerVoice user guide

PowerVoice is a focused editor for **simple voice-over work**: record mono voice, clean it up,
shape it, hit a loudness target, and export. It edits **one mono audio file at a time** (like Adobe
Audition's Waveform Editor) — there's no multitrack timeline.

## Install

See the [README](../README.md#install) for install instructions per platform, and
[`docs/building.md`](building.md) if you're building from source.

## First recording

1. **File → New Recording…** Choose the sample rate/bit depth (default 48 kHz / 24-bit; internal
   processing is always 32-bit float regardless of what you pick here) and pick your **input
   device** (and input channel, e.g. channel 1 or 2 of a stereo interface) and **output device**
   in Settings → Audio Devices if you haven't already.
2. Click **Input** to arm — this starts the input meter and monitoring without recording yet.
   Choose a monitoring mode: **Off** / **Dry** (hear your raw input) / **Through rack** (hear it
   with the effects rack applied). Off is the default: monitoring through a rack adds the rack's
   latency to what you hear.
3. Click **Record** (or press **Shift+R**) to start, and again to stop. A red **CLIP** indicator
   lights if the input clips; click it to reset. If the input drops out for a moment (device
   hiccup), PowerVoice fills the gap with silence and drops a marker there so you can find it
   later.
4. **File → Save** (**Ctrl/⌘+S**) writes the audio file plus a sidecar `name.wav.vo.json` next to
   it, holding your rack, markers, noise profile and view settings. Autosave/crash recovery runs
   continuously in the background (File → Recovery & Storage… to see or recover an interrupted
   session).

### Re-recording part of a take (punch-in)

Made a mistake in the middle of an otherwise-good take? Select the region to redo and press
**Shift+R**. With a selection active, Record does a **punch-in** by default (Record panel →
**Punch-in on selection**, on by default): PowerVoice plays a **pre-roll** (default 5 s) up to the
selection so you can match the earlier read's pace and tone, records over exactly the selected
region, then plays a **post-roll** (default 1 s) so you can hear how the join sounds. A short
crossfade (default 10 ms) blends the new audio in at both edges.

- **Mode**: **Insert** (default — adds without destroying anything) or **Overwrite** (audiobook
  "punch-and-roll" style, replaces from the record point onward).
- Turn **Punch-in on selection** off if you'd rather a selection be ignored and Record just insert
  at the selection's start.
- **Pre-roll at cursor** (off by default) turns plain cursor recording (no selection) into
  "punch-and-roll": lead-in playback before recording starts at the cursor.
- Punch-in needs an **output device** (for pre-roll/post-roll); recording at the cursor with no
  selection still works with only an input device, just without a lead-in.

All of these live in the Record panel's **Punch & pre-roll** section (same settings also appear in
**Preferences → Recording**), and are app-wide preferences, not saved per document.

### Latency calibration

If a word spoken in time with what you hear during a punch doesn't land in time on the timeline,
your interface's round-trip latency isn't compensated yet. Click **Calibrate…** next to the
**Recording offset** readout: connect a cable from an output of your interface back into the
selected input (most accurate), or hold the microphone within 5 cm of a speaker/headphone cup, then
**Start**. PowerVoice plays a short test sweep and measures the round trip 5 times; it needs most
of those 5 measurements to agree before it accepts a result. Recalibrate if you change your audio
buffer size — the readout tells you when your current buffer size no longer matches the one you
calibrated at.

## Cleaning up: the effects rack

The rack is a chain of **non-destructive** effects, processed in real time during playback,
monitoring and export — nothing is baked into the file until you export. Add modules from the rack
panel; drag to reorder; click an effect's bypass toggle to A/B it.

Built-in modules: **Noise Gate**, **Noise Reduction** (spectral, needs a captured noise print —
select 0.5–60 s of room tone/silence and use Capture Noise Print, provisionally **Shift+P**),
**Parametric EQ** (HPF/LPF + low/high shelf + 5 peaking bands), **Dynamics** (auto-gate → expander →
compressor → limiter, compressor on by default), and a **True-Peak Limiter** (a safety ceiling for
loudness delivery).

### Rack presets

Rather than build a chain from scratch, load a **rack preset** from the rack panel's preset menu:

| Preset | What it's for | Chain (roughly) |
|---|---|---|
| **Podcast voice** | A fuller, more processed voice for podcasts/streaming | rumble filter → noise gate → compressor (3.5:1) → EQ (a little low-mid cut, a little presence/air lift) → limiter at −1 dBTP |
| **Audiobook (ACX)** | Meeting Audible's ACX submission requirements with headroom | rumble filter → gentle compressor (2:1, high threshold) → limiter at −3 dBTP |
| **Gentle cleanup** | Light-touch polish that doesn't obviously "sound processed" | rumble filter → light noise gate → mild compressor (1.5:1) → limiter at −1 dBTP |

Loading a preset over a non-empty rack asks for confirmation first (rack edits aren't part of
undo/redo in v1). Every built-in module also has its own presets (its own preset menu in its
header), and you can save your own module/rack presets from the current settings.

## Exporting, and the ACX check

**File → Export…** Choose:

- **Format**: WAV (16/24/32-bit float), FLAC (16/24-bit — no 32-bit float FLAC), or MP3 (CBR/VBR,
  needs a system LAME install — see [Troubleshooting](#troubleshooting) if it's greyed out with
  "(needs libmp3lame)").
- **Range**: whole file or just the current selection.
- **ACX preset**, if you want export settings that match Audible's ACX submission format.

Exports always render through the full rack (the rack panel's "Listening only" badge on an A/B'd
module is a reminder that only *exports* reflect the true, non-bypassed signal).

Before exporting, open the **ACX Check** panel to see whether the file would pass ACX's three
numeric requirements, each measured against the whole file (or your selection):

| Rule | Limit |
|---|---|
| RMS | −23 … −18 dB |
| Peak | ≤ −3 dBFS |
| Noise floor (quietest 500 ms) | ≤ −60 dB |

A failing rule comes with a one-line hint (e.g. "RMS −27 dB is quieter than −23 dB: LUFS/RMS
normalize up.", or "Noise floor −52 dB exceeds −60 dB: capture a noise print and add Noise
Reduction."). One-click **Normalize to X dB** (peak) or **Normalize to X LUFS** favorites are
available from the toolbar/Edit menu, or use the **Audiobook (ACX)** rack preset above, which
already keeps peaks under the ACX ceiling via its limiter.

## Shortcuts

Generated from the keymap registry (`ui/src/lib/keymap/bindings.ts`) — this is the single source of
truth the app itself uses, so it can't drift from what's actually bound. Shortcuts aren't
remappable in v1. On macOS, Ctrl/⌘ in the app's own menus becomes ⌘; the table below shows both.

<!-- SHORTCUTS:START (generated by scripts/docs/generate_shortcuts.mjs) -->
| Action | Windows / Linux | macOS |
|---|---|---|
| Play / pause | `Space` | `Space` |
| Play from start | `Shift+Space` | `⇧Space` |
| Return to start | `Home` | `Home` |
| Record (start/stop) — provisional binding | `Shift+R` | `⇧R` |
| Add marker | `M` | `M` |
| Undo | `Ctrl+Z` | `⌘Z` |
| Redo | `Ctrl+Shift+Z` | `⇧⌘Z` |
| Open | `Ctrl+O` | `⌘O` |
| Save | `Ctrl+S` | `⌘S` |
| Save As | `Ctrl+Shift+S` | `⇧⌘S` |
| Zoom in (waveform) | `=` | `=` |
| Zoom out (waveform) | `-` | `-` |
| Cut | `Ctrl+X` | `⌘X` |
| Copy | `Ctrl+C` | `⌘C` |
| Paste | `Ctrl+V` | `⌘V` |
| Delete selection | `Delete` | `Delete` |
| Trim to selection (Crop) | `Ctrl+T` | `⌘T` |
| Select all | `Ctrl+A` | `⌘A` |
| Clear selection | `Esc` | `Esc` |
| Delete selected marker(s) | `Ctrl+0` | `⌘0` |
| Go to next marker | `Ctrl+Alt+→` | `⌥⌘→` |
| Go to previous marker | `Ctrl+Alt+←` | `⌥⌘←` |
| Capture Noise Print — provisional binding | `Shift+P` | `⇧P` |
| Show/hide the spectral pane | `Shift+D` | `⇧D` |
<!-- SHORTCUTS:END -->

Two bindings above are marked provisional in the keymap itself (Record, Capture Noise Print) —
they match Audition's defaults per the best sources found so far, but may still change before v1
ships (SPEC-019 finalizes the shortcut map).

## Troubleshooting

### No audio devices show up / wrong devices listed (Linux: PipeWire, JACK, ALSA)

Settings → Audio Devices lists whatever your audio backend reports, refreshed automatically on
hot-plug. On Linux, PowerVoice picks a backend at build time (via `cpal`) in this order:

1. **PipeWire**, if a PipeWire server is reachable (most current distros default to this — the
   product owner develops against PipeWire 1.6.8).
2. **ALSA**, otherwise.
3. **JACK**, listed additionally whenever a JACK server is running (`jackd`/`jackdbus`) and the
   `jack` cpal feature is compiled in.

If nothing shows up: confirm your audio server is actually running (`systemctl --user status
pipewire pipewire-pulse` or `wireplumber` for PipeWire; `jack_control status` for JACK), and that
your user is in the right groups for raw ALSA device access if you're bypassing PipeWire/JACK
entirely (Arch/most distros: the `audio` group, though PipeWire itself usually doesn't need it).
Device enumeration runs off the UI thread because it can block for hundreds of milliseconds on some
ALSA setups — a slow-to-populate list is normal on first open, not a bug by itself.

### The UI feels laggy or tears on Linux (WebKitGTK rendering)

PowerVoice's waveform and spectrogram views target 60 fps, drawn with WebGL2 (falling back to
Canvas2D automatically if WebGL2 isn't available, or if the WebGL2 context is lost mid-session).
On Linux, WebKitGTK's GPU (DMA-BUF) compositing path measured noticeably worse frame times during
this project's own testing (not limited to one GPU vendor) — so PowerVoice **disables it by
default on Linux**, automatically, before its window even opens
(`WEBKIT_DISABLE_DMABUF_RENDERER=1`, set by the app itself unless already set). You shouldn't need
to do anything. If you want to try the default WebKit DMA-BUF renderer instead (e.g. a driver
update changed the tradeoff on your system), opt out with:

```sh
POWERVOICE_WEBKIT_DMABUF=1 powervoice-app     # or however you launch the AppImage/installed binary
```

Setting `WEBKIT_DISABLE_DMABUF_RENDERER` yourself (to `0` or anything else) also takes precedence —
PowerVoice never overrides a value you've already set.

### MP3 export is greyed out / "needs libmp3lame"

MP3 encoding uses `libmp3lame` (LAME), which PowerVoice **loads at runtime rather than bundling**
(it's LGPL-licensed — see `docs/adr/ADR-007-licensing.md` §4 for why). Every other feature works
without it; only MP3 export is affected.

- **Linux (.deb)**: `sudo apt install libmp3lame0` (declared as a `Recommends`, so `apt` should
  offer it automatically; if you installed via `dpkg -i` directly, add it yourself). Arch: `sudo
  pacman -S lame`.
- **Linux (AppImage)**: install the same system package — the AppImage doesn't bundle LAME either
  (by design: it's a runtime-replaceable dependency, not something to vendor).
- **Windows**: no OS-level package for it. Download a `libmp3lame.dll` build (e.g. from
  [Rareware's LAME Windows builds](https://www.rarewares.org/mp3-lame-bundle.php)) and place it
  next to `powervoice-app.exe`.
- **macOS**: `brew install lame` (Homebrew) provides `libmp3lame.dylib`.

After installing, restart PowerVoice — it probes for the library at startup.

### Recording sounds delayed / out of sync with playback

See [Latency calibration](#latency-calibration) above. Also check the **Monitoring latency**
readout in the Record panel — a red warning there suggests a smaller audio buffer size, switching
to Dry monitoring, or removing high-latency rack modules (Noise Reduction in particular can add up
to ~50 ms; a *bypassed* module still holds its latency, so bypassing it doesn't help — remove it
instead if you need the latency back).

### A crash or power loss interrupted a recording

Open **File → Recovery & Storage…**. Interrupted takes are recovered from the crash-safe session
journal; markers show where any dropouts or the interruption itself occurred. The dialog stays open
until you've dealt with every recoverable session, so you can't accidentally lose one by dismissing
it too fast.
