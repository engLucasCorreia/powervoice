# What is PowerVoice?

A plain-language introduction — no audio or programming background needed. New words link to the
[glossary](glossary.md), where every term gets a one-line explanation and an everyday comparison.

## What it's for

PowerVoice is a desktop program for recording and cleaning up **spoken-word audio**: audiobook
chapters, podcast episodes, YouTube narration, voicemail greetings, presentation voice-overs —
anything where one person talks into one microphone. You record, tidy up the sound, make it loud
and clear enough for wherever it's going, and save a finished file.

It is **not** a music studio. It doesn't record a band, mix multiple tracks together, or add
musical effects like reverb or delay for creative sound design. It does one job — turning a raw
voice recording into a clean, correctly-loud finished file — and tries to do that job simply.

## Who it's for

- **Audiobook narrators** who need to pass Audible's ACX technical checks before submitting a
  chapter.
- **Podcasters** who record their own voice and want it to sound clear and consistent from episode
  to episode.
- **Video and course creators** recording narration or voice-over for something else.
- Anyone who wants Adobe Audition's single-file **Waveform Editor** — record, edit, clean, deliver
  — without paying for or learning the rest of a full multitrack studio.

## Compared with Adobe Audition

[Adobe Audition](https://www.adobe.com/products/audition.html) is a professional audio workstation
with two main views: a multitrack editor (many tracks, mixed together, like a music studio) and a
**Waveform Editor** for working on one file at a time. PowerVoice is an independent project, not
made by or affiliated with Adobe — it takes inspiration from that second view alone and builds a
whole, focused application around it:

| | Adobe Audition | PowerVoice |
|---|---|---|
| Scope | Multitrack studio + single-file editor | Single-file editor only |
| Files open at once | Many, on many tracks | One |
| Effects | Huge built-in library + third-party plugins | A focused set of built-ins (see below) + third-party plugins |
| Loudness / ACX | Built in | Built in — this is a primary use case |
| Learning curve | Substantial | Small — most of what you need fits in this guide |
| Price | Subscription | Free, [dual-licensed open source](../LICENSE-MIT) |

If you outgrow PowerVoice — you need multiple tracks, music mixing, or video — Audition (or another
full DAW) is the right tool. If your work is "record one voice, clean it, deliver it," PowerVoice
does that one thing without the rest of the studio around it.

## What PowerVoice does

1. **Record** your voice from a microphone, with a level meter and a clip warning, and re-record
   just the part you got wrong (see [Punch-in](user-guide.md#re-recording-part-of-a-take-punch-in)).
2. **Clean** it up: remove background hiss with [noise
   reduction](user-guide.md#remove-background-noise), and shape the tone with an
   [EQ](glossary.md#e) and [dynamics](glossary.md#d) processor (see [Make your voice sound
   better](user-guide.md#make-your-voice-sound-better)).
3. **Level** it: bring it to a target loudness with one-click
   [normalize](user-guide.md#hit-a-loudness-target) favourites, or an exact
   [LUFS](glossary.md#l) target.
4. **Check** it against Audible's [ACX](glossary.md#a) submission rules, or just listen with the
   built-in [meters and analyzer](user-guide.md#check-your-levels-analyzer-and-diagnostics).
5. **Export** a finished WAV, FLAC or MP3 file, ready to send or upload.

```mermaid
flowchart LR
  rec(["🎙️ Record<br/>your voice"]) --> clean(["🧹 Clean<br/>remove noise, shape tone"])
  clean --> level(["📏 Level<br/>hit a loudness target"])
  level --> check(["✅ Check<br/>ACX, meters, listen"])
  check --> export(["💾 Export<br/>WAV · FLAC · MP3"])
```

Every one of these steps is covered, in the order you'll actually use them, in the [user
guide](user-guide.md).

## What PowerVoice deliberately doesn't do

Depth over breadth: PowerVoice does the voice-over workflow well rather than trying to do
everything an audio editor could do. Today it does not have:

- **Multitrack editing.** There's no timeline with several tracks playing together — one mono file
  is open at a time, like Audition's Waveform Editor rather than its Multitrack Editor.
- **Stereo editing.** Recording and editing are mono. A stereo file you open is downmixed (or you
  pick one channel) on import.
- **Music production features**: no MIDI, no virtual instruments, no time-stretching for musical
  tempo, no built-in reverb/delay for creative sound design (though a [third-party
  plugin](user-guide.md#plugins) can add one to the rack).
- **Spectral editing.** The [spectral view](user-guide.md#waveform-and-spectral-views) is for
  *looking* at the sound's frequencies (to spot hum, clicks or breaths) — you can't paint or erase
  sound directly on it the way some tools allow.
- **Batch processing** of many files at once, or a cloud/online plugin catalogue.
- **Remappable shortcuts.** The keyboard shortcuts (deliberately close to Audition's own) are fixed
  in this version — see [shortcuts.md](shortcuts.md).
- **Video.** PowerVoice only ever works with audio.

If one of these is exactly what you need today, a full DAW is the right choice for that project.
Otherwise, read on: [How PowerVoice works](how-it-works.md) explains what happens to your sound
after you press Record, and the [user guide](user-guide.md) walks through every feature.
