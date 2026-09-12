# T-001 — Bootstrap: workspace, crate skeletons, lints, justfile, hooks

- **Milestone / wave:** M0 / W1
- **Tier:** Haiku (no Opus review)
- **Depends on:** — (the orchestrator has already installed the Rust stable toolchain and `just`)
- **PROMPT refs:** §2, §4 (repo layout)

## Goal
A compiling Cargo workspace with every v1 crate as an empty skeleton, shared lints, and a `justfile`, so every later ticket can run `just check`.

## Scope
**In:**
- `rust-toolchain.toml`: channel `stable`, components `rustfmt`, `clippy`; targets `x86_64-pc-windows-gnu` (for `check-cross`).
- Root `Cargo.toml`: `[workspace]` with `resolver = "3"`, members `crates/*`; `[workspace.package]` (edition 2024, version 0.1.0, `publish = false`); `[workspace.lints.rust]` (`unsafe_op_in_unsafe_fn = "deny"`, `unused_must_use = "deny"`); `[workspace.lints.clippy]` (`all = "warn"`, plus `cast_lossless`, `float_cmp`, `dbg_macro`, `todo` = "warn"). Every crate uses `[lints] workspace = true`.
- `[workspace.dependencies]` left empty except what the skeletons need (e.g. `clap` with `derive` for the CLI).
- Library skeletons (each with `src/lib.rs`, a one-line crate doc comment, and one trivial `#[test]`): `crates/module-api`, `crates/dsp`, `crates/modules`, `crates/rack`, `crates/engine`, `crates/io`, `crates/project`, `crates/testkit`.
- Binary skeleton `crates/cli` → bin name `powervoice-cli` using `clap` derive; `--version` works; subcommands `gen`, `analyze`, `render`, `bench` exist and print "not implemented yet".
- `rustfmt.toml` (`max_width = 100`), `.editorconfig`.
- `justfile` recipes:
  - `check`: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`; then, **only if `ui/package.json` exists**, `npm --prefix ui run check` and `npm --prefix ui test -- --run`.
  - `test`, `build` (`cargo build --workspace --release`), `bench` (`cargo bench --workspace`), `fixtures` (placeholder echo, T-006 fills it), `dev` (placeholder echo, T-004 fills it).
  - `check-cross`: `cargo check --workspace --exclude powervoice-app --target x86_64-pc-windows-gnu` (the exclude must not fail when the crate doesn't exist yet — just list the non-Tauri crates explicitly with `-p`).
- `.githooks/pre-commit` running `cargo fmt --all --check` only (fast); `just hooks` recipe runs `git config core.hooksPath .githooks`.

**Out:** any real functionality, `src-tauri`/`ui` (T-004), dependencies beyond `clap`.

## Acceptance tests
- [ ] `just check` passes.
- [ ] `just check-cross` passes (install the target with `rustup target add x86_64-pc-windows-gnu` if missing — that is a user-level rustup action and allowed).
- [ ] `cargo run -q -p cli -- --version` (or the crate's package name) prints `powervoice-cli 0.1.0`.

## Definition of Done
- [ ] All above green; report in CLAUDE.md format with the `just check` tail.
