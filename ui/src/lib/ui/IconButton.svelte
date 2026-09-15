<script lang="ts">
  import type { HTMLButtonAttributes } from "svelte/elements";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";
  import { toAriaKeyShortcuts } from "./kbd";
  import Tooltip from "./Tooltip.svelte";
  import type { TooltipTriggerProps } from "./types";

  /**
   * Square icon button with tooltip (H-25) — secondary actions in toolbars and panel headers.
   * `label` is required: it's the accessible name AND the tooltip. `shortcut` (from
   * `shortcutLabelForAction`) shows in the tooltip and sets `aria-keyshortcuts`. `pressed` makes
   * it a toggle (aria-pressed). `variant="record"` is the on-air button: red dot idle, filled red
   * while `active`. Sizes: sm 24, md 28, lg 32 px.
   */
  type Variant = "ghost" | "secondary" | "primary" | "record";
  type Size = "sm" | "md" | "lg";

  let {
    icon,
    label,
    shortcut,
    variant = "ghost",
    size = "md",
    pressed,
    active = false,
    filledIcon,
    disabled = false,
    tooltip = true,
    tooltipPlacement = "bottom",
    testid,
    element = $bindable(),
    onclick,
    ...rest
  }: {
    icon: IconName;
    label: string;
    shortcut?: string;
    variant?: Variant;
    size?: Size;
    pressed?: boolean;
    active?: boolean;
    filledIcon?: boolean;
    disabled?: boolean;
    tooltip?: boolean;
    tooltipPlacement?: "top" | "bottom";
    testid?: string;
    /** The rendered `<button>` (anchor for a menu, focus return). */
    element?: HTMLButtonElement;
    onclick?: (event: MouseEvent) => void;
  } & Omit<HTMLButtonAttributes, "type" | "disabled" | "children" | "onclick" | "aria-label"> =
    $props();

  const iconSize = $derived(size === "lg" ? "lg" : size === "sm" ? "sm" : "md");
  const filled = $derived(filledIcon ?? (variant === "record" && icon === "record"));
</script>

{#snippet button(trigger: TooltipTriggerProps)}
  <button
    {...rest}
    {...trigger}
    bind:this={element}
    type="button"
    class="pv-icon-button"
    data-variant={variant}
    data-size={size}
    data-active={active ? "true" : undefined}
    data-testid={testid}
    aria-label={label}
    aria-pressed={pressed === undefined ? undefined : pressed}
    aria-keyshortcuts={shortcut ? toAriaKeyShortcuts(shortcut) : undefined}
    {disabled}
    {onclick}
  >
    <Icon name={icon} size={iconSize} {filled} />
  </button>
{/snippet}

{#if tooltip}
  <Tooltip text={label} {shortcut} placement={tooltipPlacement} describe={false} {disabled}>
    {#snippet children(trigger)}{@render button(trigger)}{/snippet}
  </Tooltip>
{:else}
  {@render button({})}
{/if}

<style>
  .pv-icon-button {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--pv-control-h-md);
    height: var(--pv-control-h-md);
    padding: 0;
    border: var(--pv-border-width) solid transparent;
    border-radius: var(--pv-radius-md);
    background: transparent;
    color: var(--pv-text-secondary);
    cursor: default;
    transition:
      background-color var(--pv-duration-fast) var(--pv-ease-standard),
      color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .pv-icon-button[data-size="sm"] {
    width: var(--pv-control-h-sm);
    height: var(--pv-control-h-sm);
    border-radius: var(--pv-radius-sm);
  }

  .pv-icon-button[data-size="lg"] {
    width: var(--pv-control-h-lg);
    height: var(--pv-control-h-lg);
  }

  .pv-icon-button:hover:not(:disabled) {
    background: var(--pv-control-bg-hover);
    color: var(--pv-text-primary);
  }

  .pv-icon-button:active:not(:disabled) {
    background: var(--pv-control-bg-active);
  }

  .pv-icon-button:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  .pv-icon-button[aria-pressed="true"] {
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
  }

  .pv-icon-button[aria-pressed="true"]:hover:not(:disabled) {
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
  }

  .pv-icon-button[data-variant="secondary"] {
    background: var(--pv-control-bg);
    border-color: var(--pv-border);
    color: var(--pv-text-primary);
  }

  .pv-icon-button[data-variant="primary"] {
    background: var(--pv-accent-fill);
    color: var(--pv-text-on-accent);
  }

  .pv-icon-button[data-variant="primary"]:hover:not(:disabled) {
    background: var(--pv-accent-fill-hover);
    color: var(--pv-text-on-accent);
  }

  /* The on-air button: a red lamp at rest, a solid red key while recording. */
  .pv-icon-button[data-variant="record"] {
    color: var(--pv-record);
  }

  .pv-icon-button[data-variant="record"]:hover:not(:disabled) {
    color: var(--pv-record);
  }

  .pv-icon-button[data-variant="record"][data-active="true"] {
    background: var(--pv-record-fill);
    color: var(--pv-text-on-record);
  }

  .pv-icon-button[data-variant="record"][data-active="true"]:hover:not(:disabled) {
    background: var(--pv-record-fill-hover);
    color: var(--pv-text-on-record);
  }

  .pv-icon-button:disabled {
    background: transparent;
    border-color: transparent;
    color: var(--pv-text-disabled);
  }
</style>
