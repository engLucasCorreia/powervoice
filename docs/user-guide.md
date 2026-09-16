# PowerVoice user guide

PowerVoice is a focused editor for **simple voice-over work**: record mono voice, clean it up,
shape it, hit a loudness target, and export. It edits **one mono audio file at a time** (like Adobe
Audition's Waveform Editor) — there's no multitrack timeline. New to PowerVoice? [What is
PowerVoice?](what-is-powervoice.md) is the two-minute pitch, and [How PowerVoice
works](how-it-works.md) explains what happens to your sound as you use it. Words in *italics* or
linked to the [glossary](glossary.md) get a plain-language explanation there.

This guide is organised by task, roughly in the order you'll use them:

[Guided tours](#guided-tours) · [Install](#install) · [Set up your microphone](#set-up-your-microphone) ·
[First recording](#first-recording) · [Punch-in](#re-recording-part-of-a-take-punch-in) ·
[Latency calibration](#latency-calibration) · [Markers](#markers) · [Edit and undo](#editing-and-undo) ·
[Waveform and spectral views](#waveform-and-spectral-views) · [The effects rack](#cleaning-up-the-effects-rack) ·
[Remove background noise](#remove-background-noise) · [Make your voice sound better](#make-your-voice-sound-better) ·
[Rack presets](#rack-presets) · [Apply the rack (bake)](#apply-the-rack-bake) · [Plugins](#plugins) ·
[Presets](#presets) · [Check your levels](#check-your-levels-analyzer-and-diagnostics) ·
[Hit a loudness target](#hit-a-loudness-target) · [Export](#export) · [Themes](#themes) ·
[Preferences](#preferences) · [Shortcuts](#shortcuts) · [Troubleshooting](#troubleshooting)

## Guided tours

The first time PowerVoice starts it offers a two-minute **Welcome tour** that walks through choosing
your devices, recording a practice take, navigating the take, markers, the effects rack, noise
reduction, meters and loudness, and export. Replay it any time from **Help → Take the Tour**, or pick
any tour from **Help → Tours**: Welcome tour, Effects rack and presets, Noise print and reduction,
Loudness, ACX and normalize, Punch-in, and Plugin Manager. A **?** in the Rack, Loudness, Noise
Reduction, Punch & pre-roll and Plugin Manager headers starts a short tour of that panel. Use
**→**/**Enter** and **←** to move, **Esc** to leave.

## Install

See the [README](../README.md#download) for install instructions per platform, and
[`docs/building.md`](building.md) if you're building from source.

## Set up your microphone

Click the **Audio devices…** button in the toolbar (the gear icon) to open **Audio Devices**:

- **Host**: the audio system PowerVoice talks to (PipeWire, ALSA or JACK on Linux; WASAPI on
  Windows; Core Audio on macOS). See [No audio devices show
  up](#no-audio-devices-show-up--wrong-devices-listed-linux-pipewire-jack-alsa) if this list is
  empty.
- **Input device**, and its **input channel** if it has more than one (for example, channel 1 or 2
  of a stereo audio interface) — this is your microphone.
- **Output device** — your speakers or headphones, needed for monitoring and for punch-in's
  pre-roll/post-roll playback.
- **Sample rate** and **buffer size** for the chosen devices. A smaller buffer size lowers latency
  (the delay between making a sound and hearing it through the computer) but asks more of your
  computer; if you hear crackling, try a larger one.

The list refreshes automatically when you plug or unplug a device. Record stays unavailable until
an input device is chosen.

## First recording

1. **File → New Recording…** Choose the sample rate/bit depth (default 48 kHz / 24-bit; internal
   processing is always 32-bit float regardless of what you pick here) and pick your **input
   device** (and input channel, e.g. channel 1 or 2 of a stereo interface) and **output device**
   in [Audio Devices](#set-up-your-microphone) if you haven't already.
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
   continuously in the background — see [Recovering after a crash or power
   loss](#recovering-after-a-crash-or-power-loss).

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
**Edit → Preferences → Recording**), and are app-wide preferences, not saved per document.

### Latency calibration

If a word spoken in time with what you hear during a punch doesn't land in time on the timeline,
your interface's round-trip latency isn't compensated yet. Click **Calibrate…** next to the
**Recording offset** readout: connect a cable from an output of your interface back into the
selected input (most accurate), or hold the microphone within 5 cm of a speaker/headphone cup, then
**Start**. PowerVoice plays a short test sweep and measures the round trip 5 times; it needs most
of those 5 measurements to agree before it accepts a result. Recalibrate if you change your audio
buffer size — the readout tells you when your current buffer size no longer matches the one you
calibrated at.

## Markers

Markers are named bookmarks in your take — a retake, a cough, a chapter break — so you can find a
moment again without scrubbing through the whole recording.

- **M** drops a marker at the playhead (or **Edit → Markers → Add Marker**).
- The **markers list** panel on the left shows every marker with its start time and duration;
  clicking one jumps the playhead there. Double-click a marker's name to rename it.
- **Ctrl/⌘+Alt+→** / **←** jump to the next/previous marker (also **Edit → Markers → Next/Previous
  Marker**).
- **Ctrl/⌘+0** deletes the selected marker(s) (also **Edit → Markers → Delete Selected Marker**).
- Markers are saved in the sidecar and, on export, as WAV `cue`/`LIST adtl` chunks, so they travel
  with the file into other software that reads them.

## Editing and undo

The usual clipboard operations work on the current selection: **Cut** (**Ctrl/⌘+X**), **Copy**
(**Ctrl/⌘+C**), **Paste** (**Ctrl/⌘+V**), **Delete** (**Delete**), **Trim to Selection** (keep only
the selection, **Ctrl/⌘+T**) and **Silence** (replace the selection with silence, keeping its
length) — all in the **Edit** menu, and all destructive-but-undoable edits on your document (the
effects rack itself is separate and non-destructive — see [the effects
rack](#cleaning-up-the-effects-rack)).

**Undo** (**Ctrl/⌘+Z**) and **Redo** (**Ctrl/⌘+Shift+Z**) step back and forward through every edit,
with no limit other than free disk space — see [why undo never runs
out](how-it-works.md#why-your-original-recording-is-never-damaged). Undoing a [baked
rack](#apply-the-rack-bake) restores the rack you baked, as well as the audio.

## Waveform and spectral views

The main **waveform** view shows your take's shape over time; the **spectral view** (a
*spectrogram* — a picture where height is pitch, left-to-right is time and brightness is loudness)
shows its frequency content, which makes hum, clicks and breaths easy to spot. Toggle it with
**Shift+D** or the Spectral button in the toolbar.

- **Zoom in/out**: **=** / **-** (waveform), **Alt+=** / **Alt+-** (vertical, amplitude), **Alt+0**
  resets vertical zoom. The toolbar's **Zoom to Selection** and **Zoom Full** buttons jump straight
  to a range. You can also scroll with the mouse wheel and zoom with Ctrl + wheel.
- **Select**: drag to select a range, **Ctrl/⌘+A** selects all, **Esc** clears the selection.
  **←**/**→** nudge the cursor or selection, **Shift+←**/**Shift+→** extend it.
- **Snap to zero crossing** (View menu, off by default): selection edges snap to the nearest point
  where the waveform crosses silence, so cuts and edits never click.
- **Time format**: View → Time Format shows the ruler and readouts as Timecode, Samples or
  Seconds.
- **Loop playback** (**Ctrl/⌘+L**, or View → Loop Playback) repeats the current selection
  (10 ms or longer) instead of stopping at its end — handy for checking how a phrase or an edit
  sounds. A loop doesn't reset the effects rack at the seam, so a reverb or delay tail from a
  plugin carries across the repeat, the same as it would if you kept playing normally.

## Cleaning up: the effects rack

The rack is a chain of **non-destructive** effects, processed in real time during playback,
monitoring and export — nothing is baked into the file until you export or use [Apply the rack
(bake)](#apply-the-rack-bake). Add modules from the rack panel's **Add module** menu; drag a
module by its grip to reorder it; click a module's bypass toggle to A/B it against the rest of the
chain.

Built-in modules: **Noise Gate**, **Noise Reduction**, **Parametric EQ**, **Dynamics**, **Gain**,
and a **True-Peak Limiter** (a safety ceiling for loudness delivery). **Third-party plugins** (CLAP,
VST3, LV2, and REAPER's JSFX effects) show up in the same **Add module** menu, grouped by format —
see [Plugins](#plugins).

### Remove background noise

Every recording has some background hiss or hum, even a quiet room. PowerVoice removes it in two
steps:

1. Select **0.5–60 seconds of room tone** — a stretch with only the background noise and no
   speech, ideally right before or after your take. Choose **Effects → Capture Noise Print**
   (provisionally **Shift+P**). PowerVoice studies that noise's frequency makeup — its *noise
   print* — the same way you'd note the constant hum of an air conditioner before deciding how much
   to turn it down.
2. Add **Noise Reduction** to the rack (Capture Noise Print does this for you if it isn't there
   yet). It continuously subtracts a version of that noise print from the whole take. Its two main
   controls are **reduction** (how many dB, at most, to remove) and **amount** (what percentage of
   that reduction to apply) — start low and raise them while listening: too much makes a voice
   sound thin or "watery" (an artifact of removing too much at once). Advanced controls (FFT size,
   sensitivity, spectral/time smoothing) are there for stubborn noise but the defaults suit most
   voice recordings.

Noise Reduction adds a small, fixed delay to the signal (about 43 ms at its default setting) while
it works — PowerVoice automatically keeps your video/picture, meters and export in sync with it, so
you don't need to do anything about that delay yourself.

### Make your voice sound better

Two built-in modules shape the *tone* (which frequencies stand out) and *dynamics* (how loud vs
quiet parts compare) of your voice:

- **Parametric EQ** (equalizer) works like a set of very precise tone controls: a low-cut and
  high-cut filter (to remove rumble below or hiss above the range of speech), a low shelf and high
  shelf (turn a whole low or high region up or down), and five adjustable *peaking bands* that each
  boost or cut a chosen frequency by a chosen amount, over a chosen width (its *Q*). The rack's EQ
  graph shows the exact curve you're drawing, live, as you drag it. A little cut around 200–500 Hz
  can reduce "boominess"; a little boost around 2–5 kHz can add clarity or "presence."
- **Dynamics** evens out how loud and quiet parts of your voice are, in the style of Audition's
  Dynamics panel: a **compressor** automatically turns down parts that go above a *threshold*, by
  an amount set by its *ratio* (so a loud word doesn't jump out over a quiet one), and a
  **limiter** acts as a hard ceiling that nothing can cross. Both report how much gain reduction
  they're applying in real time, shown as a small meter in the rack.

The [Spectrum Inspector and voice diagnostics](#check-your-levels-analyzer-and-diagnostics) can
suggest specific EQ moves (for example, a de-esser frequency for harsh "s" sounds) that you can add
to the EQ with one click.

### Rack presets

Rather than build a chain from scratch, load a **rack preset** from the rack panel's preset menu
(**Effects → Rack Presets**):

| Preset | What it's for | Chain (roughly) |
|---|---|---|
| **Podcast voice** | A fuller, more processed voice for podcasts/streaming | rumble filter → noise gate → compressor (3.5:1) → EQ (a little low-mid cut, a little presence/air lift) → limiter at −1 dBTP |
| **Audiobook (ACX)** | Meeting Audible's ACX submission requirements with headroom | rumble filter → gentle compressor (2:1, high threshold) → limiter at −3 dBTP |
| **Gentle cleanup** | Light-touch polish that doesn't obviously "sound processed" | rumble filter → light noise gate → mild compressor (1.5:1) → limiter at −1 dBTP |

Loading a preset over a non-empty rack asks for confirmation first (rack edits aren't part of
undo/redo in v1). Every built-in module also has its own presets (its own preset menu in its
header) — see [Presets](#presets) for saving, renaming and sharing your own.

### Apply the rack (bake)

**Effects → Bake Rack** permanently applies the current rack to the selection (or the whole file)
as one undoable edit, and then clears the rack. Use it when you want to lock in how something
sounds — for example, before adding a different effect on top that should hear the *processed*
signal rather than the original. Undo brings back both the original audio and the rack you baked.

## Plugins

PowerVoice hosts third-party effects in **CLAP**, **VST3** and **LV2** formats, plus REAPER's
**JSFX** scripts. Every plugin runs in its own protected process — see [Why plugins run in their
own "safety box"](how-it-works.md#why-plugins-run-in-their-own-safety-box) for why that means a
misbehaving plugin can't take PowerVoice or your recording down with it. LV2 and JSFX are available
on Linux and macOS only; LV2 additionally needs the system `lilv` library (see [LV2 plugins are
unavailable](#lv2-plugins-are-unavailable) if it's missing).

Add an installed plugin to the rack the same way as a built-in: **Add module**, under **Plugins
(CLAP)**, **Plugins (VST3)**, **Plugins (LV2)** or **Plugins (JSFX)**.

### The Plugin Manager

Open it from **Effects → Manage Plugins…**. Two tabs:

- **Plugins**: every plugin PowerVoice found, with its format, channel layout, parameter count and
  **status** — OK, Disabled (hidden from Add module), Blocklisted (skipped because it crashed or
  timed out while being scanned, or you blocked it), or Flagged (crashed while running, but still
  usable). Search by name, vendor or path; **Rescan** picks up new or changed files, or scan
  everything again from scratch.
- **Folders**: where PowerVoice looks for plugins — the standard OS locations, plus any custom
  folders you add.

**Effects → Install Module…** copies a CLAP, VST3, LV2 or JSFX plugin file (or a PowerVoice
`.voxmod` package) into your own plugin folder and scans it, so you don't have to find your
system's plugin folders yourself.

### If a plugin crashes

A crash while a plugin is running doesn't take PowerVoice down: the affected rack slot mutes for a
fraction of a second, is marked *Restarting*, and comes back automatically with its last settings.
If it fails again right away, the slot shows *Failed* with a **Retry** button — the rest of the rack
and your recording are unaffected the whole time. A runtime crash is recorded as a "flag" in the
Plugin Manager (**Clear crash warning** dismisses it) but never blocks the plugin from being used
again; only repeated failures *during a scan* add it to the blocklist, which you can undo with
**Unblock and rescan**.

Some plugins have their own graphical editor window — open it from the window icon (⊟) in the
plugin's rack slot (Linux needs X11 or XWayland; not yet supported on Windows or macOS). If a
plugin's window closes because it crashed, reopen it the same way once the plugin has restarted.

## Presets

Beyond the built-in [rack presets](#rack-presets), you can save, load, rename, delete, export and
import presets for the **whole rack** or for **one module** from **Effects → Manage Presets…**
(also reachable from a module's own preset menu, or the rack panel's). Its two tabs, **Rack
presets** and **Module presets**, list the factory presets (marked *Factory*) followed by your own,
alphabetically:

- **Save as preset…** captures a module's or the whole rack's current settings under a name you
  choose (a Noise Reduction preset can optionally include its captured noise print).
- **Export…** / **Import…** write a preset to, or read one from, a file — the way to share a preset
  with someone else, back one up, or move it between computers. Importing over a preset with the
  same name asks before replacing it.
- **Reset to Default** (a module's slot menu) puts every one of its parameters back to the
  factory default without touching a captured noise print.

## Check your levels: analyzer and diagnostics

The dock along the bottom shows what you're hearing:

- **Meters**: a peak/RMS output meter (post-rack) and, while recording, an input meter with a clip
  indicator.
- **Analyzer**: a live graph of which frequencies are currently in the sound, in three modes —
  **Live** (right now), **Average** (analyze a whole selection or file, as the source or as
  processed through the rack, to see its long-term tone), and **Compare** (freeze two curves, A and
  B, and see the difference between them — handy for comparing before/after an EQ move, or the
  source against the processed signal). **Peak hold** and labelled peaks help you read it; a
  Fast/Medium/Slow response smooths how quickly it reacts.
- **Voice diagnostics** (the Analyzer panel's Diagnostics toggle): plain readouts of pitch (F0),
  tone balance (how much "mud" around 200–500 Hz, "presence" around 2–5 kHz, and "air" above
  10 kHz), sibilance (harsh "s" sounds, 4–10 kHz), mains hum, rumble, noise floor and signal-to-noise
  ratio — each with a short hint ("A bit boomy — try a gentle cut around 300 Hz") and, for several,
  an **Add EQ band here** button that inserts a matching Parametric EQ move.
- **Spectrum Inspector** (**View → Spectrum Inspector**): a larger, dedicated live spectrum view
  with its own FFT size, window and response settings, for closer inspection than the compact
  Analyzer panel — wheel to zoom, drag to pan, Shift-drag to zoom to a range, double-click to
  reset.

## Hit a loudness target

The **Loudness** panel (dock) measures your take: click **Analyze** (as **Processed**, through the
rack, or **Source**, the raw file) to get integrated loudness (**I**, in
[LUFS](glossary.md#l)), the loudest short-term (**S-max**) and momentary (**M-max**) moments, the
loudness range (**LRA**, in LU), sample peak and [true peak](glossary.md#t) (**TP**, in dBTP).

- **Normalize** in the toolbar/Edit menu holds one-click favourites for peak (**−1**, **−0.1**,
  **−3 dB**) and loudness (**−16**, **−19**, **−23 LUFS**) targets, plus a dialog for any other
  target. Like every normalize, it's a destructive but fully undoable gain change to the selection
  (or the whole file if nothing is selected).
- **ACX Check** (dock panel) tests your file against Audible's three ACX submission rules, each
  measured against the whole file or your selection:

  | Rule | Limit |
  |---|---|
  | RMS | −23 … −18 dB |
  | Peak | ≤ −3 dBFS |
  | Noise floor (quietest 500 ms) | ≤ −60 dB |

  A failing rule comes with a one-line hint (e.g. "RMS −27 dB is quieter than −23 dB: LUFS/RMS
  normalize up.", or "Noise floor −52 dB exceeds −60 dB: capture a noise print and add Noise
  Reduction."). The **Audiobook (ACX)** [rack preset](#rack-presets) already keeps peaks under the
  ACX ceiling via its limiter.

## Export

**File → Export…** Choose:

- **Format**: WAV (16/24/32-bit float), FLAC (16/24-bit — no 32-bit float FLAC), or MP3 (CBR/VBR,
  needs a system LAME install — see [MP3 export is greyed out](#mp3-export-is-greyed-out--needs-libmp3lame)
  if it's greyed out with "(needs libmp3lame)").
- **Range**: whole file or just the current selection.
- **ACX preset**, if you want export settings that match Audible's ACX submission format.

Exports always render through the full rack (the rack panel's "Listening only" badge on an A/B'd
module is a reminder that only *exports* reflect the true, non-bypassed signal) — see [why what
you export is what you heard](how-it-works.md#the-journey-of-your-voice). Check [Hit a loudness
target](#hit-a-loudness-target) before exporting if the file needs to pass an ACX submission.

## Themes

**Edit → Preferences → Display & Appearance**, or **View → Theme**, offers four choices: **Dark**
(the default), **Light** (for bright rooms), **High Contrast** (stronger text and lines, for
accessibility), or **Match System** (follows your OS's light/dark setting, and switches to High
Contrast automatically if your OS is set to prefer more contrast). A theme change applies at once,
everywhere — the waveform, spectrogram, meters and analyzer included.

## Preferences

**Edit → Preferences** groups settings into five sections: **Recording** (pre-roll/post-roll
lengths, punch crossfade, saved per-device-setup recording offsets from calibration), **Editing**
(what happens when you open a file with more than one channel), **Display & Appearance** (theme),
**Plugins** (the list of installed plugins, with a shortcut to the Plugin Manager) and **Advanced**
(how much audio PowerVoice keeps in memory at once, and how often the playhead and meters refresh —
60 Hz is smoother, 30 Hz uses less CPU). Each section, or everything at once, can be reset to
defaults from its own **Reset to defaults…** button.

## Shortcuts

See [`docs/shortcuts.md`](shortcuts.md) for the full keyboard shortcut map — generated from
`ui/src/lib/shortcuts/registry.ts`, the single source of truth the app itself uses, so it can't
drift from what's actually bound. Shortcuts aren't remappable in this version. Help ▸ Keyboard
Shortcuts shows the same list inside the app. Two bindings are still marked provisional (Record,
Capture Noise Print) — they match Audition's defaults per the best sources found so far, but
`helpx.adobe.com`'s own shortcut page still 403s to automated fetch, so neither has an independent,
authoritative confirmation (T-701).

## Troubleshooting

### No audio devices show up / wrong devices listed (Linux: PipeWire, JACK, ALSA)

The [Audio Devices](#set-up-your-microphone) dialog lists whatever your audio backend reports,
refreshed automatically on hot-plug. On Linux, PowerVoice picks a backend at build time (via
`cpal`) in this order:

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

### LV2 plugins are unavailable

LV2 is one of the plugin formats PowerVoice can host. To use LV2 plugins, your computer needs a small
free library called **lilv**. PowerVoice doesn't include it; it uses the copy installed on your system.
If lilv is missing, LV2 plugins don't appear under **Add module** and the Plugin Manager shows the
message *"LV2 support needs the lilv library (liblilv-0), which isn't installed"*. Everything else,
including CLAP and VST3 plugins, keeps working.

Install lilv:

- **Debian / Ubuntu**: `sudo apt install liblilv-0-0` (the PowerVoice .deb recommends it, so `apt`
  usually installs it for you).
- **Fedora**: `sudo dnf install lilv-libs`.
- **Arch**: `sudo pacman -S lilv`.
- **AppImage**: install lilv with your distribution's package manager, as above.
- **macOS**: `brew install lilv`.
- **Windows**: LV2 plugins aren't supported on Windows. Use CLAP or VST3 plugins instead.

You don't need to restart PowerVoice: lilv is loaded the next time an LV2 plugin is scanned, so use
**Rescan** in the Plugin Manager. Advanced: to use your own lilv build, set the `POWERVOICE_LILV`
environment variable to its full path.

### Plugin editor windows

Some plugins offer a graphical editor window. Open it by clicking the window icon (⊟) in the
plugin's rack slot. The window stays open while you edit and closes when you remove the plugin
from the rack or PowerVoice closes.

**On Linux**: the window needs X11 or XWayland. If neither is available, the window button is
disabled. LV2 plugin windows additionally need the **suil** library; LV2 plugins without suil have
no window (but still process audio normally). See [Building PowerVoice](building.md) for
installation instructions.

**On Windows and macOS**: plugin editor windows are not yet supported. CLAP and VST3 plugins
still work; they simply have no graphical window.

**After a plugin crash**: if a plugin crashes, its window closes and doesn't reopen
automatically — the plugin's sandbox restarts to recover. Reopen the window by clicking the
window icon again.

### Where JSFX effects come from

JSFX are REAPER's text-based effects. PowerVoice runs them with its own built-in JSFX engine, in the
same protected helper process as other plugins, so a broken script can't take PowerVoice down. It
finds them in three places:

- **PowerVoice's own JSFX folder**, where **Install module…** copies a `.jsfx` file (with the
  files it imports from its own folder): `~/.local/share/powervoice/Effects` on Linux,
  `~/Library/Application Support/app.powervoice.powervoice/Effects` on macOS.
- **REAPER's effects folder**, if REAPER is installed: `~/.config/REAPER/Effects` on Linux,
  `~/Library/Application Support/REAPER/Effects` on macOS. REAPER's own effects have no file
  extension; PowerVoice recognises them by their `desc:` line.
- Any **custom folder** you add in the Plugin Manager's **Folders** tab.

JSFX effects appear under **Add module → Plugins (JSFX)**. Their sliders are the effect's
parameters, so you can automate them, and they're saved with your project and presets. Their
custom graphics (`@gfx`) aren't shown yet: PowerVoice shows the sliders instead. A script that
doesn't compile is listed in the Plugin Manager but not in **Add module**. A script that freezes
while it's being checked is blocked, like any plugin that hangs. JSFX aren't supported on Windows
yet.

### Recording sounds delayed / out of sync with playback

See [Latency calibration](#latency-calibration) above. Also check the **Monitoring latency**
readout in the Record panel — a red warning there suggests a smaller audio buffer size, switching
to Dry monitoring, or removing high-latency rack modules (Noise Reduction in particular can add up
to ~50 ms; a *bypassed* module still holds its latency, so bypassing it doesn't help — remove it
instead if you need the latency back).

### Recovering after a crash or power loss

Open **File → Recovery & Storage…**. Interrupted takes are recovered from the crash-safe session
journal; markers show where any dropouts or the interruption itself occurred. The dialog stays open
until you've dealt with every recoverable session, so you can't accidentally lose one by dismissing
it too fast. See [why your original recording is never
damaged](how-it-works.md#why-your-original-recording-is-never-damaged) for how this works under the
hood.
