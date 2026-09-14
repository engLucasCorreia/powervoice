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
  <span class="check" aria-hidden="true">{#if checked}<span class="dot"></span>{/if}</span>
  <span class="label">{label}</span>
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



  .check {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--pv-icon-sm);
    color: var(--pv-accent-text);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--pv-radius-full);
    background: currentColor;
  }
</style>
