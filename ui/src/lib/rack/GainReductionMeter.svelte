<script lang="ts">
  import { t } from "../i18n";
  import { formatNumber } from "../ui/units";

  /**
   * A slot-header gain-reduction meter (H-03; SPEC-017 §2.3 "Meter", SPEC-016 §4.12): a bar that
   * grows leftwards from 0 dB over the channel's display range (the true-peak limiter: 0 … −24 dB)
   * plus the value in dB. `value` is the deepest reduction since the previous telemetry frame (the
   * module's Min hold); `undefined` (no frame yet) reads as rest.
   */
  let {
    value,
    min,
    max,
    name,
  }: {
    value: number | undefined;
    min: number;
    max: number;
    name: string;
  } = $props();

  const db = $derived(
    value === undefined || !Number.isFinite(value) ? max : Math.min(max, Math.max(min, value)),
  );
  const fraction = $derived(max > min ? (max - db) / (max - min) : 0);
  const text = $derived(formatNumber(db, 1));
  const label = $derived(t("rack.slot.meter.gain_reduction", { name, value: text }));
</script>

<span
  class="gr"
  role="meter"
  aria-label={label}
  aria-valuemin={min}
  aria-valuemax={max}
  aria-valuenow={db}
  title={label}
  data-testid="rack-slot-gr-meter"
>
  <span class="track">
    <span class="fill" data-testid="rack-slot-gr-fill" style:width={`${(fraction * 100).toFixed(1)}%`}
    ></span>
  </span>
  <span class="readout" data-testid="rack-slot-gr-value">{text}</span>
</span>

<style>
  .gr {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
  }

  .track {
    position: relative;
    width: 3.5rem;
    height: 6px;
    overflow: hidden;
    border-radius: var(--pv-radius-full);
    background: var(--pv-meter-track);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border);
  }

  .fill {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    background: var(--pv-meter-caution);
  }

  .readout {
    min-width: 2.4rem;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
</style>
