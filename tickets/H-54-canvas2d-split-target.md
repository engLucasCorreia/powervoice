# H-54 — The last failing performance target: Canvas2D split view

- **Tier:** Sonnet (renderer plus a possible spec amendment)
- **The row:** `frame_spectral_canvas2d_2126x850_p95_ms` = 21.2 ms against SPEC-007 AC-10's ≤ 16.7 ms — the **software** (Canvas2D) renderer, waveform and spectral shown together, at a 2126×850 window, during a continuous zoom and scroll sweep. Every other row passes; the WebGL2 path passes everywhere.
- **Read first:** CLAUDE.md, MEMORY.md (H-47's Canvas2D work — the bit-exact `CODE_DB` table, "never rewrite the division as `* (1/255)`", the upload budget, and the hop-independent preview templates that make the bench meaningful; H-43's frame scheduler; T-704/T-110 bench tooling, `just bench-ui`, `docs/performance.md`, the `BENCH_RESULT` convention), specs/SPEC-007 (AC-10 and its wording about renderers), `ui/src/lib/spectrogram/*`, `ui/src/lib/waveform/*`.

## Scope (in)
1. **Profile first** (CDP, the T-704/H-47 way) and report where the 21.2 ms actually goes at that size; H-47 already took `columnDb` from 66 % to a table lookup, so say what dominates now.
2. **Either** optimise until the row passes (ideas, not a mandate: draw the spectral pane at a lower internal resolution and upscale, cache the column image between frames when only the playhead moves, skip work for panes that didn't change, coalesce the two panes' work), **or**, if the remaining cost is inherent to software rendering at that size, amend SPEC-007 AC-10 to scope the 60 fps target per renderer and window size — with the measured numbers in the amendment, and the fallback still required to stay usable (say what "usable" is, e.g. ≥ 30 fps).
3. Whatever you choose, the WebGL2 path must not regress, and idle CPU (H-43) must stay at its ~0.01 % level.

## Tests
- Before/after `just bench-ui` numbers for both renderers at 1280×720 and 2126×850, into `docs/performance.md` via the usual `BENCH_RESULT` rows.
- Any pure logic you add is unit-tested; existing renderer tests stay green.

`just check` must pass.
