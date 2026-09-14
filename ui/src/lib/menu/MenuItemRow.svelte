<script lang="ts">
  import Icon from "../ui/Icon.svelte";
  /**
   * One plain `role="menuitem"` row (H-19): a label, an optional right-aligned shortcut label, and
   * an optional trailing `▸` for a submenu trigger (`isSubmenuTrigger`, paired with
   * `aria-haspopup`/`aria-expanded`). Every top-level menu (`DocumentMenu`, `EditMenu`, `ViewMenu`,
   * `EffectsMenu`, `HelpMenu`) uses this for its non-checkable items, so a11y markup and styling
   * stay in one place.
   */
  let {
    label,
    shortcut,
    disabled = false,
    muted = false,
    testid,
    isSubmenuTrigger = false,
    expanded = false,
    onSelect,
  }: {
    label: string;
    shortcut?: string;
    disabled?: boolean;
    /** H-15: dims the label without disabling the row — a missing Recent Files entry is still
     * clickable (it opens the "can't be found" dialog), just shown as unavailable. */
    muted?: boolean;
    testid?: string;
    isSubmenuTrigger?: boolean;
    expanded?: boolean;
    onSelect: () => void;
  } = $props();
</script>

<button
  type="button"
  role="menuitem"
  tabindex="-1"
  data-testid={testid}
  disabled={disabled}
  class:muted
  aria-haspopup={isSubmenuTrigger ? "menu" : undefined}
  aria-expanded={isSubmenuTrigger ? expanded : undefined}
  onclick={onSelect}
>
  <span class="label">{label}</span>
  {#if shortcut}
    <span class="shortcut">{shortcut}</span>
  {/if}
  {#if isSubmenuTrigger}
    <span class="arrow" aria-hidden="true"><Icon name="chevronRight" size="sm" /></span>
  {/if}
</button>

<style>
  /* H-25: calm 24 px menu rows — a neutral highlight (not a full accent fill), quiet shortcuts. */
  button {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    width: 100%;
    min-height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: none;
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    text-align: left;
    cursor: default;
  }

  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--pv-control-bg-active);
    outline: none;
  }

  button:disabled {
    color: var(--pv-text-disabled);
  }

  .label {
    flex: 1;
    white-space: nowrap;
  }

  .shortcut {
    margin-left: var(--pv-space-6);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  button:disabled .shortcut {
    color: var(--pv-text-disabled);
  }

  button.muted .label {
    color: var(--pv-text-tertiary);
  }

  .arrow {
    display: inline-flex;
    color: var(--pv-text-tertiary);
  }
</style>
