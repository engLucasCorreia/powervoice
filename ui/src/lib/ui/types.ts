/** Public prop types of the component kit (H-25), importable from `.ts` and `.svelte` alike. */
import type { DialogActionRole } from "./dialogActions";
import type { IconName } from "./icons";

export interface SegmentOption<V extends string = string> {
  value: V;
  label: string;
  icon?: IconName;
  /** Draw only the icon; `label` still names the segment for assistive tech. */
  iconOnly?: boolean;
  disabled?: boolean;
  testid?: string;
}

export interface SelectOption<V extends string | number = string> {
  value: V;
  /** Carries its unit ("−120 dB", "48 kHz"). */
  label: string;
  disabled?: boolean;
}

export interface TabItem<V extends string = string> {
  id: V;
  label: string;
  icon?: IconName;
  badge?: string;
  disabled?: boolean;
}

/** What Tooltip's `children` snippet receives to spread on its focusable trigger. */
export type TooltipTriggerProps = { "aria-describedby"?: string };

/**
 * One dialog footer button (H-26). Dialogs list their buttons by role; `Dialog` orders them for
 * the platform (`dialogActions.ts`), so the primary action lands first on Windows and last on
 * macOS/Linux without any dialog hand-ordering its footer.
 */
export interface DialogAction {
  label: string;
  role: DialogActionRole;
  onclick: () => void;
  testid?: string;
  icon?: IconName;
  /** Defaults from the role: primary → primary, destructive/utility → ghost, others → secondary. */
  variant?: "primary" | "secondary" | "ghost" | "danger" | "record";
  disabled?: boolean;
  loading?: boolean;
  title?: string;
}

/** What a popover or menu is anchored to: an element, or a point (a context menu opens at the
 * pointer). */
export type PopoverAnchor = HTMLElement | { x: number; y: number };
