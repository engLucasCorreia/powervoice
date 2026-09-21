#!/usr/bin/env python3
"""Generates the in-app Help Centre's content from the real end-user docs (H-107).

**Decision (H-107 item 4):** the Help Centre is *generated at build time* from
`docs/user-guide.md` and `docs/faq.md`, not a hand-maintained copy. Those two files are already
the single source of truth for "how do I use PowerVoice" (kept accurate by H-105/T-706's own
`scripts/docs/check.py`, which verifies every link and anchor in them); duplicating their prose by
hand into the UI would drift the moment either page changed, and silently start lying to a user who
has no other way to tell. Parsing them at generation time means there is exactly one place that
knows "how do I install an LV2 plugin," and the app can never show an answer the docs no longer
agree with. `just check` runs this script with `--check` (same pattern as
`scripts/notices/generate.py` and `scripts/docs/generate_shortcuts.mjs`) and fails if the committed
generated file is stale.

Only `docs/user-guide.md` and `docs/faq.md` are included: together they already cover every topic
H-107 lists (getting started, installing/managing third-party plugins, the effects rack, noise
reduction/loudness/ACX/export, and troubleshooting). `docs/glossary.md`, `docs/how-it-works.md` and
`docs/what-is-powervoice.md` are deliberately left out for now — the glossary's entries are
`**Term**` bold paragraphs rather than real headings (no natural "section" to browse to), and
`how-it-works.md` embeds a Mermaid diagram this generator has no business trying to render inside
the app. Extending `DOC_FILES` below is how a future ticket would add them.

Usage:
    python3 scripts/help/generate.py            # (re)write ui/src/lib/help/content.generated.ts
    python3 scripts/help/generate.py --check    # fail if the committed file is stale

Reuses `scripts/docs/check.py`'s `github_slug` (and its `strip_inline_markdown`) so a Help Centre
section id is always exactly the anchor `docs/docs/check.py` itself already validates every
`#anchor` link in these docs against — one slug algorithm, not two that could disagree.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DOCS_DIR = REPO_ROOT / "docs"
OUTPUT = REPO_ROOT / "ui" / "src" / "lib" / "help" / "content.generated.ts"

sys.path.insert(0, str(REPO_ROOT / "scripts" / "docs"))
import check  # noqa: E402  (github_slug / strip_inline_markdown, the same anchor rules docs/check.py enforces)

# Doc id -> filename under docs/. Order here is nav order in the Help Centre.
DOC_FILES: dict[str, str] = {
    "user-guide": "user-guide.md",
    "faq": "faq.md",
}

HEADING_RE = re.compile(r"^(#{1,3})\s+(.*?)\s*#*\s*$")
FENCE_RE = check.FENCE_RE
BULLET_RE = re.compile(r"^[-*]\s+(.*)$")
ORDERED_RE = re.compile(r"^\d+\.\s+(.*)$")
CONTINUATION_RE = re.compile(r"^\s+\S")
TABLE_ROW_RE = re.compile(r"^\s*\|")
TABLE_SEP_CELL_RE = re.compile(r"^:?-{2,}:?$")


# --------------------------------------------------------------------------------------------
# Pass 1: raw block parsing (markdown kept intact — inline formatting resolved in pass 2, once
# every doc's headings are known and links can be resolved).
# --------------------------------------------------------------------------------------------


@dataclass
class RawHeading:
    level: int
    text: str
    id: str


@dataclass
class RawPara:
    text: str


@dataclass
class RawList:
    kind: str  # "ul" | "ol"
    items: list[str]


@dataclass
class RawCode:
    lang: str
    text: str


@dataclass
class RawTable:
    header: list[str]
    rows: list[list[str]]


RawBlock = RawHeading | RawPara | RawList | RawCode | RawTable


def is_block_start(line: str) -> bool:
    if not line.strip():
        return True
    if HEADING_RE.match(line) or FENCE_RE.match(line):
        return True
    if TABLE_ROW_RE.match(line):
        return True
    if BULLET_RE.match(line) or ORDERED_RE.match(line):
        return True
    return False


def split_table_row(line: str) -> list[str]:
    stripped = line.strip()
    if stripped.startswith("|"):
        stripped = stripped[1:]
    if stripped.endswith("|"):
        stripped = stripped[:-1]
    return [cell.strip() for cell in stripped.split("|")]


def is_table_separator(line: str) -> bool:
    if not TABLE_ROW_RE.match(line):
        return False
    return all(TABLE_SEP_CELL_RE.match(cell) for cell in split_table_row(line) if cell != "") and bool(
        split_table_row(line)
    )


def parse_blocks(text: str) -> list[RawBlock]:
    lines = text.split("\n")
    n = len(lines)
    i = 0
    blocks: list[RawBlock] = []
    seen_ids: dict[str, int] = {}

    def make_id(plain: str) -> str:
        slug = check.github_slug(plain)
        if slug in seen_ids:
            seen_ids[slug] += 1
            return f"{slug}-{seen_ids[slug]}"
        seen_ids[slug] = 0
        return slug

    while i < n:
        line = lines[i]
        if not line.strip():
            i += 1
            continue

        m = FENCE_RE.match(line)
        if m:
            marker = m.group(1)
            lang = m.group(2).strip()
            i += 1
            body: list[str] = []
            while i < n:
                close = FENCE_RE.match(lines[i])
                if close and close.group(1)[0] == marker[0] and len(close.group(1)) >= len(marker) and not close.group(2).strip():
                    i += 1
                    break
                body.append(lines[i])
                i += 1
            blocks.append(RawCode(lang=lang, text="\n".join(body)))
            continue

        m = HEADING_RE.match(line)
        if m:
            level = len(m.group(1))
            plain = check.strip_inline_markdown(m.group(2)).strip()
            blocks.append(RawHeading(level=level, text=m.group(2).strip(), id=make_id(plain)))
            i += 1
            continue

        if TABLE_ROW_RE.match(line) and i + 1 < n and is_table_separator(lines[i + 1]):
            header = split_table_row(line)
            i += 2
            rows: list[list[str]] = []
            while i < n and TABLE_ROW_RE.match(lines[i]):
                rows.append(split_table_row(lines[i]))
                i += 1
            blocks.append(RawTable(header=header, rows=rows))
            continue

        m_ul = BULLET_RE.match(line)
        m_ol = ORDERED_RE.match(line)
        if m_ul or m_ol:
            kind = "ul" if m_ul else "ol"
            items: list[str] = []
            cur = (m_ul or m_ol).group(1).strip()
            i += 1
            while i < n:
                nxt = lines[i]
                if not nxt.strip():
                    break
                nm_ul = BULLET_RE.match(nxt)
                nm_ol = ORDERED_RE.match(nxt)
                if kind == "ul" and nm_ul:
                    items.append(cur)
                    cur = nm_ul.group(1).strip()
                    i += 1
                    continue
                if kind == "ol" and nm_ol:
                    items.append(cur)
                    cur = nm_ol.group(1).strip()
                    i += 1
                    continue
                if nm_ul or nm_ol or HEADING_RE.match(nxt) or FENCE_RE.match(nxt) or TABLE_ROW_RE.match(nxt):
                    break
                if CONTINUATION_RE.match(nxt):
                    cur += " " + nxt.strip()
                    i += 1
                    continue
                break
            items.append(cur)
            blocks.append(RawList(kind=kind, items=items))
            continue

        para_lines = [line.strip()]
        i += 1
        while i < n and not is_block_start(lines[i]):
            para_lines.append(lines[i].strip())
            i += 1
        blocks.append(RawPara(text=" ".join(para_lines)))

    return blocks


# --------------------------------------------------------------------------------------------
# Sections: group blocks under their H2 (an "overview" section holds the H1 + anything before the
# first H2); an H3 becomes a heading block *inside* the enclosing H2 section rather than a
# section of its own (a Help Centre topic reads as one page, like the doc it came from).
# --------------------------------------------------------------------------------------------


@dataclass
class Section:
    id: str
    title: str
    blocks: list[RawBlock] = field(default_factory=list)


@dataclass
class Doc:
    id: str
    title: str
    sections: list[Section] = field(default_factory=list)


def group_sections(doc_id: str, blocks: list[RawBlock]) -> Doc:
    doc_title = doc_id
    overview = Section(id="overview", title=doc_id)
    sections: list[Section] = [overview]
    current = overview
    seen_h1 = False
    for b in blocks:
        if isinstance(b, RawHeading):
            if b.level == 1 and not seen_h1:
                doc_title = check.strip_inline_markdown(b.text).strip()
                overview.title = doc_title
                seen_h1 = True
                continue
            if b.level == 2:
                current = Section(id=b.id, title=check.strip_inline_markdown(b.text).strip())
                sections.append(current)
                continue
            # H3 (or a stray H1): keep it as content inside the current section.
            current.blocks.append(b)
            continue
        current.blocks.append(b)
    # Drop the overview section if the doc had no intro content before its first H2 heading.
    if overview is sections[0] and not overview.blocks:
        sections = sections[1:] if len(sections) > 1 else sections
    return Doc(id=doc_id, title=doc_title, sections=sections)


def build_anchor_index(docs: dict[str, Doc]) -> dict[tuple[str, str | None], str]:
    index: dict[tuple[str, str | None], str] = {}
    for doc in docs.values():
        first_id = doc.sections[0].id if doc.sections else "overview"
        index[(doc.id, None)] = first_id
        for section in doc.sections:
            index[(doc.id, section.id)] = section.id
            for b in section.blocks:
                if isinstance(b, RawHeading):
                    index[(doc.id, b.id)] = section.id
    return index


# --------------------------------------------------------------------------------------------
# Pass 2: inline markdown -> spans, resolving cross-doc links against the anchor index.
# --------------------------------------------------------------------------------------------

INLINE_RE = re.compile(
    r"\[(?P<ltext>[^\]]+)\]\((?P<lurl>[^)\s]+)(?:\s+\"[^\"]*\")?\)"
    r"|\*\*(?P<bold>[^*]+?)\*\*"
    r"|`(?P<code>[^`]+)`"
    r"|\*(?P<italic>[^*]+?)\*"
)

LINK_TARGET_RE = re.compile(r"^(user-guide|faq)\.md(?:#(.+))?$")


def resolve_link(url: str, current_doc: str, anchor_index: dict[tuple[str, str | None], str]) -> dict[str, str] | None:
    url = url.strip()
    if url.startswith("#"):
        doc_id, anchor = current_doc, url[1:]
    else:
        m = LINK_TARGET_RE.match(url)
        if not m:
            return None
        doc_id, anchor = m.group(1), m.group(2)
    section_id = anchor_index.get((doc_id, anchor))
    if section_id is None:
        return None
    return {"doc": doc_id, "section": section_id}


def parse_inline(text: str, current_doc: str, anchor_index: dict[tuple[str, str | None], str]) -> list[dict]:
    spans: list[dict] = []
    pos = 0
    for m in INLINE_RE.finditer(text):
        if m.start() > pos:
            spans.append({"text": text[pos : m.start()]})
        if m.group("ltext") is not None:
            link = resolve_link(m.group("lurl"), current_doc, anchor_index)
            for sp in parse_inline(m.group("ltext"), current_doc, anchor_index):
                if link is not None:
                    sp["link"] = link
                spans.append(sp)
        elif m.group("bold") is not None:
            spans.append({"text": m.group("bold"), "bold": True})
        elif m.group("code") is not None:
            spans.append({"text": m.group("code"), "code": True})
        elif m.group("italic") is not None:
            spans.append({"text": m.group("italic"), "italic": True})
        pos = m.end()
    if pos < len(text):
        spans.append({"text": text[pos:]})
    if not spans:
        spans.append({"text": ""})
    return spans


def render_block(b: RawBlock, doc_id: str, anchor_index: dict[tuple[str, str | None], str]) -> dict:
    if isinstance(b, RawHeading):
        return {"type": "h3", "id": b.id, "spans": parse_inline(b.text, doc_id, anchor_index)}
    if isinstance(b, RawPara):
        return {"type": "p", "spans": parse_inline(b.text, doc_id, anchor_index)}
    if isinstance(b, RawList):
        return {
            "type": b.kind,
            "items": [parse_inline(item, doc_id, anchor_index) for item in b.items],
        }
    if isinstance(b, RawCode):
        return {"type": "code", "lang": b.lang, "text": b.text}
    if isinstance(b, RawTable):
        return {
            "type": "table",
            "head": [parse_inline(c, doc_id, anchor_index) for c in b.header],
            "rows": [[parse_inline(c, doc_id, anchor_index) for c in row] for row in b.rows],
        }
    raise TypeError(f"unhandled block: {b!r}")


def build_docs() -> list[dict]:
    raw_docs: dict[str, Doc] = {}
    for doc_id, filename in DOC_FILES.items():
        path = DOCS_DIR / filename
        text = path.read_text(encoding="utf-8")
        raw_docs[doc_id] = group_sections(doc_id, parse_blocks(text))

    anchor_index = build_anchor_index(raw_docs)

    out: list[dict] = []
    for doc_id in DOC_FILES:
        doc = raw_docs[doc_id]
        out.append(
            {
                "id": doc.id,
                "title": doc.title,
                "sections": [
                    {
                        "id": s.id,
                        "title": s.title,
                        "blocks": [render_block(b, doc.id, anchor_index) for b in s.blocks],
                    }
                    for s in doc.sections
                ],
            }
        )
    return out


HEADER = """\
// GENERATED FILE — DO NOT EDIT BY HAND.
//
// H-107: the in-app Help Centre's content, parsed from docs/user-guide.md and docs/faq.md by
// `scripts/help/generate.py` (`just help-content`). Regenerate after editing either doc and commit
// the result — `just check` runs `python3 scripts/help/generate.py --check` and fails if this file
// no longer matches the docs. See that script's module docstring for why the Help Centre is
// generated rather than a hand-maintained copy (one source of truth instead of two that drift).
import type { HelpDoc } from "./content";

export const HELP_DOCS: readonly HelpDoc[] = \
"""


def render_ts(docs: list[dict]) -> str:
    body = json.dumps(docs, indent=2, ensure_ascii=False)
    return HEADER + body + ";\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true", help="fail if the generated file is stale")
    args = parser.parse_args(argv)

    docs = build_docs()
    generated = render_ts(docs)

    if args.check:
        current = OUTPUT.read_text(encoding="utf-8") if OUTPUT.exists() else None
        if current != generated:
            print(
                f"{OUTPUT.relative_to(REPO_ROOT)} is stale — run `just help-content` and commit the result.",
                file=sys.stderr,
            )
            return 1
        print("help content: up to date")
        return 0

    OUTPUT.write_text(generated, encoding="utf-8")
    print(f"wrote {OUTPUT.relative_to(REPO_ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
