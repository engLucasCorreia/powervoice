/** Public prop types of the component kit (H-25), importable from `.ts` and `.svelte` alike. */
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
