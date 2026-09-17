# H-83 — Svelte "updated at" ownership warning in the recording view

- **Tier:** Sonnet
- **From:** H-68. Running `?preview&scene=recording` in headless Chromium logs repeated dev-mode `console.error("updated at", …)` from Svelte's ownership check — `waveformView.svelte.ts`'s `startSample`/`samplesPerPixel` setters called from `EditorView.svelte`'s `$effect`. No other bench-ui scene surfaces it, and it doesn't affect correctness or frame times (every frame stayed ≤ 16.8 ms), but it is Svelte telling us a rune's state is being mutated from outside the component that owns it.
- **Read first:** CLAUDE.md, MEMORY.md (H-43's frame scheduler and draw-on-demand rule; H-68's harness and how to reproduce), `ui/src/lib/waveform/waveformView.svelte.ts`, `ui/src/lib/layout/EditorView.svelte`, Svelte 5's ownership rules for `$state` shared across modules.

## Scope (in)
1. Reproduce it (the H-68 harness is the quickest route: `scene=recording`).
2. Fix the ownership pattern properly — the state should be mutated by whoever owns it, through an explicit setter/action, not written into from another component's effect. Don't silence the warning.
3. Check whether the same pattern exists elsewhere in the UI and say what you find (fix only what this ticket's change naturally covers; ticket the rest).

## Tests
A test that fails on the warning (or on the ownership violation) before the fix and passes after; the existing waveform/editor suites stay green.

`just check` must pass.
