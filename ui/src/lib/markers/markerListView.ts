/**
 * H-64 (SPEC-009 §2.8): pure filter/sort/virtualization math for the Markers panel, shared by the
 * store (`markers.svelte.ts`) and `MarkersProperties.svelte`. Kept pure and dependency-free so
 * AC-14's 10 000-marker filter latency and AC-15's virtualization can be tested without mounting
 * the component.
 */

import type { MarkerDto } from "../ipc/bindings";

/** SPEC-009 §3 `panel_row_px`: the virtualized list's fixed row height. */
export const PANEL_ROW_PX = 22;
/** SPEC-009 §3 `panel_overscan_rows`: rows rendered beyond the viewport, each side. */
export const PANEL_OVERSCAN_ROWS = 10;

/** The panel's type filter (§2.8's header control): All / Points / Regions / Dropouts. */
export type MarkerTypeFilter = "all" | "points" | "regions" | "dropouts";

/** The panel's sortable columns (§2.8). */
export type MarkerSortColumn = "name" | "start" | "end" | "duration" | "type";

export type MarkerSortDirection = "asc" | "desc";

/**
 * SPEC-009 §2.8's type filter partition:
 * - **Points** = user or unknown-kind point markers (`kind !== "dropout"` and a point);
 * - **Regions** = `len_samples > 0`, dropout or not (dropouts are points in v1, so this never
 *   actually includes one, but the rule is written this way in the spec);
 * - **Dropouts** = `kind === "dropout"`.
 */
export function markerMatchesTypeFilter(marker: MarkerDto, filter: MarkerTypeFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "regions":
      return marker.len_samples > 0;
    case "dropouts":
      return marker.kind === "dropout";
    case "points":
      return marker.kind !== "dropout" && marker.len_samples === 0;
  }
}

/**
 * SPEC-009 §2.8's text filter: case-insensitive substring match on the name, folded with
 * `String.prototype.toLowerCase()` (locale-independent, per spec). An empty/whitespace-only
 * filter matches every marker.
 */
export function markerMatchesTextFilter(marker: MarkerDto, text: string): boolean {
  const needle = text.trim().toLowerCase();
  if (needle === "") {
    return true;
  }
  return marker.name.toLowerCase().includes(needle);
}

/** Both filters together, preserving the input order (sorting is a separate step). */
export function filterMarkers(
  markers: readonly MarkerDto[],
  text: string,
  type: MarkerTypeFilter,
): MarkerDto[] {
  return markers.filter((m) => markerMatchesTypeFilter(m, type) && markerMatchesTextFilter(m, text));
}

/** SPEC-009 §2.1's canonical order (position, then id) — every sort's tie-break, and the default
 * order itself. */
export function compareCanonical(a: MarkerDto, b: MarkerDto): number {
  return a.pos_samples - b.pos_samples || a.id - b.id;
}

/** SPEC-009 §2.8: "Type sorts Point < Region < Dropout." Dropout outranks Region/Point regardless
 * of `len_samples`, matching §2.1's Type precedence (dropouts are points in v1, so this and
 * "Region" never overlap in practice, but the rank order follows the spec's own wording). */
function typeRank(marker: MarkerDto): 0 | 1 | 2 {
  if (marker.kind === "dropout") {
    return 2;
  }
  return marker.len_samples > 0 ? 1 : 0;
}

/**
 * SPEC-009 §2.8's sort: `column`/`direction`, with `collator` for Name (`Intl.Collator(locale, {
 * numeric: true, sensitivity: "base" })`, created once by the caller — §4.6). Duration sorts
 * points as 0 (already true of `len_samples`). Ties **always** fall back to canonical order,
 * regardless of `direction` — reversing the sort reverses genuinely-different values, never the
 * tie-break.
 */
export function sortMarkers(
  markers: readonly MarkerDto[],
  column: MarkerSortColumn,
  direction: MarkerSortDirection,
  collator: Pick<Intl.Collator, "compare">,
): MarkerDto[] {
  const sign = direction === "asc" ? 1 : -1;
  return [...markers].sort((a, b) => {
    let cmp: number;
    switch (column) {
      case "name":
        cmp = collator.compare(a.name, b.name);
        break;
      case "start":
        cmp = a.pos_samples - b.pos_samples;
        break;
      case "end":
        cmp = a.pos_samples + a.len_samples - (b.pos_samples + b.len_samples);
        break;
      case "duration":
        cmp = a.len_samples - b.len_samples;
        break;
      case "type":
        cmp = typeRank(a) - typeRank(b);
        break;
    }
    return cmp !== 0 ? cmp * sign : compareCanonical(a, b);
  });
}

/** A half-open `[start, end)` row-index range to render (§2.8 virtualization). */
export interface VirtualRowRange {
  start: number;
  end: number;
}

/**
 * SPEC-009 §2.8/AC-15: which rows of a `totalRows`-long, fixed-`rowHeightPx` list to keep in the
 * DOM for a viewport `viewportHeightPx` tall scrolled to `scrollTopPx` — the visible rows plus
 * `overscanRows` above and below. Returns `{ start: 0, end: 0 }` for zero rows or an unknown
 * (`<= 0`) row height, so a caller can render nothing rather than divide by zero.
 */
export function virtualRowRange(
  scrollTopPx: number,
  viewportHeightPx: number,
  rowHeightPx: number,
  overscanRows: number,
  totalRows: number,
): VirtualRowRange {
  if (totalRows <= 0 || rowHeightPx <= 0) {
    return { start: 0, end: 0 };
  }
  const clampedScroll = Math.max(0, scrollTopPx);
  const firstVisible = Math.floor(clampedScroll / rowHeightPx);
  // +1: the row straddling the viewport's bottom edge is still (partially) visible.
  const visibleRows = Math.max(0, Math.ceil(viewportHeightPx / rowHeightPx)) + 1;
  const start = Math.max(0, firstVisible - overscanRows);
  const end = Math.min(totalRows, firstVisible + visibleRows + overscanRows);
  return { start, end: Math.max(start, end) };
}
