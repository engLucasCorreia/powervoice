default:
    @just --list

# Format check, lint, and run all tests
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    just check-types
    python3 scripts/notices/generate.py --check
    @if [ -f ui/package.json ]; then node scripts/docs/generate_shortcuts.mjs --check; fi
    @if [ -f ui/package.json ]; then npm --prefix ui run check; fi
    @if [ -f ui/package.json ]; then npm --prefix ui test -- --run; fi

# Run tests only
test:
    cargo test --workspace

# T-101 big-document tests (60-min fixture, SPEC-004 AC-5; real 4 GiB take rollover). Release build,
# ~5 GB scratch under target/big-tests. Uses fixtures/generated/long-60min-48k-mono.wav if present.
# H-03: first the full SPEC-017 true-peak limiter matrix (AC-3/AC-4, 44.1/48/96 kHz × ceilings ×
# input gains × look-ahead/release, multi-threaded) and the 60 s AC-16 timing run.
# H-17: SPEC-004 AC-3 (undo/redo on 20 000 pieces, ≤ 50 ms) and AC-11 (60-min/1000-record
# recovery, < 5 s) on a real disk under target/big-tests.
test-big:
    cargo test --release -p vox-modules --test true_peak_limiter -- --ignored --nocapture --test-threads=1
    mkdir -p target/big-tests
    POWERVOICE_TEST_TMP="{{justfile_directory()}}/target/big-tests" \
        cargo test --release -p vox-project --test big -- --ignored --nocapture --test-threads=1
    POWERVOICE_TEST_TMP="{{justfile_directory()}}/target/big-tests" \
        cargo test --release -p vox-project --test history_exact -- --ignored --nocapture --test-threads=1
    POWERVOICE_TEST_TMP="{{justfile_directory()}}/target/big-tests" \
        cargo test --release -p vox-project --test recovery -- --ignored --nocapture --test-threads=1
    POWERVOICE_TEST_TMP="{{justfile_directory()}}/target/big-tests" \
        cargo test --release -p vox-project --test sidecar_perf -- --ignored --nocapture --test-threads=1

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

# Regenerate THIRD_PARTY_NOTICES (+ its ui/ copy for Help -> About) from cargo metadata and
# ui/package.json (T-705, ADR-007). Run after any dependency change and commit the result;
# `just check` fails if it's stale.
notices:
    python3 scripts/notices/generate.py

# Regenerate docs/shortcuts.md from ui/src/lib/shortcuts/registry.ts (T-705, rehomed to its own
# file and checked by `just check` at T-701). Run after any registry/label change and commit the
# result.
shortcuts-table:
    node scripts/docs/generate_shortcuts.mjs

# Build release binary (+ the Linux Tauri bundle: AppImage + .deb, T-705)
build:
    cargo build --workspace --release
    npm --prefix ui run tauri build
    python3 scripts/packaging/check_desktop_entry.py

# Run benchmarks (T-110): `cargo bench --workspace`, then write target/bench/summary.md (each
# metric against its PROMPT/SPEC target where one exists — not committed, T-704's baseline).
# Never runs as part of `just check`.
bench:
    python3 scripts/bench/summary.py

# T-110 (ADR-002 §2 follow-up): the callback-time histogram — drives the real output callback
# path (FakeBackend + ManualEngine) with a typical voice rack for N seconds per realtime
# sub-block size, reporting p50/p95/p99/max against the block deadline.
# VOX_ENGINE_BENCH_SECONDS overrides the per-row duration (default 3 s).
bench-callback:
    cargo bench -p vox-engine --bench callback_histogram

# Generate test fixtures into fixtures/generated/ (gitignored)
fixtures:
    cargo run --release -p powervoice-cli --bin gen-fixtures

# Run app in dev mode. On Linux, defaults WEBKIT_DISABLE_DMABUF_RENDERER=1 (ADR-009 Amendment 1 /
# MEMORY D-016) unless it's already set, or POWERVOICE_WEBKIT_DMABUF=1 opts out and keeps the
# default WebKit DMA-BUF renderer (see ui/README.md). `powervoice-app` itself applies the same
# rule at startup (src-tauri/src/webkit.rs) — this mirrors it for `npm run dev`'s own Vite/Tauri
# CLI process, which starts before the Rust binary does.
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname -s)" = "Linux" ] && [ -z "${WEBKIT_DISABLE_DMABUF_RENDERER:-}" ] && [ "${POWERVOICE_WEBKIT_DMABUF:-0}" != "1" ]; then
        export WEBKIT_DISABLE_DMABUF_RENDERER=1
    fi
    # T-802: the plugin sandbox binary lives next to powervoice-app (POWERVOICE_DEV_PLUGINS=1
    # shows the sandboxed test plugins in the Add-module menu).
    cargo build -p powervoice-sandbox
    npm --prefix ui run tauri dev

# T-007 platform spike (ADR-009): builds powervoice-app with the `spike` cargo feature and launches
# it. POWERVOICE_SPIKE=1 (default here) makes the spike view auto-run its measurement suite and
# write results to bench-results/spike-<timestamp>.json; set POWERVOICE_SPIKE_EXIT=1 beforehand to
# also close the window once results are written (used for automated/scripted runs). Without
# POWERVOICE_SPIKE_EXIT the window stays open for the owner's manual input checks (ADR-009). Same
# Linux WEBKIT_DISABLE_DMABUF_RENDERER default/opt-out as `just dev` (see above); set
# POWERVOICE_WEBKIT_DMABUF=1 beforehand to force the default WebKit DMA-BUF renderer instead.
spike:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ "$(uname -s)" = "Linux" ] && [ -z "${WEBKIT_DISABLE_DMABUF_RENDERER:-}" ] && [ "${POWERVOICE_WEBKIT_DMABUF:-0}" != "1" ]; then
        export WEBKIT_DISABLE_DMABUF_RENDERER=1
    fi
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
    cargo check -p vox-sandbox-ipc --target x86_64-pc-windows-gnu
    cargo check -p vox-plugin-host --target x86_64-pc-windows-gnu
    cargo check -p powervoice-sandbox --target x86_64-pc-windows-gnu
    cargo check -p vox-clap-abi --target x86_64-pc-windows-gnu
    cargo check -p vox-test-clap --target x86_64-pc-windows-gnu
    cargo check -p vox-test-vst3 --target x86_64-pc-windows-gnu
    cargo check -p vox-lv2-abi --target x86_64-pc-windows-gnu
    cargo check -p vox-test-lv2 --target x86_64-pc-windows-gnu
    cargo check -p vox-ysfx-sys --target x86_64-pc-windows-gnu

# Build the roadmap dashboard (target/roadmap/index.html) from the board, git log and agent transcripts
roadmap:
    python3 scripts/roadmap/build.py

# Setup git hooks
hooks:
    git config core.hooksPath .githooks
