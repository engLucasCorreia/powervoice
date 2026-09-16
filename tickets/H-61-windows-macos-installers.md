# H-61 — Windows and macOS installers from CI (T-705 remainder)

- **Tier:** Sonnet (CI and packaging)
- **From:** T-705's row — "done (Win/mac installers documented, not built)". Nobody has ever built or run either.
- **Read first:** CLAUDE.md, MEMORY.md (H-45 packaging scripts `scripts/packaging/{build_sandbox.sh,tauri_build.sh,check_bundle.py}` and the release workflow; H-38/H-49 runtime dependencies; T-806/T-807/T-808 platform gating — LV2 and JSFX are unix-only, plugin windows are X11-only), `.github/workflows/release.yml`, `docs/building.md`.

## Scope (in)
1. Extend the release workflow with **windows-latest** and **macos-latest** jobs that build the app and its bundles (MSI/NSIS on Windows, .dmg/.app on macOS), staging `powervoice-sandbox` the same way the Linux job does (the staging script must work on those hosts, or gain a documented per-OS branch).
2. Mark both jobs `continue-on-error` at first if they prove flaky, so a failure there never blocks the Linux release, and say so in the workflow comments.
3. Extend `check_bundle.py` (or add a sibling) so each OS's bundle is verified to contain the sandbox binary beside the app.
4. Attach whatever builds successfully to the release, clearly labelled **unverified — never run by the maintainer**, and say the same in the README download section.
5. No code signing or notarization (out of scope; note it in the docs).

## Tests
- The workflows are valid; the packaging script's per-OS logic is unit-tested where it's pure.
- A real CI run on a branch shows what each OS produces; report the result honestly, including failures.

`just check` must pass.
