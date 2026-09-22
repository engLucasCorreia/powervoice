# H-124 — Audit every effect that starts async work for H-122's restart bug

- **Tier:** Sonnet
- **Found by H-122:** a Svelte 5 `$effect` tracks every `$state` read made synchronously inside an async function it calls, up to the first `await`. The live-peaks poll read the 60 Hz record head there, so each telemetry frame re-ran the effect and its cleanup discarded the reply in flight — the waveform never drew while recording (broken since H-07, invisible to instant mocks). Other effects that poll, fetch, subscribe or start jobs may have the same shape.

- **Read first:** CLAUDE.md, MEMORY.md (H-121/H-122 entry), `git show` the H-122 commit, `ui/src/lib/waveform/WaveformView.livePoll.test.ts`.

## Scope (in)
1. Find every `$effect` / `$effect.pre` in `ui/src` that calls an async function or starts a timer/subscription, and whose cleanup cancels, disposes or clears something. List them all in your report with a verdict each.
2. For each one that reads frequently-changing state (telemetry, playhead, meters, job progress, anything updated per frame or per event) before its first `await` — or in its synchronous setup — without meaning to depend on it: fix with `untrack` (or restructure), and add a test with a realistic (slower than one frame) mocked round trip that proves replies are not discarded.
3. Make the preview/vitest IPC mocks for polled or streaming commands answer with a realistic delay where that would have hidden this class of bug, without slowing the suite noticeably.

## Tests
One per fixed effect, as above. `just check` must pass.
