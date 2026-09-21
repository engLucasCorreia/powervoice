# H-114 — The waveform lags ~10 s behind while recording (owner-reported)

- **Tier:** Sonnet. **User-facing and important:** a recorder that looks idle while it records makes people stop and restart takes.
- **Owner's words:** "When I am recording, the waveform drawing is very late, maybe 10 seconds or something like that, which makes me feel no recording is being done. In Audition this is almost live, and in Audacity too. We need to draw the waveform live, in a smooth way, so we have a good UX."

## What the code says — the data path should be fast
- The backend keeps live min/max peaks at **256 samples per bucket** (`LIVE_PEAKS_SPB`, ≈ 5 ms at 48 kHz) — `crates/engine/src/record.rs`, `capture.rs`.
- The waveform polls `record_peaks_get` every **100 ms** (`LIVE_POLL_MS`) — `WaveformView.svelte` ~line 168.
- So nothing in the *granularity* explains a 10-second delay. It comes from somewhere else. Leads, to be tested rather than assumed:
  1. **`LIVE_MIN_WINDOW_SECONDS = 10`** (~line 166) matches the owner's figure too closely to ignore. The live view zooms to fit "at least 10 s" — check whether anything is withheld, or drawn too small to see, until the take passes that length.
  2. **The capture-writer's drain cadence**: if it appends to the live peaks only when a large buffer fills, `len_samples` advances in big steps however often the UI polls.
  3. **Frame invalidation**: H-83 changed the viewport store so no-op writes no longer notify. If the live-zoom path relied on those writes to trigger a redraw, peaks can arrive without a frame being drawn (H-43's draw-on-demand scheduler only draws when invalidated).
  4. The recording may be into an **empty Untitled document**, which has its own zoom-to-fit path (H-76 found a similar bug for imports).

- **Read first:** CLAUDE.md, MEMORY.md (H-07's live take drawing, H-10, H-21, H-23, H-43's frame scheduler, H-68's recording harness, H-76, H-83), specs/SPEC-002 (what it requires of the live waveform's latency), `WaveformView.svelte`'s live-take path, `crates/engine/src/capture.rs`, `scripts/bench/ui_frames.mjs` (its `scene=recording` pass already drives a live capture).

## Scope (in)
1. **Measure first.** Time from a sample being captured to it being drawn, at the start of a take and later on. State the number before and after. "Feels faster" is not a result.
2. **Find the cause** and fix it — the owner's bar is Audition/Audacity: the waveform should appear to grow in step with the voice, within a frame or two of real time.
3. **Smooth:** the growing take should animate at display rate, not advance in visible jumps every poll. Consider interpolating the record head between polls, and poll or stream as fast as the display needs.
4. The early seconds of a take must be **visible at a sensible size** — whatever the minimum window, the first word should be clearly drawn the moment it is spoken.
5. Keep H-43's rule: draw on demand, no perpetual loop when nothing is recording; and keep recording's CPU cost in check (H-68's budget still applies).

## Tests
The latency measurement as a repeatable check (in `just bench-ui` if it is too slow for `just check`), and a unit test for whatever the root cause turns out to be.

## Verification
The owner judges this one by feel, so describe exactly how you verified it looks live — and capture a short sequence of frames from the real app or the recording preview showing the take growing.

`just check` must pass.
