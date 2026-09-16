#!/usr/bin/env python3
"""Fails if two tracked paths would collide on a case-insensitive filesystem (H-69).

PowerVoice is developed on Linux (case-sensitive ext4) but CI also builds on Windows (NTFS) and
macOS (APFS), both case-insensitive by default. H-61 found the release workflow's `windows` and
`macos` jobs both failing at `npm run build`: `ui/src/lib/menu/menubar.svelte.ts` (a Svelte 5
rune module) sat next to `ui/src/lib/menu/MenuBar.svelte` (a component), differing only by case,
and likewise `ui/src/lib/help/shortcutsDialog.svelte.ts` next to `ui/src/lib/help/ShortcutsDialog.svelte`.

That pair of paths doesn't even look identical once case is ignored, because a `*.svelte.ts` rune
module is always imported by a specifier that drops only the trailing `.ts`
(`import ... from "./menubar.svelte"`, never `"./menubar.svelte.ts"`) -- see any `*.svelte.ts`
file in `ui/src`. On a case-sensitive filesystem that bare specifier doesn't match any real file,
so the bundler's module resolution falls through to trying it with each configured extension
appended and finds `menubar.svelte.ts`. On a case-insensitive filesystem the bare specifier
`menubar.svelte` matches `MenuBar.svelte` directly (case-insensitively) before resolution ever
gets to append an extension, so the bundler silently resolves to the wrong file and every named
export the component doesn't have becomes a `MISSING_EXPORT` build error.

So this check compares every tracked path's own name (case-insensitively) *and*, for any `.ts`
file that isn't a `.d.ts` declaration, the name with the trailing `.ts` stripped -- exactly the
set of specifiers that could resolve to that file -- against every other tracked file's same set.
A plain "do any two full paths differ only by case" check would miss the H-61 pair entirely (the
paths differ by more than case: one ends in `.svelte`, the other in `.svelte.ts`); this check
catches the real collision instead of a narrower one that happens to have a simpler description.

Usage:
    python3 scripts/check_case_collisions.py

Run directly (not part of `just check`'s other checks' timing): `just check` shells out to this
after the UI checks. No stdlib-external dependency; `git ls-files` is the only subprocess call.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]


def tracked_paths(root: Path) -> list[str]:
    """Every path git tracks in `root`, repo-relative, forward-slash separated."""
    out = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        capture_output=True,
        check=True,
    ).stdout
    return [p for p in out.decode("utf-8").split("\0") if p]


def resolution_candidates(path: str) -> set[str]:
    """Every specifier that a JS/TS bundler could resolve to `path`.

    A path is always resolvable by itself. A `.ts` file that isn't a `.d.ts` declaration is also
    resolvable by the same path with the trailing `.ts` removed (bundlers try appending
    configured extensions -- `.ts` among them -- to an otherwise-unresolved specifier).
    """
    candidates = {path}
    if path.endswith(".ts") and not path.endswith(".d.ts"):
        candidates.add(path[: -len(".ts")])
    return candidates


def find_collisions(paths: list[str]) -> list[tuple[str, str, str]]:
    """Pairs of distinct tracked paths that share a resolution candidate once lowercased.

    Returns a sorted, de-duplicated list of (shared_candidate, path_a, path_b) triples.
    """
    by_lower: dict[str, dict[str, str]] = {}
    for path in paths:
        for candidate in resolution_candidates(path):
            by_lower.setdefault(candidate.lower(), {})[path] = candidate

    collisions: list[tuple[str, str, str]] = []
    seen_pairs: set[tuple[str, str]] = set()
    for lowered, sources in by_lower.items():
        if len(sources) < 2:
            continue
        names = sorted(sources)
        for i in range(len(names)):
            for j in range(i + 1, len(names)):
                pair = (names[i], names[j])
                if pair in seen_pairs:
                    continue
                seen_pairs.add(pair)
                collisions.append((lowered, names[i], names[j]))
    return sorted(collisions)


def main(argv: list[str] | None = None) -> int:
    del argv  # no options today
    paths = tracked_paths(REPO_ROOT)
    collisions = find_collisions(paths)
    if collisions:
        for lowered, a, b in collisions:
            print(
                f"case collision: {a!r} and {b!r} would both resolve to {lowered!r} on a "
                "case-insensitive filesystem (NTFS/APFS) -- rename one so it no longer collides",
                file=sys.stderr,
            )
        print(f"case collision check: {len(collisions)} problem(s)", file=sys.stderr)
        return 1
    print(f"case collision check: {len(paths)} tracked paths OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
