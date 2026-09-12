# CLAUDE.md — VoxEdit agent rules

VoxEdit is a Tauri 2 + Rust + Svelte 5 mono voice-over editor (Audition alternative). Development is
spec-driven and orchestrated: one orchestrator session dispatches tickets to subagents.

## Read order (every agent, every ticket)
1. This file.
2. Your ticket: `tickets/T-NNN-*.md`.
3. The specs it references (`specs/`) and ADRs (`docs/adr/`).
4. `PROMPT.md` — always §2 (LOCKED decisions) plus the sections your ticket names.
5. `MEMORY.md` — conventions, gotchas, risks.
6. `docs/references.md` when you need standards or reference numbers.

## Hard rules
- **Never edit** `PROMPT.md`, `MEMORY.md`, `tickets/BOARD.md`, or other tickets. Put notes for them in your final report.
- **Stay in ticket scope.** If a spec is ambiguous, contradictory or wrong, stop and report — don't invent behavior.
- **Tests first**: turn the spec's acceptance criteria into tests before implementing.
- **`just check` must pass** before you report done. Paste the tail of its output in your report.
- **No new dependencies** unless the ticket or an ADR names them. Propose others in your report.
- **Worktrees**: commit freely on your branch; the orchestrator squash-merges into `main` as `T-NNN: <summary>`.
- Don't install system packages, change global config, or touch files outside the repo.

## Real-time rules (`engine`, `rack`, `modules`, `dsp`, `module-api`)
- Audio callbacks and `process()`: **no allocation, no locks, no I/O, no logging, no syscalls, no panics on valid input, no unbounded loops.**
- UI↔audio communication through `rtrb` SPSC queues and atomics only. Drop heavy objects off the audio thread (`basedrop` or a worker thread).
- Enable flush-to-zero/denormals-are-zero on audio threads; DSP must still be denormal-safe.
- Tests run `process()` under `assert_no_alloc` (debug/test builds).
- The audio thread never reads the memory-mapped chunk store — a reader thread prefetches into a ring.

## Code conventions
- Rust stable, edition 2024. `unsafe` only with a `// SAFETY:` comment. Clippy warnings are errors.
- Units in identifiers: `_db`, `_dbfs`, `_lufs`, `_ms`, `_hz`, `_samples`. Audio samples `f32`; time positions `u64` samples; accumulators `f64`.
- Libraries use `thiserror`; only binaries (`cli`, `src-tauri`) use `anyhow`.
- `src-tauri` is a thin command/event layer — no business logic.
- UI: Svelte 5 runes, TypeScript strict. Every user-facing string goes through i18n keys (`ui/src/lib/i18n/`). Types shared with Rust are generated, never hand-copied.
- IPC bulk data (peaks, spectrogram tiles, audio) is binary (`ipc::Response` / `ipc::Channel`), never JSON float arrays.

## Commands
| Command | What |
|---|---|
| `just check` | fmt check + clippy (-D warnings) + all Rust tests + svelte-check + vitest |
| `just test` | tests only |
| `just dev` | run the app in dev mode |
| `just build` | release build |
| `just bench` | benchmarks |
| `just fixtures` | generate test fixtures into `fixtures/generated/` (not committed) |
| `just check-cross` | `cargo check` non-Tauri crates for Windows |

## Report format (your final message)
1. Summary of changes (files/crates).
2. Tests added and `just check` result (tail of output).
3. Deviations from ticket/spec, with reasons.
4. Notes for MEMORY.md (gotchas, conventions discovered).
5. Open questions.
