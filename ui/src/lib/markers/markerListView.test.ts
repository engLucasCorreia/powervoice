import { describe, expect, it } from "vitest";
import type { MarkerDto } from "../ipc/bindings";
import {
  compareCanonical,
  filterMarkers,
  markerMatchesTextFilter,
  markerMatchesTypeFilter,
  sortMarkers,
  virtualRowRange,
} from "./markerListView";

function marker(
  id: number,
  pos: number,
  len = 0,
  name = `m${id}`,
  kind: MarkerDto["kind"] = "user",
): MarkerDto {
  return { id, pos_samples: pos, len_samples: len, name, kind };
}

describe("markerMatchesTypeFilter (SPEC-009 §2.8)", () => {
  const point = marker(1, 0, 0, "Point", "user");
  const region = marker(2, 100, 50, "Region", "user");
  const dropout = marker(3, 200, 0, "Dropout 10 ms", "dropout");
  const other = marker(4, 300, 0, "Chapter", "other");

  it("all matches everything", () => {
    for (const m of [point, region, dropout, other]) {
      expect(markerMatchesTypeFilter(m, "all")).toBe(true);
    }
  });

  it("points = user/unknown-kind point markers, never a dropout", () => {
    expect(markerMatchesTypeFilter(point, "points")).toBe(true);
    expect(markerMatchesTypeFilter(other, "points")).toBe(true);
    expect(markerMatchesTypeFilter(region, "points")).toBe(false);
    expect(markerMatchesTypeFilter(dropout, "points")).toBe(false);
  });

  it("regions = len_samples > 0", () => {
    expect(markerMatchesTypeFilter(region, "regions")).toBe(true);
    expect(markerMatchesTypeFilter(point, "regions")).toBe(false);
    expect(markerMatchesTypeFilter(dropout, "regions")).toBe(false);
  });

  it("dropouts = kind dropout", () => {
    expect(markerMatchesTypeFilter(dropout, "dropouts")).toBe(true);
    expect(markerMatchesTypeFilter(point, "dropouts")).toBe(false);
    expect(markerMatchesTypeFilter(region, "dropouts")).toBe(false);
  });
});

describe("markerMatchesTextFilter (SPEC-009 §2.8)", () => {
  it("case-insensitive substring match, locale-independent folding", () => {
    const m = marker(1, 0, 0, "Retake P.12");
    expect(markerMatchesTextFilter(m, "retake")).toBe(true);
    expect(markerMatchesTextFilter(m, "P.12")).toBe(true);
    expect(markerMatchesTextFilter(m, "xyz")).toBe(false);
  });

  it("empty or whitespace-only filter matches everything", () => {
    const m = marker(1, 0, 0, "Anything");
    expect(markerMatchesTextFilter(m, "")).toBe(true);
    expect(markerMatchesTextFilter(m, "   ")).toBe(true);
  });
});

describe("filterMarkers (AC-14)", () => {
  it("combines the text and type filters, preserving input order", () => {
    const markers = [
      marker(1, 0, 0, "Take 1"),
      marker(2, 100, 50, "Take 2"),
      marker(3, 200, 0, "Intro"),
      marker(4, 300, 0, "Take 3 dropout", "dropout"),
    ];
    expect(filterMarkers(markers, "take", "all").map((m) => m.id)).toEqual([1, 2, 4]);
    expect(filterMarkers(markers, "take", "regions").map((m) => m.id)).toEqual([2]);
    expect(filterMarkers(markers, "", "dropouts").map((m) => m.id)).toEqual([4]);
  });
});

describe("compareCanonical (SPEC-009 §2.1)", () => {
  it("sorts by position, then id", () => {
    const markers = [marker(3, 10), marker(1, 10), marker(2, 5)];
    expect([...markers].sort(compareCanonical).map((m) => m.id)).toEqual([2, 1, 3]);
  });
});

const collator = { compare: (a: string, b: string) => a.localeCompare(b, "en", { numeric: true, sensitivity: "base" }) };

describe("sortMarkers (SPEC-009 §2.8, AC-12)", () => {
  it("Name sorts numerically and case-insensitively, ties by canonical order", () => {
    const markers = [
      marker(1, 0, 0, "Marker 10"),
      marker(2, 10, 0, "Marker 2"),
      marker(3, 20, 0, "intro"),
      marker(4, 30, 0, "Intro"),
    ];
    const sorted = sortMarkers(markers, "name", "asc", collator);
    // "intro"/"Intro" tie under { sensitivity: "base" } -> canonical order (pos 20 before 30).
    expect(sorted.map((m) => m.name)).toEqual(["intro", "Intro", "Marker 2", "Marker 10"]);
  });

  it("Start is the default sort, ascending", () => {
    const markers = [marker(1, 300), marker(2, 100), marker(3, 200)];
    expect(sortMarkers(markers, "start", "asc", collator).map((m) => m.id)).toEqual([2, 3, 1]);
    expect(sortMarkers(markers, "start", "desc", collator).map((m) => m.id)).toEqual([1, 3, 2]);
  });

  it("Duration sorts points as 0", () => {
    const markers = [marker(1, 0, 500), marker(2, 100, 0), marker(3, 200, 100)];
    expect(sortMarkers(markers, "duration", "asc", collator).map((m) => m.id)).toEqual([2, 3, 1]);
  });

  it("Type sorts Point < Region < Dropout", () => {
    const point = marker(1, 300, 0, "P", "user");
    const region = marker(2, 100, 50, "R", "user");
    const dropout = marker(3, 200, 0, "D", "dropout");
    const sorted = sortMarkers([dropout, point, region], "type", "asc", collator);
    expect(sorted.map((m) => m.id)).toEqual([1, 2, 3]);
  });

  it("ties always fall back to canonical order, even sorted descending", () => {
    // Two points at the same Start; id 5 is canonically first (lower id at the same position).
    const markers = [marker(9, 100), marker(5, 100)];
    expect(sortMarkers(markers, "start", "asc", collator).map((m) => m.id)).toEqual([5, 9]);
    expect(sortMarkers(markers, "start", "desc", collator).map((m) => m.id)).toEqual([5, 9]);
  });
});

describe("virtualRowRange (SPEC-009 §2.8/AC-15)", () => {
  it("renders nothing for zero rows or an unknown row height", () => {
    expect(virtualRowRange(0, 300, 22, 10, 0)).toEqual({ start: 0, end: 0 });
    expect(virtualRowRange(0, 300, 0, 10, 100)).toEqual({ start: 0, end: 0 });
  });

  it("at the top, renders the visible rows plus overscan below only (no negative start)", () => {
    // viewport 300px / 22px rows ~= 13.6 -> 14 visible + 1 straddling = 15; + 10 below.
    const range = virtualRowRange(0, 300, 22, 10, 10_000);
    expect(range.start).toBe(0);
    expect(range.end).toBe(25);
  });

  it("scrolled into the middle, overscans both above and below", () => {
    // firstVisible = floor(2200/22) = 100.
    const range = virtualRowRange(2_200, 300, 22, 10, 10_000);
    expect(range.start).toBe(90);
    expect(range.end).toBe(125);
  });

  it("clamps the end to the total row count near the bottom of a short list", () => {
    const range = virtualRowRange(2_000, 300, 22, 10, 100);
    expect(range.end).toBe(100);
  });

  it("AC-15: at 10 000 markers, the rendered window never exceeds visible + 2*overscan rows", () => {
    const range = virtualRowRange(50_000, 500, 22, 10, 10_000);
    const rendered = range.end - range.start;
    const visible = Math.ceil(500 / 22) + 1;
    expect(rendered).toBeLessThanOrEqual(visible + 20);
  });
});
