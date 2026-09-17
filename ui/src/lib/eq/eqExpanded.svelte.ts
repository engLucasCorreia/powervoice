/**
 * The EQ graph's expanded-view store (H-84, SPEC-015 §2.6.1 "Expanded view"): which rack slot (by
 * stable `uid`, since a slot's array index can change under `rack_move`, exactly like
 * `slotTelemetry`'s own keying) currently has its EQ expanded, plus the floating window's
 * position/size, kept "in the view state" across opens for the session (not persisted to
 * `Settings` — SPEC-015 doesn't ask for that, and the Spectrum Inspector's own `rect` does the
 * same: local, session-lived state that survives close/reopen but not a reload).
 *
 * One global instance, mirroring how `SpectrumInspector`/`diagnosticsState()` is opened from
 * anywhere and mounted once at `App.svelte` level (H-84 ticket: "reachable ... consistent with
 * how the Spectrum Inspector opens").
 */

export const EQ_EXPANDED_DEFAULT_W = 900;
export const EQ_EXPANDED_DEFAULT_H = 400;
export const EQ_EXPANDED_MIN_W = 480;
export const EQ_EXPANDED_MIN_H = 240;

export interface EqExpandedRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

let openUid = $state<number | null>(null);
let rect = $state<EqExpandedRect | null>(null);
let placed = false;

export function eqExpandedState(): { readonly openUid: number | null; readonly rect: EqExpandedRect | null } {
  return {
    get openUid() {
      return openUid;
    },
    get rect() {
      return rect;
    },
  };
}

/** Opens (or re-focuses) slot `uid`'s expanded EQ view. */
export function openEqExpanded(uid: number): void {
  openUid = uid;
}

/** Closes the expanded view (its close button, Esc, or the slot disappearing). */
export function closeEqExpanded(): void {
  openUid = null;
}

/** Centres the window the first time it's ever placed this session; later opens keep the last
 * position/size (mirrors `SpectrumInspector.svelte`'s own `placed` guard). */
export function placeEqExpandedIfNeeded(viewportW: number, viewportH: number): void {
  if (placed) {
    return;
  }
  placed = true;
  const w = Math.max(EQ_EXPANDED_MIN_W, Math.min(EQ_EXPANDED_DEFAULT_W, viewportW - 48));
  const h = Math.max(EQ_EXPANDED_MIN_H, Math.min(EQ_EXPANDED_DEFAULT_H, viewportH - 96));
  rect = { x: Math.max(16, (viewportW - w) / 2), y: Math.max(40, (viewportH - h) / 2), w, h };
}

export function setEqExpandedRect(next: EqExpandedRect): void {
  rect = next;
}

/** Test/teardown helper. */
export function resetEqExpandedForTest(): void {
  openUid = null;
  rect = null;
  placed = false;
}
