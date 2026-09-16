default:
    @just --list

# Format check, lint, and run all tests
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    just check-types
    python3 scripts/packaging/test_check_bundle.py
    python3 scripts/packaging/test_check_bundle_windows.py
    python3 scripts/notices/generate.py --check
    python3 scripts/docs/test_check.py
    python3 scripts/docs/check.py
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
# T-704: + the 60-min import (cold/warm page cache), SPEC-007 AC-9 (spectrogram tile latency on a
# 60-min document) and the real open path (`powervoice-app` `perf_big`: open → first overview,
# memory with the document open). Everything is tee'd into target/bench/big.log, whose
# BENCH_RESULT lines are merged into target/bench/summary.md (and docs/performance.md via
# `just perf-matrix`).
test-big:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target/big-tests target/bench
    log=target/bench/big.log
    : > "$log"
    run() { "$@" 2>&1 | tee -a "$log"; }
    run cargo test --release -p vox-modules --test true_peak_limiter -- --ignored --nocapture --test-threads=1
    export POWERVOICE_TEST_TMP="{{justfile_directory()}}/target/big-tests"
    for t in big history_exact recovery sidecar_perf; do
        run cargo test --release -p vox-project --test "$t" -- --ignored --nocapture --test-threads=1
    done
    run cargo test --release -p vox-engine --test spectro ac9 -- --ignored --nocapture --test-threads=1
    run cargo test --release -p powervoice-app --lib perf_big -- --ignored --nocapture --test-threads=1
    python3 scripts/bench/summary.py --no-run

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

# T-706: regenerate the generated sections of the architecture docs (crate graph from `cargo
# metadata`, IPC command/event tables from src-tauri/src/ipc) and check links, anchors and Mermaid
# fences. `just check` runs the check without --write and fails when a section is stale.
docs:
    python3 scripts/docs/check.py --write

# Build release binary (+ the Linux Tauri bundle: AppImage + .deb, T-705)
build:
    cargo build --workspace --release
    scripts/packaging/build_sandbox.sh
    scripts/packaging/tauri_build.sh
    python3 scripts/packaging/check_desktop_entry.py
    python3 scripts/packaging/check_bundle.py

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

# T-704: the headless UI frame-time sweep (PROMPT §2 "60 fps scroll/zoom", SPEC-006 AC-18, SPEC-007
# AC-10) — its own Vite server on port 5193 + headless Chromium over CDP, the preview App with a
# 60-min document, zoom/scroll sweeps at 1280×720 and 2126×850. BENCH_RESULT lines go to
# target/bench/ui.log and are merged into target/bench/summary.md. Needs `chromium` on PATH.
bench-ui:
    node scripts/bench/ui_frames.mjs
    python3 scripts/bench/summary.py --no-run

# T-704: rewrite the targets matrix in docs/performance.md from the BENCH_RESULT lines of the last
# `just bench`, `just test-big` and `just bench-ui` runs (target/bench/{raw,big,ui}.log). Commit the
# result when the numbers are worth recording.
perf-matrix:
    python3 scripts/bench/matrix.py

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

# T-805 (ADR-006 §3): build a module package crate — e.g. `just voxmod voxmod-gain` — as a release
# `.clap` and zip it with the crate's `voxmod.json`, the licenses and checksums into
# target/voxmod/<id>-<version>.voxmod ("Install module…" accepts it). Linux and Windows binaries;
# macOS `.clap` bundles aren't assembled by this recipe yet. H-44: any `crates/<crate>/presets/
# *.vopreset.json` and `crates/<crate>/locales/*.json` the crate ships are packed too (factory
# presets and translated strings — both optional; a crate with neither packs exactly as before).
voxmod crate:
    #!/usr/bin/env bash
    set -euo pipefail
    pkg="$(sed -n 's/^name = "\(.*\)"$/\1/p' crates/{{crate}}/Cargo.toml | head -n1)"
    if [ -z "$pkg" ] || [ ! -f crates/{{crate}}/voxmod.json ]; then
        echo "crates/{{crate}} isn't a module package crate (needs Cargo.toml and voxmod.json)" >&2
        exit 1
    fi
    cargo build --release -p "$pkg"
    case "$(uname -s)" in
        Linux*) lib="lib${pkg//-/_}.so" ;;
        Darwin*) echo "macOS .clap bundles aren't built by this recipe yet" >&2; exit 1 ;;
        *) lib="${pkg//-/_}.dll" ;;
    esac
    preset_args=()
    for f in crates/{{crate}}/presets/*.vopreset.json; do
        [ -e "$f" ] && preset_args+=(--preset "$f")
    done
    locale_args=()
    for f in crates/{{crate}}/locales/*.json; do
        [ -e "$f" ] && locale_args+=(--locale "$f")
    done
    cargo run --release -q -p powervoice-cli --bin voxmod -- \
        --manifest crates/{{crate}}/voxmod.json \
        --binary "target/release/$lib" \
        --license LICENSE-MIT --license LICENSE-APACHE --license THIRD_PARTY_NOTICES \
        "${preset_args[@]}" "${locale_args[@]}" \
        --out target/voxmod

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
    cargo check -p vox-module-clap --target x86_64-pc-windows-gnu
    cargo check -p vox-voxmod-gain --target x86_64-pc-windows-gnu

# Build the roadmap dashboard (target/roadmap/index.html) from the board, git log and agent transcripts
roadmap:
    python3 scripts/roadmap/build.py

# Setup git hooks
hooks:
    git config core.hooksPath .githooks
