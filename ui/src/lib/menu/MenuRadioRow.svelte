<script lang="ts">
  /** One `role="menuitemradio"` row (H-19): View → Renderer's Automatic/WebGL2/Canvas2D choice. */
  let {
    label,
    checked,
    disabled = false,
    testid,
    onSelect,
  }: {
    label: string;
    checked: boolean;
    disabled?: boolean;
    testid?: string;
    onSelect: () => void;
  } = $props();
</script>

<button
  type="button"
  role="menuitemradio"
  tabindex="-1"
  aria-checked={checked}
  data-testid={testid}
  disabled={disabled}
  onclick={onSelect}
>
  <span class="check" aria-hidden="true">{checked ? "●" : ""}</span>
  <span class="label">{label}</span>
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
    font-size: 0.7em;
  }

  .label {
    flex: 1;
    white-space: nowrap;
  }
</style>
