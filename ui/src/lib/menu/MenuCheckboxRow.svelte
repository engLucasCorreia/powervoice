<script lang="ts">
  import Icon from "../ui/Icon.svelte";
  /** One `role="menuitemcheckbox"` row (H-19): View → Spectral/Analyzer. Closes its menu on
   * toggle, same as a plain item — the checkmark shows the new state next time it's opened. */
  let {
    label,
    checked,
    shortcut,
    disabled = false,
    testid,
    onToggle,
  }: {
    label: string;
    checked: boolean;
    shortcut?: string;
    disabled?: boolean;
    testid?: string;
    onToggle: () => void;
  } = $props();
</script>

<button
  type="button"
  role="menuitemcheckbox"
  tabindex="-1"
  aria-checked={checked}
  data-testid={testid}
  disabled={disabled}
  onclick={onToggle}
>
  <span class="check" aria-hidden="true">{#if checked}<Icon name="check" size="sm" />{/if}</span>
  <span class="label">{label}</span>
  {#if shortcut}
    <span class="shortcut">{shortcut}</span>
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

  .check {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--pv-icon-sm);
    color: var(--pv-accent-text);
  }

</style>
