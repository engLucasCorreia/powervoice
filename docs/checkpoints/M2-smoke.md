# M2 manual smoke checklist (owner, Arch/PipeWire)

Appended to by each M2 UI ticket. Run against a real build (`just dev`), not the test suite.

## T-209 — File menu & dialogs

- [ ] Open a long (several-minute) MP3/FLAC file: a progress bar with Cancel appears under the
  time ruler; Cancel stops the import and leaves whatever was open before untouched.
- [ ] Open a stereo file with one obviously silent channel: the channel-choice dialog appears with
  the silent channel identified and the active one preselected; "Always do this for multichannel
  files" is remembered (Settings → Files → Multichannel files) so the next multichannel open skips
  the dialog.
- [ ] Open a dual-mono (identical L/R) file: no dialog appears.
- [ ] File → Save As… on a WAV document: format row offers WAV and FLAC; choosing FLAC narrows
  the bit-depth choices to 16/24; the saved FLAC opens back correctly in another player.
- [ ] Open an MP3, then File → Save (Ctrl+S): it opens the Save As dialog with WAV 24-bit and
  `<name>.wav` preselected instead of silently overwriting the MP3.
- [ ] Raise a document's peak above 0 dBFS (e.g. via a gain effect once available, or open an
  already-hot file) and Save as 16-bit: the clip prompt shows the correct count and peak in dBFS;
  "Clip and save", "Save as 32-bit float instead" and "Cancel" each do what they say.
- [ ] Ctrl+O / Ctrl+S / Ctrl+Shift+S keyboard shortcuts still work as before.
