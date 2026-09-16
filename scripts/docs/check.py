#!/usr/bin/env python3
"""Documentation checks and generated sections (T-706). Python standard library only.

Usage:
    python3 scripts/docs/check.py            # check (run by `just check`)
    python3 scripts/docs/check.py --write    # regenerate the generated sections, then check

What it checks, over `README.md` and every Markdown file under `docs/`:

1. **Links.** Every relative link (`[text](path)`, `[text](path#anchor)`, `[text](#anchor)`,
   reference definitions, and `<a href>` / `<img src>`) points at a file or directory that exists;
   an anchor into a Markdown file matches one of its headings (GitHub's slug rules) or an explicit
   `<a id>`/`<a name>`. Links inside code blocks and inline code are ignored; so are URLs with a
   scheme (`https:`, `mailto:` ...).
2. **Mermaid.** Every ```` ```mermaid ```` fence is non-empty, starts with a known diagram type,
   has balanced brackets outside quoted strings, balanced `subgraph`/`end` (flowcharts) and
   `alt|opt|loop|par|critical|break|rect|box`/`end` (sequence diagrams) blocks, no `;` in sequence
   message text, and no unquoted flowchart label containing brackets. This is a sanity check that
   catches what usually breaks GitHub's renderer, not a full parser.
3. **Generated sections are current.** A generated section sits between
   `<!-- BEGIN GENERATED: <name> -->` and `<!-- END GENERATED: <name> -->`. The generators:
   - `crate-graph`: the workspace crate graph and crate table from
     `cargo metadata --format-version 1 --no-deps`;
   - `ipc-commands`: every Tauri command registered in `src-tauri/src/ipc/mod.rs` (production
     build, not the `spike` feature), with its handler file, transport and first doc sentence;
   - `ipc-events`: every event in `src-tauri/src/ipc/events.rs`'s `ipc_events!`, with the files
     that emit it.
   The sections listed in `REQUIRED_SECTIONS` must exist.
4. **The hub is complete.** Every Markdown file under `docs/` is linked from `docs/README.md`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

REPO_ROOT = Path(__file__).resolve().parents[2]

HUB = "docs/README.md"

# Where each generated section must exist (repo-relative doc path -> section names).
REQUIRED_SECTIONS: dict[str, list[str]] = {
    "docs/architecture/overview.md": ["crate-graph"],
    "docs/architecture/ipc.md": ["ipc-commands", "ipc-events"],
}

MERMAID_TYPES = (
    "flowchart",
    "graph",
    "sequenceDiagram",
    "classDiagram",
    "stateDiagram",
    "stateDiagram-v2",
    "erDiagram",
    "gantt",
    "pie",
    "journey",
    "gitGraph",
    "mindmap",
    "timeline",
    "quadrantChart",
    "requirementDiagram",
    "C4Context",
    "C4Container",
    "C4Component",
    "C4Dynamic",
    "C4Deployment",
    "sankey-beta",
    "xychart-beta",
    "block-beta",
    "architecture-beta",
)

SEQUENCE_BLOCK_OPENERS = ("alt", "opt", "loop", "par", "critical", "break", "rect", "box")

BEGIN_RE = re.compile(r"<!--\s*BEGIN GENERATED:\s*([\w-]+)\s*-->")
END_RE_TEMPLATE = r"<!--\s*END GENERATED:\s*{name}\s*-->"


@dataclass
class Problem:
    path: str
    line: int
    message: str

    def __str__(self) -> str:
        return f"{self.path}:{self.line}: {self.message}"


# --------------------------------------------------------------------------------------------
# Markdown helpers
# --------------------------------------------------------------------------------------------

FENCE_RE = re.compile(r"^\s{0,3}(`{3,}|~{3,})(.*)$")


def split_fences(text: str) -> tuple[list[str], list[tuple[int, str, list[str]]]]:
    """Returns (prose lines with fenced blocks blanked, [(start line no, info, body lines)]).

    Line numbers are 1-based and refer to the fence's opening line. Blanking keeps line numbers
    stable for error messages.
    """
    lines = text.split("\n")
    prose: list[str] = []
    blocks: list[tuple[int, str, list[str]]] = []
    i = 0
    while i < len(lines):
        m = FENCE_RE.match(lines[i])
        if not m:
            prose.append(lines[i])
            i += 1
            continue
        marker, info = m.group(1), m.group(2).strip()
        start = i + 1
        body: list[str] = []
        prose.append("")
        i += 1
        while i < len(lines):
            close = FENCE_RE.match(lines[i])
            if close and close.group(1)[0] == marker[0] and len(close.group(1)) >= len(marker) \
                    and not close.group(2).strip():
                prose.append("")
                i += 1
                break
            body.append(lines[i])
            prose.append("")
            i += 1
        blocks.append((start, info, body))
    return prose, blocks


INLINE_CODE_RE = re.compile(r"(`+)(.+?)\1")


def blank_inline_code(line: str) -> str:
    return INLINE_CODE_RE.sub(lambda m: " " * len(m.group(0)), line)


def strip_inline_markdown(text: str) -> str:
    """The rendered text of a heading, as far as slugging cares."""
    text = re.sub(r"!\[([^\]]*)\]\([^)]*\)", r"\1", text)  # images -> alt text
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)  # links -> text
    text = re.sub(r"<[^>]+>", "", text)  # inline HTML tags
    text = text.replace("`", "")
    text = re.sub(r"(\*\*|\*|~~)", "", text)
    return text


def github_slug(heading: str) -> str:
    """GitHub's heading anchor: lowercase, punctuation dropped, each space -> '-'."""
    text = strip_inline_markdown(heading).strip().lower()
    text = re.sub(r"[^\w\- ]", "", text)
    return text.replace(" ", "-")


HEADING_RE = re.compile(r"^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$")
EXPLICIT_ANCHOR_RE = re.compile(r"""<a\s+[^>]*?(?:id|name)\s*=\s*["']([^"']+)["']""", re.I)


def anchors_of(text: str) -> set[str]:
    prose, _ = split_fences(text)
    seen: dict[str, int] = {}
    anchors: set[str] = set()
    for line in prose:
        m = HEADING_RE.match(line)
        if m:
            slug = github_slug(m.group(2))
            if slug in seen:
                seen[slug] += 1
                anchors.add(f"{slug}-{seen[slug]}")
            else:
                seen[slug] = 0
                anchors.add(slug)
        for a in EXPLICIT_ANCHOR_RE.finditer(line):
            anchors.add(a.group(1))
    return anchors


INLINE_LINK_RE = re.compile(r"!?\[(?:[^\[\]]|\[[^\]]*\])*\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
REF_DEF_RE = re.compile(r"^\s{0,3}\[[^\]]+\]:\s*<?(\S+?)>?(?:\s+.*)?$")
HTML_LINK_RE = re.compile(r"""<(?:a|img)\s[^>]*?(?:href|src)\s*=\s*["']([^"']+)["']""", re.I)
SCHEME_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")


def links_of(text: str) -> list[tuple[int, str]]:
    prose, _ = split_fences(text)
    found: list[tuple[int, str]] = []
    for n, raw in enumerate(prose, start=1):
        line = blank_inline_code(raw)
        for rx in (INLINE_LINK_RE, HTML_LINK_RE):
            for m in rx.finditer(line):
                found.append((n, m.group(1)))
        m = REF_DEF_RE.match(line)
        if m:
            found.append((n, m.group(1)))
    return found


# --------------------------------------------------------------------------------------------
# Checks
# --------------------------------------------------------------------------------------------


def doc_files(root: Path) -> list[Path]:
    files = []
    readme = root / "README.md"
    if readme.exists():
        files.append(readme)
    files.extend(sorted((root / "docs").rglob("*.md")))
    return files


def rel(root: Path, path: Path) -> str:
    return path.relative_to(root).as_posix()


def check_links(root: Path, files: list[Path]) -> list[Problem]:
    problems: list[Problem] = []
    anchor_cache: dict[Path, set[str]] = {}

    def anchors(path: Path) -> set[str]:
        if path not in anchor_cache:
            anchor_cache[path] = anchors_of(path.read_text(encoding="utf-8"))
        return anchor_cache[path]

    for doc in files:
        text = doc.read_text(encoding="utf-8")
        for line, target in links_of(text):
            if SCHEME_RE.match(target) or target.startswith("//"):
                continue
            path_part, _, fragment = target.partition("#")
            path_part = path_part.split("?", 1)[0]
            if path_part:
                base = root if path_part.startswith("/") else doc.parent
                resolved = (base / path_part.lstrip("/")).resolve()
                try:
                    resolved.relative_to(root)
                except ValueError:
                    problems.append(Problem(rel(root, doc), line, f"link leaves the repository: {target}"))
                    continue
                if not resolved.exists():
                    problems.append(Problem(rel(root, doc), line, f"broken link: {target}"))
                    continue
            else:
                resolved = doc
            if fragment and resolved.is_file() and resolved.suffix == ".md":
                if fragment not in anchors(resolved):
                    problems.append(Problem(rel(root, doc), line, f"missing anchor: {target}"))
    return problems


def check_hub(root: Path) -> list[Problem]:
    hub = root / HUB
    if not hub.exists():
        return [Problem(HUB, 1, "the documentation hub is missing")]
    linked: set[Path] = set()
    for _, target in links_of(hub.read_text(encoding="utf-8")):
        if SCHEME_RE.match(target):
            continue
        path_part = target.partition("#")[0]
        if path_part:
            linked.add((hub.parent / path_part).resolve())
    problems = []
    for doc in sorted((root / "docs").rglob("*.md")):
        if doc.resolve() == hub.resolve():
            continue
        if doc.resolve() not in linked:
            problems.append(Problem(HUB, 1, f"{rel(root, doc)} is not linked from the hub"))
    return problems


def _strip_quoted(line: str) -> str:
    return re.sub(r'"[^"]*"', '""', line)


def _bracket_problems(lines: list[tuple[int, str]]) -> list[tuple[int, str]]:
    pairs = {")": "(", "]": "[", "}": "{"}
    stack: list[tuple[str, int]] = []
    out: list[tuple[int, str]] = []
    for n, line in lines:
        for ch in _strip_quoted(line):
            if ch in "([{":
                stack.append((ch, n))
            elif ch in pairs:
                if not stack or stack[-1][0] != pairs[ch]:
                    out.append((n, f"unbalanced '{ch}'"))
                    return out
                stack.pop()
    for ch, n in stack:
        out.append((n, f"unclosed '{ch}'"))
    return out


# An unquoted flowchart node label: `id[...]`, `id(...)`, `id{...}` and the double forms.
NODE_OPEN_RE = re.compile(r"(?<![\w\"])([A-Za-z_][\w-]*)\s*(\[\[|\[\(|\(\[|\(\(|\{\{|\[/|\[\\|\[|\(|\{)")
CLOSERS = {"[[": "]]", "[(": ")]", "([": "])", "((": "))", "{{": "}}", "[/": "/]", "[\\": "\\]",
           "[": "]", "(": ")", "{": "}"}


def _flowchart_label_problems(n: int, line: str) -> list[tuple[int, str]]:
    out = []
    pos = 0
    while True:
        m = NODE_OPEN_RE.search(line, pos)
        if not m:
            return out
        opener = m.group(2)
        start = m.end()
        rest = line[start:]
        if rest.lstrip().startswith('"'):
            # Quoted label: skip to the closing quote.
            q = line.find('"', start)
            q2 = line.find('"', q + 1)
            pos = (q2 + 1) if q2 != -1 else len(line)
            continue
        close = CLOSERS[opener]
        end = line.find(close, start)
        if end == -1:
            return out  # bracket balance reports it
        label = line[start:end]
        if any(c in label for c in "()[]{}\""):
            out.append((n, f"unquoted label with brackets: {m.group(0)}{label}{close} (quote it: id[\"...\"])"))
        pos = end + len(close)


def lint_mermaid(body: list[str], first_line: int = 1) -> list[tuple[int, str]]:
    """Returns [(line number, message)] for one Mermaid block (body = lines inside the fence)."""
    lines = [(first_line + i, ln) for i, ln in enumerate(body)]
    content = [(n, ln.strip()) for n, ln in lines if ln.strip() and not ln.strip().startswith("%%")]
    if not content:
        return [(first_line, "empty mermaid block")]
    head_n, head = content[0]
    kind = head.split()[0]
    if kind not in MERMAID_TYPES:
        return [(head_n, f"unknown diagram type '{kind}'")]
    out: list[tuple[int, str]] = []
    if kind in ("flowchart", "graph"):
        opened = sum(1 for _, ln in content if re.match(r"^subgraph\b", ln))
        closed = sum(1 for _, ln in content if ln == "end")
        if opened != closed:
            out.append((head_n, f"{opened} subgraph(s) but {closed} 'end' line(s)"))
        for n, ln in content[1:]:
            if re.search(r"(-->|---|-\.->|==>)\s*end\b|^end\s*(-->|---|-\.->|==>)", ln):
                out.append((n, "a node named 'end' breaks flowcharts"))
            out.extend(_flowchart_label_problems(n, ln))
        out.extend(_bracket_problems(content[1:]))
    elif kind == "sequenceDiagram":
        depth = 0
        for n, ln in content[1:]:
            word = ln.split()[0]
            if word in SEQUENCE_BLOCK_OPENERS:
                depth += 1
            elif ln == "end":
                depth -= 1
                if depth < 0:
                    out.append((n, "'end' without an open block"))
                    depth = 0
            if ":" in ln and ";" in ln.split(":", 1)[1]:
                out.append((n, "';' in sequence text ends the statement (use ',' or '#59;')"))
        if depth > 0:
            out.append((head_n, f"{depth} block(s) not closed with 'end'"))
        # Message and note text after ':' is free text; only check the structural part.
        out.extend(_bracket_problems([(n, ln.split(":", 1)[0]) for n, ln in content[1:]]))
    else:
        out.extend(_bracket_problems(content[1:]))
    return out


def check_mermaid(root: Path, files: list[Path]) -> tuple[list[Problem], int]:
    problems: list[Problem] = []
    count = 0
    for doc in files:
        _, blocks = split_fences(doc.read_text(encoding="utf-8"))
        for start, info, body in blocks:
            if info.split()[:1] != ["mermaid"]:
                continue
            count += 1
            for n, msg in lint_mermaid(body, start + 1):
                problems.append(Problem(rel(root, doc), n, f"mermaid: {msg}"))
    return problems, count


# --------------------------------------------------------------------------------------------
# Generated sections
# --------------------------------------------------------------------------------------------


def mask_code(text: str) -> str:
    """The text with code fences and inline code blanked, keeping every offset intact.

    Lets a document mention the `<!-- BEGIN GENERATED: … -->` markers inside code without the
    checker treating them as a real generated section.
    """
    lines = text.split("\n")
    out: list[str] = []
    fence: str | None = None
    for line in lines:
        m = FENCE_RE.match(line)
        if fence is None and m:
            fence = m.group(1)
            out.append(" " * len(line))
        elif fence is not None:
            out.append(" " * len(line))
            if m and m.group(1)[0] == fence[0] and len(m.group(1)) >= len(fence) and not m.group(2).strip():
                fence = None
        else:
            out.append(blank_inline_code(line))
    return "\n".join(out)


def find_sections(text: str) -> list[tuple[str, int, int]]:
    """[(name, start offset of the content, end offset of the content)]."""
    out = []
    text = mask_code(text)
    for m in BEGIN_RE.finditer(text):
        name = m.group(1)
        end = re.compile(END_RE_TEMPLATE.format(name=re.escape(name))).search(text, m.end())
        if not end:
            raise ValueError(f"generated section '{name}' has no END marker")
        out.append((name, m.end(), end.start()))
    return out


def replace_section(text: str, name: str, content: str) -> str:
    for sec_name, start, end in find_sections(text):
        if sec_name == name:
            return text[:start] + "\n" + content.rstrip("\n") + "\n" + text[end:]
    raise ValueError(f"no generated section '{name}'")


def relpath(target: Path, doc: Path) -> str:
    return Path(os.path.relpath(target, doc.parent)).as_posix()


def cargo_metadata(root: Path) -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps",
         "--manifest-path", str(root / "Cargo.toml")],
        check=True, capture_output=True, text=True,
    )
    return json.loads(out.stdout)


def node_id(name: str) -> str:
    return re.sub(r"\W", "_", name)


def gen_crate_graph(root: Path, doc: Path, metadata: dict | None = None) -> str:
    meta = metadata if metadata is not None else cargo_metadata(root)
    packages = sorted(meta["packages"], key=lambda p: p["name"])
    names = {p["name"] for p in packages}

    def target_kinds(p: dict) -> list[str]:
        kinds: list[str] = []
        for t in p["targets"]:
            for k in t["kind"]:
                if k in ("lib", "rlib", "bin", "cdylib", "staticlib", "proc-macro") and k not in kinds:
                    kinds.append(k)
        return kinds

    depended_on = {d["name"] for p in packages for d in p["dependencies"]
                   if d["name"] in names and d["kind"] in (None, "build") and d["name"] != p["name"]}

    def group(p: dict) -> str:
        kinds = target_kinds(p)
        # An application: a binary nothing else links (a library crate that also ships helper or
        # test binaries, like vox-sandbox-ipc, stays a library).
        if "bin" in kinds and p["name"] not in depended_on:
            return "Binaries"
        if "cdylib" in kinds and "lib" not in kinds:
            return "Plugin libraries (cdylib)"
        return "Libraries"

    def internal(p: dict, kind: str | None) -> list[str]:
        return sorted({d["name"] for d in p["dependencies"] if d["name"] in names and d["kind"] == kind
                       and d["name"] != p["name"]})

    lines = ["```mermaid", "flowchart TD"]
    for grp in ("Binaries", "Libraries", "Plugin libraries (cdylib)"):
        members = [p for p in packages if group(p) == grp]
        if not members:
            continue
        lines.append(f'  subgraph {node_id(grp)}["{grp}"]')
        for p in members:
            directory = Path(p["manifest_path"]).parent.relative_to(root).as_posix()
            lines.append(f'    {node_id(p["name"])}["{p["name"]}<br/>{directory}"]')
        lines.append("  end")
    for p in packages:
        for dep in sorted(set(internal(p, None)) | set(internal(p, "build"))):
            lines.append(f"  {node_id(p['name'])} --> {node_id(dep)}")
    lines.append("```")
    lines.append("")
    lines.append("| Package | Directory | Targets | Depends on (workspace crates) | Dev-only (tests, benches) |")
    lines.append("|---|---|---|---|---|")
    for p in packages:
        directory = Path(p["manifest_path"]).parent
        link = f"[`{directory.relative_to(root).as_posix()}`]({relpath(directory, doc)})"
        normal = ", ".join(f"`{d}`" for d in internal(p, None)) or "none"
        dev_only = [d for d in internal(p, "dev") if d not in internal(p, None)]
        dev = ", ".join(f"`{d}`" for d in dev_only) or "none"
        lines.append(f"| `{p['name']}` | {link} | {', '.join(target_kinds(p))} | {normal} | {dev} |")
    return "\n".join(lines)


COMMAND_FN_RE = re.compile(
    r"((?:^[ \t]*///[^\n]*\n)*)"  # doc comment
    r"(?:^[ \t]*#\[[^\n]*\]\n)*?"  # other attributes before
    r"^[ \t]*#\[tauri::command[^\n]*\]\n"
    r"(?:^[ \t]*#\[[^\n]*\]\n)*"  # other attributes after
    r"^[ \t]*pub(?:\([^)]*\))?\s+(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\((.*?)\)\s*(?:->\s*([^{]*))?\{",
    re.M | re.S,
)


@dataclass
class CommandInfo:
    name: str
    file: Path
    doc: str
    params: str
    returns: str


def first_sentence(doc: str) -> str:
    text = " ".join(doc.split())
    m = re.search(r"(?<!e\.g)(?<!i\.e)(?<!\bvs)\.(\s|$)", text)
    sentence = text[: m.start() + 1] if m else text
    return sentence.replace("|", "\\|")


def parse_commands(root: Path) -> list[str]:
    mod = (root / "src-tauri/src/ipc/mod.rs").read_text(encoding="utf-8")
    m = re.search(r'#\[cfg\(not\(feature = "spike"\)\)\]\s*crate::ipc_commands!\((.*?)\);', mod, re.S)
    if not m:
        raise ValueError("src-tauri/src/ipc/mod.rs: no production ipc_commands! block")
    return [n.strip() for n in m.group(1).split(",") if n.strip()]


def parse_events(root: Path) -> list[str]:
    events = (root / "src-tauri/src/ipc/events.rs").read_text(encoding="utf-8")
    m = re.search(r"crate::ipc_events!\((.*?)\);", events, re.S)
    if not m:
        raise ValueError("src-tauri/src/ipc/events.rs: no ipc_events! block")
    return [n.strip() for n in m.group(1).split(",") if n.strip()]


def non_test_source(path: Path) -> str:
    text = path.read_text(encoding="utf-8")
    m = re.search(r"^#\[cfg\(test\)\]\s*\n\s*mod \w+", text, re.M)
    return text[: m.start()] if m else text


def command_handlers(root: Path) -> dict[str, CommandInfo]:
    found: dict[str, CommandInfo] = {}
    for path in sorted((root / "src-tauri/src").rglob("*.rs")):
        if "spike" in path.relative_to(root / "src-tauri/src").parts:
            continue
        text = non_test_source(path)
        for m in COMMAND_FN_RE.finditer(text):
            doc = "\n".join(re.sub(r"^\s*///\s?", "", ln) for ln in m.group(1).splitlines())
            found[m.group(2)] = CommandInfo(m.group(2), path, doc, m.group(3), (m.group(4) or "").strip())
    return found


def transport_of(info: CommandInfo) -> str:
    if re.search(r"\bChannel\b", info.params):
        return "stream (`Channel`)"
    if re.search(r"\bResponse\b", info.returns):
        return "binary (`Response`)"
    return "JSON"


def gen_ipc_commands(root: Path, doc: Path) -> str:
    handlers = command_handlers(root)
    lines = [
        "| # | Command | Handler | Reply | What it does (first line of the handler's doc comment) |",
        "|---|---|---|---|---|",
    ]
    for i, name in enumerate(parse_commands(root), start=1):
        info = handlers.get(name)
        if info is None:
            raise ValueError(f"command '{name}' is registered but no #[tauri::command] fn was found")
        file_link = f"[`{info.file.name}`]({relpath(info.file, doc)})"
        lines.append(f"| {i} | `{name}` | {file_link} | {transport_of(info)} | {first_sentence(info.doc) or '—'} |")
    return "\n".join(lines)


def gen_ipc_events(root: Path, doc: Path) -> str:
    src = root / "src-tauri/src"
    files = [p for p in sorted(src.rglob("*.rs"))
             if "spike" not in p.relative_to(src).parts and p.name != "macros.rs"]
    sources = {p: non_test_source(p) for p in files}
    events_rs = src / "ipc/events.rs"
    # emit_* helpers in events.rs that emit a given event: callers count as emitters.
    helpers: dict[str, list[str]] = {}
    for m in re.finditer(r"pub fn (emit_\w+)[^{]*\{(.*?)\n\}", sources[events_rs], re.S):
        for ev in re.findall(r"EventName::(\w+)", m.group(2)):
            helpers.setdefault(ev, []).append(m.group(1))
    lines = [
        "| # | Event | Emitted from |",
        "|---|---|---|",
    ]
    for i, name in enumerate(parse_events(root), start=1):
        emitters: set[Path] = set()
        for path, text in sources.items():
            if path == events_rs:
                continue
            if re.search(rf"EventName::{name}\b", text):
                emitters.add(path)
            for helper in helpers.get(name, []):
                if re.search(rf"\b{helper}\s*\(", text):
                    emitters.add(path)
        if not emitters and re.search(rf"EventName::{name}\b", sources[events_rs]):
            emitters.add(events_rs)
        cells = ", ".join(
            f"[`{p.relative_to(src).as_posix()}`]({relpath(p, doc)})" for p in sorted(emitters)
        ) or "not emitted"
        lines.append(f"| {i} | `{name}` | {cells} |")
    return "\n".join(lines)


GENERATORS: dict[str, Callable[[Path, Path], str]] = {
    "crate-graph": gen_crate_graph,
    "ipc-commands": gen_ipc_commands,
    "ipc-events": gen_ipc_events,
}


def check_generated(root: Path, files: list[Path], write: bool) -> list[Problem]:
    problems: list[Problem] = []
    present: dict[str, set[str]] = {}
    for doc in files:
        text = doc.read_text(encoding="utf-8")
        try:
            sections = find_sections(text)
        except ValueError as e:
            problems.append(Problem(rel(root, doc), 1, str(e)))
            continue
        if not sections:
            continue
        new_text = text
        for name, start, end in sections:
            present.setdefault(rel(root, doc), set()).add(name)
            if name not in GENERATORS:
                problems.append(Problem(rel(root, doc), text.count("\n", 0, start) + 1,
                                        f"unknown generated section '{name}'"))
                continue
            try:
                generated = GENERATORS[name](root, doc)
            except (ValueError, subprocess.CalledProcessError, OSError) as e:
                problems.append(Problem(rel(root, doc), 1, f"generator '{name}' failed: {e}"))
                continue
            new_text = replace_section(new_text, name, generated)
            if new_text != text and not write:
                current = text[start:end].strip("\n")
                if current != generated.strip("\n"):
                    problems.append(Problem(
                        rel(root, doc), text.count("\n", 0, start) + 1,
                        f"generated section '{name}' is stale; run `python3 scripts/docs/check.py --write`",
                    ))
        if write and new_text != text:
            doc.write_text(new_text, encoding="utf-8")
            print(f"updated {rel(root, doc)}")
    for doc_path, names in REQUIRED_SECTIONS.items():
        for name in names:
            if name not in present.get(doc_path, set()):
                problems.append(Problem(doc_path, 1, f"required generated section '{name}' is missing"))
    return problems


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--write", action="store_true", help="regenerate generated sections first")
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    root = args.root.resolve()
    files = doc_files(root)

    problems = check_generated(root, files, args.write)
    problems += check_links(root, files)
    mermaid_problems, diagrams = check_mermaid(root, files)
    problems += mermaid_problems
    problems += check_hub(root)

    for p in problems:
        print(p, file=sys.stderr)
    if problems:
        print(f"docs check: {len(problems)} problem(s)", file=sys.stderr)
        return 1
    print(f"docs check: {len(files)} files, {diagrams} mermaid diagrams, links/anchors/generated sections OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
