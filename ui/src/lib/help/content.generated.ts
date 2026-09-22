// GENERATED FILE — DO NOT EDIT BY HAND.
//
// H-107: the in-app Help Centre's content, parsed from docs/user-guide.md and docs/faq.md by
// `scripts/help/generate.py` (`just help-content`). Regenerate after editing either doc and commit
// the result — `just check` runs `python3 scripts/help/generate.py --check` and fails if this file
// no longer matches the docs. See that script's module docstring for why the Help Centre is
// generated rather than a hand-maintained copy (one source of truth instead of two that drift).
import type { HelpDoc } from "./content";

export const HELP_DOCS: readonly HelpDoc[] = [
  {
    "id": "user-guide",
    "title": "PowerVoice user guide",
    "sections": [
      {
        "id": "overview",
        "title": "PowerVoice user guide",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "PowerVoice is a focused editor for "
              },
              {
                "text": "simple voice-over work",
                "bold": true
              },
              {
                "text": ": record mono voice, clean it up, shape it, hit a loudness target, and export. It edits "
              },
              {
                "text": "one mono audio file at a time",
                "bold": true
              },
              {
                "text": " (like Adobe Audition's Waveform Editor) — there's no multitrack timeline. New to PowerVoice? "
              },
              {
                "text": "What is PowerVoice?"
              },
              {
                "text": " is the two-minute pitch, and "
              },
              {
                "text": "How PowerVoice works"
              },
              {
                "text": " explains what happens to your sound as you use it. Words in "
              },
              {
                "text": "italics",
                "italic": true
              },
              {
                "text": " or linked to the "
              },
              {
                "text": "glossary"
              },
              {
                "text": " get a plain-language explanation there."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "This guide is organised by task, roughly in the order you'll use them:"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Guided tours",
                "link": {
                  "doc": "user-guide",
                  "section": "guided-tours"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Install",
                "link": {
                  "doc": "user-guide",
                  "section": "install"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Set up your microphone",
                "link": {
                  "doc": "user-guide",
                  "section": "set-up-your-microphone"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "First recording",
                "link": {
                  "doc": "user-guide",
                  "section": "first-recording"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Punch-in",
                "link": {
                  "doc": "user-guide",
                  "section": "first-recording"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Latency calibration",
                "link": {
                  "doc": "user-guide",
                  "section": "first-recording"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Markers",
                "link": {
                  "doc": "user-guide",
                  "section": "markers"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Edit and undo",
                "link": {
                  "doc": "user-guide",
                  "section": "editing-and-undo"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Waveform and spectral views",
                "link": {
                  "doc": "user-guide",
                  "section": "waveform-and-spectral-views"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "The effects rack",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Remove background noise",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Make your voice sound better",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Rack presets",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Apply the rack (bake)",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Plugins",
                "link": {
                  "doc": "user-guide",
                  "section": "plugins"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Presets",
                "link": {
                  "doc": "user-guide",
                  "section": "presets"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Check your levels",
                "link": {
                  "doc": "user-guide",
                  "section": "check-your-levels-analyzer-and-diagnostics"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Explain My Voice",
                "link": {
                  "doc": "user-guide",
                  "section": "explain-my-voice"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Hit a loudness target",
                "link": {
                  "doc": "user-guide",
                  "section": "hit-a-loudness-target"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Export",
                "link": {
                  "doc": "user-guide",
                  "section": "export"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Themes",
                "link": {
                  "doc": "user-guide",
                  "section": "themes"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Preferences",
                "link": {
                  "doc": "user-guide",
                  "section": "preferences"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Shortcuts",
                "link": {
                  "doc": "user-guide",
                  "section": "shortcuts"
                }
              },
              {
                "text": " · "
              },
              {
                "text": "Troubleshooting",
                "link": {
                  "doc": "user-guide",
                  "section": "troubleshooting"
                }
              }
            ]
          }
        ]
      },
      {
        "id": "guided-tours",
        "title": "Guided tours",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "The first time PowerVoice starts it offers a two-minute "
              },
              {
                "text": "Welcome tour",
                "bold": true
              },
              {
                "text": " that walks through choosing your devices, recording a practice take, navigating the take, markers, the effects rack, noise reduction, meters and loudness, and export. Replay it any time from "
              },
              {
                "text": "Help → Take the Tour",
                "bold": true
              },
              {
                "text": ", or pick any tour from "
              },
              {
                "text": "Help → Tours",
                "bold": true
              },
              {
                "text": ": Welcome tour, Effects rack and presets, Noise print and reduction, Loudness, ACX and normalize, Punch-in, and Plugin Manager. A "
              },
              {
                "text": "?",
                "bold": true
              },
              {
                "text": " in the Rack, Loudness, Noise Reduction, Punch & pre-roll and Plugin Manager headers starts a short tour of that panel. Use "
              },
              {
                "text": "→",
                "bold": true
              },
              {
                "text": "/"
              },
              {
                "text": "Enter",
                "bold": true
              },
              {
                "text": " and "
              },
              {
                "text": "←",
                "bold": true
              },
              {
                "text": " to move, "
              },
              {
                "text": "Esc",
                "bold": true
              },
              {
                "text": " to leave."
              }
            ]
          }
        ]
      },
      {
        "id": "install",
        "title": "Install",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "See the "
              },
              {
                "text": "README"
              },
              {
                "text": " for install instructions per platform, and "
              },
              {
                "text": "docs/building.md",
                "code": true
              },
              {
                "text": " if you're building from source."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Linux AppImage:",
                "bold": true
              },
              {
                "text": " a downloaded file has no run permission yet, so double-clicking it does nothing useful — some file managers show "
              },
              {
                "text": "\"There is no app installed for AppImage application bundle\"",
                "italic": true
              },
              {
                "text": ", which despite how it reads isn't about a missing dependency. Make it executable once, then run it:"
              }
            ]
          },
          {
            "type": "code",
            "lang": "sh",
            "text": "chmod +x PowerVoice_*_amd64.AppImage\n./PowerVoice_*_amd64.AppImage"
          }
        ]
      },
      {
        "id": "set-up-your-microphone",
        "title": "Set up your microphone",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "Click the "
              },
              {
                "text": "Audio devices…",
                "bold": true
              },
              {
                "text": " button in the toolbar (the gear icon) to open "
              },
              {
                "text": "Audio Devices",
                "bold": true
              },
              {
                "text": ":"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Host",
                  "bold": true
                },
                {
                  "text": ": the audio system PowerVoice talks to (PipeWire, ALSA or JACK on Linux; WASAPI on Windows; Core Audio on macOS). See "
                },
                {
                  "text": "No audio devices show up",
                  "link": {
                    "doc": "user-guide",
                    "section": "troubleshooting"
                  }
                },
                {
                  "text": " if this list is empty."
                }
              ],
              [
                {
                  "text": "Input device",
                  "bold": true
                },
                {
                  "text": ", and its "
                },
                {
                  "text": "input channel",
                  "bold": true
                },
                {
                  "text": " if it has more than one (for example, channel 1 or 2 of a stereo audio interface) — this is your microphone."
                }
              ],
              [
                {
                  "text": "Output device",
                  "bold": true
                },
                {
                  "text": " — your speakers or headphones, needed for monitoring and for punch-in's pre-roll/post-roll playback."
                }
              ],
              [
                {
                  "text": "Sample rate",
                  "bold": true
                },
                {
                  "text": " and "
                },
                {
                  "text": "buffer size",
                  "bold": true
                },
                {
                  "text": " for the chosen devices. A smaller buffer size lowers latency (the delay between making a sound and hearing it through the computer) but asks more of your computer; if you hear crackling, try a larger one."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "The list refreshes automatically when you plug or unplug a device. Record stays unavailable until an input device is chosen."
              }
            ]
          }
        ]
      },
      {
        "id": "first-recording",
        "title": "First recording",
        "blocks": [
          {
            "type": "ol",
            "items": [
              [
                {
                  "text": "File → New Recording…",
                  "bold": true
                },
                {
                  "text": " Choose the sample rate/bit depth (default 48 kHz / 24-bit; internal processing is always 32-bit float regardless of what you pick here) and pick your "
                },
                {
                  "text": "input device",
                  "bold": true
                },
                {
                  "text": " (and input channel, e.g. channel 1 or 2 of a stereo interface) and "
                },
                {
                  "text": "output device",
                  "bold": true
                },
                {
                  "text": " in "
                },
                {
                  "text": "Audio Devices",
                  "link": {
                    "doc": "user-guide",
                    "section": "set-up-your-microphone"
                  }
                },
                {
                  "text": " if you haven't already."
                }
              ],
              [
                {
                  "text": "Click "
                },
                {
                  "text": "Input",
                  "bold": true
                },
                {
                  "text": " to arm — this starts the input meter and monitoring without recording yet. Choose a monitoring mode: "
                },
                {
                  "text": "Off",
                  "bold": true
                },
                {
                  "text": " / "
                },
                {
                  "text": "Dry",
                  "bold": true
                },
                {
                  "text": " (hear your raw input) / "
                },
                {
                  "text": "Through rack",
                  "bold": true
                },
                {
                  "text": " (hear it with the effects rack applied). Off is the default: monitoring through a rack adds the rack's latency to what you hear."
                }
              ],
              [
                {
                  "text": "Click "
                },
                {
                  "text": "Record",
                  "bold": true
                },
                {
                  "text": " (or press "
                },
                {
                  "text": "Shift+R",
                  "bold": true
                },
                {
                  "text": ") to start, and again to stop. The waveform grows live as you speak, the same as Audition or Audacity — it isn't just a record head crawling across a blank track. A red "
                },
                {
                  "text": "CLIP",
                  "bold": true
                },
                {
                  "text": " indicator lights if the input clips; click it to reset. If the input drops out for a moment (device hiccup), PowerVoice fills the gap with silence and drops a marker there so you can find it later."
                }
              ],
              [
                {
                  "text": "File → Save",
                  "bold": true
                },
                {
                  "text": " ("
                },
                {
                  "text": "Ctrl/⌘+S",
                  "bold": true
                },
                {
                  "text": ") writes the audio file plus a sidecar "
                },
                {
                  "text": "name.wav.vo.json",
                  "code": true
                },
                {
                  "text": " next to it, holding your rack, markers, noise profile and view settings. A long save (a big file, or a slow disk) shows a progress bar with a "
                },
                {
                  "text": "Cancel",
                  "bold": true
                },
                {
                  "text": " button; PowerVoice checks there's enough free disk space before writing a single byte, and a clear message if the save fails partway (disk full, no permission, and so on) rather than a silent, half-written file. Autosave/crash recovery runs continuously in the background — see "
                },
                {
                  "text": "Recovering after a crash or power loss",
                  "link": {
                    "doc": "user-guide",
                    "section": "troubleshooting"
                  }
                },
                {
                  "text": "."
                }
              ]
            ]
          },
          {
            "type": "h3",
            "id": "re-recording-part-of-a-take-punch-in",
            "spans": [
              {
                "text": "Re-recording part of a take (punch-in)"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Made a mistake in the middle of an otherwise-good take? Select the region to redo and press "
              },
              {
                "text": "Shift+R",
                "bold": true
              },
              {
                "text": ". With a selection active, Record does a "
              },
              {
                "text": "punch-in",
                "bold": true
              },
              {
                "text": " by default (Record panel → "
              },
              {
                "text": "Punch-in on selection",
                "bold": true
              },
              {
                "text": ", on by default): PowerVoice plays a "
              },
              {
                "text": "pre-roll",
                "bold": true
              },
              {
                "text": " (default 5 s) up to the selection so you can match the earlier read's pace and tone, records over exactly the selected region, then plays a "
              },
              {
                "text": "post-roll",
                "bold": true
              },
              {
                "text": " (default 1 s) so you can hear how the join sounds. A short crossfade (default 10 ms) blends the new audio in at both edges."
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Mode",
                  "bold": true
                },
                {
                  "text": ": "
                },
                {
                  "text": "Insert",
                  "bold": true
                },
                {
                  "text": " (default — adds without destroying anything) or "
                },
                {
                  "text": "Overwrite",
                  "bold": true
                },
                {
                  "text": " (audiobook \"punch-and-roll\" style, replaces from the record point onward)."
                }
              ],
              [
                {
                  "text": "Turn "
                },
                {
                  "text": "Punch-in on selection",
                  "bold": true
                },
                {
                  "text": " off if you'd rather a selection be ignored and Record just insert at the selection's start."
                }
              ],
              [
                {
                  "text": "Pre-roll at cursor",
                  "bold": true
                },
                {
                  "text": " (off by default) turns plain cursor recording (no selection) into \"punch-and-roll\": lead-in playback before recording starts at the cursor."
                }
              ],
              [
                {
                  "text": "Punch-in needs an "
                },
                {
                  "text": "output device",
                  "bold": true
                },
                {
                  "text": " (for pre-roll/post-roll); recording at the cursor with no selection still works with only an input device, just without a lead-in."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "All of these live in the Record panel's "
              },
              {
                "text": "Punch & pre-roll",
                "bold": true
              },
              {
                "text": " section (same settings also appear in "
              },
              {
                "text": "Edit → Preferences → Recording",
                "bold": true
              },
              {
                "text": "), and are app-wide preferences, not saved per document."
              }
            ]
          },
          {
            "type": "h3",
            "id": "latency-calibration",
            "spans": [
              {
                "text": "Latency calibration"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "If a word spoken in time with what you hear during a punch doesn't land in time on the timeline, your interface's round-trip latency isn't compensated yet. Click "
              },
              {
                "text": "Calibrate…",
                "bold": true
              },
              {
                "text": " next to the "
              },
              {
                "text": "Recording offset",
                "bold": true
              },
              {
                "text": " readout: connect a cable from an output of your interface back into the selected input (most accurate), or hold the microphone within 5 cm of a speaker/headphone cup, then "
              },
              {
                "text": "Start",
                "bold": true
              },
              {
                "text": ". PowerVoice plays a short test sweep and measures the round trip 5 times; it needs most of those 5 measurements to agree before it accepts a result. Recalibrate if you change your audio buffer size — the readout tells you when your current buffer size no longer matches the one you calibrated at."
              }
            ]
          }
        ]
      },
      {
        "id": "markers",
        "title": "Markers",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "Markers are named bookmarks in your take — a retake, a cough, a chapter break — so you can find a moment again without scrubbing through the whole recording."
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "M",
                  "bold": true
                },
                {
                  "text": " drops a marker at the playhead (or "
                },
                {
                  "text": "Edit → Markers → Add Marker",
                  "bold": true
                },
                {
                  "text": "). Made with a selection active, it becomes a "
                },
                {
                  "text": "region",
                  "bold": true
                },
                {
                  "text": " marker spanning that selection rather than a single point."
                }
              ],
              [
                {
                  "text": "The "
                },
                {
                  "text": "markers list",
                  "bold": true
                },
                {
                  "text": " panel on the left shows every marker with its type (Point, Region or Dropout), start time and duration; clicking a marker (in the panel or on the waveform) jumps the playhead there — and if it's a region, also sets the selection to exactly that region, so you can immediately play, trim or delete just that stretch. Double-click a marker's name to rename it."
                }
              ],
              [
                {
                  "text": "Ctrl/⌘+Alt+→",
                  "bold": true
                },
                {
                  "text": " / "
                },
                {
                  "text": "←",
                  "bold": true
                },
                {
                  "text": " jump to the next/previous marker (also "
                },
                {
                  "text": "Edit → Markers → Next/Previous Marker",
                  "bold": true
                },
                {
                  "text": ") without changing the selection."
                }
              ],
              [
                {
                  "text": "Drag a marker's flag",
                  "bold": true
                },
                {
                  "text": " on the waveform to move it (a region's edges resize individually; Shift-drag moves the whole region instead). Drags snap to the cursor, the selection edges and other markers when close, and "
                },
                {
                  "text": "Esc",
                  "bold": true
                },
                {
                  "text": " cancels a drag in progress."
                }
              ],
              [
                {
                  "text": "Ctrl/⌘+0",
                  "bold": true
                },
                {
                  "text": " deletes the selected marker(s) (also "
                },
                {
                  "text": "Edit → Markers → Delete Selected Marker",
                  "bold": true
                },
                {
                  "text": ")."
                }
              ],
              [
                {
                  "text": "A red "
                },
                {
                  "text": "Dropout",
                  "bold": true
                },
                {
                  "text": " marker is added automatically where a recording had a brief input hiccup (see "
                },
                {
                  "text": "First recording",
                  "link": {
                    "doc": "user-guide",
                    "section": "first-recording"
                  }
                },
                {
                  "text": ") — it's a kind of marker, not something you place yourself."
                }
              ],
              [
                {
                  "text": "Markers are saved in the sidecar and, on export, as WAV "
                },
                {
                  "text": "cue",
                  "code": true
                },
                {
                  "text": "/"
                },
                {
                  "text": "LIST adtl",
                  "code": true
                },
                {
                  "text": " chunks, so they travel with the file into other software that reads them. If a file's markers can't be read, or one falls outside the audio (both signs the file was edited by something else), PowerVoice drops the unreadable ones and tells you with a notice rather than silently keeping bad data."
                }
              ]
            ]
          }
        ]
      },
      {
        "id": "editing-and-undo",
        "title": "Editing and undo",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "The usual clipboard operations work on the current selection: "
              },
              {
                "text": "Cut",
                "bold": true
              },
              {
                "text": " ("
              },
              {
                "text": "Ctrl/⌘+X",
                "bold": true
              },
              {
                "text": "), "
              },
              {
                "text": "Copy",
                "bold": true
              },
              {
                "text": " ("
              },
              {
                "text": "Ctrl/⌘+C",
                "bold": true
              },
              {
                "text": "), "
              },
              {
                "text": "Paste",
                "bold": true
              },
              {
                "text": " ("
              },
              {
                "text": "Ctrl/⌘+V",
                "bold": true
              },
              {
                "text": "), "
              },
              {
                "text": "Delete",
                "bold": true
              },
              {
                "text": " ("
              },
              {
                "text": "Delete",
                "bold": true
              },
              {
                "text": "), "
              },
              {
                "text": "Trim to Selection",
                "bold": true
              },
              {
                "text": " (keep only the selection, "
              },
              {
                "text": "Ctrl/⌘+T",
                "bold": true
              },
              {
                "text": ") and "
              },
              {
                "text": "Silence",
                "bold": true
              },
              {
                "text": " (replace the selection with silence, keeping its length) — all in the "
              },
              {
                "text": "Edit",
                "bold": true
              },
              {
                "text": " menu and, on the waveform, its right-click menu (also reachable from the keyboard with the Menu key or Shift+F10) — and all destructive-but-undoable edits on your document (the effects rack itself is separate and non-destructive — see "
              },
              {
                "text": "the effects rack",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": "). "
              },
              {
                "text": "Edit → Insert Silence…",
                "bold": true
              },
              {
                "text": " adds a chosen length of silence at the cursor (or the start of the selection, if there is one) — enter a duration in seconds, timecode or samples."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "The clipboard holds one item and survives switching documents (open a different file, and Paste still has what you last copied) — the Edit menu's Paste item is only enabled when there's something to paste."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "While a file is still being imported (opening it, or pasting from another format), every edit above — including Insert Silence — is unavailable for the few moments that takes: the view is still showing the "
              },
              {
                "text": "importing",
                "italic": true
              },
              {
                "text": " file, so an edit fired mid-import could apply one file's selection to another file's audio. Select All and Undo/Redo are unaffected."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Undo",
                "bold": true
              },
              {
                "text": " ("
              },
              {
                "text": "Ctrl/⌘+Z",
                "bold": true
              },
              {
                "text": ") and "
              },
              {
                "text": "Redo",
                "bold": true
              },
              {
                "text": " ("
              },
              {
                "text": "Ctrl/⌘+Shift+Z",
                "bold": true
              },
              {
                "text": ") step back and forward through every edit, with no limit other than free disk space — see "
              },
              {
                "text": "why undo never runs out"
              },
              {
                "text": ". Undoing a "
              },
              {
                "text": "baked rack",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " restores the rack you baked, as well as the audio."
              }
            ]
          }
        ]
      },
      {
        "id": "waveform-and-spectral-views",
        "title": "Waveform and spectral views",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "The main "
              },
              {
                "text": "waveform",
                "bold": true
              },
              {
                "text": " view shows your take's shape over time; the "
              },
              {
                "text": "spectral view",
                "bold": true
              },
              {
                "text": " (a "
              },
              {
                "text": "spectrogram",
                "italic": true
              },
              {
                "text": " — a picture where height is pitch, left-to-right is time and brightness is loudness) shows its frequency content, which makes hum, clicks and breaths easy to spot. Toggle it with "
              },
              {
                "text": "Shift+D",
                "bold": true
              },
              {
                "text": " or the Spectral button in the toolbar."
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Zoom in/out",
                  "bold": true
                },
                {
                  "text": ": "
                },
                {
                  "text": "=",
                  "bold": true
                },
                {
                  "text": " / "
                },
                {
                  "text": "-",
                  "bold": true
                },
                {
                  "text": " (waveform), "
                },
                {
                  "text": "Alt+=",
                  "bold": true
                },
                {
                  "text": " / "
                },
                {
                  "text": "Alt+-",
                  "bold": true
                },
                {
                  "text": " (vertical, amplitude), "
                },
                {
                  "text": "Alt+0",
                  "bold": true
                },
                {
                  "text": " resets vertical zoom. The toolbar's "
                },
                {
                  "text": "Zoom to Selection",
                  "bold": true
                },
                {
                  "text": " and "
                },
                {
                  "text": "Zoom Full",
                  "bold": true
                },
                {
                  "text": " buttons jump straight to a range. You can also scroll with the mouse wheel and zoom with Ctrl + wheel."
                }
              ],
              [
                {
                  "text": "Select",
                  "bold": true
                },
                {
                  "text": ": drag to select a range, "
                },
                {
                  "text": "Ctrl/⌘+A",
                  "bold": true
                },
                {
                  "text": " selects all, "
                },
                {
                  "text": "Esc",
                  "bold": true
                },
                {
                  "text": " clears the selection. "
                },
                {
                  "text": "←",
                  "bold": true
                },
                {
                  "text": "/"
                },
                {
                  "text": "→",
                  "bold": true
                },
                {
                  "text": " nudge the cursor or selection, "
                },
                {
                  "text": "Shift+←",
                  "bold": true
                },
                {
                  "text": "/"
                },
                {
                  "text": "Shift+→",
                  "bold": true
                },
                {
                  "text": " extend it. A selection is a see-through tint with a clear edge in both panes — you can still read the waveform or the spectrogram underneath it, not a flat block hiding them. The spectral pane's time/Hz/dB hover readout hides while you're actively dragging a selection, so it doesn't cover what you're selecting, and reappears once you release."
                }
              ],
              [
                {
                  "text": "Snap to zero crossing",
                  "bold": true
                },
                {
                  "text": " (View menu, off by default): when you extend a selection (drag or Shift-click), its new edge snaps to the nearest point where the waveform crosses silence, so cuts and edits never click."
                }
              ],
              [
                {
                  "text": "Time format",
                  "bold": true
                },
                {
                  "text": ": View → Time Format shows the ruler and readouts as Timecode, Samples or Seconds."
                }
              ],
              [
                {
                  "text": "Amplitude Ruler",
                  "bold": true
                },
                {
                  "text": " (View menu): shows the vertical scale as "
                },
                {
                  "text": "dBFS",
                  "bold": true
                },
                {
                  "text": " (the default, matching the rest of the app's loudness numbers) or "
                },
                {
                  "text": "Percent",
                  "bold": true
                },
                {
                  "text": " (linear, ±100%, if you find that more intuitive for reading levels at a glance)."
                }
              ],
              [
                {
                  "text": "Loop playback",
                  "bold": true
                },
                {
                  "text": " ("
                },
                {
                  "text": "Ctrl/⌘+L",
                  "bold": true
                },
                {
                  "text": ", or View → Loop Playback) repeats the current selection (10 ms or longer) instead of stopping at its end — handy for checking how a phrase or an edit sounds. With no selection (or one shorter than 10 ms), it loops the whole document instead of doing nothing, so the button being lit always means \"actually looping.\" A loop doesn't reset the effects rack at the seam, so a reverb or delay tail from a plugin carries across the repeat, the same as it would if you kept playing normally."
                }
              ]
            ]
          }
        ]
      },
      {
        "id": "cleaning-up-the-effects-rack",
        "title": "Cleaning up: the effects rack",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "The rack is a chain of "
              },
              {
                "text": "non-destructive",
                "bold": true
              },
              {
                "text": " effects, processed in real time during playback, monitoring and export — nothing is baked into the file until you export or use "
              },
              {
                "text": "Apply the rack (bake)",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": ". Add modules from the rack panel's "
              },
              {
                "text": "Add module",
                "bold": true
              },
              {
                "text": " menu; drag a module by its "
              },
              {
                "text": "grip",
                "bold": true
              },
              {
                "text": " (the handle at the left of its header — only the grip starts a drag, so dragging a knob, slider or graph inside the module always adjusts that control instead) to reorder it, or focus the grip and press "
              },
              {
                "text": "↑",
                "bold": true
              },
              {
                "text": "/"
              },
              {
                "text": "↓",
                "bold": true
              },
              {
                "text": " to move it without a mouse; click a module's bypass toggle to A/B it against the rest of the chain."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Built-in modules: "
              },
              {
                "text": "Noise Gate",
                "bold": true
              },
              {
                "text": ", "
              },
              {
                "text": "Noise Reduction",
                "bold": true
              },
              {
                "text": ", "
              },
              {
                "text": "Parametric EQ",
                "bold": true
              },
              {
                "text": ", "
              },
              {
                "text": "Dynamics",
                "bold": true
              },
              {
                "text": ", "
              },
              {
                "text": "Gain",
                "bold": true
              },
              {
                "text": ", and a "
              },
              {
                "text": "True-Peak Limiter",
                "bold": true
              },
              {
                "text": " (a safety ceiling for loudness delivery). "
              },
              {
                "text": "Third-party plugins",
                "bold": true
              },
              {
                "text": " (CLAP, VST3, LV2, and REAPER's JSFX effects) show up in the same "
              },
              {
                "text": "Add module",
                "bold": true
              },
              {
                "text": " menu, grouped by format — see "
              },
              {
                "text": "Plugins",
                "link": {
                  "doc": "user-guide",
                  "section": "plugins"
                }
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "remove-background-noise",
            "spans": [
              {
                "text": "Remove background noise"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Every recording has some background hiss or hum, even a quiet room. PowerVoice removes it in two steps:"
              }
            ]
          },
          {
            "type": "ol",
            "items": [
              [
                {
                  "text": "Select "
                },
                {
                  "text": "0.5–60 seconds of room tone",
                  "bold": true
                },
                {
                  "text": " — a stretch with only the background noise and no speech, ideally right before or after your take. Choose "
                },
                {
                  "text": "Effects → Capture Noise Print",
                  "bold": true
                },
                {
                  "text": " (provisionally "
                },
                {
                  "text": "Shift+P",
                  "bold": true
                },
                {
                  "text": "). PowerVoice studies that noise's frequency makeup — its "
                },
                {
                  "text": "noise print",
                  "italic": true
                },
                {
                  "text": " — the same way you'd note the constant hum of an air conditioner before deciding how much to turn it down."
                }
              ],
              [
                {
                  "text": "Add "
                },
                {
                  "text": "Noise Reduction",
                  "bold": true
                },
                {
                  "text": " to the rack (Capture Noise Print does this for you if it isn't there yet). It continuously subtracts a version of that noise print from the whole take. Its two main controls are "
                },
                {
                  "text": "reduction",
                  "bold": true
                },
                {
                  "text": " (how many dB, at most, to remove) and "
                },
                {
                  "text": "amount",
                  "bold": true
                },
                {
                  "text": " (what percentage of that reduction to apply) — start low and raise them while listening: too much makes a voice sound thin or \"watery\" (an artifact of removing too much at once). Advanced controls (FFT size, sensitivity, spectral/time smoothing) are there for stubborn noise but the defaults suit most voice recordings."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Noise Reduction adds a small, fixed delay to the signal (about 43 ms at its default setting) while it works — PowerVoice automatically keeps your video/picture, meters and export in sync with it, so you don't need to do anything about that delay yourself."
              }
            ]
          },
          {
            "type": "h3",
            "id": "make-your-voice-sound-better",
            "spans": [
              {
                "text": "Make your voice sound better"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Two built-in modules shape the "
              },
              {
                "text": "tone",
                "italic": true
              },
              {
                "text": " (which frequencies stand out) and "
              },
              {
                "text": "dynamics",
                "italic": true
              },
              {
                "text": " (how loud vs quiet parts compare) of your voice:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Parametric EQ",
                  "bold": true
                },
                {
                  "text": " (equalizer) works like a set of very precise tone controls: a low-cut and high-cut filter (to remove rumble below or hiss above the range of speech), a low shelf and high shelf (turn a whole low or high region up or down), and five adjustable "
                },
                {
                  "text": "peaking bands",
                  "italic": true
                },
                {
                  "text": " that each boost or cut a chosen frequency by a chosen amount, over a chosen width (its "
                },
                {
                  "text": "Q",
                  "italic": true
                },
                {
                  "text": "). The rack's EQ graph shows the exact curve you're drawing, live, as you drag it. A little cut around 200–500 Hz can reduce \"boominess\"; a little boost around 2–5 kHz can add clarity or \"presence.\" - "
                },
                {
                  "text": "Hover the graph",
                  "bold": true
                },
                {
                  "text": " to read the frequency and gain under the pointer; hover a node to see its band, frequency, gain and Q (or slope, for the HP/LP bands), updating live while you drag it. - "
                },
                {
                  "text": "Right-click",
                  "bold": true
                },
                {
                  "text": " the curve or empty graph for "
                },
                {
                  "text": "Add band here",
                  "bold": true
                },
                {
                  "text": " (enables the nearest disabled band at that frequency — it says \"No free bands\" if all five are already in use); right-click a node for "
                },
                {
                  "text": "Delete band",
                  "bold": true
                },
                {
                  "text": " (or "
                },
                {
                  "text": "Enable band",
                  "bold": true
                },
                {
                  "text": ", if you clicked a disabled one), "
                },
                {
                  "text": "Reset band",
                  "bold": true
                },
                {
                  "text": " and, on HP/LP, its "
                },
                {
                  "text": "Slope",
                  "bold": true
                },
                {
                  "text": ". - "
                },
                {
                  "text": "Double-click",
                  "bold": true
                },
                {
                  "text": " the graph background, or the "
                },
                {
                  "text": "Expand",
                  "bold": true
                },
                {
                  "text": " button in its header, opens the same graph much larger in its own window — everything above works there too. Drag the "
                },
                {
                  "text": "Rack",
                  "bold": true
                },
                {
                  "text": " column's left edge to widen the whole panel if you'd rather work in place."
                }
              ],
              [
                {
                  "text": "Dynamics",
                  "bold": true
                },
                {
                  "text": " evens out how loud and quiet parts of your voice are, in the style of Audition's Dynamics panel, as up to four stages in order: an "
                },
                {
                  "text": "auto-gate",
                  "bold": true
                },
                {
                  "text": " (quiets the sound between phrases, off by default), an "
                },
                {
                  "text": "expander",
                  "bold": true
                },
                {
                  "text": " (widens the gap between quiet and loud, off by default), a "
                },
                {
                  "text": "compressor",
                  "bold": true
                },
                {
                  "text": " — on by default — that automatically turns down parts that go above a "
                },
                {
                  "text": "threshold",
                  "italic": true
                },
                {
                  "text": ", by an amount set by its "
                },
                {
                  "text": "ratio",
                  "italic": true
                },
                {
                  "text": " (so a loud word doesn't jump out over a quiet one), and a "
                },
                {
                  "text": "limiter",
                  "bold": true
                },
                {
                  "text": " (a hard ceiling that nothing can cross, off by default). Each stage reports how much gain reduction it's applying in real time, shown as a small meter in the rack, and expanding the module shows an interactive "
                },
                {
                  "text": "transfer curve",
                  "bold": true
                },
                {
                  "text": ": a graph of what comes out for a given level going in, with a draggable handle on the curve for each enabled stage's threshold, so you can set it by eye as well as by ear."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "The "
              },
              {
                "text": "Spectrum Inspector and voice diagnostics",
                "link": {
                  "doc": "user-guide",
                  "section": "check-your-levels-analyzer-and-diagnostics"
                }
              },
              {
                "text": " can suggest specific EQ moves (for example, a de-esser frequency for harsh \"s\" sounds) that you can add to the EQ with one click."
              }
            ]
          },
          {
            "type": "h3",
            "id": "rack-presets",
            "spans": [
              {
                "text": "Rack presets"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Rather than build a chain from scratch, load a "
              },
              {
                "text": "rack preset",
                "bold": true
              },
              {
                "text": " from the rack panel's preset menu ("
              },
              {
                "text": "Effects → Rack Presets",
                "bold": true
              },
              {
                "text": "):"
              }
            ]
          },
          {
            "type": "table",
            "head": [
              [
                {
                  "text": "Preset"
                }
              ],
              [
                {
                  "text": "What it's for"
                }
              ],
              [
                {
                  "text": "Chain (roughly)"
                }
              ]
            ],
            "rows": [
              [
                [
                  {
                    "text": "Podcast voice",
                    "bold": true
                  }
                ],
                [
                  {
                    "text": "A fuller, more processed voice for podcasts/streaming"
                  }
                ],
                [
                  {
                    "text": "rumble filter → noise gate → compressor (3.5:1) → EQ (a little low-mid cut, a little presence/air lift) → limiter at −1 dBTP"
                  }
                ]
              ],
              [
                [
                  {
                    "text": "Audiobook (ACX)",
                    "bold": true
                  }
                ],
                [
                  {
                    "text": "Meeting Audible's ACX submission requirements with headroom"
                  }
                ],
                [
                  {
                    "text": "rumble filter → gentle compressor (2:1, high threshold) → limiter at −3 dBTP"
                  }
                ]
              ],
              [
                [
                  {
                    "text": "Gentle cleanup",
                    "bold": true
                  }
                ],
                [
                  {
                    "text": "Light-touch polish that doesn't obviously \"sound processed\""
                  }
                ],
                [
                  {
                    "text": "rumble filter → light noise gate → mild compressor (1.5:1) → limiter at −1 dBTP"
                  }
                ]
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Loading a preset over a non-empty rack asks for confirmation first (rack edits aren't part of undo/redo in v1). Every built-in module also has its own presets (its own preset menu in its header) — see "
              },
              {
                "text": "Presets",
                "link": {
                  "doc": "user-guide",
                  "section": "presets"
                }
              },
              {
                "text": " for saving, renaming and sharing your own."
              }
            ]
          },
          {
            "type": "h3",
            "id": "apply-the-rack-bake",
            "spans": [
              {
                "text": "Apply the rack (bake)"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Effects → Bake Rack",
                "bold": true
              },
              {
                "text": " permanently applies the current rack to the selection (or the whole file) as one undoable edit, and then clears the rack. Use it when you want to lock in how something sounds — for example, before adding a different effect on top that should hear the "
              },
              {
                "text": "processed",
                "italic": true
              },
              {
                "text": " signal rather than the original. Undo brings back both the original audio and the rack you baked."
              }
            ]
          }
        ]
      },
      {
        "id": "plugins",
        "title": "Plugins",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "PowerVoice hosts third-party effects in "
              },
              {
                "text": "CLAP",
                "bold": true
              },
              {
                "text": ", "
              },
              {
                "text": "VST3",
                "bold": true
              },
              {
                "text": " and "
              },
              {
                "text": "LV2",
                "bold": true
              },
              {
                "text": " formats, plus REAPER's "
              },
              {
                "text": "JSFX",
                "bold": true
              },
              {
                "text": " scripts. Every plugin runs in its own protected process — see "
              },
              {
                "text": "Why plugins run in their own \"safety box\""
              },
              {
                "text": " for why that means a misbehaving plugin can't take PowerVoice or your recording down with it. LV2 and JSFX are available on Linux and macOS only; LV2 additionally needs the system "
              },
              {
                "text": "lilv",
                "code": true
              },
              {
                "text": " library (see "
              },
              {
                "text": "LV2 plugins are unavailable",
                "link": {
                  "doc": "user-guide",
                  "section": "troubleshooting"
                }
              },
              {
                "text": " if it's missing)."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Add an installed plugin to the rack the same way as a built-in: "
              },
              {
                "text": "Add module",
                "bold": true
              },
              {
                "text": ", under "
              },
              {
                "text": "Plugins (CLAP)",
                "bold": true
              },
              {
                "text": ", "
              },
              {
                "text": "Plugins (VST3)",
                "bold": true
              },
              {
                "text": ", "
              },
              {
                "text": "Plugins (LV2)",
                "bold": true
              },
              {
                "text": " or "
              },
              {
                "text": "Plugins (JSFX)",
                "bold": true
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "the-plugin-manager",
            "spans": [
              {
                "text": "The Plugin Manager"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Open it from "
              },
              {
                "text": "Effects → Manage Plugins…",
                "bold": true
              },
              {
                "text": ". Two tabs:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Plugins",
                  "bold": true
                },
                {
                  "text": ": every plugin PowerVoice found, with its format, channel layout, parameter count and "
                },
                {
                  "text": "status",
                  "bold": true
                },
                {
                  "text": " — OK, Disabled (hidden from Add module), Blocklisted (skipped because it crashed or timed out while being scanned, or you blocked it), or Flagged (crashed while running, but still usable). Search by name, vendor or path; "
                },
                {
                  "text": "Rescan",
                  "bold": true
                },
                {
                  "text": " picks up new or changed files, or scan everything again from scratch."
                }
              ],
              [
                {
                  "text": "Folders",
                  "bold": true
                },
                {
                  "text": ": where PowerVoice looks for plugins — the standard OS locations, plus any custom folders you add."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Effects → Install Module…",
                "bold": true
              },
              {
                "text": " copies a CLAP, VST3, LV2 or JSFX plugin file (or a PowerVoice "
              },
              {
                "text": ".voxmod",
                "code": true
              },
              {
                "text": " package) into your own plugin folder and scans it, so you don't have to find your system's plugin folders yourself."
              }
            ]
          },
          {
            "type": "h3",
            "id": "if-a-plugin-crashes",
            "spans": [
              {
                "text": "If a plugin crashes"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "A crash while a plugin is running doesn't take PowerVoice down: the affected rack slot mutes for a fraction of a second, is marked "
              },
              {
                "text": "Restarting",
                "italic": true
              },
              {
                "text": ", and comes back automatically with its last settings. If it fails again right away, the slot shows "
              },
              {
                "text": "Failed",
                "italic": true
              },
              {
                "text": " with a "
              },
              {
                "text": "Retry",
                "bold": true
              },
              {
                "text": " button — the rest of the rack and your recording are unaffected the whole time. A runtime crash is recorded as a \"flag\" in the Plugin Manager ("
              },
              {
                "text": "Clear crash warning",
                "bold": true
              },
              {
                "text": " dismisses it) but never blocks the plugin from being used again; only repeated failures "
              },
              {
                "text": "during a scan",
                "italic": true
              },
              {
                "text": " add it to the blocklist, which you can undo with "
              },
              {
                "text": "Unblock and rescan",
                "bold": true
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Some plugins have their own graphical editor window — open it from the window icon (⊟) in the plugin's rack slot (Linux needs X11 or XWayland; implemented but unverified on Windows; not yet supported on macOS). If a plugin's window closes because it crashed, reopen it the same way once the plugin has restarted."
              }
            ]
          }
        ]
      },
      {
        "id": "presets",
        "title": "Presets",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "Beyond the built-in "
              },
              {
                "text": "rack presets",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": ", you can save, load, rename, delete, export and import presets for the "
              },
              {
                "text": "whole rack",
                "bold": true
              },
              {
                "text": " or for "
              },
              {
                "text": "one module",
                "bold": true
              },
              {
                "text": " from "
              },
              {
                "text": "Effects → Manage Presets…",
                "bold": true
              },
              {
                "text": " (also reachable from a module's own preset menu, or the rack panel's). Its two tabs, "
              },
              {
                "text": "Rack presets",
                "bold": true
              },
              {
                "text": " and "
              },
              {
                "text": "Module presets",
                "bold": true
              },
              {
                "text": ", list the factory presets (marked "
              },
              {
                "text": "Factory",
                "italic": true
              },
              {
                "text": ") followed by your own, alphabetically:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Save as preset…",
                  "bold": true
                },
                {
                  "text": " captures a module's or the whole rack's current settings under a name you choose (a Noise Reduction preset can optionally include its captured noise print)."
                }
              ],
              [
                {
                  "text": "Export…",
                  "bold": true
                },
                {
                  "text": " / "
                },
                {
                  "text": "Import…",
                  "bold": true
                },
                {
                  "text": " write a preset to, or read one from, a file — the way to share a preset with someone else, back one up, or move it between computers. Importing over a preset with the same name asks before replacing it."
                }
              ],
              [
                {
                  "text": "Reset to Default",
                  "bold": true
                },
                {
                  "text": " (a module's slot menu) puts every one of its parameters back to the factory default without touching a captured noise print."
                }
              ]
            ]
          }
        ]
      },
      {
        "id": "check-your-levels-analyzer-and-diagnostics",
        "title": "Check your levels: analyzer and diagnostics",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "The dock along the bottom shows what you're hearing:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Meters",
                  "bold": true
                },
                {
                  "text": ": a vertical peak/RMS output meter (post-rack, with safe/loud/hot colour zones and a peak-hold tick) and, while recording, a matching vertical input meter next to it (with its own selectable scale floor — −60, −80 or −120 dBFS, for seeing a quiet mic or the room's noise floor). Either meter's "
                },
                {
                  "text": "CLIP",
                  "bold": true
                },
                {
                  "text": " lamp latches on the moment a sample clips and stays lit until you click it — so a brief clip you missed while looking away still gets your attention. A "
                },
                {
                  "text": "Speed",
                  "bold": true
                },
                {
                  "text": " control above the two meters (Fast/Medium/Slow) sets how quickly the bars fall back after a peak — shared by both meters, remembered, and separate from the Analyzer's own Fast/Medium/Slow response speed below."
                }
              ],
              [
                {
                  "text": "Analyzer",
                  "bold": true
                },
                {
                  "text": ": a live graph of which frequencies are currently in the sound, in three modes — "
                },
                {
                  "text": "Live",
                  "bold": true
                },
                {
                  "text": " (right now), "
                },
                {
                  "text": "Average",
                  "bold": true
                },
                {
                  "text": " (analyze a whole selection or file, as the source or as processed through the rack, to see its long-term tone), and "
                },
                {
                  "text": "Compare",
                  "bold": true
                },
                {
                  "text": " (freeze two curves, A and B, and see the difference between them — handy for comparing before/after an EQ move, or the source against the processed signal). "
                },
                {
                  "text": "Peak hold",
                  "bold": true
                },
                {
                  "text": " and labelled peaks help you read it; a Fast/Medium/Slow response smooths how quickly it reacts."
                }
              ],
              [
                {
                  "text": "Voice diagnostics",
                  "bold": true
                },
                {
                  "text": " (the Analyzer panel's Diagnostics toggle): plain readouts of pitch (F0), tone balance (how much \"mud\" around 200–500 Hz, \"presence\" around 2–5 kHz, and \"air\" around 10–16 kHz), sibilance (harsh \"s\" sounds, 4–10 kHz), mains hum, rumble (below 80 Hz), noise floor and signal-to-noise ratio — each with a short hint (\"A bit boomy — try a gentle cut around 300 Hz\") and, for several, an "
                },
                {
                  "text": "Add EQ band here",
                  "bold": true
                },
                {
                  "text": " button that inserts a matching Parametric EQ move."
                }
              ],
              [
                {
                  "text": "Spectrum Inspector",
                  "bold": true
                },
                {
                  "text": " ("
                },
                {
                  "text": "View → Spectrum Inspector",
                  "bold": true
                },
                {
                  "text": ", no default shortcut): a larger, dedicated live spectrum view with its own FFT size, window and response settings, for closer inspection than the compact Analyzer panel — wheel to zoom, drag to pan, Shift-drag to zoom to a range, double-click to reset. A legend explains every curve it draws: your voice (Live or Average), the dashed gray "
                },
                {
                  "text": "room tone",
                  "bold": true
                },
                {
                  "text": " curve (the spectrum of the quiet stretches between phrases, captured alongside an Average analysis — compare it with the voice curve to judge noise across frequency), and Snapshot A/B in Compare mode. Its own "
                },
                {
                  "text": "Explain My Voice",
                  "bold": true
                },
                {
                  "text": " button opens the same analysis the dock button does; if the Inspector already holds an Average result for the current section, it opens straight from that instead of running a new job."
                }
              ]
            ]
          }
        ]
      },
      {
        "id": "explain-my-voice",
        "title": "Explain My Voice",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "Explain My Voice",
                "bold": true
              },
              {
                "text": " (the button next to Diagnostics in the Analyzer panel — dock at the bottom) gives you a plain-language, one-page read of your voice: what its pitch is, how its tone balances across low, mid and high frequencies, whether there's sibilance, hum or rumble, and how clean the recording is — the same measurements as "
              },
              {
                "text": "Voice diagnostics",
                "link": {
                  "doc": "user-guide",
                  "section": "check-your-levels-analyzer-and-diagnostics"
                }
              },
              {
                "text": ", written out in full sentences instead of one-line hints. It needs an open file (and, ideally, a selection of clean speech); if nothing is selected it analyzes the whole file. While it works, the button shows real progress (\"Analyzing… 42 %\") with a "
              },
              {
                "text": "Cancel",
                "bold": true
              },
              {
                "text": ", so a slow analysis on a long file never looks the same as a stuck one."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "It opens a window titled "
              },
              {
                "text": "Voice Spectrum Analysis",
                "bold": true
              },
              {
                "text": ", sized to a large share of your screen (not a fixed pixel size, so it makes real use of a big or high-resolution display) with a "
              },
              {
                "text": "maximise",
                "bold": true
              },
              {
                "text": " control next to the export button for the full screen. It has three parts:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "A graph",
                  "bold": true
                },
                {
                  "text": " of your voice's frequency content, from 20 Hz up to 24 kHz, with your pitch and its harmonics marked, seven shaded bands for the regions engineers talk about (rumble, fundamental, low-mids, midrange, presence, sibilance, air), and callout cards pointing at whatever stood out. Toggle Raw FFT, Smoothed, Harmonics, Voice Bands and EQ Advice on or off above the graph, or turn off "
                },
                {
                  "text": "Annotations",
                  "bold": true
                },
                {
                  "text": " to hide the callout cards and their leader lines and see the curves, markers and bands unobstructed (remembered for next time). When EQ Advice is on but this take crossed no threshold by enough to suggest a change, a note next to the toggle says so — the EQ curve staying blank is the correct result, not a bug, and the note's tooltip explains why."
                }
              ],
              [
                {
                  "text": "An "
                },
                {
                  "text": "engineering summary",
                  "bold": true
                },
                {
                  "text": ": a \"voice profile\" (pitch, body, presence, sibilance, rumble, hum, each read as in the usual range, above it or below it) and a "
                },
                {
                  "text": "suggested focus",
                  "bold": true
                },
                {
                  "text": " — one line per measurement worth a closer listen, each with a button to act on it (add a matching EQ band, or copy a suggested frequency) where a fix is actually justified."
                }
              ],
              [
                {
                  "text": "An "
                },
                {
                  "text": "\"Also measured\"",
                  "bold": true
                },
                {
                  "text": " list underneath, for readings (noise floor, signal-to-noise, or \"no hum detected\") that don't point at one spot on the graph."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Export",
                "bold": true
              },
              {
                "text": " (the button beside maximise) saves what's on screen so you can send it to an audio engineer: an "
              },
              {
                "text": "image (PNG)",
                "bold": true
              },
              {
                "text": " of the full analysis, or a self-contained "
              },
              {
                "text": "report (HTML)",
                "bold": true
              },
              {
                "text": " with the graph, every finding's measured value and interpretation, and the take's duration and date. Either opens your system's native save dialog."
              }
            ]
          },
          {
            "type": "h3",
            "id": "its-an-average-over-time-not-a-snapshot",
            "spans": [
              {
                "text": "It's an average over time, not a snapshot"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "The graph is "
              },
              {
                "text": "not",
                "bold": true
              },
              {
                "text": " what your voice sounds like at this exact instant — it's the average tone over the whole span you analyzed (the selection, or the whole file), including the quiet pauses between words. The subtitle under the title names exactly how many seconds that was. This matters because a single instant of live audio only shows whichever vowel you happened to be saying; an average over a real stretch of speech is what actually describes "
              },
              {
                "text": "your voice",
                "italic": true
              },
              {
                "text": ", the same way a photo of one wave doesn't describe the tide. The pitch, sibilance and a few other numbers below the graph are measured over active speech only (the pauses are excluded from those, but not from the graph's curve), so the two can describe slightly different — though overlapping — material; the window says so."
              }
            ]
          },
          {
            "type": "h3",
            "id": "what-the-annotations-mean",
            "spans": [
              {
                "text": "What the annotations mean"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Every finding separates "
              },
              {
                "text": "what was measured",
                "bold": true
              },
              {
                "text": " from "
              },
              {
                "text": "what it might mean",
                "bold": true
              },
              {
                "text": " — a measured number never comes with a verdict attached for free:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Pitch (F0)",
                  "bold": true
                },
                {
                  "text": ": the median pitch of your voiced speech and the range it moved in (as a note name and in Hz). This is a description of your voice, not a judgement — there's no \"good\" or \"bad\" pitch."
                }
              ],
              [
                {
                  "text": "Strongest partial",
                  "bold": true
                },
                {
                  "text": ": the loudest single peak in your spectrum isn't always your pitch itself — for many voices it's the "
                },
                {
                  "text": "second",
                  "italic": true
                },
                {
                  "text": " harmonic (twice the pitch frequency) or another one. Both are completely ordinary; this finding just tells you which one is happening in your recording and why that doesn't change what your actual pitch is. If the peak lines up with a harmonic your take moved too much to measure separately (see "
                },
                {
                  "text": "\"Unresolved\" harmonics",
                  "link": {
                    "doc": "user-guide",
                    "section": "explain-my-voice"
                  }
                },
                {
                  "text": " below), it says so and names the likely harmonic instead of guessing — it only calls a peak a possible room or voice resonance once it genuinely doesn't line up with any harmonic of the pitch range you actually used."
                }
              ],
              [
                {
                  "text": "Harmonic series",
                  "bold": true
                },
                {
                  "text": ": which of the pitch's overtones (H1, H2, H3…) stand out clearly enough in the spectrum to measure individually."
                }
              ],
              [
                {
                  "text": "Low-mid body, Presence, Air, Sibilance, Rumble",
                  "bold": true
                },
                {
                  "text": ": how much energy sits in each frequency region, compared with what's typical for a voice."
                }
              ],
              [
                {
                  "text": "Mains hum",
                  "bold": true
                },
                {
                  "text": ": whether a 50 Hz or 60 Hz electrical hum (and its harmonics) was found in the quiet moments between phrases."
                }
              ],
              [
                {
                  "text": "Noise floor and Signal-to-noise",
                  "bold": true
                },
                {
                  "text": ": how quiet your quietest moment was, and how far your voice sits above it."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "A reading only escalates to something worth acting on once it clearly crosses a documented threshold — a measurement a hair past the line reads as a mild description, not a problem, and the "
              },
              {
                "text": "Voice diagnostics",
                "link": {
                  "doc": "user-guide",
                  "section": "check-your-levels-analyzer-and-diagnostics"
                }
              },
              {
                "text": " panel's one-line hints use the same thresholds, so the two never disagree about the same number. If nothing in your recording crosses a threshold, the summary says so plainly instead of inventing something to report."
              }
            ]
          },
          {
            "type": "h3",
            "id": "unresolved-harmonics",
            "spans": [
              {
                "text": "\"Unresolved\" harmonics"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Sometimes the harmonic series section says a harmonic "
              },
              {
                "text": "can't be measured",
                "bold": true
              },
              {
                "text": " rather than giving you a weak or missing reading for it. This isn't a bug: your pitch moves while you talk (a rising question, a stressed word), so each harmonic isn't a single frequency but a "
              },
              {
                "text": "band",
                "italic": true
              },
              {
                "text": " — the harmonic's number times your pitch's low end, up to that number times your pitch's high end. High enough up the series, those bands from neighbouring harmonics start to overlap, and once they do there's no way to tell where one ends and the next begins. Rather than guess, PowerVoice says the honest thing: nothing can be measured there, and tells you which harmonics that affects. A narrower pitch range resolves more harmonics; a wider one resolves fewer — it's a property of how your pitch moved during the take, not a fault in your voice or in the measurement."
              }
            ]
          },
          {
            "type": "h3",
            "id": "why-it-wont-suggest-fixing-your-voice-to-sound-flat",
            "spans": [
              {
                "text": "Why it won't suggest \"fixing\" your voice to sound flat"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "A natural voice's frequency spectrum isn't flat, and it isn't supposed to be — it rolls off toward the top the same way most microphones do. Explain My Voice is built to describe a voice honestly, not to nudge you toward one \"correct\" shape:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Low "
                },
                {
                  "text": "air",
                  "bold": true
                },
                {
                  "text": " (very high frequencies) is described, never treated as something to boost — boosting that region to flatten the curve would mostly add hiss and sibilance, not presence."
                }
              ],
              [
                {
                  "text": "\"Forward,\" \"boomy\" or \"harsh\" language, and any suggested EQ move, appears only once a measurement clears its threshold by a real margin — a reading a hair past the line is described mildly and offered no fix at all."
                }
              ],
              [
                {
                  "text": "Every recommendation stays conservative (about ±3 dB) and, where it applies, suggests checking microphone distance or working angle "
                },
                {
                  "text": "before",
                  "italic": true
                },
                {
                  "text": " reaching for EQ, because those change these numbers more than a filter does."
                }
              ],
              [
                {
                  "text": "The "
                },
                {
                  "text": "EQ Advice",
                  "bold": true
                },
                {
                  "text": " toggle draws its suggested moves as a dashed curve over your measured spectrum — never changing it — so you can see exactly what a suggested change would do before deciding whether you want it in your rack at all."
                }
              ]
            ]
          },
          {
            "type": "h3",
            "id": "read-together-with-voice-diagnostics",
            "spans": [
              {
                "text": "Read together with Voice diagnostics"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "The live "
              },
              {
                "text": "Voice diagnostics",
                "link": {
                  "doc": "user-guide",
                  "section": "check-your-levels-analyzer-and-diagnostics"
                }
              },
              {
                "text": " panel gives you the same kind of information continuously, as short hints; Explain My Voice gives you the considered, full-sentence version once, over a real stretch of your recording. Use diagnostics while you work and Explain My Voice when you want the fuller picture — a session, a note, or something to compare notes on with an audio engineer."
              }
            ]
          }
        ]
      },
      {
        "id": "hit-a-loudness-target",
        "title": "Hit a loudness target",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "The "
              },
              {
                "text": "Loudness",
                "bold": true
              },
              {
                "text": " panel (dock) measures your take: click "
              },
              {
                "text": "Analyze",
                "bold": true
              },
              {
                "text": " (as "
              },
              {
                "text": "Processed",
                "bold": true
              },
              {
                "text": ", through the rack, or "
              },
              {
                "text": "Source",
                "bold": true
              },
              {
                "text": ", the raw file) to get integrated loudness ("
              },
              {
                "text": "I",
                "bold": true
              },
              {
                "text": ", in "
              },
              {
                "text": "LUFS"
              },
              {
                "text": "), the loudest short-term ("
              },
              {
                "text": "S-max",
                "bold": true
              },
              {
                "text": ") and momentary ("
              },
              {
                "text": "M-max",
                "bold": true
              },
              {
                "text": ") moments, the loudness range ("
              },
              {
                "text": "LRA",
                "bold": true
              },
              {
                "text": ", in LU), sample peak and "
              },
              {
                "text": "true peak"
              },
              {
                "text": " ("
              },
              {
                "text": "TP",
                "bold": true
              },
              {
                "text": ", in dBTP)."
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Normalize",
                  "bold": true
                },
                {
                  "text": " in the toolbar/Edit menu holds one-click favourites for peak ("
                },
                {
                  "text": "−1",
                  "bold": true
                },
                {
                  "text": ", "
                },
                {
                  "text": "−0.1",
                  "bold": true
                },
                {
                  "text": ", "
                },
                {
                  "text": "−3 dB",
                  "bold": true
                },
                {
                  "text": ") and loudness ("
                },
                {
                  "text": "−16",
                  "bold": true
                },
                {
                  "text": ", "
                },
                {
                  "text": "−19",
                  "bold": true
                },
                {
                  "text": ", "
                },
                {
                  "text": "−23 LUFS",
                  "bold": true
                },
                {
                  "text": ") targets, plus a dialog for any other target. Like every normalize, it's a destructive but fully undoable gain change to the selection (or the whole file if nothing is selected)."
                }
              ],
              [
                {
                  "text": "ACX Check",
                  "bold": true
                },
                {
                  "text": " (dock panel) tests your file against Audible's three ACX submission rules, each measured against the whole file or your selection:"
                }
              ]
            ]
          },
          {
            "type": "table",
            "head": [
              [
                {
                  "text": "Rule"
                }
              ],
              [
                {
                  "text": "Limit"
                }
              ]
            ],
            "rows": [
              [
                [
                  {
                    "text": "RMS"
                  }
                ],
                [
                  {
                    "text": "−23 … −18 dB"
                  }
                ]
              ],
              [
                [
                  {
                    "text": "Peak"
                  }
                ],
                [
                  {
                    "text": "≤ −3 dBFS"
                  }
                ]
              ],
              [
                [
                  {
                    "text": "Noise floor (quietest 500 ms)"
                  }
                ],
                [
                  {
                    "text": "≤ −60 dB"
                  }
                ]
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "A failing rule comes with a one-line hint (e.g. \"RMS −27 dB is quieter than −23 dB: LUFS/RMS normalize up.\", or \"Noise floor −52 dB exceeds −60 dB: capture a noise print and add Noise Reduction.\"). The "
              },
              {
                "text": "Audiobook (ACX)",
                "bold": true
              },
              {
                "text": " "
              },
              {
                "text": "rack preset",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " already keeps peaks under the ACX ceiling via its limiter."
              }
            ]
          }
        ]
      },
      {
        "id": "export",
        "title": "Export",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "File → Export…",
                "bold": true
              },
              {
                "text": " Choose:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Format",
                  "bold": true
                },
                {
                  "text": ": WAV (16/24/32-bit float), FLAC (16/24-bit — no 32-bit float FLAC), or MP3 (CBR/VBR, needs a system LAME install — see "
                },
                {
                  "text": "MP3 export is greyed out",
                  "link": {
                    "doc": "user-guide",
                    "section": "troubleshooting"
                  }
                },
                {
                  "text": " if it's greyed out with \"(needs libmp3lame)\")."
                }
              ],
              [
                {
                  "text": "Range",
                  "bold": true
                },
                {
                  "text": ": whole file or just the current selection."
                }
              ],
              [
                {
                  "text": "ACX preset",
                  "bold": true
                },
                {
                  "text": ", if you want export settings that match Audible's ACX submission format."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Exports always render through the full rack (the rack panel's \"Listening only\" badge on an A/B'd module is a reminder that only "
              },
              {
                "text": "exports",
                "italic": true
              },
              {
                "text": " reflect the true, non-bypassed signal) — see "
              },
              {
                "text": "why what you export is what you heard"
              },
              {
                "text": ". Check "
              },
              {
                "text": "Hit a loudness target",
                "link": {
                  "doc": "user-guide",
                  "section": "hit-a-loudness-target"
                }
              },
              {
                "text": " before exporting if the file needs to pass an ACX submission."
              }
            ]
          }
        ]
      },
      {
        "id": "themes",
        "title": "Themes",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "Edit → Preferences → Display & Appearance",
                "bold": true
              },
              {
                "text": ", or "
              },
              {
                "text": "View → Theme",
                "bold": true
              },
              {
                "text": ", offers four choices: "
              },
              {
                "text": "Dark",
                "bold": true
              },
              {
                "text": " (the default), "
              },
              {
                "text": "Light",
                "bold": true
              },
              {
                "text": " (for bright rooms), "
              },
              {
                "text": "High Contrast",
                "bold": true
              },
              {
                "text": " (stronger text and lines, for accessibility), or "
              },
              {
                "text": "Match System",
                "bold": true
              },
              {
                "text": " (follows your OS's light/dark setting, and switches to High Contrast automatically if your OS is set to prefer more contrast). A theme change applies at once, everywhere — the waveform, spectrogram, meters and analyzer included."
              }
            ]
          }
        ]
      },
      {
        "id": "preferences",
        "title": "Preferences",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "Edit → Preferences",
                "bold": true
              },
              {
                "text": " groups settings into five sections: "
              },
              {
                "text": "Recording",
                "bold": true
              },
              {
                "text": " (pre-roll/post-roll lengths, punch crossfade, saved per-device-setup recording offsets from calibration), "
              },
              {
                "text": "Editing",
                "bold": true
              },
              {
                "text": " (what happens when you open a file with more than one channel), "
              },
              {
                "text": "Display & Appearance",
                "bold": true
              },
              {
                "text": " (theme), "
              },
              {
                "text": "Plugins",
                "bold": true
              },
              {
                "text": " (the list of installed plugins, with a shortcut to the Plugin Manager) and "
              },
              {
                "text": "Advanced",
                "bold": true
              },
              {
                "text": " (how much audio PowerVoice keeps in memory at once, and how often the playhead and meters refresh — 60 Hz is smoother, 30 Hz uses less CPU). Each section, or everything at once, can be reset to defaults from its own "
              },
              {
                "text": "Reset to defaults…",
                "bold": true
              },
              {
                "text": " button."
              }
            ]
          }
        ]
      },
      {
        "id": "shortcuts",
        "title": "Shortcuts",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "See "
              },
              {
                "text": "docs/shortcuts.md",
                "code": true
              },
              {
                "text": " for the full keyboard shortcut map — generated from "
              },
              {
                "text": "ui/src/lib/shortcuts/registry.ts",
                "code": true
              },
              {
                "text": ", the single source of truth the app itself uses, so it can't drift from what's actually bound. Shortcuts aren't remappable in this version. Help ▸ Keyboard Shortcuts shows the same list inside the app. Two bindings are still marked provisional (Record, Capture Noise Print) — they match Audition's defaults per the best sources found so far, but "
              },
              {
                "text": "helpx.adobe.com",
                "code": true
              },
              {
                "text": "'s own shortcut page still 403s to automated fetch, so neither has an independent, authoritative confirmation (T-701)."
              }
            ]
          }
        ]
      },
      {
        "id": "troubleshooting",
        "title": "Troubleshooting",
        "blocks": [
          {
            "type": "h3",
            "id": "no-audio-devices-show-up--wrong-devices-listed-linux-pipewire-jack-alsa",
            "spans": [
              {
                "text": "No audio devices show up / wrong devices listed (Linux: PipeWire, JACK, ALSA)"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "The "
              },
              {
                "text": "Audio Devices",
                "link": {
                  "doc": "user-guide",
                  "section": "set-up-your-microphone"
                }
              },
              {
                "text": " dialog lists whatever your audio backend reports, refreshed automatically on hot-plug. On Linux, PowerVoice picks a backend at build time (via "
              },
              {
                "text": "cpal",
                "code": true
              },
              {
                "text": ") in this order:"
              }
            ]
          },
          {
            "type": "ol",
            "items": [
              [
                {
                  "text": "PipeWire",
                  "bold": true
                },
                {
                  "text": ", if a PipeWire server is reachable (most current distros default to this — the product owner develops against PipeWire 1.6.8)."
                }
              ],
              [
                {
                  "text": "ALSA",
                  "bold": true
                },
                {
                  "text": ", otherwise."
                }
              ],
              [
                {
                  "text": "JACK",
                  "bold": true
                },
                {
                  "text": ", listed additionally whenever a JACK server is running ("
                },
                {
                  "text": "jackd",
                  "code": true
                },
                {
                  "text": "/"
                },
                {
                  "text": "jackdbus",
                  "code": true
                },
                {
                  "text": ") and the "
                },
                {
                  "text": "jack",
                  "code": true
                },
                {
                  "text": " cpal feature is compiled in."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "If nothing shows up: confirm your audio server is actually running ("
              },
              {
                "text": "systemctl --user status pipewire pipewire-pulse",
                "code": true
              },
              {
                "text": " or "
              },
              {
                "text": "wireplumber",
                "code": true
              },
              {
                "text": " for PipeWire; "
              },
              {
                "text": "jack_control status",
                "code": true
              },
              {
                "text": " for JACK), and that your user is in the right groups for raw ALSA device access if you're bypassing PipeWire/JACK entirely (Arch/most distros: the "
              },
              {
                "text": "audio",
                "code": true
              },
              {
                "text": " group, though PipeWire itself usually doesn't need it). Device enumeration runs off the UI thread because it can block for hundreds of milliseconds on some ALSA setups — a slow-to-populate list is normal on first open, not a bug by itself."
              }
            ]
          },
          {
            "type": "h3",
            "id": "the-ui-feels-laggy-or-tears-on-linux-webkitgtk-rendering",
            "spans": [
              {
                "text": "The UI feels laggy or tears on Linux (WebKitGTK rendering)"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "PowerVoice's waveform and spectrogram views target 60 fps, drawn with WebGL2 (falling back to Canvas2D automatically if WebGL2 isn't available, or if the WebGL2 context is lost mid-session). On Linux, WebKitGTK's GPU (DMA-BUF) compositing path measured noticeably worse frame times during this project's own testing (not limited to one GPU vendor) — so PowerVoice "
              },
              {
                "text": "disables it by default on Linux",
                "bold": true
              },
              {
                "text": ", automatically, before its window even opens ("
              },
              {
                "text": "WEBKIT_DISABLE_DMABUF_RENDERER=1",
                "code": true
              },
              {
                "text": ", set by the app itself unless already set). You shouldn't need to do anything. If you want to try the default WebKit DMA-BUF renderer instead (e.g. a driver update changed the tradeoff on your system), opt out with:"
              }
            ]
          },
          {
            "type": "code",
            "lang": "sh",
            "text": "POWERVOICE_WEBKIT_DMABUF=1 powervoice-app     # or however you launch the AppImage/installed binary"
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Setting "
              },
              {
                "text": "WEBKIT_DISABLE_DMABUF_RENDERER",
                "code": true
              },
              {
                "text": " yourself (to "
              },
              {
                "text": "0",
                "code": true
              },
              {
                "text": " or anything else) also takes precedence — PowerVoice never overrides a value you've already set."
              }
            ]
          },
          {
            "type": "h3",
            "id": "mp3-export-is-greyed-out--needs-libmp3lame",
            "spans": [
              {
                "text": "MP3 export is greyed out / \"needs libmp3lame\""
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "MP3 encoding uses "
              },
              {
                "text": "libmp3lame",
                "code": true
              },
              {
                "text": " (LAME), which PowerVoice "
              },
              {
                "text": "loads at runtime rather than bundling",
                "bold": true
              },
              {
                "text": " (it's LGPL-licensed — see "
              },
              {
                "text": "docs/adr/ADR-007-licensing.md",
                "code": true
              },
              {
                "text": " §4 for why). Every other feature works without it; only MP3 export is affected."
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Linux (.deb)",
                  "bold": true
                },
                {
                  "text": ": "
                },
                {
                  "text": "sudo apt install libmp3lame0",
                  "code": true
                },
                {
                  "text": " (declared as a "
                },
                {
                  "text": "Recommends",
                  "code": true
                },
                {
                  "text": ", so "
                },
                {
                  "text": "apt",
                  "code": true
                },
                {
                  "text": " should offer it automatically; if you installed via "
                },
                {
                  "text": "dpkg -i",
                  "code": true
                },
                {
                  "text": " directly, add it yourself). Arch: "
                },
                {
                  "text": "sudo pacman -S lame",
                  "code": true
                },
                {
                  "text": "."
                }
              ],
              [
                {
                  "text": "Linux (AppImage)",
                  "bold": true
                },
                {
                  "text": ": install the same system package — the AppImage doesn't bundle LAME either (by design: it's a runtime-replaceable dependency, not something to vendor)."
                }
              ],
              [
                {
                  "text": "Windows",
                  "bold": true
                },
                {
                  "text": ": no OS-level package for it. Download a "
                },
                {
                  "text": "libmp3lame.dll",
                  "code": true
                },
                {
                  "text": " build (e.g. from "
                },
                {
                  "text": "Rareware's LAME Windows builds"
                },
                {
                  "text": ") and place it next to "
                },
                {
                  "text": "powervoice-app.exe",
                  "code": true
                },
                {
                  "text": "."
                }
              ],
              [
                {
                  "text": "macOS",
                  "bold": true
                },
                {
                  "text": ": "
                },
                {
                  "text": "brew install lame",
                  "code": true
                },
                {
                  "text": " (Homebrew) provides "
                },
                {
                  "text": "libmp3lame.dylib",
                  "code": true
                },
                {
                  "text": "."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "After installing, restart PowerVoice — it probes for the library at startup."
              }
            ]
          },
          {
            "type": "h3",
            "id": "lv2-plugins-are-unavailable",
            "spans": [
              {
                "text": "LV2 plugins are unavailable"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "LV2 is one of the plugin formats PowerVoice can host. To use LV2 plugins, your computer needs a small free library called "
              },
              {
                "text": "lilv",
                "bold": true
              },
              {
                "text": ". PowerVoice doesn't include it; it uses the copy installed on your system. If lilv is missing, LV2 plugins don't appear under "
              },
              {
                "text": "Add module",
                "bold": true
              },
              {
                "text": " and the Plugin Manager shows the message "
              },
              {
                "text": "\"LV2 support needs the lilv library (liblilv-0), which isn't installed\"",
                "italic": true
              },
              {
                "text": ". Everything else, including CLAP and VST3 plugins, keeps working."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Install lilv:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "Debian / Ubuntu",
                  "bold": true
                },
                {
                  "text": ": "
                },
                {
                  "text": "sudo apt install liblilv-0-0",
                  "code": true
                },
                {
                  "text": " (the PowerVoice .deb recommends it, so "
                },
                {
                  "text": "apt",
                  "code": true
                },
                {
                  "text": " usually installs it for you)."
                }
              ],
              [
                {
                  "text": "Fedora",
                  "bold": true
                },
                {
                  "text": ": "
                },
                {
                  "text": "sudo dnf install lilv-libs",
                  "code": true
                },
                {
                  "text": "."
                }
              ],
              [
                {
                  "text": "Arch",
                  "bold": true
                },
                {
                  "text": ": "
                },
                {
                  "text": "sudo pacman -S lilv",
                  "code": true
                },
                {
                  "text": "."
                }
              ],
              [
                {
                  "text": "AppImage",
                  "bold": true
                },
                {
                  "text": ": install lilv with your distribution's package manager, as above."
                }
              ],
              [
                {
                  "text": "macOS",
                  "bold": true
                },
                {
                  "text": ": "
                },
                {
                  "text": "brew install lilv",
                  "code": true
                },
                {
                  "text": "."
                }
              ],
              [
                {
                  "text": "Windows",
                  "bold": true
                },
                {
                  "text": ": LV2 plugins aren't supported on Windows. Use CLAP or VST3 plugins instead."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "You don't need to restart PowerVoice: lilv is loaded the next time an LV2 plugin is scanned, so use "
              },
              {
                "text": "Rescan",
                "bold": true
              },
              {
                "text": " in the Plugin Manager. Advanced: to use your own lilv build, set the "
              },
              {
                "text": "POWERVOICE_LILV",
                "code": true
              },
              {
                "text": " environment variable to its full path."
              }
            ]
          },
          {
            "type": "h3",
            "id": "plugin-editor-windows",
            "spans": [
              {
                "text": "Plugin editor windows"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Some plugins offer a graphical editor window. Open it by clicking the window icon (⊟) in the plugin's rack slot. The window stays open while you edit and closes when you remove the plugin from the rack or PowerVoice closes."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "On Linux",
                "bold": true
              },
              {
                "text": ": the window needs X11 or XWayland. If neither is available, the window button is disabled. LV2 plugin windows additionally need the "
              },
              {
                "text": "suil",
                "bold": true
              },
              {
                "text": " library; LV2 plugins without suil have no window (but still process audio normally). See "
              },
              {
                "text": "Building PowerVoice"
              },
              {
                "text": " for installation instructions."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "On Windows",
                "bold": true
              },
              {
                "text": ": plugin editor windows are implemented (a native window, no X11 needed) but, like the rest of the Windows build, have never been run by the maintainer — treat them as unverified."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "On macOS",
                "bold": true
              },
              {
                "text": ": plugin editor windows are not yet supported. CLAP and VST3 plugins still work; they simply have no graphical window."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "After a plugin crash",
                "bold": true
              },
              {
                "text": ": if a plugin crashes, its window closes and doesn't reopen automatically — the plugin's sandbox restarts to recover. Reopen the window by clicking the window icon again."
              }
            ]
          },
          {
            "type": "h3",
            "id": "where-jsfx-effects-come-from",
            "spans": [
              {
                "text": "Where JSFX effects come from"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "JSFX are REAPER's text-based effects. PowerVoice runs them with its own built-in JSFX engine, in the same protected helper process as other plugins, so a broken script can't take PowerVoice down. It finds them in three places:"
              }
            ]
          },
          {
            "type": "ul",
            "items": [
              [
                {
                  "text": "PowerVoice's own JSFX folder",
                  "bold": true
                },
                {
                  "text": ", where "
                },
                {
                  "text": "Install module…",
                  "bold": true
                },
                {
                  "text": " copies a "
                },
                {
                  "text": ".jsfx",
                  "code": true
                },
                {
                  "text": " file (with the files it imports from its own folder): "
                },
                {
                  "text": "~/.local/share/powervoice/Effects",
                  "code": true
                },
                {
                  "text": " on Linux, "
                },
                {
                  "text": "~/Library/Application Support/app.powervoice.powervoice/Effects",
                  "code": true
                },
                {
                  "text": " on macOS."
                }
              ],
              [
                {
                  "text": "REAPER's effects folder",
                  "bold": true
                },
                {
                  "text": ", if REAPER is installed: "
                },
                {
                  "text": "~/.config/REAPER/Effects",
                  "code": true
                },
                {
                  "text": " on Linux, "
                },
                {
                  "text": "~/Library/Application Support/REAPER/Effects",
                  "code": true
                },
                {
                  "text": " on macOS. REAPER's own effects have no file extension; PowerVoice recognises them by their "
                },
                {
                  "text": "desc:",
                  "code": true
                },
                {
                  "text": " line."
                }
              ],
              [
                {
                  "text": "Any "
                },
                {
                  "text": "custom folder",
                  "bold": true
                },
                {
                  "text": " you add in the Plugin Manager's "
                },
                {
                  "text": "Folders",
                  "bold": true
                },
                {
                  "text": " tab."
                }
              ]
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "JSFX effects appear under "
              },
              {
                "text": "Add module → Plugins (JSFX)",
                "bold": true
              },
              {
                "text": ". Their sliders are the effect's parameters, so you can automate them, and they're saved with your project and presets. Their custom graphics ("
              },
              {
                "text": "@gfx",
                "code": true
              },
              {
                "text": ") aren't shown yet: PowerVoice shows the sliders instead. A script that doesn't compile is listed in the Plugin Manager but not in "
              },
              {
                "text": "Add module",
                "bold": true
              },
              {
                "text": ". A script that freezes while it's being checked is blocked, like any plugin that hangs. JSFX aren't supported on Windows yet."
              }
            ]
          },
          {
            "type": "h3",
            "id": "recording-sounds-delayed--out-of-sync-with-playback",
            "spans": [
              {
                "text": "Recording sounds delayed / out of sync with playback"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "See "
              },
              {
                "text": "Latency calibration",
                "link": {
                  "doc": "user-guide",
                  "section": "first-recording"
                }
              },
              {
                "text": " above. Also check the "
              },
              {
                "text": "Monitoring latency",
                "bold": true
              },
              {
                "text": " readout in the Record panel — a red warning there suggests a smaller audio buffer size, switching to Dry monitoring, or removing high-latency rack modules (Noise Reduction in particular can add up to ~50 ms; a "
              },
              {
                "text": "bypassed",
                "italic": true
              },
              {
                "text": " module still holds its latency, so bypassing it doesn't help — remove it instead if you need the latency back)."
              }
            ]
          },
          {
            "type": "h3",
            "id": "recovering-after-a-crash-or-power-loss",
            "spans": [
              {
                "text": "Recovering after a crash or power loss"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Open "
              },
              {
                "text": "File → Recovery & Storage…",
                "bold": true
              },
              {
                "text": ". Interrupted takes are recovered from the crash-safe session journal; markers show where any dropouts or the interruption itself occurred. The dialog stays open until you've dealt with every recoverable session, so you can't accidentally lose one by dismissing it too fast. See "
              },
              {
                "text": "why your original recording is never damaged"
              },
              {
                "text": " for how this works under the hood."
              }
            ]
          }
        ]
      }
    ]
  },
  {
    "id": "faq",
    "title": "FAQ and troubleshooting",
    "sections": [
      {
        "id": "overview",
        "title": "FAQ and troubleshooting",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "Quick answers to the questions people actually ask. For step-by-step instructions, see the "
              },
              {
                "text": "user guide",
                "link": {
                  "doc": "user-guide",
                  "section": "overview"
                }
              },
              {
                "text": "; for platform-specific problems (no devices found, MP3 export greyed out, LV2/plugin windows, sync/latency), see its "
              },
              {
                "text": "Troubleshooting",
                "link": {
                  "doc": "user-guide",
                  "section": "troubleshooting"
                }
              },
              {
                "text": " section — this page links to it rather than repeating it."
              }
            ]
          }
        ]
      },
      {
        "id": "general",
        "title": "General",
        "blocks": [
          {
            "type": "h3",
            "id": "what-is-powervoice-in-one-sentence",
            "spans": [
              {
                "text": "What is PowerVoice, in one sentence?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "A desktop program for recording your voice, cleaning it up, and exporting a finished file — see "
              },
              {
                "text": "What is PowerVoice?"
              },
              {
                "text": " for the full pitch, or "
              },
              {
                "text": "How PowerVoice works"
              },
              {
                "text": " for what happens to your sound as you use it."
              }
            ]
          },
          {
            "type": "h3",
            "id": "is-powervoice-a-digital-audio-workstation-daw-like-audition-or-pro-tools",
            "spans": [
              {
                "text": "Is PowerVoice a Digital Audio Workstation (DAW) like Audition or Pro Tools?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Not a full one. It edits one mono file at a time and doesn't do multitrack mixing, MIDI or music-production features — see "
              },
              {
                "text": "What PowerVoice deliberately doesn't do"
              },
              {
                "text": ". If you need those, use a full DAW instead; if your job is \"record one voice, clean it, deliver it,\" PowerVoice is built for exactly that."
              }
            ]
          },
          {
            "type": "h3",
            "id": "is-powervoice-free",
            "spans": [
              {
                "text": "Is PowerVoice free?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Yes. It's dual-licensed under "
              },
              {
                "text": "MIT"
              },
              {
                "text": " or "
              },
              {
                "text": "Apache-2.0"
              },
              {
                "text": ", at your choice — see the "
              },
              {
                "text": "README"
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "is-powervoice-affiliated-with-adobe-or-does-it-open-audition-files",
            "spans": [
              {
                "text": "Is PowerVoice affiliated with Adobe, or does it open Audition files?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "No affiliation — \"Adobe Audition\" is only mentioned to describe the kind of tool PowerVoice is inspired by. PowerVoice opens plain audio files (WAV, FLAC, MP3, M4A, OGG) plus its own small sidecar file, not Audition's session format."
              }
            ]
          },
          {
            "type": "h3",
            "id": "what-platforms-does-powervoice-run-on",
            "spans": [
              {
                "text": "What platforms does PowerVoice run on?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "It's developed and tested on "
              },
              {
                "text": "Linux",
                "bold": true
              },
              {
                "text": ". Windows and macOS builds compile from source but haven't been verified yet — see the "
              },
              {
                "text": "README status note"
              },
              {
                "text": " and "
              },
              {
                "text": "Where files live"
              },
              {
                "text": " for the platform differences that are and aren't tested. If you try Windows or macOS, reports of what does and doesn't work are welcome."
              }
            ]
          },
          {
            "type": "h3",
            "id": "the-appimage-wont-open--says-there-is-no-app-installed-for-appimage-application-bundle",
            "spans": [
              {
                "text": "The AppImage won't open / says \"There is no app installed for AppImage application bundle\""
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "A downloaded file isn't executable yet, and some file managers show that error instead of anything about permissions. Make it executable once, then run it — see "
              },
              {
                "text": "Install",
                "link": {
                  "doc": "user-guide",
                  "section": "install"
                }
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "can-i-record-more-than-one-microphone-or-a-stereo-source",
            "spans": [
              {
                "text": "Can I record more than one microphone, or a stereo source?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Recording is mono, from one chosen input channel — see "
              },
              {
                "text": "Set up your microphone",
                "link": {
                  "doc": "user-guide",
                  "section": "set-up-your-microphone"
                }
              },
              {
                "text": ". A stereo file you "
              },
              {
                "text": "open",
                "italic": true
              },
              {
                "text": " is downmixed to mono (or you pick one channel) on import, based on your "
              },
              {
                "text": "Preferences → Editing → Multichannel files",
                "bold": true
              },
              {
                "text": " setting."
              }
            ]
          },
          {
            "type": "h3",
            "id": "can-i-have-more-than-one-file-open-or-a-multitrack-timeline",
            "spans": [
              {
                "text": "Can I have more than one file open, or a multitrack timeline?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "No — PowerVoice edits one file at a time by design (see "
              },
              {
                "text": "What is PowerVoice?"
              },
              {
                "text": "). Open another file with "
              },
              {
                "text": "File → Open…",
                "bold": true
              },
              {
                "text": " and it replaces the current one (after asking about unsaved changes)."
              }
            ]
          }
        ]
      },
      {
        "id": "recording-and-editing",
        "title": "Recording and editing",
        "blocks": [
          {
            "type": "h3",
            "id": "why-is-record-greyed-out",
            "spans": [
              {
                "text": "Why is \"Record\" greyed out?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "You need an input device chosen first — open "
              },
              {
                "text": "Audio Devices",
                "link": {
                  "doc": "user-guide",
                  "section": "set-up-your-microphone"
                }
              },
              {
                "text": " (the toolbar's gear icon) and pick one."
              }
            ]
          },
          {
            "type": "h3",
            "id": "how-do-i-fix-just-one-word-or-sentence-without-re-recording-the-whole-take",
            "spans": [
              {
                "text": "How do I fix just one word or sentence without re-recording the whole take?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Select the part to redo and press "
              },
              {
                "text": "Shift+R",
                "bold": true
              },
              {
                "text": " — this is a "
              },
              {
                "text": "punch-in",
                "italic": true
              },
              {
                "text": ": PowerVoice plays a lead-in, records over exactly your selection, then plays a bit after so you can hear the join. See "
              },
              {
                "text": "Re-recording part of a take (punch-in)",
                "link": {
                  "doc": "user-guide",
                  "section": "first-recording"
                }
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "the-timing-of-my-punch-in-doesnt-line-up-with-the-original--whats-wrong",
            "spans": [
              {
                "text": "The timing of my punch-in doesn't line up with the original — what's wrong?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Your audio interface's own round-trip delay (the time between PowerVoice sending a sound and your microphone hearing it back) probably isn't measured yet. Run "
              },
              {
                "text": "Latency calibration",
                "link": {
                  "doc": "user-guide",
                  "section": "first-recording"
                }
              },
              {
                "text": " once per device/buffer-size combination."
              }
            ]
          },
          {
            "type": "h3",
            "id": "i-deleted-or-changed-something-by-mistake--can-i-get-it-back",
            "spans": [
              {
                "text": "I deleted or changed something by mistake — can I get it back?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Yes: "
              },
              {
                "text": "Ctrl/⌘+Z",
                "bold": true
              },
              {
                "text": " (Undo) as many times as you need — PowerVoice's undo has no built-in limit other than free disk space. See "
              },
              {
                "text": "Editing and undo",
                "link": {
                  "doc": "user-guide",
                  "section": "editing-and-undo"
                }
              },
              {
                "text": " and "
              },
              {
                "text": "why undo never runs out"
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "does-adding-effects-change-my-original-recording",
            "spans": [
              {
                "text": "Does adding effects change my original recording?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "No. The effects rack is "
              },
              {
                "text": "non-destructive",
                "bold": true
              },
              {
                "text": ": it's applied live while you listen, but your document underneath is unchanged until you explicitly "
              },
              {
                "text": "bake",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " the rack or export. Remove an effect, or undo a bake, and you're back to the untouched audio."
              }
            ]
          }
        ]
      },
      {
        "id": "noise-tone-and-loudness",
        "title": "Noise, tone and loudness",
        "blocks": [
          {
            "type": "h3",
            "id": "how-do-i-remove-background-hiss-or-hum",
            "spans": [
              {
                "text": "How do I remove background hiss or hum?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Select a short stretch of silence (room tone only, no speech), capture a "
              },
              {
                "text": "noise print",
                "italic": true
              },
              {
                "text": " from it, then add Noise Reduction to the rack. See "
              },
              {
                "text": "Remove background noise",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "i-added-noise-reduction-and-now-my-voice-sounds-thin-or-underwater--why",
            "spans": [
              {
                "text": "I added Noise Reduction and now my voice sounds thin or \"underwater\" — why?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "That's a sign of too much reduction. Lower the "
              },
              {
                "text": "reduction",
                "bold": true
              },
              {
                "text": " (dB) or "
              },
              {
                "text": "amount",
                "bold": true
              },
              {
                "text": " (%) controls until the artifact goes away, or capture a cleaner, longer noise print (0.5–60 s, no speech in it)."
              }
            ]
          },
          {
            "type": "h3",
            "id": "what-is-acx-and-how-do-i-know-if-my-file-will-pass",
            "spans": [
              {
                "text": "What is ACX, and how do I know if my file will pass?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "ACX"
              },
              {
                "text": " is Audible's technical checklist for audiobook submissions (loudness, peak level and background noise limits). Open the "
              },
              {
                "text": "ACX Check",
                "bold": true
              },
              {
                "text": " panel to test your file against all three rules with plain-language hints on any failure — see "
              },
              {
                "text": "Hit a loudness target",
                "link": {
                  "doc": "user-guide",
                  "section": "hit-a-loudness-target"
                }
              },
              {
                "text": ". The "
              },
              {
                "text": "Audiobook (ACX)",
                "bold": true
              },
              {
                "text": " "
              },
              {
                "text": "rack preset",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": " is built to pass it with headroom to spare."
              }
            ]
          },
          {
            "type": "h3",
            "id": "whats-the-difference-between-normalize-and-hitting-an-acx-target",
            "spans": [
              {
                "text": "What's the difference between \"Normalize\" and \"hitting an ACX target\"?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Normalize",
                "bold": true
              },
              {
                "text": " changes the overall level (peak or "
              },
              {
                "text": "LUFS"
              },
              {
                "text": ") to one target number in one click. The "
              },
              {
                "text": "ACX check",
                "bold": true
              },
              {
                "text": " tests three separate numbers at once (loudness, peak, and noise floor) — normalizing alone doesn't guarantee the noise-floor rule passes, which is why the noise floor hint suggests noise reduction, not normalizing."
              }
            ]
          },
          {
            "type": "h3",
            "id": "my-acx-check-fails-on-noise-floor-even-though-the-recording-sounds-clean--why",
            "spans": [
              {
                "text": "My ACX check fails on \"noise floor\" even though the recording sounds clean — why?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "The noise floor rule looks at the "
              },
              {
                "text": "quietest half-second",
                "bold": true
              },
              {
                "text": " in the whole file, which is often quieter than what you notice while listening to speech. Capture a noise print from your quietest stretch of room tone and add Noise Reduction — see "
              },
              {
                "text": "Remove background noise",
                "link": {
                  "doc": "user-guide",
                  "section": "cleaning-up-the-effects-rack"
                }
              },
              {
                "text": "."
              }
            ]
          }
        ]
      },
      {
        "id": "explain-my-voice",
        "title": "Explain My Voice",
        "blocks": [
          {
            "type": "h3",
            "id": "what-does-explain-my-voice-actually-do",
            "spans": [
              {
                "text": "What does Explain My Voice actually do?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "It measures your voice's pitch, tone balance, sibilance, hum and cleanliness over a real stretch of your recording (the selection, or the whole file), and writes out what it found in full sentences instead of the Diagnostics panel's one-line hints. See "
              },
              {
                "text": "Explain My Voice",
                "link": {
                  "doc": "user-guide",
                  "section": "explain-my-voice"
                }
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "is-the-graph-what-my-voice-sounds-like-right-now",
            "spans": [
              {
                "text": "Is the graph what my voice sounds like right now?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "No — it's a "
              },
              {
                "text": "long-term average",
                "bold": true
              },
              {
                "text": " over the whole span you analyzed, pauses between words included, not an instantaneous snapshot. A single instant only shows whichever sound you happened to be making; the average is what actually describes your voice. See "
              },
              {
                "text": "Explain My Voice: it's an average over time, not a snapshot",
                "link": {
                  "doc": "user-guide",
                  "section": "explain-my-voice"
                }
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "what-does-unresolved-mean-next-to-a-harmonic",
            "spans": [
              {
                "text": "What does \"unresolved\" mean next to a harmonic?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "It means that harmonic genuinely can't be measured from your recording, not that it's weak or missing. Your pitch moves while you talk, so each harmonic spans a band of frequencies rather than one exact frequency; high enough in the series, neighbouring harmonics' bands overlap and nothing can be said about them individually. PowerVoice says so rather than guessing. See "
              },
              {
                "text": "Explain My Voice: \"unresolved\" harmonics",
                "link": {
                  "doc": "user-guide",
                  "section": "explain-my-voice"
                }
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "why-doesnt-it-just-tell-me-how-to-make-my-voice-sound-better",
            "spans": [
              {
                "text": "Why doesn't it just tell me how to make my voice sound \"better\"?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Because a natural voice isn't supposed to be flat — every voice and every microphone rolls off toward very high frequencies, so \"fixing\" that would mostly add hiss. Explain My Voice only recommends a change once a measurement clearly crosses a documented threshold, keeps every suggestion conservative, and prefers \"check your microphone distance\" over an EQ move wherever that's the more likely cause. See "
              },
              {
                "text": "Explain My Voice: why it won't suggest \"fixing\" your voice to sound flat",
                "link": {
                  "doc": "user-guide",
                  "section": "explain-my-voice"
                }
              },
              {
                "text": "."
              }
            ]
          }
        ]
      },
      {
        "id": "plugins",
        "title": "Plugins",
        "blocks": [
          {
            "type": "h3",
            "id": "powervoice-crashed--was-it-a-plugin",
            "spans": [
              {
                "text": "PowerVoice crashed — was it a plugin?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Usually not: third-party plugins run in their own protected process precisely so a plugin crash can't take PowerVoice down with it — see "
              },
              {
                "text": "Why plugins run in their own \"safety box\""
              },
              {
                "text": ". If PowerVoice itself closes unexpectedly, that's a PowerVoice bug — please report it with the log file ("
              },
              {
                "text": "logs/powervoice.log",
                "code": true
              },
              {
                "text": " — see "
              },
              {
                "text": "Where files live"
              },
              {
                "text": ")."
              }
            ]
          },
          {
            "type": "h3",
            "id": "a-plugin-keeps-failing-to-load--is-blocklisted--what-do-i-do",
            "spans": [
              {
                "text": "A plugin keeps failing to load / is \"Blocklisted\" — what do I do?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "It crashed or timed out while PowerVoice was scanning it. Open "
              },
              {
                "text": "Effects → Manage Plugins…",
                "bold": true
              },
              {
                "text": ", find it, and use "
              },
              {
                "text": "Unblock and rescan",
                "bold": true
              },
              {
                "text": " — see "
              },
              {
                "text": "If a plugin crashes",
                "link": {
                  "doc": "user-guide",
                  "section": "plugins"
                }
              },
              {
                "text": ". If it fails the same way again, the plugin itself likely has a real problem on your system."
              }
            ]
          },
          {
            "type": "h3",
            "id": "why-dont-i-see-any-lv2-plugins",
            "spans": [
              {
                "text": "Why don't I see any LV2 plugins?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Your system is missing the "
              },
              {
                "text": "lilv",
                "code": true
              },
              {
                "text": " library, which PowerVoice needs to host LV2 plugins — see "
              },
              {
                "text": "LV2 plugins are unavailable",
                "link": {
                  "doc": "user-guide",
                  "section": "troubleshooting"
                }
              },
              {
                "text": " for how to install it per platform."
              }
            ]
          },
          {
            "type": "h3",
            "id": "can-i-use-vst2-plugins",
            "spans": [
              {
                "text": "Can I use VST2 plugins?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "No. Steinberg stopped licensing the VST2 SDK in 2018, so there is no legitimate way to add VST2 hosting, and PowerVoice has decided not to ship it. CLAP, VST3, LV2 and JSFX are all supported, and practically every maintained plugin offers one of them."
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "If you have an old VST2-only plugin you can't replace, you can wrap it yourself — "
              },
              {
                "text": "Carla"
              },
              {
                "text": " and VST2-to-VST3 wrappers both work — and load the wrapper in PowerVoice as a supported format."
              }
            ]
          }
        ]
      },
      {
        "id": "files-and-safety",
        "title": "Files and safety",
        "blocks": [
          {
            "type": "h3",
            "id": "where-are-my-recordings-actually-stored-while-im-working",
            "spans": [
              {
                "text": "Where are my recordings actually stored while I'm working?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "In a private working copy (a "
              },
              {
                "text": "session",
                "italic": true
              },
              {
                "text": ") separate from your actual file, so your original is never touched until you save — see "
              },
              {
                "text": "Why your original recording is never damaged"
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "powervoice-or-my-computer-crashed-while-i-was-working--did-i-lose-everything",
            "spans": [
              {
                "text": "PowerVoice (or my computer) crashed while I was working — did I lose everything?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Almost certainly not. Open "
              },
              {
                "text": "File → Recovery & Storage…",
                "bold": true
              },
              {
                "text": " next time you start PowerVoice — it finds your interrupted session and offers to bring it back, typically losing at most a fraction of a second of work. See "
              },
              {
                "text": "Recovering after a crash or power loss",
                "link": {
                  "doc": "user-guide",
                  "section": "troubleshooting"
                }
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "what-is-the-vojson-file-next-to-my-audio-file",
            "spans": [
              {
                "text": "What is the "
              },
              {
                "text": ".vo.json",
                "code": true
              },
              {
                "text": " file next to my audio file?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "That's the "
              },
              {
                "text": "sidecar",
                "bold": true
              },
              {
                "text": " — a small text file that remembers your effects rack, markers and view settings for that file, written when you Save. Deleting it just resets those (your audio is unaffected); see "
              },
              {
                "text": "glossary: Sidecar"
              },
              {
                "text": "."
              }
            ]
          },
          {
            "type": "h3",
            "id": "can-i-edit-stereo-files-or-files-with-more-than-2-channels",
            "spans": [
              {
                "text": "Can I edit stereo files, or files with more than 2 channels?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "You can "
              },
              {
                "text": "open",
                "italic": true
              },
              {
                "text": " them — PowerVoice downmixes to mono or lets you pick one channel on import (your choice, or a saved preference, under "
              },
              {
                "text": "Preferences → Editing → Multichannel files",
                "bold": true
              },
              {
                "text": ") — but editing itself is always mono."
              }
            ]
          },
          {
            "type": "h3",
            "id": "what-formats-can-i-import-and-export",
            "spans": [
              {
                "text": "What formats can I import and export?"
              }
            ]
          },
          {
            "type": "p",
            "spans": [
              {
                "text": "Import: WAV, FLAC, MP3, M4A (AAC) and OGG (Vorbis). Export: WAV (16/24/32-bit float), FLAC (16/24-bit) and MP3 (needs a system LAME install — see "
              },
              {
                "text": "MP3 export is greyed out",
                "link": {
                  "doc": "user-guide",
                  "section": "troubleshooting"
                }
              },
              {
                "text": " if it isn't available). A file imported from a lossy format (MP3, M4A, OGG) can only be "
              },
              {
                "text": "saved",
                "italic": true
              },
              {
                "text": " as WAV or FLAC, since re-saving as the same lossy format would lose quality twice over."
              }
            ]
          }
        ]
      },
      {
        "id": "didnt-find-your-question",
        "title": "Didn't find your question?",
        "blocks": [
          {
            "type": "p",
            "spans": [
              {
                "text": "Check the full "
              },
              {
                "text": "Troubleshooting",
                "link": {
                  "doc": "user-guide",
                  "section": "troubleshooting"
                }
              },
              {
                "text": " section of the user guide, or the "
              },
              {
                "text": "glossary"
              },
              {
                "text": " if a term is unfamiliar. Still stuck? Open an issue on the "
              },
              {
                "text": "GitHub repository"
              },
              {
                "text": " with what you were doing, what you expected, and (if PowerVoice crashed) the relevant "
              },
              {
                "text": "crashes/crash-*.log",
                "code": true
              },
              {
                "text": " or "
              },
              {
                "text": "logs/powervoice.log",
                "code": true
              },
              {
                "text": " file — see "
              },
              {
                "text": "Where files live"
              },
              {
                "text": " for their locations."
              }
            ]
          }
        ]
      }
    ]
  }
];
