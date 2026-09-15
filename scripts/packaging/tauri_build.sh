#!/usr/bin/env bash
# H-45: runs `tauri build`, declaring the sandbox as an external binary on the command line
# (rather than in the committed `src-tauri/tauri.conf.json`) so `cargo build`/`just check`/
# `just dev` keep working even when `src-tauri/binaries/powervoice-sandbox-<triple>` hasn't been
# staged yet — `tauri-build` validates `externalBin` paths at compile time, only when they're
# actually declared. Run `scripts/packaging/build_sandbox.sh` first.
#
# Shared by `just build` and `.github/workflows/release.yml` (one source of truth for the exact
# bundling command); pass extra `tauri build` flags (e.g. `--bundles appimage,deb`) as arguments.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

npm --prefix ui run tauri build -- \
    --config '{"bundle":{"externalBin":["binaries/powervoice-sandbox"]}}' \
    "$@"
