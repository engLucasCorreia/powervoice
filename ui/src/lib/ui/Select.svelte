<script lang="ts" module>
  let uid = 0;
</script>

<script lang="ts" generics="T extends string | number">
  import Icon from "./Icon.svelte";
  import type { SelectOption } from "./types";

  /**
   * Dropdown (H-25): a native `<select>` (keyboard, type-ahead and the OS popup for free) drawn
   * in the system's field style with a Lucide chevron. Option labels carry their unit
   * ("−120 dB", "48 kHz") so it's read and seen. `layout="stacked"` puts the label above (dialogs);
   * `inline` (default) beside it (toolbars). Values are typed: numbers come back as numbers.
   */
  let {
    options,
    value = $bindable(),
    label,
    hideLabel = false,
    layout = "inline",
    size = "md",
    disabled = false,
    width,
    testid,
    onchange,
  }: {
    options: SelectOption<T>[];
    value: T;
    label: string;
    hideLabel?: boolean;
    layout?: "inline" | "stacked";
    size?: "sm" | "md";
    disabled?: boolean;
    width?: string;
    testid?: string;
    onchange?: (value: T) => void;
  } = $props();

  uid += 1;
  const id = `pv-select-${uid}`;
  const selectedIndex = $derived(options.findIndex((o) => o.value === value));

  function handleChange(event: Event & { currentTarget: HTMLSelectElement }): void {
    const option = options[Number(event.currentTarget.value)];
    if (!option) {
      return;
    }
    value = option.value;
    onchange?.(option.value);
  }
</script>

<div class="pv-select" data-layout={layout} data-size={size}>
  <label for={id} class="label" class:visually-hidden={hideLabel}>{label}</label>
  <span class="field" style:width>
    <select {id} value={String(selectedIndex)} {disabled} data-testid={testid} onchange={handleChange}>
      {#each options as option, i (i)}
        <option value={String(i)} disabled={option.disabled}>{option.label}</option>
      {/each}
    </select>
    <span class="chevron" aria-hidden="true"><Icon name="chevronDown" size="sm" /></span>
  </span>
</div>

<style>
  .pv-select {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-width: 0;
    font-family: var(--pv-font-sans);
  }

  .pv-select[data-layout="stacked"] {
    flex-direction: column;
    align-items: stretch;
    gap: var(--pv-space-1);
  }

  .label {
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    color: var(--pv-text-secondary);
    white-space: nowrap;
  }

  .field {
    position: relative;
    display: inline-flex;
    min-width: 0;
  }

  select {
    appearance: none;
    width: 100%;
    min-width: 0;
    height: var(--pv-control-h-md);
    padding-inline: var(--pv-control-px-md) calc(var(--pv-control-px-md) + var(--pv-icon-sm) + var(--pv-space-1));
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-md);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    font-variant-numeric: tabular-nums;
    cursor: default;
    transition: border-color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .pv-select[data-size="sm"] select {
    height: var(--pv-control-h-sm);
    padding-inline: var(--pv-control-px-sm) calc(var(--pv-control-px-sm) + var(--pv-icon-sm) + var(--pv-space-1));
    font-size: var(--pv-text-sm);
    border-radius: var(--pv-radius-sm);
  }

  select:hover:not(:disabled) {
    background: var(--pv-control-bg-hover);
  }

  select:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
    border-color: var(--pv-accent);
  }

  select:disabled {
    border-color: var(--pv-border);
    color: var(--pv-text-disabled);
  }

  .chevron {
    position: absolute;
    top: 50%;
    right: var(--pv-space-2);
    display: inline-flex;
    transform: translateY(-50%);
    color: var(--pv-text-secondary);
    pointer-events: none;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
