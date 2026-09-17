# H-88 — A shared way to test UI that reacts to live analyzer frames

- **Tier:** Sonnet (test infrastructure; small)
- **From:** H-87's open question. Several small canvas graphs subscribe to a live analyzer `Channel<ArrayBuffer>` and derive UI from the frames. There is no established way to push a decoded `VXSA` frame into a mounted component, so those paths are only tested in their *absence* — H-87's hover readout can assert "Live −∞ dB" but not a real live level, and `spectrumFeed.test.ts` only manages it because that module exposes `handleMessage` directly.
- **Read first:** CLAUDE.md, MEMORY.md (H-87's note about this gap; H-42's analyzer work; H-59's VXPK golden-fixture pattern; H-84's `eq/spectrumFeed.ts`, which was deliberately written to be testable without the Channel), `ui/src/lib/ipc/`'s Channel plumbing, `ui/src/lib/eq/spectrumFeed.test.ts` and its `VXSA_FIXTURE_HEX`, `ui/src/lib/test/`.

## Scope (in)
1. A helper in `ui/src/lib/test/` that delivers a `VXSA` frame (built from real fixture bytes, not a hand-rolled object) to a mounted component's own analyzer subscription, so tests can assert on what the UI actually shows.
2. Adopt it in at least the two places that wanted it: `NoiseProfileGraph`'s hover readout live level, and the EQ graph's spectrum overlay.
3. If a component can't be reached without changing it, prefer the smallest testability seam that doesn't distort production code — and say what you chose.

## Tests
The helper itself, plus the two adoptions asserting a real live value rather than its absence.

`just check` must pass.
