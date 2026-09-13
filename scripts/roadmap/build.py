#!/usr/bin/env python3
"""Builds a project roadmap dashboard from a ticket board, git history and agent transcripts.

Output: one self-contained HTML page (data inlined). For PowerVoice: `just roadmap`.
Works for any repo with a markdown ticket board (tables with ID / Title / Status columns,
optional Deps) and git; tokens and agent time need Claude Code transcripts.

- Tickets, status, deps: the board. `## Slice …` sections are the build order, `## Hardening …`
  follows them, every other section is the full-spec backlog.
- Finish time: the ticket's squash-merge commit ("S1-03: ..."); "fix"/"follow-up" commits show
  as fix ticks, not as new finishes.
- Tokens and agent time: Claude Code transcripts (~/.claude/projects/<repo path, "/" -> "-">).
  Every assistant message's usage counts once (deduplicated by message id). A subagent is
  attributed to the first ticket ID in its first prompt; top-level sessions are the orchestrator.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
from datetime import datetime, timedelta, timezone
from pathlib import Path

TEMPLATE = Path(__file__).with_name("template.html")
DEFAULT_REPO = Path(__file__).resolve().parents[2]
# Agent time: a gap longer than this between transcript entries counts as idle.
ACTIVE_GAP = timedelta(minutes=10)
# Project timeline: no activity for longer than this is a pause (folded on the chart).
SPAN_GAP = timedelta(minutes=45)
MAX_SERIES_POINTS = 400
MILESTONES = [
    (re.compile(r"M0 checkpoint"), "M0 checkpoint"),
    (re.compile(r"Strategy change"), "Vertical slices"),
    (re.compile(r"Slice (\d) complete"), "Slice {0} complete"),
]
TOKEN_KEYS = ("input", "cache_write", "cache_read", "output")

DEFAULT_IDS = r"T-\d{3}|S\d-\d{2}|H-\d{2}"
ID_RE = re.compile(rf"\b({DEFAULT_IDS})\b")


def parse_ts(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def iso(dt: datetime | None) -> str | None:
    if dt is None:
        return None
    return dt.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def clean(text: str) -> str:
    return re.sub(r"[`*]", "", text).strip()


def norm_status(raw: str) -> str:
    s = raw.lower()
    if s.startswith("done"):
        return "done"
    if s.startswith(("in progress", "in-progress", "review")):
        return "active"
    if s.startswith("paused"):
        return "paused"
    if s.startswith(("gated", "blocked")):
        return "gated"
    if s.startswith("→"):
        return "partial"
    return "todo"


def parse_board(board: Path) -> list[dict]:
    sections: list[dict] = []
    current: dict | None = None
    header: list[str] | None = None
    for line in board.read_text(encoding="utf-8").splitlines():
        if line.startswith("#"):
            title = line.lstrip("#").strip()
            if line.startswith("# ") or title.startswith("▶"):
                current = None
                continue
            name, _, sub = title.partition(" — ")
            sub = re.sub(r"\s*\(.*\)$", "", sub)
            if name.startswith("Slice"):
                kind = "slice"
            elif name.startswith("Hardening"):
                kind = "hardening"
            else:
                kind = "milestone"
            current = {"name": name.strip(), "subtitle": sub.strip(), "kind": kind, "tickets": []}
            sections.append(current)
            header = None
            continue
        if current is None or not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if header is None:
            header = [c.lower() for c in cells]
            continue
        row = dict(zip(header, cells))
        ticket_id = row.get("id", "")
        if not ID_RE.fullmatch(ticket_id):
            continue
        current["tickets"].append(
            {
                "id": ticket_id,
                "title": clean(row.get("title", "")),
                "tier": row.get("tier", ""),
                "deps": ID_RE.findall(row.get("deps", "")),
                "status": norm_status(row.get("status", "")),
                "status_text": clean(row.get("status", "")),
            }
        )
    return [s for s in sections if s["tickets"]]


def git_commits(repo: Path) -> list[dict]:
    out = subprocess.run(
        ["git", "-C", str(repo), "log", "--reverse", "--format=%aI%x1f%s"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    commits = []
    for line in out.splitlines():
        stamp, _, subject = line.partition("\x1f")
        commits.append({"t": parse_ts(stamp), "subject": subject})
    return commits


def commit_events(commits: list[dict]) -> tuple[dict, list, list]:
    """Finish time per ticket (latest exact "ID:" commit, else latest commit), fixes, milestones."""
    latest: dict[str, datetime] = {}
    exact: dict[str, datetime] = {}
    fixes: list[dict] = []
    milestones: list[dict] = []
    seen: set[str] = set()
    for commit in commits:
        subject = commit["subject"]
        for pattern, label in MILESTONES:
            match = pattern.search(subject)
            if match:
                text = label.format(*match.groups())
                if text not in seen:
                    seen.add(text)
                    milestones.append({"t": iso(commit["t"]), "label": text})
        if subject.startswith("orchestrator"):
            continue
        prefix = subject.split(":", 1)[0]
        ids = ID_RE.findall(prefix)
        if not ids:
            continue
        if re.search(r"\b(fix|follow-up)\b", prefix):
            fixes.append({"t": iso(commit["t"]), "ids": ids, "subject": subject})
            continue
        for ticket_id in ids:
            latest[ticket_id] = commit["t"]
            if prefix.strip() == ticket_id:
                exact[ticket_id] = commit["t"]
    return {**latest, **exact}, fixes, milestones


def short_model(model: str) -> str | None:
    for name in ("opus", "sonnet", "haiku", "fable"):
        if name in model:
            return name
    return None


def read_transcript(path: Path) -> dict:
    first_prompt: str | None = None
    usage_by_message: dict[str, tuple[str, dict, datetime | None]] = {}
    stamps: list[datetime] = []
    with path.open(encoding="utf-8") as lines:
        for line in lines:
            try:
                entry = json.loads(line)
            except json.JSONDecodeError:
                continue
            stamp = parse_ts(entry["timestamp"]) if entry.get("timestamp") else None
            if stamp:
                stamps.append(stamp)
            message = entry.get("message") or {}
            kind = entry.get("type")
            if kind == "user" and first_prompt is None:
                content = message.get("content")
                if isinstance(content, str):
                    first_prompt = content
                elif isinstance(content, list):
                    first_prompt = " ".join(
                        block.get("text", "") for block in content if isinstance(block, dict)
                    )
            elif kind == "assistant" and message.get("usage"):
                key = message.get("id") or entry.get("uuid")
                usage_by_message[key] = (message.get("model", ""), message["usage"], stamp)
    tokens = dict.fromkeys(TOKEN_KEYS, 0)
    models: dict[str, int] = {}
    events: list[tuple[datetime, int]] = []
    for model, usage, stamp in usage_by_message.values():
        counts = {
            "input": usage.get("input_tokens", 0) or 0,
            "cache_write": usage.get("cache_creation_input_tokens", 0) or 0,
            "cache_read": usage.get("cache_read_input_tokens", 0) or 0,
            "output": usage.get("output_tokens", 0) or 0,
        }
        for key, value in counts.items():
            tokens[key] += value
        total = sum(counts.values())
        name = short_model(model)
        if name:
            models[name] = models.get(name, 0) + total
        if stamp:
            events.append((stamp, total))
    stamps.sort()
    active = sum((min(b - a, ACTIVE_GAP) for a, b in zip(stamps, stamps[1:])), timedelta())
    return {
        "prompt": first_prompt or "",
        "tokens": tokens,
        "models": models,
        "active_s": active.total_seconds(),
        "stamps": stamps,
        "events": events,
    }


def empty_agg() -> dict:
    return {
        "tokens": dict.fromkeys(TOKEN_KEYS, 0),
        "active_s": 0.0,
        "agents": 0,
        "models": {},
        "first": None,
        "last": None,
    }


def add(agg: dict, info: dict) -> None:
    for key in TOKEN_KEYS:
        agg["tokens"][key] += info["tokens"][key]
    agg["active_s"] += info["active_s"]
    agg["agents"] += 1
    for model, count in info["models"].items():
        agg["models"][model] = agg["models"].get(model, 0) + count
    if info["stamps"]:
        first, last = info["stamps"][0], info["stamps"][-1]
        agg["first"] = min(agg["first"] or first, first)
        agg["last"] = max(agg["last"] or last, last)


def activity_spans(stamps: list[datetime]) -> list[list[str | None]]:
    stamps = sorted(stamps)
    spans: list[list[datetime]] = []
    start = prev = stamps[0]
    for stamp in stamps[1:]:
        if stamp - prev > SPAN_GAP:
            spans.append([start, prev])
            start = stamp
        prev = stamp
    spans.append([start, prev])
    return [[iso(a), iso(b)] for a, b in spans]


def token_series(events: list[tuple[datetime, int]]) -> list[list]:
    """Cumulative tokens over time, thinned to at most MAX_SERIES_POINTS points."""
    events.sort(key=lambda e: e[0])
    running = 0
    points = []
    for stamp, count in events:
        running += count
        points.append([stamp, running])
    if not points:
        return []
    step = max(1, len(points) // MAX_SERIES_POINTS)
    thinned = points[::step]
    if thinned[-1] is not points[-1]:
        thinned.append(points[-1])
    return [[iso(stamp), total] for stamp, total in thinned]


def public_agg(agg: dict) -> dict:
    return {
        "tokens": agg["tokens"],
        "active_s": round(agg["active_s"]),
        "agents": agg["agents"],
        "models": agg["models"],
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--name", default="PowerVoice", help="project name shown on the page")
    parser.add_argument("--repo", type=Path, default=DEFAULT_REPO, help="git repository root")
    parser.add_argument("--board", type=Path, help="ticket board (default: <repo>/tickets/BOARD.md)")
    parser.add_argument("--out", type=Path, help="output page (default: <repo>/target/roadmap/index.html)")
    parser.add_argument("--transcripts", type=Path, help="Claude Code project transcript directory")
    parser.add_argument("--ids", default=DEFAULT_IDS, help="ticket ID regex (alternatives, no groups)")
    return parser.parse_args()


def main() -> None:
    global ID_RE
    args = parse_args()
    repo = args.repo.resolve()
    ID_RE = re.compile(rf"\b({args.ids})\b")
    board = args.board or repo / "tickets" / "BOARD.md"
    out = args.out or repo / "target" / "roadmap" / "index.html"
    tdir = args.transcripts or Path.home() / ".claude" / "projects" / str(repo).replace("/", "-")

    now = datetime.now(timezone.utc)
    sections = parse_board(board)
    commits = git_commits(repo)
    finished_at, fixes, milestones = commit_events(commits)
    board_ids = {t["id"] for s in sections for t in s["tickets"]}

    orchestrator = empty_agg()
    other = empty_agg()
    per_ticket: dict[str, dict] = {}
    stamps: list[datetime] = [c["t"] for c in commits] + [now]
    events: list[tuple[datetime, int]] = []
    for path in sorted(tdir.glob("*.jsonl")):
        info = read_transcript(path)
        add(orchestrator, info)
        stamps += info["stamps"]
        events += info["events"]
    for path in sorted(tdir.glob("**/subagents/*.jsonl")):
        info = read_transcript(path)
        match = ID_RE.search(info["prompt"])
        if match and match.group(1) in board_ids:
            add(per_ticket.setdefault(match.group(1), empty_agg()), info)
        else:
            add(other, info)
        stamps += info["stamps"]
        events += info["events"]

    for section in sections:
        for ticket in section["tickets"]:
            agg = per_ticket.get(ticket["id"])
            if ticket["status"] == "done":
                ticket["finished_at"] = iso(finished_at.get(ticket["id"]))
            if agg:
                ticket.update(public_agg(agg))
                ticket["started_at"] = iso(agg["first"])
                ticket["model"] = max(agg["models"], key=agg["models"].get) if agg["models"] else None

    aggs = [orchestrator, other, *per_ticket.values()]
    data = {
        "project": args.name,
        "generated_at": iso(now),
        "sections": sections,
        "fixes": fixes,
        "milestones": milestones,
        "spans": activity_spans(stamps),
        "token_series": token_series(events),
        "orchestrator": public_agg(orchestrator),
        "other_agents": public_agg(other),
        "totals": {key: sum(a["tokens"][key] for a in aggs) for key in TOKEN_KEYS},
    }
    payload = json.dumps(data, ensure_ascii=False, separators=(",", ":")).replace("</", "<\\/")
    html = (
        TEMPLATE.read_text(encoding="utf-8")
        .replace("__PROJECT__", args.name.replace("<", "&lt;"))
        .replace("__ROADMAP_DATA__", payload)
    )
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(html, encoding="utf-8")
    tickets = [t for s in sections for t in s["tickets"]]
    done = sum(1 for t in tickets if t["status"] == "done")
    print(f"wrote {out}: {done}/{len(tickets)} tickets done")


if __name__ == "__main__":
    main()
