# S1-01 — Playback end-to-end (Slice 1)

- **Tier:** Opus (one Opus review, blocking findings only)
- **Depends on:** T-101 (vox-project), T-102 (vox-engine devices), T-103 (vox-rack) — all merged
- **Specs (essential subset only):** SPEC-003 §2.1 transport (Play/Pause, Stop → play-start position, Play from start, Return to start, seek), §2.2 playhead contract, §2.3 start latency; SPEC-001 §2.1–§2.2 device choice + fallback, §2.4 device loss = stop + notice; SPEC-012 (empty rack / Gain in the path). ADR-002 (threads, RT rules), ADR-003 (VXTM, commands/events).
- **Read first:** CLAUDE.md, MEMORY.md (D-022 vertical slices; ticket learnings T-101/T-102/T-103), tickets/T-105-engine-core.md "Handoff requirements" (they apply here).

## Goal
The app can play a document through the chosen output device with a working transport, a moving
playhead and an output meter. This is the audio backbone the rest of Slice 1 plugs into.

## Scope (in)
- `vox-engine`: `Engine` facade owned by the app (control thread with 16 ms tick; command queue UI→engine; RT event ring; return ring). Output stream on the configured or default output device (PipeWire default) via T-102; output callback = reader ring → `LiveRack::process` → device, with ~5 ms fades on start/stop/seek, epochs, FTZ/DAZ guard (`vox-dsp`). Reader/prefetch thread over a `vox-project` `DocSnapshot` (`SnapshotReader`, ~200 ms). If document rate ≠ device rate, resample in the reader with `rubato` (in `vox-dsp`).
- Transport state machine: play, pause (keep), stop (return to play start), play_from_start (selection start or 0 — selection can be a stub field for now), return_to_start, seek. Heard position = rendered − rack latency − output latency using the callbacks' `now_ns`.
- Telemetry: `VXTM` frames at 60 Hz over a Tauri `Channel` (playhead sample + time, playing flag, output peak/RMS max-hold). Device loss / stall → stop + `notice`.
- Engine holds the current `Session`/snapshot handle with a simple API the next tickets extend: `set_document(snapshot)` (used by S1-03 open and S1-04 record).
- `src-tauri`: engine in managed state; commands `devices_list`, `devices_select` (output; input selection is stored for S1-04), `transport_play|pause|stop|play_from_start|return_to_start|seek`, `telemetry_subscribe`, `clock_now_ns`; `transport_state` + `devices_changed` events; DTOs via ts-rs; map T-104's `DevicePrefsDto` ↔ engine `DevicePrefs`.
- UI (minimal, dark theme, i18n): transport bar (buttons + time readout) wired to the keymap (Space, Shift+Space, Home); output peak/RMS meter; Audio Devices dialog (host/output/input/rate/buffer dropdowns from `devices_list`); playhead extrapolation helper (SPEC-003 §2.2) exported for the waveform view.

## Out (deferred to hardening — list them in your report)
Loop playback, playhead follow (needs the waveform, S1-03), SPEC-001 full banner/state table, xrun counters UI, bench harness, advanced telemetry fields.

## Tests
- Fake-backend integration: play a known snapshot through `[Gain 0 dB]` → device output equals the source (≤ 1e-6 after alignment); Stop returns the heard position to the play start; Pause keeps it; start latency < 50 ms from command to first non-silent sub-block; the output callback allocates nothing (`test_util::no_alloc`).
- Rate mismatch: 44.1 kHz document on a 48 kHz fake device → 1 kHz tone stays 1 kHz ± 0.05 %.
- Vitest: transport buttons/keys call the right commands (mockIPC); `VXTM` decoder round-trips a Rust-generated fixture.
- Manual: `just dev` launches, Audio Devices lists the owner's PipeWire devices (describe what you verified; you can't hear audio).

## Definition of Done
`just check` green; report in CLAUDE.md format incl. the deferred list.
