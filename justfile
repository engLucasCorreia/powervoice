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
        cargo test -p voxedit-app export_bindings

# Fail if ui/src/lib/ipc/bindings.ts is stale relative to the Rust DTOs (ADR-003)
check-types:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    TS_RS_EXPORT_DIR="$tmp" TS_RS_LARGE_INT=number \
        cargo test -p voxedit-app export_bindings >/dev/null
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
    cargo run --release -p voxedit-cli --bin gen-fixtures

# Run app in dev mode. Set VOXEDIT_WEBKIT_SAFE=1 to work around slow/broken WebKitGTK GPU
# compositing on some Linux setups (see ui/README.md).
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "${VOXEDIT_WEBKIT_SAFE:-0}" = "1" ]; then
        export WEBKIT_DISABLE_DMABUF_RENDERER=1
    fi
    npm --prefix ui run tauri dev

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
    cargo check -p voxedit-cli --target x86_64-pc-windows-gnu

# Setup git hooks
hooks:
    git config core.hooksPath .githooks
