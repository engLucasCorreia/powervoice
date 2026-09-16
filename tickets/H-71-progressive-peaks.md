# H-71 — Progressive waveform while a long import runs

- **Tier:** Sonnet (no review loop; blocking findings only)
- **From:** H-60, H-20 and SPEC-005 §2.3 / SPEC-006 AC-13 — flagged three times. Opening a 60-minute file takes ~1.7 s and the waveform appears all at once at the end; the spec wants it to fill in progressively. VXPK's PARTIAL bit and its decoder already exist; nothing ever emits them.
- **Read first:** CLAUDE.md, MEMORY.md (T-704's import work — decode runs on its own thread, the vectorised pyramid; H-43's frame scheduler — call `invalidate()` when new peaks arrive, never add a loop; H-47's upload budget; ADR-003 and H-59's Amendment 6 for the IPC contract; T-706's note that `just docs` regenerates the IPC tables), specs/SPEC-005 §2.3, SPEC-006 §2.3 and AC-13, ADR-004 §5, `crates/project/src/import.rs` and `peaks_query.rs`, the import job in `src-tauri`, `ui/src/lib/waveform/*`.

## Scope (in)
1. Emit partial peaks as the import proceeds — through the existing `job_progress{kind: Import}` channel plus VXPK's PARTIAL bit, or whatever ADR-003 Amendment 6 says is the right shape. Don't invent a second mechanism.
2. The waveform draws what exists so far and fills in, without flicker and without a busy loop: the renderer redraws on arrival via the frame scheduler.
3. The document stays unusable-for-editing until the import completes, exactly as it is today, unless the spec says otherwise — this is display only.
4. Update the IPC docs (`just docs`) if you add a command or event.

## Tests
- Partial frames arrive in order and the final state equals a non-progressive import (bit-exact peaks).
- The renderer draws partial data and stops animating when the import ends.
- SPEC-006 AC-13.

`just check` must pass; report the perceived improvement on a 60-minute file (`just bench-ui` or a timed trace).
