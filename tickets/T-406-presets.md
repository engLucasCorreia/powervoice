# T-406 — Module & rack presets

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** S3-01 (rack panel), T-306 (sidecar rack state / module state blobs), H-19 (menu bar), H-03/S3-06 (module state incl. noise print).
- **Read first:** CLAUDE.md, MEMORY.md (S3-01, S3-06, T-306, H-19 notes; module ids `org.powervoice.*`; DTO literal gotcha; pre-commit `cargo fmt --all --check`), ADR-005 (module state, `save_state`/`load_state`, ids), SPEC-012 (rack) and the module specs' preset sections (SPEC-013/014/015/016/017), crates/rack (RackModel/SlotModel serialization), crates/engine/src/rack_api.rs (`rack_load_model`), src-tauri/src/settings.rs (app config dir), ui/src/lib/rack/*, ui/src/lib/menu/*.

## Goal
The owner saves a tuned effect or a whole chain ("Podcast voice": HP 80 Hz → gate → compressor → EQ → TP limiter −1 dBTP) and recalls it on any recording in one click. Factory presets give sensible starting points.

## Scope (in)
- **Module presets:** per module id, save/load/rename/delete named presets of the module's parameters + state blob (noise print optional: "include noise print" checkbox, off by default); a preset menu in each rack slot header; "Reset to default".
- **Rack presets:** save the whole chain (modules, order, bypass, parameters, states) as a named rack preset; Effects → Rack Presets ▸ load / save / manage. Loading replaces the live rack via `rack_load_model` (one undoable rack change if rack changes are in history; otherwise a confirm when the current rack differs from the last saved state).
- **Storage:** versioned JSON files in the app config dir (`presets/modules/<module-id>/<name>.json`, `presets/racks/<name>.json`), atomic writes, unknown fields preserved, unknown module ids in a rack preset load as placeholder slots with a notice.
- **Factory presets** (read-only, shipped in the binary): at least "Podcast voice", "Audiobook (ACX)", "Gentle cleanup" rack presets and 2–3 per module where the specs suggest values.

## Tests
Round trip module/rack presets (params + state bit-exact), factory presets load and pass `ModuleTestHost` validation, unknown module id → placeholder + notice, name sanitisation (no path traversal), atomic write survives a simulated crash (temp file left → ignored).

## Out
Preset sharing/import-export UI beyond the files on disk, cloud sync.

`just check` must pass.
