# H-109 — A selection drag released outside the waveform never ends (owner-reported)

- **Tier:** Haiku/Sonnet (small, and the cause is already known)
- **Owner's words:** "When selecting the audio waveform to cut or delete part of the recording, if my mouse leaves the wave area and reaches the rack area or marker area and I click there, it does not finish the selection — it stays following my mouse. It only works if I release inside the waveform."

## Cause (confirmed by reading the code)
`WaveformView.svelte` calls `setPointerCapture` **only for marker drags** (H-64, ~line 1685). The
plain selection drag and the selection-handle drag do not capture the pointer. So when the button is
released over another panel, **that panel receives the `pointerup`**, the waveform never learns the
drag ended, `dragging` stays true, and every later `pointermove` over the waveform keeps extending
the selection.

- **Read first:** CLAUDE.md, MEMORY.md (T-206's selection model, H-57/H-64's marker drag and its capture, H-66's primary-button rule), `ui/src/lib/waveform/WaveformView.svelte` (`onPointerDown`, `onPointerUp`, the handle-drag path).

## Scope (in)
1. Capture the pointer for **every** drag the waveform starts — plain selection and handle drags, not just markers — and release it on `pointerup`/`pointercancel`/`lostpointercapture`.
2. Releasing anywhere, including over another panel or outside the window, ends the drag and **keeps the selection exactly where it was at release**.
3. While dragging past the waveform's edge, the selection should follow the pointer's position clamped to the document — the same auto-scroll behaviour H-64 gave marker drags, if it fits naturally.
4. Keep H-66's rule: only the primary button drives a drag.

## Tests
Drag started in the waveform and released over another element ends the drag with the selection
fixed; subsequent pointer movement does not change it; handle drags behave the same; the marker
drag's existing capture is unaffected.

`just check` must pass.
