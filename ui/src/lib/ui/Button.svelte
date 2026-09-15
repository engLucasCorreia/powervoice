<script lang="ts">
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  /**
   * Text button (H-25). Variants: `primary` (the one main action in a dialog/panel — at most one
   * per group), `secondary` (default), `ghost` (toolbar/panel actions that shouldn't compete with
   * content), `danger` (destructive confirmation only). Sizes: sm 24 px, md 28 px, lg 32 px.
   * `loading` keeps focus (aria-disabled, not native disabled) and swallows clicks.
   * `variant="record"` is the transport's Record key: a red lamp at rest, filled red while
   * `active` (recording). `aria-pressed="true"` (passed through) draws the accent "on" state.
   */
  type Variant = "primary" | "secondary" | "ghost" | "danger" | "record";
  type Size = "sm" | "md" | "lg";

  let {
    variant = "secondary",
    size = "md",
    icon,
    iconEnd,
    loading = false,
    active = false,
    disabled = false,
    fullWidth = false,
    type = "button",
    testid,
    element = $bindable(),
    onclick,
    children,
    ...rest
  }: {
    variant?: Variant;
    size?: Size;
    icon?: IconName;
    iconEnd?: IconName;
    loading?: boolean;
    active?: boolean;
    disabled?: boolean;
    fullWidth?: boolean;
    type?: "button" | "submit" | "reset";
    testid?: string;
    /** The rendered `<button>` (anchor for a menu, focus return). */
    element?: HTMLButtonElement;
    onclick?: (event: MouseEvent) => void;
    children?: Snippet;
  } & Omit<HTMLButtonAttributes, "type" | "disabled" | "children" | "onclick"> = $props();

  const iconSize = $derived(size === "sm" ? "sm" : "md");

  function handleClick(event: MouseEvent): void {
    if (loading) {
      event.preventDefault();
      return;
    }
    onclick?.(event);
  }
</script>

<button
  {...rest}
  bind:this={element}
  {type}
  class="pv-button"
  class:full={fullWidth}
  data-variant={variant}
  data-size={size}
  data-active={active ? "true" : undefined}
  data-testid={testid}
  {disabled}
  aria-busy={loading ? "true" : undefined}
  aria-disabled={loading ? "true" : undefined}
  onclick={handleClick}
>
  {#if loading}
    <span class="spinner"><Icon name="loading" size={iconSize} /></span>
  {:else if icon}
    <Icon name={icon} size={iconSize} filled={variant === "record" && icon === "record"} />
  {/if}
  {#if children}
    <span class="label">{@render children()}</span>
  {/if}
  {#if iconEnd}
    <Icon name={iconEnd} size={iconSize} />
  {/if}
</button>

<style>
  .pv-button {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    gap: calc(var(--pv-space-1) + var(--pv-space-half));
    height: var(--pv-control-h-md);
    padding-inline: var(--pv-control-px-md);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    font-weight: var(--pv-weight-medium);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    user-select: none;
    cursor: default;
    transition:
      background-color var(--pv-duration-fast) var(--pv-ease-standard),
      border-color var(--pv-duration-fast) var(--pv-ease-standard),
      color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .pv-button[data-size="sm"] {
    height: var(--pv-control-h-sm);
    padding-inline: var(--pv-control-px-sm);
    gap: var(--pv-space-1);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .pv-button[data-size="lg"] {
    height: var(--pv-control-h-lg);
    padding-inline: var(--pv-control-px-lg);
  }

  .pv-button.full {
    width: 100%;
  }

  .pv-button:hover:not(:disabled) {
    background: var(--pv-control-bg-hover);
    border-color: var(--pv-border-strong);
  }

  .pv-button:active:not(:disabled) {
    background: var(--pv-control-bg-active);
  }

  .pv-button:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  /* Primary: the one filled accent button per group. */
  .pv-button[data-variant="primary"] {
    background: var(--pv-accent-fill);
    border-color: var(--pv-accent-fill);
    color: var(--pv-text-on-accent);
  }

  .pv-button[data-variant="primary"]:hover:not(:disabled) {
    background: var(--pv-accent-fill-hover);
    border-color: var(--pv-accent-fill-hover);
  }

  .pv-button[data-variant="primary"]:active:not(:disabled) {
    background: var(--pv-accent-fill-active);
    border-color: var(--pv-accent-fill-active);
  }

  /* Ghost: recedes until hovered — toolbars and panel headers. */
  .pv-button[data-variant="ghost"] {
    background: transparent;
    border-color: transparent;
    color: var(--pv-text-secondary);
  }

  .pv-button[data-variant="ghost"]:hover:not(:disabled) {
    background: var(--pv-control-bg-hover);
    border-color: transparent;
    color: var(--pv-text-primary);
  }

  .pv-button[data-variant="danger"] {
    background: var(--pv-danger-fill);
    border-color: var(--pv-danger-fill);
    color: var(--pv-text-on-danger);
  }

  .pv-button[data-variant="danger"]:hover:not(:disabled) {
    background: var(--pv-danger-fill-hover);
    border-color: var(--pv-danger-fill-hover);
  }

  /* Pressed toggles (aria-pressed passed through): the accent "on" state. */
  .pv-button[aria-pressed="true"],
  .pv-button[aria-pressed="true"]:hover:not(:disabled) {
    background: var(--pv-accent-soft);
    border-color: var(--pv-accent);
    color: var(--pv-accent-text);
  }

  /* The Record key: red lamp at rest, a solid red key on air. */
  .pv-button[data-variant="record"] :global(svg) {
    color: var(--pv-record);
  }

  .pv-button[data-variant="record"][data-active="true"] {
    background: var(--pv-record-fill);
    border-color: var(--pv-record-fill);
    color: var(--pv-text-on-record);
  }

  .pv-button[data-variant="record"][data-active="true"]:hover:not(:disabled) {
    background: var(--pv-record-fill-hover);
    border-color: var(--pv-record-fill-hover);
  }

  .pv-button[data-variant="record"][data-active="true"] :global(svg) {
    color: currentColor;
  }

  .pv-button[data-variant="record"]:disabled :global(svg) {
    color: var(--pv-text-disabled);
  }

  .pv-button:disabled {
    background: var(--pv-control-bg);
    border-color: var(--pv-border-subtle);
    color: var(--pv-text-disabled);
  }

  .pv-button[data-variant="ghost"]:disabled {
    background: transparent;
    border-color: transparent;
  }

  .pv-button[aria-busy="true"] {
    cursor: progress;
  }

  .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .spinner {
    display: inline-flex;
    animation: pv-spin 0.9s linear infinite;
  }

  @keyframes pv-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation-duration: 2.4s;
    }
  }
</style>
