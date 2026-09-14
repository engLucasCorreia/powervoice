<script lang="ts">
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
    <span class="arrow" aria-hidden="true">▸</span>
  {/if}
</button>

<style>
  button {
    display: flex;
    align-items: center;
    width: 100%;
    gap: 1.5rem;
    background: none;
    color: var(--text-primary);
    border: none;
    border-radius: 4px;
    padding: 0.35rem 0.6rem;
    text-align: left;
    font-size: inherit;
  }

  button:hover:not(:disabled),
  button:focus-visible {
    background: var(--accent);
    color: var(--text-on-accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }

  button.muted .label {
    color: var(--text-disabled);
  }

  .label {
    flex: 1;
    white-space: nowrap;
  }

  .shortcut {
    color: var(--text-secondary);
    font-size: 0.9em;
  }

  button:hover:not(:disabled) .shortcut,
  button:focus-visible .shortcut {
    color: inherit;
  }

  .arrow {
    color: var(--text-secondary);
  }

  button:hover:not(:disabled) .arrow,
  button:focus-visible .arrow {
    color: inherit;
  }
</style>
