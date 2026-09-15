/**
 * Finding a tour step's target in the live DOM (T-709). Steps name `data-tour="…"` anchors; the
 * first one that is actually on screen wins. A hidden (`[hidden]` ancestor, a collapsed panel) or
 * zero-size anchor counts as missing, so the step falls back to the next candidate or a centred
 * card.
 */
import type { AnchorRect } from "../ui/placement";

export function findTourTarget(names: readonly string[] | undefined, root: ParentNode = document): HTMLElement | null {
  for (const name of names ?? []) {
    const el = root.querySelector<HTMLElement>(`[data-tour="${name}"]`);
    if (el && isShown(el)) {
      return el;
    }
  }
  return null;
}

function isShown(el: HTMLElement): boolean {
  if (el.closest("[hidden]")) {
    return false;
  }
  const rect = el.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
}

/**
 * A modal dialog the tour must step aside for: any `aria-modal="true"` element that doesn't
 * contain the step's target (the plugin-manager tour points *into* its dialog, so that one is
 * fine). While one is open the tour pauses — the user answers the dialog (New Recording, Audio
 * Devices, crash recovery) and the tour comes back.
 */
export function blockingModal(target: Element | null, card: Element | null, root: ParentNode = document): boolean {
  for (const modal of root.querySelectorAll('[aria-modal="true"]')) {
    if (card?.contains(modal)) {
      continue;
    }
    if (!target || !modal.contains(target)) {
      return true;
    }
  }
  return false;
}

const CLIPS = /(auto|scroll|hidden|clip)/;

/**
 * The part of `el` that is actually on screen: its box cut by every scrolling/clipping ancestor
 * (the rack's module list is taller than its panel — the spotlight stops at the panel's edge).
 * `null` when nothing of it shows.
 */
export function visibleRect(el: HTMLElement): AnchorRect | null {
  const r = el.getBoundingClientRect();
  let left = r.left;
  let top = r.top;
  let right = r.left + r.width;
  let bottom = r.top + r.height;
  for (let p = el.parentElement; p && p !== document.body && p !== document.documentElement; p = p.parentElement) {
    const style = getComputedStyle(p);
    if (!CLIPS.test(`${style.overflowX} ${style.overflowY}`)) {
      continue;
    }
    const c = p.getBoundingClientRect();
    left = Math.max(left, c.left);
    top = Math.max(top, c.top);
    right = Math.min(right, c.left + c.width);
    bottom = Math.min(bottom, c.top + c.height);
  }
  if (right - left <= 0 || bottom - top <= 0) {
    return null;
  }
  return { left, top, width: right - left, height: bottom - top };
}
