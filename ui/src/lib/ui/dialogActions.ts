/**
 * Dialog footer button order per platform (H-26, design-system §12). Every dialog describes its
 * buttons by *role*; this decides where each one goes, so no dialog hand-orders its footer.
 *
 * - macOS / Linux (Apple HIG, GNOME HIG): primary action last (rightmost), Cancel just left of
 *   it, other responses left of Cancel; a destructive alternative ("Don't save", "Remove from
 *   list") sits apart on the far left.
 *   `[utility][destructive] ··· [alternate][cancel][primary]`
 * - Windows: primary action first, Cancel last, everything right-aligned together.
 *   `[utility] ··· [primary][alternate][destructive][cancel]`
 *
 * `utility` is a button that isn't an answer to the dialog (Export's "ACX preset"): always far
 * left. Buttons that share a role keep the order the dialog gave them.
 */
import type { Platform } from "./platform";

export type DialogActionRole = "primary" | "cancel" | "alternate" | "destructive" | "utility";

export interface OrderedActions<T> {
  /** Left of the flexible gap. */
  leading: T[];
  /** Right of the flexible gap. */
  trailing: T[];
}

const TRAILING_ORDER: Record<Platform, readonly DialogActionRole[]> = {
  windows: ["primary", "alternate", "destructive", "cancel"],
  mac: ["alternate", "cancel", "primary"],
  linux: ["alternate", "cancel", "primary"],
};

const LEADING_ORDER: Record<Platform, readonly DialogActionRole[]> = {
  windows: ["utility"],
  mac: ["utility", "destructive"],
  linux: ["utility", "destructive"],
};

export function orderDialogActions<T extends { role: DialogActionRole }>(
  actions: readonly T[],
  platform: Platform,
): OrderedActions<T> {
  const pick = (roles: readonly DialogActionRole[]): T[] =>
    roles.flatMap((role) => actions.filter((action) => action.role === role));
  return { leading: pick(LEADING_ORDER[platform]), trailing: pick(TRAILING_ORDER[platform]) };
}
