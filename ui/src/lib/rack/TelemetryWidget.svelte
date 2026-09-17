<script lang="ts">
  import { t } from "../i18n";
  import type { TelemetryChannelDto } from "../ipc/bindings";
  import { formatNumber, formatWithUnit, unitSuffix } from "../ui/units";
  import GainReductionMeter from "./GainReductionMeter.svelte";
  import { localized } from "./localized";

  /**
   * One module telemetry channel as a widget (H-77; ADR-005 §13, SPEC-016 §2.6 "Generic
   * fallback"): `GainReduction` → a GR meter, `Level` → a level bar over the channel's range,
   * `Indicator` → a lamp, `Value` → a numeric readout. Generic — it is fed only by the channel
   * description that travels with the rack state and the latest `VXMT` value, so any module gets
   * the same widgets in its slot header (a channel with no group) or in a section header.
   *
   * `value === undefined` is rest: no frame yet, or none for 250 ms (SPEC-016 §2.6 "Stale":
   * meters read 0 and lamps go off).
   */
  let {
    channel,
    value,
    ticks = false,
  }: {
    channel: TelemetryChannelDto;
    value: number | undefined;
    /** Draw the GR meter's scale ticks (a section header has the room). */
    ticks?: boolean;
  } = $props();

  const name = $derived(localized(channel.name));
  const known = $derived(value !== undefined && Number.isFinite(value));
  const lit = $derived(known && value! >= 0.5);
  const level = $derived(known ? Math.min(channel.max, Math.max(channel.min, value!)) : channel.min);
  const levelFraction = $derived(
    channel.max > channel.min ? (level - channel.min) / (channel.max - channel.min) : 0,
  );
  const levelText = $derived(
    known ? formatWithUnit(level, unitSuffix(channel.unit), 1) : t("rack.slot.meter.no_signal"),
  );
  const valueText = $derived(
    known ? formatWithUnit(value!, unitSuffix(channel.unit), 1) : formatNumber(0, 1),
  );
</script>

{#if channel.kind === "gain_reduction"}
  <GainReductionMeter {value} min={channel.min} max={channel.max} {name} {ticks} />
{:else if channel.kind === "level"}
  <span
    class="level"
    role="meter"
    aria-label={t("rack.slot.meter.level", { name, value: levelText })}
    aria-valuemin={channel.min}
    aria-valuemax={channel.max}
    aria-valuenow={level}
    aria-valuetext={levelText}
    title={t("rack.slot.meter.level", { name, value: levelText })}
    data-testid="rack-slot-level-meter"
  >
    <span class="track">
      <span
        class="fill"
        data-testid="rack-slot-level-fill"
        style:width={`${(levelFraction * 100).toFixed(1)}%`}
      ></span>
    </span>
    <span class="readout" data-testid="rack-slot-level-value">{levelText}</span>
  </span>
{:else if channel.kind === "indicator"}
  <span
    class="lamp"
    class:lit
    role="status"
    aria-label={t(lit ? "rack.slot.lamp.on" : "rack.slot.lamp.off", { name })}
    title={t(lit ? "rack.slot.lamp.on" : "rack.slot.lamp.off", { name })}
    data-testid="rack-slot-lamp"
    data-lit={lit}
  >
    <span class="dot" aria-hidden="true"></span>
    <span class="lamp-name">{name}</span>
  </span>
{:else}
  <span class="value" title={name} data-testid="rack-slot-telemetry-value">{valueText}</span>
{/if}

<style>
  .level,
  .lamp {
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
    left: 0;
    bottom: 0;
    background: var(--pv-meter-safe);
  }

  .readout,
  .value {
    min-width: 3.4rem;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  /* The lamp carries its name as text, so it never signals with colour alone. */
  .dot {
    width: 8px;
    height: 8px;
    border-radius: var(--pv-radius-full);
    background: var(--pv-meter-track);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border);
  }

  .lamp.lit .dot {
    background: var(--pv-success-text);
    box-shadow: none;
  }

  .lamp-name {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .lamp.lit .lamp-name {
    color: var(--pv-text-secondary);
  }
</style>
