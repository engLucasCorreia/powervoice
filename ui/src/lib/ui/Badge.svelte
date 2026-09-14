<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  import type { IconName } from "./icons";

  /**
   * Small status label (H-25): "Bypassed", "3 dropouts", "Pass", "A/B: listening only". `soft`
   * (tinted background, coloured text) is the default and sits quietly in panels; `solid` is for
   * the rare state that must shout (recording, clip). Text always states the meaning — tone only
   * reinforces it.
   */
  type Tone = "neutral" | "accent" | "success" | "warning" | "danger" | "record";

  let {
    tone = "neutral",
    variant = "soft",
    icon,
    testid,
    children,
  }: {
    tone?: Tone;
    variant?: "soft" | "solid";
    icon?: IconName;
    testid?: string;
    children: Snippet;
  } = $props();
</script>

<span class="pv-badge" data-tone={tone} data-variant={variant} data-testid={testid}>
  {#if icon}<Icon name={icon} size={12} />{/if}
  {@render children()}
</span>

<style>
  .pv-badge {
    display: inline-flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-1);
    height: 20px;
    padding-inline: calc(var(--pv-space-1) + var(--pv-space-half));
    border-radius: var(--pv-radius-sm);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    font-weight: var(--pv-weight-medium);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    background: var(--pv-control-bg);
    color: var(--pv-text-secondary);
  }

  .pv-badge[data-variant="soft"][data-tone="accent"] {
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
  }
  .pv-badge[data-variant="soft"][data-tone="success"] {
    background: var(--pv-success-soft);
    color: var(--pv-success-text);
  }
  .pv-badge[data-variant="soft"][data-tone="warning"] {
    background: var(--pv-warning-soft);
    color: var(--pv-warning-text);
  }
  .pv-badge[data-variant="soft"][data-tone="danger"] {
    background: var(--pv-danger-soft);
    color: var(--pv-danger-text);
  }
  .pv-badge[data-variant="soft"][data-tone="record"] {
    background: var(--pv-record-soft);
    color: var(--pv-record-text);
  }

  .pv-badge[data-variant="solid"] {
    background: var(--pv-control-bg-selected);
    color: var(--pv-text-primary);
  }
  .pv-badge[data-variant="solid"][data-tone="accent"] {
    background: var(--pv-accent-fill);
    color: var(--pv-text-on-accent);
  }
  .pv-badge[data-variant="solid"][data-tone="success"] {
    background: var(--pv-success-fill);
    color: var(--pv-text-on-success);
  }
  .pv-badge[data-variant="solid"][data-tone="warning"] {
    background: var(--pv-warning-fill);
    color: var(--pv-text-on-warning);
  }
  .pv-badge[data-variant="solid"][data-tone="danger"] {
    background: var(--pv-danger-fill);
    color: var(--pv-text-on-danger);
  }
  .pv-badge[data-variant="solid"][data-tone="record"] {
    background: var(--pv-record-fill);
    color: var(--pv-text-on-record);
  }
</style>
