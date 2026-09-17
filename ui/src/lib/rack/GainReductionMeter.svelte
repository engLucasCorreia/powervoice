<script lang="ts">
  import { t } from "../i18n";
  import { formatNumber } from "../ui/units";
  import { grAtFloor, grFraction, grScaleMin, grTicks } from "./grMeter";

  /**
   * A gain-reduction meter (H-03, H-77; SPEC-016 §2.6, SPEC-017 §2.3 "Meter"): a bar that grows
   * leftwards from 0 dB over the channel's scale (0 … −30 dB, or a narrower declared range such
   * as the true-peak limiter's 0 … −24), with ticks and the value in dB. `value` is the deepest
   * reduction since the previous telemetry frame (the module's Min hold); `undefined` — no frame
   * yet, or none for 250 ms — reads as rest.
   *
   * It sits in the slot header (a channel with no group) and in a section header (a channel in
   * that group), which is the whole of Dynamics' per-section metering.
   */
  let {
    value,
    min,
    max,
    name,
    ticks = false,
  }: {
    value: number | undefined;
    /** The channel's declared floor; reaching it reads "≤ −60 dB". */
    min: number;
    max: number;
    name: string;
    /** Draw the scale's ticks (a section header has room; the slot header does not). */
    ticks?: boolean;
  } = $props();

  const rest = $derived(value === undefined || !Number.isFinite(value));
  /** The received value, not clamped: past the scale the bar pins but the readout keeps it. */
  const db = $derived(rest ? max : value!);
  const scaleMin = $derived(grScaleMin(min));
  const fraction = $derived(grFraction(db, scaleMin, max));
  const atFloor = $derived(!rest && grAtFloor(db, min));
  const text = $derived(
    atFloor
      ? t("rack.slot.meter.floor", { value: formatNumber(min, 1) })
      : t("rack.slot.meter.value", { value: formatNumber(db, 1) }),
  );
  const label = $derived(t("rack.slot.meter.gain_reduction", { name, value: text }));
  const scaleTicks = $derived(ticks ? grTicks(scaleMin, max) : []);
</script>

<span
  class="gr"
  role="meter"
  aria-label={label}
  aria-valuemin={scaleMin}
  aria-valuemax={max}
  aria-valuenow={Math.min(max, Math.max(scaleMin, db))}
  aria-valuetext={text}
  title={label}
  data-testid="rack-slot-gr-meter"
>
  <span class="track">
    {#each scaleTicks as tick (tick.db)}
      <span class="tick" style:right={`${(tick.fraction * 100).toFixed(2)}%`} aria-hidden="true"
      ></span>
    {/each}
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

  /* SPEC-016 §2.6: the scale's ticks, behind the bar so a deep reduction covers them. */
  .tick {
    position: absolute;
    top: 0;
    bottom: 0;
    width: var(--pv-border-width);
    background: var(--pv-border);
  }

  .fill {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    background: var(--pv-meter-caution);
  }

  .readout {
    min-width: 3.4rem;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
</style>
