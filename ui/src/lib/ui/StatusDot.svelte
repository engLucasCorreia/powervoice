<script lang="ts">
  /**
   * Status lamp (H-25): device connected, input armed, recording. `label` is required — the dot
   * never carries meaning by colour alone. Dot-only it's an image named by `label`; with
   * `showLabel` the text is visible and the dot decorative. `pulse` (recording only) breathes
   * slowly and stops under prefers-reduced-motion.
   */
  type Tone = "neutral" | "accent" | "success" | "warning" | "danger" | "record";

  let {
    tone = "neutral",
    label,
    showLabel = false,
    pulse = false,
    testid,
  }: {
    tone?: Tone;
    label: string;
    showLabel?: boolean;
    pulse?: boolean;
    testid?: string;
  } = $props();
</script>

{#if showLabel}
  <span class="pv-status" data-tone={tone} data-pulse={pulse ? "true" : undefined} data-testid={testid}>
    <span class="dot" aria-hidden="true"></span>
    <span class="text">{label}</span>
  </span>
{:else}
  <span
    class="pv-status"
    role="img"
    aria-label={label}
    data-tone={tone}
    data-pulse={pulse ? "true" : undefined}
    data-testid={testid}
  >
    <span class="dot"></span>
  </span>
{/if}

<style>
  .pv-status {
    display: inline-flex;
    align-items: center;
    gap: calc(var(--pv-space-1) + var(--pv-space-half));
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    color: var(--pv-text-secondary);
  }

  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: var(--pv-radius-full);
    background: var(--pv-text-tertiary);
  }

  .pv-status[data-tone="accent"] .dot {
    background: var(--pv-accent);
  }
  .pv-status[data-tone="success"] .dot {
    background: var(--pv-success);
  }
  .pv-status[data-tone="warning"] .dot {
    background: var(--pv-warning);
  }
  .pv-status[data-tone="danger"] .dot {
    background: var(--pv-danger-text);
  }
  .pv-status[data-tone="record"] .dot {
    background: var(--pv-record);
    box-shadow: 0 0 0 3px var(--pv-record-soft);
  }
  .pv-status[data-tone="record"] .text {
    color: var(--pv-record-text);
    font-weight: var(--pv-weight-semibold);
  }

  .pv-status[data-pulse="true"] .dot {
    animation: pv-breathe 1.6s ease-in-out infinite;
  }

  @keyframes pv-breathe {
    50% {
      opacity: 0.45;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pv-status[data-pulse="true"] .dot {
      animation: none;
    }
  }
</style>
