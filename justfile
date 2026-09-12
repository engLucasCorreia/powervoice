default:
    @just --list

# Format check, lint, and run all tests
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    just check-types
    @if [ -f ui/package.json ]; then npm --prefix ui run check; fi
    @if [ -f ui/package.json ]; then npm --prefix ui test -- --run; fi

# Run tests only
test:
    cargo test --workspace

# Regenerate Rust -> TS shared types (ADR-003) into ui/src/lib/ipc/bindings.ts
gen-types:
    TS_RS_EXPORT_DIR="{{justfile_directory()}}/ui/src/lib/ipc" TS_RS_LARGE_INT=number \
        cargo test -p powervoice-app export_bindings

# Fail if ui/src/lib/ipc/bindings.ts is stale relative to the Rust DTOs (ADR-003)
check-types:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    TS_RS_EXPORT_DIR="$tmp" TS_RS_LARGE_INT=number \
        cargo test -p powervoice-app export_bindings >/dev/null
    stale=0
    for generated in "$tmp"/*; do
        name="$(basename "$generated")"
        checked_in="ui/src/lib/ipc/$name"
        if [ ! -f "$checked_in" ] || ! diff -u "$checked_in" "$generated"; then
            stale=1
        fi
    done
    if [ "$stale" -ne 0 ]; then
        echo "generated types are stale — run \`just gen-types\`" >&2
        exit 1
    fi

# Build release binary (+ the Linux Tauri bundle)
build:
    cargo build --workspace --release
    npm --prefix ui run tauri build

# Run benchmarks
bench:
    cargo bench --workspace

# Generate test fixtures into fixtures/generated/ (gitignored)
fixtures:
    cargo run --release -p powervoice-cli --bin gen-fixtures

# Run app in dev mode. Set POWERVOICE_WEBKIT_SAFE=1 to work around slow/broken WebKitGTK GPU
# compositing on some Linux setups (see ui/README.md).
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "${POWERVOICE_WEBKIT_SAFE:-0}" = "1" ]; then
        export WEBKIT_DISABLE_DMABUF_RENDERER=1
    fi
    npm --prefix ui run tauri dev

# T-007 platform spike (ADR-009): builds powervoice-app with the `spike` cargo feature and launches
# it. POWERVOICE_SPIKE=1 (default here) makes the spike view auto-run its measurement suite and
# write results to bench-results/spike-<timestamp>.json; set POWERVOICE_SPIKE_EXIT=1 beforehand to
# also close the window once results are written (used for automated/scripted runs). Without
# POWERVOICE_SPIKE_EXIT the window stays open for the owner's manual input checks (ADR-009). Set
# WEBKIT_DISABLE_DMABUF_RENDERER=1 beforehand to run that configuration.
spike:
    #!/usr/bin/env bash
    set -euo pipefail
    export POWERVOICE_SPIKE="${POWERVOICE_SPIKE:-1}"
    npm --prefix ui run tauri dev -- --features spike

# Check for Windows cross-compilation
check-cross:
    cargo check -p vox-module-api --target x86_64-pc-windows-gnu
    cargo check -p vox-dsp --target x86_64-pc-windows-gnu
    cargo check -p vox-modules --target x86_64-pc-windows-gnu
    cargo check -p vox-rack --target x86_64-pc-windows-gnu
    cargo check -p vox-engine --target x86_64-pc-windows-gnu
    cargo check -p vox-io --target x86_64-pc-windows-gnu
    cargo check -p vox-project --target x86_64-pc-windows-gnu
    cargo check -p vox-testkit --target x86_64-pc-windows-gnu
    cargo check -p powervoice-cli --target x86_64-pc-windows-gnu

# Setup git hooks
hooks:
    git config core.hooksPath .githooks
