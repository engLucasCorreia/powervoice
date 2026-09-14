<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  /**
   * Two-state text button (H-25): Spectral, A/B, Peak hold in a toolbar. `aria-pressed`, accent-
   * tinted when on. For one-of-many choices use SegmentedControl; for settings use Toggle.
   */
  let {
    pressed = $bindable(false),
    icon,
    size = "md",
    disabled = false,
    testid,
    onchange,
    children,
  }: {
    pressed?: boolean;
    icon?: IconName;
    size?: "sm" | "md" | "lg";
    disabled?: boolean;
    testid?: string;
    onchange?: (pressed: boolean) => void;
    children: Snippet;
  } = $props();

  function toggle(): void {
    pressed = !pressed;
    onchange?.(pressed);
  }
</script>

<button
  type="button"
  class="pv-toggle-button"
  data-size={size}
  data-testid={testid}
  aria-pressed={pressed}
  {disabled}
  onclick={toggle}
>
  {#if icon}<Icon name={icon} size={size === "sm" ? "sm" : "md"} />{/if}
  {@render children()}
</button>

<style>
  .pv-toggle-button {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: calc(var(--pv-space-1) + var(--pv-space-half));
    height: var(--pv-control-h-md);
    padding-inline: var(--pv-control-px-md);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-control-bg);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    font-weight: var(--pv-weight-medium);
    white-space: nowrap;
    cursor: default;
    transition:
      background-color var(--pv-duration-fast) var(--pv-ease-standard),
      border-color var(--pv-duration-fast) var(--pv-ease-standard),
      color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .pv-toggle-button[data-size="sm"] {
    height: var(--pv-control-h-sm);
    padding-inline: var(--pv-control-px-sm);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    border-radius: var(--pv-radius-sm);
  }

  .pv-toggle-button[data-size="lg"] {
    height: var(--pv-control-h-lg);
    padding-inline: var(--pv-control-px-lg);
  }

  .pv-toggle-button:hover:not(:disabled) {
    background: var(--pv-control-bg-hover);
    color: var(--pv-text-primary);
  }

  .pv-toggle-button:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  .pv-toggle-button[aria-pressed="true"],
  .pv-toggle-button[aria-pressed="true"]:hover:not(:disabled) {
    background: var(--pv-accent-soft);
    border-color: var(--pv-accent);
    color: var(--pv-accent-text);
  }

  .pv-toggle-button:disabled {
    border-color: var(--pv-border-subtle);
    color: var(--pv-text-disabled);
  }
</style>
