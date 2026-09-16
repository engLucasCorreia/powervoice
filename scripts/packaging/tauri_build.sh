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

# H-61: ui/package.json's "tauri" script is `TAURI_APP_PATH=../src-tauri tauri` (a POSIX inline env
# assignment). npm normally runs package.json scripts through the OS's default shell — cmd.exe on
# Windows — which can't parse that syntax and fails with "'TAURI_APP_PATH' is not recognized...".
# This script itself already requires bash (its shebang, and the Windows CI job runs it with
# `shell: bash`, i.e. Git for Windows' bash.exe), so just tell npm to run *its* scripts through
# that same bash instead of cmd.exe — harmless on Linux/macOS, where bash is already npm's default.
if [ "${OS:-}" = "Windows_NT" ]; then
    export npm_config_script_shell="$(command -v bash)"
fi

npm --prefix ui run tauri build -- \
    --config '{"bundle":{"externalBin":["binaries/powervoice-sandbox"]}}' \
    "$@"
