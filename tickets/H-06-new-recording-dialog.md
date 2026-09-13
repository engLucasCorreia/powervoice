# H-06 — New Recording dialog + capture resampling

- **Tier:** Sonnet (no review loop; blocking findings only)
- **Depends on:** S1-04 (recording), S4-02 (`vox_dsp::resample` offline resampler; a streaming variant may be needed), H-05 (if merged first, follow its findings).
- **Read first:** CLAUDE.md (RT rules), MEMORY.md (S1-04, H-07 notes), specs/SPEC-002-recording.md (new recording format, rate handling), crates/engine/src/{record.rs,capture.rs}, src-tauri/src/recording.rs, ui/src/lib/record/*, ui/src/lib/state/record.svelte.ts.

## Goal
The owner picks the new recording's sample rate and bit depth, and recording works even when the input device runs at a different rate.

## Scope (in)
- New Recording dialog (File → New Recording…): sample rate (44.1/48/96 kHz), save bit depth (16/24/32f), remembered in settings as the default format; Record with no document uses the default without the dialog.
- Capture-writer resampling: when the input stream rate ≠ document rate, resample on the capture-writer thread (never the RT callback) with a streaming resampler; latency accounted in the take start.
- Clear notice when the device rate changes mid-session.

## Tests
- Dialog stores the default format; Record uses it.
- 48 kHz input into a 44.1 kHz document: a 1 kHz tone keeps its frequency ±0.05 % and level ±0.1 dB, take length matches wall time ±1 block.
- RT input callback still no-alloc.

## Out
Multi-channel recording, punch-in (T-304).

`just check` must pass.
