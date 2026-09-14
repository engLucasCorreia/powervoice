<script lang="ts" generics="T extends string">
  import Icon from "./Icon.svelte";
  import { nextRovingIndex } from "./roving";
  import type { SegmentOption } from "./types";

  /**
   * One-of-many choice drawn as a pill track (H-25, WAI-ARIA radio group): Fast/Medium/Slow,
   * dB/%, Processed/Source, Insert/Overwrite. Roving tabindex; arrows move AND select (radio
   * semantics), Home/End jump, disabled segments are skipped. For views that swap content use
   * Tabs instead.
   */
  let {
    options,
    value = $bindable(),
    label,
    size = "md",
    disabled = false,
    testid,
    onchange,
  }: {
    options: SegmentOption<T>[];
    value: T;
    label: string;
    size?: "sm" | "md";
    disabled?: boolean;
    testid?: string;
    onchange?: (value: T) => void;
  } = $props();

  let root: HTMLElement | undefined = $state();

  const disabledFlags = $derived(options.map((o) => disabled || o.disabled === true));
  const selectedIndex = $derived(options.findIndex((o) => o.value === value));
  const tabStop = $derived(
    selectedIndex >= 0 && !disabledFlags[selectedIndex]
      ? selectedIndex
      : disabledFlags.findIndex((d) => !d),
  );

  function select(index: number): void {
    const option = options[index];
    if (!option || disabledFlags[index]) {
      return;
    }
    if (option.value !== value) {
      value = option.value;
      onchange?.(option.value);
    }
  }

  function onKeydown(event: KeyboardEvent, index: number): void {
    const next = nextRovingIndex(index, event.key, disabledFlags);
    if (next === null) {
      return;
    }
    event.preventDefault();
    select(next);
    root?.querySelectorAll<HTMLButtonElement>('[role="radio"]')[next]?.focus();
  }
</script>

<div
  bind:this={root}
  class="pv-segmented"
  role="radiogroup"
  aria-label={label}
  aria-disabled={disabled ? "true" : undefined}
  data-size={size}
  data-testid={testid}
>
  {#each options as option, i (option.value)}
    <button
      type="button"
      role="radio"
      class="segment"
      aria-checked={option.value === value}
      aria-label={option.iconOnly ? option.label : undefined}
      tabindex={i === tabStop ? 0 : -1}
      disabled={disabledFlags[i]}
      data-testid={option.testid}
      onclick={() => select(i)}
      onkeydown={(e) => onKeydown(e, i)}
    >
      {#if option.icon}<Icon name={option.icon} size={size === "sm" ? "sm" : "md"} />{/if}
      {#if !option.iconOnly}<span>{option.label}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .pv-segmented {
    display: inline-flex;
    flex: none;
    align-items: stretch;
    gap: var(--pv-space-half);
    height: var(--pv-control-h-md);
    padding: var(--pv-space-half);
    border-radius: var(--pv-radius-md);
    background: var(--pv-control-track);
  }

  .pv-segmented[data-size="sm"] {
    height: var(--pv-control-h-sm);
    border-radius: var(--pv-radius-sm);
  }

  .segment {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--pv-space-1);
    min-width: var(--pv-hit-min);
    padding-inline: var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    font-weight: var(--pv-weight-medium);
    white-space: nowrap;
    cursor: default;
    transition:
      background-color var(--pv-duration-fast) var(--pv-ease-standard),
      color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .pv-segmented[data-size="sm"] .segment {
    padding-inline: calc(var(--pv-space-1) + var(--pv-space-half));
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    border-radius: 3px;
  }

  .segment:hover:not(:disabled) {
    color: var(--pv-text-primary);
  }

  .segment[aria-checked="true"] {
    background: var(--pv-control-bg-selected);
    color: var(--pv-text-primary);
    box-shadow: var(--pv-shadow-1);
  }

  .segment:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .segment:disabled {
    color: var(--pv-text-disabled);
  }
</style>
