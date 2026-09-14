<script lang="ts">
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
  <span class="check" aria-hidden="true">{checked ? "✓" : ""}</span>
  <span class="label">{label}</span>
  {#if shortcut}
    <span class="shortcut">{shortcut}</span>
  {/if}
</button>

<style>
  button {
    display: flex;
    align-items: center;
    width: 100%;
    gap: 0.5rem;
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

  .check {
    width: 1em;
    text-align: center;
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
</style>
