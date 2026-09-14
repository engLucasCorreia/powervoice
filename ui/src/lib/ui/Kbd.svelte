<script lang="ts">
  import { splitShortcut, usesSeparators } from "./kbd";

  /**
   * Shortcut chip (H-25): pass the label from `shortcutLabelForAction()` — never hand-type a
   * shortcut. Nested `<kbd>` per the HTML spec for key combinations; `+` separators stay in the
   * text (screen readers read "Ctrl+Shift+Z") but are drawn quietly.
   */
  let { keys, size = "sm" }: { keys: string; size?: "xs" | "sm" } = $props();

  const parts = $derived(splitShortcut(keys));
  const separated = $derived(usesSeparators(keys));
</script>

<kbd class="pv-kbd" data-size={size}>
  {#each parts as part, i (i)}{#if i > 0 && separated}<span class="sep">+</span>{/if}<kbd
      class="key">{part}</kbd
    >{/each}
</kbd>

<style>
  .pv-kbd {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-half);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    color: var(--pv-text-secondary);
    white-space: nowrap;
  }

  .key {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 18px;
    padding-inline: var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border);
    border-bottom-width: 2px;
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-raised);
    font: inherit;
    font-variant-numeric: tabular-nums;
  }

  .pv-kbd[data-size="xs"] .key {
    min-width: 16px;
    height: 16px;
    border-bottom-width: 1px;
  }

  .sep {
    color: var(--pv-text-tertiary);
  }
</style>
