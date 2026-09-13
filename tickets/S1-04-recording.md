# S1-04 — Recording end-to-end (Slice 1)

- **Tier:** Opus (one Opus review for the capture/durability path, blocking findings only)
- **Depends on:** S1-01 (engine), T-101 (take writer), T-102 (input devices)
- **Specs (essential subset only):** SPEC-002 §2.1 arm + input meter (peak + 300 ms RMS) + clip latch, §2.2 new recording (48 kHz/24-bit default; capture 32f; take → one undoable "Record" edit), §2.3 crash-safety basics (take WAV via `TakeCapture` + background `TakeSyncHandle::sync()` ~1 s, drain ≥ every 50 ms), §2.7 monitoring **Off / Dry** only. SPEC-001 input device + input channel selection.
- **Read first:** CLAUDE.md, MEMORY.md (D-022; T-101/T-102 learnings incl. `commit_take` rules), tickets/T-106-recording.md "Handoff requirements".

## Goal
The owner picks a mic (and input channel), sees the input meter move, presses Record (Shift+R or the
button), speaks, presses Stop, and the take appears as the document — playable and savable.

## Scope (in)
- Engine: input stream on the selected input device/channel (deinterleave via T-102 `MonoInput`), arm/disarm, input meter values into `VXTM`, clip detection latch; capture ring → capture-writer thread → `TakeCapture` (WAV + chunks) with a background sync thread; start/stop alignment from capture `now_ns`; on Stop: `finish()` → `commit_take` → new snapshot → `Engine::set_document`; recording into an empty/new document only (New Recording replaces the current document after the unsaved-changes prompt); monitoring Off/Dry (dry = input added after the rack at unity, audible only while armed/recording; simple ring, no drift servo yet).
- Live waveform while recording: refresh peaks for the growing take at ~10 Hz via `peaks_get` (the `VXRP` channel is hardening).
- `src-tauri` + UI: input device/channel in the Audio Devices dialog, Arm toggle, Record button + Shift+R, recording time readout, input meter with clip latch, monitoring Off/Dry selector.
- Tests: fake-backend recording of a known source → take equals source (bit-exact, ±1 sample alignment), undo removes the take, the capture callback allocates nothing; `kill -9` style test for the take WAV is already covered in `vox-project` — add one engine-level test that a take survives a simulated crash (child process) if cheap, else defer.

## Out (deferred to hardening)
Dropout detection/fill/markers, disk-space rules, capture-ring overflow policy, through-rack monitoring + drift servo + latency readout (T-107), device-loss-during-recording notices beyond "stop and keep", recording at cursor / punch-in (T-304), VXRP channel.

## Definition of Done
`just check` green; report in CLAUDE.md format incl. the deferred list.
