# H-45 — Packaging: bundle the plugin sandbox locally + CI checks

- **Tier:** Sonnet (no review loop)
- **Found by:** the orchestrator while publishing v0.1.0.
  - The app finds the plugin sandbox beside its own executable (`vox_plugin_host::SandboxOptions::beside_current_exe`), but `src-tauri/tauri.conf.json` has no `externalBin`. So a local `just build` produces an AppImage/.deb **without** `powervoice-sandbox`, and third-party plugins can't load.
  - The GitHub release workflow (`.github/workflows/release.yml`) works around this by building the sandbox and passing `--config '{"bundle":{"externalBin":["binaries/powervoice-sandbox"]}}'`.
  - The v0.1.0 `.deb` was confirmed to contain `usr/bin/powervoice-sandbox`.
- **Read first:**
  - CLAUDE.md, MEMORY.md (T-705 packaging, T-802 sandbox, H-38 deb recommends, and the GitHub notes: CI needs PipeWire/SPA/JACK/libclang dev packages on ubuntu-24.04);
  - `justfile` (the `build` recipe), `src-tauri/tauri.conf.json`, `src-tauri/build.rs`, `.github/workflows/release.yml`, `docs/building.md`.

## Scope (in)
1. **Local bundles include the sandbox.** `just build` builds `powervoice-sandbox` (release), copies it to `src-tauri/binaries/powervoice-sandbox-<target-triple>` (the triple from `rustc -vV`), and bundles it as an external binary.
   - Plain `cargo build`, `just check` and `just dev` must keep working when the file isn't there. `tauri-build` validates `externalBin` paths, so either keep passing it via `--config` in the recipe (as the workflow does) or make the build step create it. Pick the simplest option that doesn't break dev.
   - Add `src-tauri/binaries/` to `.gitignore`.
   - The release workflow uses the same recipe or script, so there's one source of truth.
2. **Packaging check.** `scripts/packaging/check_bundle.py`, run at the end of `just build` next to the desktop-entry check, fails if the built `.deb` or AppImage lacks `powervoice-sandbox` beside `powervoice-app`.
3. **CI checks.** A `.github/workflows/ci.yml` runs on pushes and PRs to `main`, on ubuntu-24.04, with the same system packages as the release workflow plus lilv/lame if the tests need them. It runs `npm ci --prefix ui` and then `just check`, installing `just` with a pinned action or binary.
   - Tests that need real devices or displays already skip.
   - Cache cargo and npm.
   - If a test is flaky in CI, report it; don't weaken it.
4. **Docs.** Update `docs/building.md`: what `just build` produces, and that plugins need the bundled sandbox.

## Tests
- The `check_bundle.py` logic, unit-tested with a fake bundle tree.
- `just build` run once locally, if it fits the time budget, and the bundle check passes.
- The CI workflow YAML is valid. Lint it with `actionlint` if it's available; otherwise review it carefully.

`just check` must pass. Don't push; the orchestrator pushes.
