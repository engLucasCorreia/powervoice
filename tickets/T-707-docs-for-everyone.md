# T-707 — Documentation for everyone: non-technical guide, how-it-works, FAQ (owner request)

- **Tier:** Sonnet (writing; no review loop)
- **Owner request (2026-09-15):** docs that make "everybody understand this app even for the non technical people", with Mermaid graphs. This is the non-technical half; T-706 is the technical half. Both must be done before the project is finished.
- **Depends on:** T-708 (themes) and T-709 (tour), so both can be documented, and on T-706 for the shared glossary and hub. Run as one of the last tickets.
- **Read first:**
  - `docs/user-guide.md`, `README.md`, `docs/README.md` (from T-706), `docs/glossary.md`;
  - the app itself through `?preview` scenes and the Tour content;
  - PROMPT.md §1 (the product).

## Deliverables (Markdown in `docs/`; plain language, short sentences, no jargon without a glossary link)
1. **`docs/what-is-powervoice.md`:**
   - what the app is for, who it is for, what it does compared with Adobe Audition, and what it deliberately doesn't do;
   - a simple Mermaid journey: record → clean → level → check → export.
2. **`docs/how-it-works.md`:** how sound travels through PowerVoice, explained without code.
   - Mermaid flowcharts: microphone → recording → your document → effects rack → meters → export.
   - Why your original recording is never damaged: non-destructive editing, autosave and recovery.
   - Why plugins run in their own "safety box" (the sandbox).
3. **`docs/user-guide.md`, expanded into a complete guide** covering every feature, with task-oriented chapters:
   - set up your microphone; record a clean voice-over; re-record a mistake (punch-in); markers;
   - edit and undo; waveform and spectral views; remove background noise (noise print plus reduction; the gate);
   - make your voice sound better (EQ, dynamics); hit loudness targets (LUFS, true peak, the ACX check, normalize favourites);
   - export (WAV/FLAC/MP3); presets; plugins (install, manage, what to do if one crashes);
   - themes (dark/light/system/high contrast); the guided tour; preferences; shortcuts.

   Add screenshots or ASCII layout sketches where they help.
4. **`docs/faq.md` and troubleshooting:** the common problems and questions, answered in plain words.
5. **`docs/glossary.md`:** extend T-706's glossary with every audio term (dB, dBFS, LUFS, true peak, noise floor, gate, compressor, limiter, EQ band, ACX, sample rate, bit depth, dither, latency…). Each term gets a one-line plain-language explanation and an everyday analogy.
6. **`README.md`:** a friendly front page with what it is, a screenshot, how to install, links to the docs hub, and a short feature list.

## Quality bar
- A non-technical reader can follow the "record a clean voice-over and export it for ACX" chapter end to end.
- Every UI name in the docs matches the app's real English i18n strings. Check them against `ui/src/lib/i18n/`.
- `scripts/docs/check.py` (from T-706) passes.

`just check` must pass.
