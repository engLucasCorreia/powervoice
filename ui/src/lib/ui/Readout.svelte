<script lang="ts">
  import { formatNumber } from "./units";

  /**
   * Numeric readout (H-25): "Integrated −23.0 LUFS", "Peak −1.2 dBFS", the transport time. Value
   * in tabular figures and primary text; unit smaller in tertiary text so columns of readouts
   * align on the number. Pass `value` + `decimals` (formatted with the true minus sign and −∞),
   * or preformatted `text` for timecode.
   */
  type Tone = "neutral" | "record" | "warning" | "danger" | "success" | "muted";

  let {
    value,
    text,
    unit = "",
    decimals = 1,
    signed = false,
    label,
    tone = "neutral",
    size = "md",
    testid,
  }: {
    value?: number;
    text?: string;
    unit?: string;
    decimals?: number;
    signed?: boolean;
    label?: string;
    tone?: Tone;
    size?: "sm" | "md" | "xl";
    testid?: string;
  } = $props();

  const shown = $derived(text ?? (value === undefined ? "—" : formatNumber(value, decimals, { signed })));
</script>

<span class="pv-readout" data-tone={tone} data-size={size} data-testid={testid}>
  {#if label}<span class="label">{label}</span>{/if}
  <span class="value">{shown}</span>{#if unit}<span class="unit">{unit}</span>{/if}
</span>

<style>
  .pv-readout {
    display: inline-flex;
    align-items: baseline;
    gap: var(--pv-space-1);
    font-family: var(--pv-font-sans);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .label {
    margin-right: var(--pv-space-1);
    font-size: var(--pv-text-sm);
    color: var(--pv-text-secondary);
  }

  .value {
    font-size: var(--pv-text-md);
    font-weight: var(--pv-weight-medium);
    color: var(--pv-text-primary);
  }

  .unit {
    font-size: var(--pv-text-xs);
    color: var(--pv-text-tertiary);
  }

  .pv-readout[data-size="sm"] .value {
    font-size: var(--pv-text-sm);
  }

  .pv-readout[data-size="xl"] .value {
    font-size: var(--pv-text-xl);
    line-height: var(--pv-leading-xl);
    font-weight: var(--pv-weight-regular);
    letter-spacing: 0.01em;
  }

  .pv-readout[data-tone="record"] .value {
    color: var(--pv-record-text);
  }
  .pv-readout[data-tone="warning"] .value {
    color: var(--pv-warning-text);
  }
  .pv-readout[data-tone="danger"] .value {
    color: var(--pv-danger-text);
  }
  .pv-readout[data-tone="success"] .value {
    color: var(--pv-success-text);
  }
  .pv-readout[data-tone="muted"] .value {
    color: var(--pv-text-tertiary);
  }
</style>
