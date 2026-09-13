# T-306 — Sidecar `.vo.json`, identity check, sidecar-only saves, recent files, second-instance warning

- **Milestone / wave:** M3 / W3
- **Tier:** Sonnet (+ Opus review)
- **Depends on:** T-302, T-303, T-201 (save pipeline), T-103 (RackModel + placeholders)
- **Spec refs:** SPEC-018 (all ACs), SPEC-009 (open precedence sidecar vs WAV cue), SPEC-005 (save/Save As), SPEC-004 (dirty flag, recovery), SPEC-012 AC-13 (placeholder preservation) · **ADR refs:** ADR-004 Amendment 2 (saved-record fields, opaque sidecar sections), ADR-005 §10 (slot schema) · MEMORY A-005

## Goal
Everything the user sets up around a file — rack, markers, noise profile, view — comes back exactly when
the file is reopened, and PowerVoice never writes to user files except on an explicit Save.

## Scope
**In:**
- Sidecar schema v1 (`org.powervoice.sidecar`, `version`, `compat_version`), deterministic writer, unknown fields preserved, in-memory migration hook; file name = full audio name + `.vo.json`.
- Identity: sample rate + length + CRC-32 of samples (combined from chunk CRCs on open; computed over written values on save); mismatch → ignore with a notice and `.bak` copy.
- Save/Save As integration: atomic sidecar write alongside the audio (T-201 job), **sidecar-only saves** when only rack/view/FLAC markers changed, `sidecar_dirty` by content, journal `saved` record fields (`audio_crc32`, `sidecar`), changed-on-disk confirmation, up-front refusal when the sidecar can't be replaced, `.bak` rule.
- Open integration: read sidecar → restore rack through the registry (placeholders for unknown modules), markers with the SPEC-009 precedence vs WAV cue, view state, per-document spectral settings; no usable sidecar → keep the current rack.
- Recent files: settings `recent_files` (10, de-duplicated, most recent first), File → Open Recent, missing files greyed with Remove, Clear.
- Second-instance warning via the session lock (Open Anyway / Cancel).

**Out:** crash recovery (T-301), export (M6).

## Acceptance tests to write
- [ ] SPEC-018 AC-1…AC-19 per its test plan (bit-exact rack state via `f64::to_bits` + offline render hash; placeholder JSON-equal; SIGKILL at 20 points during save; unknown-field preservation; mismatch → `.bak`; recent-files ordering/dedup/cap; migration test with a synthetic schema step).

## Definition of Done
- [ ] Tests first, passing; `just check` green; Opus review passed; report in CLAUDE.md format.
