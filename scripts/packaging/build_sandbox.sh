#!/usr/bin/env bash
# H-45: builds the `powervoice-sandbox` release binary and stages it where Tauri's `externalBin`
# expects it (`src-tauri/binaries/powervoice-sandbox-<host-triple>`), so a bundle can embed it next
# to `powervoice-app` (`vox_plugin_host::SandboxOptions::beside_current_exe`).
#
# Shared by `just build` (see `justfile`) and `.github/workflows/release.yml` — one source of
# truth for how the sandbox gets staged, so the two never drift.
#
# Doesn't touch `src-tauri/tauri.conf.json`: `externalBin` is passed on the `tauri build`
# command line instead (`--config '{"bundle":{"externalBin":["binaries/powervoice-sandbox"]}}'`),
# so plain `cargo build`, `just check` and `just dev` keep working even when this script hasn't
# run yet (tauri-build only validates `externalBin` paths that are actually declared).

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

cargo build --release -p powervoice-sandbox

host_triple="$(rustc -vV | sed -n 's/^host: //p')"
if [ -z "$host_triple" ]; then
    echo "build_sandbox.sh: couldn't determine the host triple from 'rustc -vV'" >&2
    exit 1
fi

ext=""
case "$host_triple" in
    *windows*) ext=".exe" ;;
esac

mkdir -p src-tauri/binaries
cp "target/release/powervoice-sandbox${ext}" "src-tauri/binaries/powervoice-sandbox-${host_triple}${ext}"
echo "build_sandbox.sh: staged src-tauri/binaries/powervoice-sandbox-${host_triple}${ext}"
