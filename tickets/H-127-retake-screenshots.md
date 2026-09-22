# H-127 — Retake the four README screenshots (they show a pre-v0.4.1 app)

- **Tier:** Sonnet (needs the real app running, so it needs a moment when the owner is not using it)
- **Found by H-126.** `docs/images/*.png` have no staleness check and all four are now wrong:
  - `powervoice-light-spectral.png`, `powervoice-tour.png`: the Meters dock shows one short horizontal "Output level" bar — no vertical input meter, no Fast/Medium/Slow Speed row (H-112/H-123).
  - `powervoice-dark.png`, `powervoice-high-contrast.png`: narrow rack column, an EQ graph without any of H-111's readouts, and a waveform selection as a flat blue-gray wash instead of the translucent tint (H-121).
  - None shows a selection in the spectral pane, so none demonstrates the tint fix.

## Scope (in)
Retake all four in the real built app (`just build`, run the AppImage — **ask the orchestrator first**, the owner uses this machine), same window size and themes as the originals, showing: both vertical meters with the Speed row, a widened rack with an EQ node tooltip or the right-click menu open, and a translucent selection in both the waveform and the spectral pane. Keep the file names and update the README captions only if they no longer describe the picture.

## Tests
`just check` (the docs checks cover image links). No fabricated or edited images — capture what the app really draws.
