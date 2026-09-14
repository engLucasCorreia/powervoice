<script lang="ts" module>
  let uid = 0;
</script>

<script lang="ts">
  /**
   * On/off switch (H-25, `role="switch"`) for settings that apply immediately: Peak hold, Hear
   * original, Pre-roll at cursor. Replaces bare checkboxes in panels and popovers (checkboxes
   * stay for multi-select lists and form-style dialogs that apply on OK). The visible label names
   * the switch (`aria-labelledby`) and clicking it toggles.
   */
  let {
    checked = $bindable(false),
    label,
    description,
    size = "md",
    disabled = false,
    testid,
    onchange,
  }: {
    checked?: boolean;
    label: string;
    description?: string;
    size?: "sm" | "md";
    disabled?: boolean;
    testid?: string;
    onchange?: (checked: boolean) => void;
  } = $props();

  uid += 1;
  const base = `pv-switch-${uid}`;

  function toggle(): void {
    checked = !checked;
    onchange?.(checked);
  }
</script>

<div class="pv-switch" data-size={size} data-disabled={disabled ? "true" : undefined}>
  <button
    type="button"
    role="switch"
    id="{base}-control"
    class="track"
    aria-checked={checked}
    aria-labelledby="{base}-label"
    aria-describedby={description ? `${base}-desc` : undefined}
    data-testid={testid}
    {disabled}
    onclick={toggle}
  >
    <span class="knob"></span>
  </button>
  <span class="text">
    <label id="{base}-label" for="{base}-control">{label}</label>
    {#if description}<span id="{base}-desc" class="description">{description}</span>{/if}
  </span>
</div>

<style>
  .pv-switch {
    display: inline-flex;
    align-items: flex-start;
    gap: var(--pv-space-2);
    font-family: var(--pv-font-sans);
  }

  .track {
    position: relative;
    flex: none;
    width: 28px;
    height: 16px;
    margin-top: 1px;
    padding: 0;
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-full);
    background: var(--pv-control-track);
    cursor: default;
    transition:
      background-color var(--pv-duration-fast) var(--pv-ease-standard),
      border-color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 10px;
    height: 10px;
    border-radius: var(--pv-radius-full);
    background: var(--pv-text-secondary);
    transition:
      transform var(--pv-duration-base) var(--pv-ease-standard),
      background-color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  .track[aria-checked="true"] {
    background: var(--pv-accent);
    border-color: var(--pv-accent);
  }

  .track[aria-checked="true"] .knob {
    transform: translateX(12px);
    background: var(--pv-text-on-accent);
  }

  .track:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  .track:disabled {
    border-color: var(--pv-border);
    opacity: 0.5;
  }

  .text {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-half);
  }

  label {
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    color: var(--pv-text-primary);
    cursor: default;
  }

  .pv-switch[data-size="sm"] label {
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .pv-switch[data-disabled="true"] label {
    color: var(--pv-text-disabled);
  }

  .description {
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    color: var(--pv-text-secondary);
  }
</style>
