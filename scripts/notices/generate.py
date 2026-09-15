#!/usr/bin/env python3
"""Generates `THIRD_PARTY_NOTICES` from the actual dependency tree (T-705).

No new tool dependency (ADR-007 proposed `cargo-about`/`cargo-deny`/an npm license checker but none
are added): this reads `cargo metadata` (already part of `cargo`) for the Rust side and
`ui/package.json` + the installed `ui/node_modules/*/package.json` files for the JS side.

Usage:
    python3 scripts/notices/generate.py            # (re)write THIRD_PARTY_NOTICES
    python3 scripts/notices/generate.py --check     # fail if the committed file is stale

Every run asserts full coverage first (every direct Cargo dependency of every workspace crate,
in any dependency kind, and every `dependencies`/`devDependencies` key in `ui/package.json`, is
named somewhere in the generated text) — that assertion *is* the ticket's "a script test that the
notices file lists every direct dependency" test, run from `just check` via `--check`.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
OUTPUT = REPO_ROOT / "THIRD_PARTY_NOTICES"
UI_DIR = REPO_ROOT / "ui"
# A copy inside `ui/src` so the About dialog can `import ... from "./thirdPartyNotices.generated.txt?raw"`
# (Vite's dev server restricts serving files outside the project root, so the root copy above can't
# be imported directly without loosening that; a second copy is simpler and keeps both usable).
UI_COPY = UI_DIR / "src" / "lib" / "help" / "thirdPartyNotices.generated.txt"

HEADER = """\
PowerVoice — Third-Party Notices
=================================

PowerVoice itself is licensed under MIT OR Apache-2.0 (see LICENSE-MIT / LICENSE-APACHE at the
repository root). It's built with the open-source components listed below. This file is generated
by `scripts/notices/generate.py` (`just notices`) from `cargo metadata` and
`ui/package.json`/`ui/node_modules/*/package.json` — do not hand-edit it, regenerate it instead.
It is shown in the app under Help -> About -> Third-party notices.

See docs/adr/ADR-007-licensing.md for the linking policy and the reasoning behind the flagged
items below.

Flagged components (ADR-007)
-----------------------------

- **libmp3lame (LAME), LGPL-2.0-or-later.** Never bundled or statically linked. PowerVoice loads
  `libmp3lame.so.0` / `libmp3lame.dll` / `libmp3lame.dylib` at *runtime* via `libloading` (ISC) —
  see `docs/building.md` and `docs/user-guide.md` for where to install it on each OS. Without it,
  MP3 export is disabled with an explanatory message; every other feature works. Source:
  https://lame.sourceforge.io/ (packaged e.g. as `lame`/`libmp3lame0` on Linux distributions).
- **symphonia, MPL-2.0** (file-level copyleft). Used unmodified from crates.io, statically linked
  into `vox-io`. Source for the exact version in use: https://github.com/pdeljanov/Symphonia (see
  the pinned version below). We have not modified any symphonia file; if we ever do, that file
  must be published under MPL-2.0 (ADR-007 §3).
- **CLAP headers, MIT** (Copyright (c) 2021 Alexandre BIQUE). The crate `vox-clap-abi` transcribes
  the CLAP 1.2 type definitions from https://github.com/free-audio/clap; the full MIT license text
  is in `crates/clap-abi/LICENSE-CLAP`. Third-party CLAP plugins are not part of PowerVoice: they
  are loaded at runtime, only inside the separate `powervoice-sandbox` process (ADR-008), through
  `libloading` (ISC).
- **VST 3 SDK interfaces, MIT** (Copyright (c) 2025, Steinberg Media Technologies GmbH). The
  `vst3` crate's bindings (coupler-rs, MIT OR Apache-2.0) are generated from the VST 3 SDK 3.8.0
  `pluginterfaces` headers; the full MIT license text is in `crates/sandbox/LICENSE-VST3-SDK`.
  Only the separate `powervoice-sandbox` process links them (ADR-008); third-party VST3 plugins
  are loaded at runtime there only. VST is a trademark of Steinberg Media Technologies GmbH;
  PowerVoice hosts VST3 plugins and uses no VST logo.
- **System libraries** (dynamically linked, provided by the OS, never bundled except where noted):
  WebKitGTK / GTK3 (Linux), ALSA `libasound.so.2` (Linux, LGPL-2.1-or-later), PipeWire / JACK
  client libraries (Linux, MIT / LGPL respectively), WebView2 (Windows, Microsoft), CoreAudio /
  WebKit.framework (macOS, Apple). A Linux AppImage may bundle a copy of some of these as separate
  files inside the image; when it does, their own license texts and source pointers travel with
  the AppImage rather than being statically linked into the PowerVoice binary.

Rust dependencies
------------------

Every package in the Cargo dependency graph (`cargo metadata`), across all workspace crates,
excluding PowerVoice's own crates. Kind is one of `normal` (compiled into every build, including
the shipped binary), `build` (used by a build script only) or `dev` (tests/benches only — never
shipped). A package can appear with more than one kind if different crates depend on it
differently.

"""

NPM_HEADER = """

JavaScript dependencies (ui/)
-------------------------------

Shipped in the built UI bundle (`ui/package.json` "dependencies"):

"""

NPM_DEV_HEADER = """

Build-time only, not shipped (`ui/package.json` "devDependencies" — Vite, Vitest, svelte-check,
TypeScript, the Tauri CLI, etc.):

"""

FOOTER = """

Regenerating
-------------

Run `just notices` (wraps `python3 scripts/notices/generate.py`) after any dependency change, and
commit the result. `just check` fails if this file is stale.
"""


def run_cargo_metadata() -> dict:
    proc = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(proc.stdout)


def license_text(pkg: dict) -> str:
    lic = pkg.get("license")
    if lic:
        return lic
    license_file = pkg.get("license_file")
    if license_file:
        return f"(see bundled {license_file})"
    return "UNKNOWN — needs manual review"


def collect_rust_section(metadata: dict) -> tuple[str, set[str]]:
    """Returns (rendered section text, set of direct-dependency crate names)."""
    packages = {p["id"]: p for p in metadata["packages"]}
    workspace_members = set(metadata["workspace_members"])
    nodes = {n["id"]: n for n in metadata["resolve"]["nodes"]}

    direct_names: set[str] = set()
    kinds_by_id: dict[str, set[str]] = {}
    for member_id in workspace_members:
        node = nodes.get(member_id)
        if node is None:
            continue
        for dep in node["deps"]:
            dep_pkg = packages.get(dep["pkg"])
            if dep_pkg is None or dep["pkg"] in workspace_members:
                continue
            direct_names.add(dep_pkg["name"])
            kinds = kinds_by_id.setdefault(dep["pkg"], set())
            for dk in dep["dep_kinds"]:
                kinds.add(dk["kind"] or "normal")

    third_party = [p for p in packages.values() if p["id"] not in workspace_members]
    third_party.sort(key=lambda p: (p["name"].lower(), p["version"]))

    lines = []
    seen: set[tuple[str, str]] = set()
    for pkg in third_party:
        key = (pkg["name"], pkg["version"])
        if key in seen:
            continue
        seen.add(key)
        kinds = sorted(kinds_by_id.get(pkg["id"], {"transitive"}))
        lines.append(f"- {pkg['name']} {pkg['version']} — {license_text(pkg)} [{', '.join(kinds)}]")

    return "\n".join(lines) + "\n", direct_names


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def npm_license(pkg_name: str) -> str:
    pkg_dir = UI_DIR / "node_modules" / pkg_name
    pkg_json = pkg_dir / "package.json"
    if not pkg_json.exists():
        return "UNKNOWN — run `npm ci --prefix ui` first"
    data = read_json(pkg_json)
    lic = data.get("license")
    if isinstance(lic, dict):
        lic = lic.get("type", "UNKNOWN")
    version = data.get("version", "?")
    return f"{version} — {lic or 'UNKNOWN — needs manual review'}"


def collect_npm_section(deps: dict[str, str]) -> str:
    lines = []
    for name in sorted(deps, key=str.lower):
        lines.append(f"- {name} {npm_license(name)}")
    return "\n".join(lines) + "\n"


def generate() -> str:
    metadata = run_cargo_metadata()
    rust_section, direct_cargo_names = collect_rust_section(metadata)

    package_json = read_json(UI_DIR / "package.json")
    runtime_deps: dict[str, str] = package_json.get("dependencies", {})
    dev_deps: dict[str, str] = package_json.get("devDependencies", {})

    text = (
        HEADER
        + rust_section
        + NPM_HEADER
        + collect_npm_section(runtime_deps)
        + NPM_DEV_HEADER
        + collect_npm_section(dev_deps)
        + FOOTER
    )

    missing = sorted(
        name
        for name in {*direct_cargo_names, *runtime_deps, *dev_deps}
        if name not in text
    )
    if missing:
        raise SystemExit(
            "generate_notices: these direct dependencies are missing from the generated "
            f"text: {missing}"
        )

    return text


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check",
        action="store_true",
        help="don't write the file; fail if it would change (staleness check for `just check`)",
    )
    args = parser.parse_args()

    text = generate()

    if args.check:
        stale = []
        for path in (OUTPUT, UI_COPY):
            current = path.read_text(encoding="utf-8") if path.exists() else ""
            if current != text:
                stale.append(path)
        if stale:
            names = ", ".join(str(p.relative_to(REPO_ROOT)) for p in stale)
            print(
                f"stale: {names} — run `just notices` and commit the result.",
                file=sys.stderr,
            )
            return 1
        print(f"THIRD_PARTY_NOTICES is up to date ({len(text.splitlines())} lines).")
        return 0

    UI_COPY.parent.mkdir(parents=True, exist_ok=True)
    OUTPUT.write_text(text, encoding="utf-8")
    UI_COPY.write_text(text, encoding="utf-8")
    print(f"wrote {OUTPUT} and {UI_COPY} ({len(text.splitlines())} lines).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
