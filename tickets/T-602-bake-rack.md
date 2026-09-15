# T-602 — Bake rack (offline render into the document)

- **Tier:** Opus (engine plus document history; blocking findings only)
- **Depends on:** M5 (done), T-803/T-804 (sandboxed plugins render offline: `SandboxFault::OfflineDeadline` aborts the render with an error, ADR-008 §5).
- **Read first:**
  - CLAUDE.md (RT rules), MEMORY.md (T-103 crossfade/latency notes, T-803 async loading, S4-04/H-08 export rendering via `EngineHandle::rack_model()`);
  - specs/SPEC-004 (the Bake rack history entry, OD-4, AC-16);
  - specs/SPEC-000 §realtime/offline render, specs/SPEC-012 (offline rendering, latency, tail), specs/SPEC-008 (edit ops, including selection-scoped ops);
  - the export and loudness job services in `src-tauri` (the same render path).

## Goal
"Apply rack to audio": the owner renders the current effects chain into the document so the processed audio becomes the new take. By default the rack is then reset. The whole thing is one undo step that restores both the audio and the pre-bake rack.

## Scope (in)
1. **Offline render of the document (or the selection)** through the live rack model:
   - pre-roll so stateful modules settle;
   - trim the reported latency;
   - render the tail where the spec allows it (at the document end);
   - bit-exact with export's render path for the same rack.

   It runs as a job service with progress, cancel and the busy rules, following the export/loudness pattern. While it runs, the spectral tiles use half the workers (SPEC-007).
2. **History:** one `Bake rack` entry carrying the rack attachment (slots, parameter values, bypass flags, state blobs).
   - Undo restores the audio hash and the pre-bake rack.
   - Redo re-applies the baked audio and the reset (SPEC-004 AC-16).
   - After the bake the rack is reset, following the OD-4 default.
3. **Failure:** if any slot fails or a plugin crashes, the render aborts with an error and the document is unchanged.
4. **UI:**
   - Effects → "Apply Rack to Audio" (or the spec's exact name), on the selection if there is one, otherwise the whole document.
   - A confirmation that follows the spec, a progress dialog built on the H-26 `Dialog` actions with Cancel, and a toast when it's done.
   - Disabled while recording, or when the rack is empty or fully bypassed.

## Tests
- A bake equals export's render, bit-exact, including latency and tail.
- A selection-only bake leaves the outside audio bit-exact.
- AC-16: undo and redo restore the audio and the rack.
- Cancel leaves no change.
- A plugin crash aborts the bake with the document unchanged (uses the test CLAP crash mode).
- The busy rules.
- UI: the menu state, and the dialog flow.

`just check` must pass.
