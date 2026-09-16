# FAQ and troubleshooting

Quick answers to the questions people actually ask. For step-by-step instructions, see the [user
guide](user-guide.md); for platform-specific problems (no devices found, MP3 export greyed out,
LV2/plugin windows, sync/latency), see its [Troubleshooting](user-guide.md#troubleshooting)
section — this page links to it rather than repeating it.

## General

### What is PowerVoice, in one sentence?

A desktop program for recording your voice, cleaning it up, and exporting a finished file — see
[What is PowerVoice?](what-is-powervoice.md) for the full pitch, or [How PowerVoice
works](how-it-works.md) for what happens to your sound as you use it.

### Is PowerVoice a Digital Audio Workstation (DAW) like Audition or Pro Tools?

Not a full one. It edits one mono file at a time and doesn't do multitrack mixing, MIDI or
music-production features — see [What PowerVoice deliberately doesn't
do](what-is-powervoice.md#what-powervoice-deliberately-doesnt-do). If you need those, use a full
DAW instead; if your job is "record one voice, clean it, deliver it," PowerVoice is built for
exactly that.

### Is PowerVoice free?

Yes. It's dual-licensed under [MIT](../LICENSE-MIT) or [Apache-2.0](../LICENSE-APACHE), at your
choice — see the [README](../README.md#license).

### Is PowerVoice affiliated with Adobe, or does it open Audition files?

No affiliation — "Adobe Audition" is only mentioned to describe the kind of tool PowerVoice is
inspired by. PowerVoice opens plain audio files (WAV, FLAC, MP3, M4A, OGG) plus its own small
sidecar file, not Audition's session format.

### What platforms does PowerVoice run on?

It's developed and tested on **Linux**. Windows and macOS builds compile from source but haven't
been verified yet — see the [README status note](../README.md) and [Where files
live](architecture/data.md#where-files-live) for the platform differences that are and aren't
tested. If you try Windows or macOS, reports of what does and doesn't work are welcome.

### Can I record more than one microphone, or a stereo source?

Recording is mono, from one chosen input channel — see [Set up your
microphone](user-guide.md#set-up-your-microphone). A stereo file you *open* is downmixed to mono
(or you pick one channel) on import, based on your **Preferences → Editing → Multichannel files**
setting.

### Can I have more than one file open, or a multitrack timeline?

No — PowerVoice edits one file at a time by design (see [What is
PowerVoice?](what-is-powervoice.md#compared-with-adobe-audition)). Open another file with **File →
Open…** and it replaces the current one (after asking about unsaved changes).

## Recording and editing

### Why is "Record" greyed out?

You need an input device chosen first — open [Audio Devices](user-guide.md#set-up-your-microphone)
(the toolbar's gear icon) and pick one.

### How do I fix just one word or sentence without re-recording the whole take?

Select the part to redo and press **Shift+R** — this is a *punch-in*: PowerVoice plays a lead-in,
records over exactly your selection, then plays a bit after so you can hear the join. See
[Re-recording part of a take (punch-in)](user-guide.md#re-recording-part-of-a-take-punch-in).

### The timing of my punch-in doesn't line up with the original — what's wrong?

Your audio interface's own round-trip delay (the time between PowerVoice sending a sound and your
microphone hearing it back) probably isn't measured yet. Run [Latency
calibration](user-guide.md#latency-calibration) once per device/buffer-size combination.

### I deleted or changed something by mistake — can I get it back?

Yes: **Ctrl/⌘+Z** (Undo) as many times as you need — PowerVoice's undo has no built-in limit other
than free disk space. See [Editing and undo](user-guide.md#editing-and-undo) and [why undo never
runs out](how-it-works.md#why-your-original-recording-is-never-damaged).

### Does adding effects change my original recording?

No. The effects rack is **non-destructive**: it's applied live while you listen, but your document
underneath is unchanged until you explicitly [bake](user-guide.md#apply-the-rack-bake) the rack or
export. Remove an effect, or undo a bake, and you're back to the untouched audio.

## Noise, tone and loudness

### How do I remove background hiss or hum?

Select a short stretch of silence (room tone only, no speech), capture a *noise print* from it,
then add Noise Reduction to the rack. See [Remove background
noise](user-guide.md#remove-background-noise).

### I added Noise Reduction and now my voice sounds thin or "underwater" — why?

That's a sign of too much reduction. Lower the **reduction** (dB) or **amount** (%) controls until
the artifact goes away, or capture a cleaner, longer noise print (0.5–60 s, no speech in it).

### What is ACX, and how do I know if my file will pass?

[ACX](glossary.md#a) is Audible's technical checklist for audiobook submissions (loudness, peak
level and background noise limits). Open the **ACX Check** panel to test your file against all
three rules with plain-language hints on any failure — see [Hit a loudness
target](user-guide.md#hit-a-loudness-target). The **Audiobook (ACX)** [rack
preset](user-guide.md#rack-presets) is built to pass it with headroom to spare.

### What's the difference between "Normalize" and "hitting an ACX target"?

**Normalize** changes the overall level (peak or [LUFS](glossary.md#l)) to one target number in
one click. The **ACX check** tests three separate numbers at once (loudness, peak, and noise
floor) — normalizing alone doesn't guarantee the noise-floor rule passes, which is why the noise
floor hint suggests noise reduction, not normalizing.

### My ACX check fails on "noise floor" even though the recording sounds clean — why?

The noise floor rule looks at the **quietest half-second** in the whole file, which is often
quieter than what you notice while listening to speech. Capture a noise print from your quietest
stretch of room tone and add Noise Reduction — see [Remove background
noise](user-guide.md#remove-background-noise).

## Plugins

### PowerVoice crashed — was it a plugin?

Usually not: third-party plugins run in their own protected process precisely so a plugin crash
can't take PowerVoice down with it — see [Why plugins run in their own "safety
box"](how-it-works.md#why-plugins-run-in-their-own-safety-box). If PowerVoice itself closes
unexpectedly, that's a PowerVoice bug — please report it with the log file
(`logs/powervoice.log` — see [Where files live](architecture/data.md#where-files-live)).

### A plugin keeps failing to load / is "Blocklisted" — what do I do?

It crashed or timed out while PowerVoice was scanning it. Open **Effects → Manage Plugins…**,
find it, and use **Unblock and rescan** — see [If a plugin crashes](user-guide.md#if-a-plugin-crashes).
If it fails the same way again, the plugin itself likely has a real problem on your system.

### Why don't I see any LV2 plugins?

Your system is missing the `lilv` library, which PowerVoice needs to host LV2 plugins — see [LV2
plugins are unavailable](user-guide.md#lv2-plugins-are-unavailable) for how to install it per
platform.

### Can I use VST2 plugins?

No — VST2 hosting needs headers Steinberg no longer licenses, so it's gated pending a legal
decision and isn't implemented. CLAP, VST3, LV2 and JSFX are supported today.

## Files and safety

### Where are my recordings actually stored while I'm working?

In a private working copy (a *session*) separate from your actual file, so your original is never
touched until you save — see [Why your original recording is never
damaged](how-it-works.md#why-your-original-recording-is-never-damaged).

### PowerVoice (or my computer) crashed while I was working — did I lose everything?

Almost certainly not. Open **File → Recovery & Storage…** next time you start PowerVoice — it
finds your interrupted session and offers to bring it back, typically losing at most a fraction of
a second of work. See [Recovering after a crash or power
loss](user-guide.md#recovering-after-a-crash-or-power-loss).

### What is the `.vo.json` file next to my audio file?

That's the **sidecar** — a small text file that remembers your effects rack, markers and view
settings for that file, written when you Save. Deleting it just resets those (your audio is
unaffected); see [glossary: Sidecar](glossary.md#s).

### Can I edit stereo files, or files with more than 2 channels?

You can *open* them — PowerVoice downmixes to mono or lets you pick one channel on import (your
choice, or a saved preference, under **Preferences → Editing → Multichannel files**) — but editing
itself is always mono.

### What formats can I import and export?

Import: WAV, FLAC, MP3, M4A (AAC) and OGG (Vorbis). Export: WAV (16/24/32-bit float), FLAC
(16/24-bit) and MP3 (needs a system LAME install — see [MP3 export is greyed
out](user-guide.md#mp3-export-is-greyed-out--needs-libmp3lame) if it isn't available). A file
imported from a lossy format (MP3, M4A, OGG) can only be *saved* as WAV or FLAC, since re-saving as
the same lossy format would lose quality twice over.

## Didn't find your question?

Check the full [Troubleshooting](user-guide.md#troubleshooting) section of the user guide, or the
[glossary](glossary.md) if a term is unfamiliar. Still stuck? Open an issue on the [GitHub
repository](https://github.com/engLucasCorreia/powervoice) with what you were doing, what you
expected, and (if PowerVoice crashed) the relevant `crashes/crash-*.log` or `logs/powervoice.log`
file — see [Where files live](architecture/data.md#where-files-live) for their locations.
