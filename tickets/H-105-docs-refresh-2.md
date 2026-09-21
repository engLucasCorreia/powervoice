# H-105 — Bring the documentation back up to date (everything since H-78)

- **Tier:** Sonnet (writing plus verification)
- **Why:** H-78 refreshed the docs on 2026-09-16. Since then the app gained a whole feature and a dozen behaviour changes, so the docs are stale again — and stale documentation is worse than none, because it is believed.
- **Read first:** CLAUDE.md, MEMORY.md's ticket learnings from H-79 onward (each entry says what changed), `docs/README.md`, `docs/user-guide.md`, `docs/how-it-works.md`, `docs/what-is-powervoice.md`, `docs/faq.md`, `docs/glossary.md`, `docs/architecture/*`, `docs/building.md`, `README.md`, `HANDOFF.md`, and `scripts/docs/check.py` (links, anchors, Mermaid and generated sections are enforced).

## What changed and must be documented
- **Explain My Voice** (H-91…H-104) — the whole feature: what it measures, what the annotations mean, that the curve is a long-term average over a section rather than an instant, what "unresolved harmonics" means, and why it refuses to recommend flattening a voice. This deserves its own section of the user guide, written for someone who is not an audio engineer.
- **Voice diagnostics** — pitch statistics are now octave-guarded, and a shaky estimate is marked as one (H-91, H-97).
- **The advice the app gives** — it no longer suggests boosting air, and a threshold crossed by a hair reads mild (H-99, SPEC-007 Amendment 2).
- **Editing and playback** — the selection's own colour (H-79), loop over the whole file with no selection (H-80), loop-off fixed (H-81), editing gated during an import (H-82).
- **Exporting** — jobs report their state reliably now, and the export no longer appears to hang (H-96); `job_status` exists as a fallback.
- **Platform reality** — the AppImage no longer bundles `libpipewire` or `libwayland-client`, and why that mattered (H-89, H-90); the release verification drill.
- **Architecture** — the `rack_response_curve_preview` command and what it exists for (H-101), the shared `JobStatusRegistry` (H-96), the GUI repro harness (H-98). Run `just docs` so the generated crate graph and IPC tables are current.

## Also
Every UI name must match `ui/src/lib/i18n/en.json` exactly — check, don't assume. Update the test-count claim in README.md. Where something is platform-limited or unverified, say so plainly.

## Tests
`scripts/docs/check.py` passes; `just check` must pass. In your report list every file you touched and, per feature, how you verified it against the code rather than the ticket text.
