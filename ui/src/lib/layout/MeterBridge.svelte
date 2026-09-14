<script lang="ts">
  import { t } from "../i18n";
  import InputMeter from "../record/InputMeter.svelte";
  import { transportState } from "../state/transport.svelte";

  /** Meter scale floor (dBFS); the top is 0 dBFS. */
  const FLOOR_DB = -60;

  const transport = transportState();

  function percent(db: number): number {
    if (!Number.isFinite(db)) {
      return 0;
    }
    return Math.min(100, Math.max(0, ((db - FLOOR_DB) / -FLOOR_DB) * 100));
  }

  function label(db: number): string {
    return Number.isFinite(db) ? db.toFixed(1) : t("meter.silence");
  }
</script>

<footer class="meter-bridge" data-testid="meter-bridge">
  <InputMeter />
  <div class="row">
    <span class="row-label">{t("meter.output")}</span>
    <div
      class="meter"
      role="meter"
      aria-label={t("meter.output")}
      aria-valuemin={FLOOR_DB}
      aria-valuemax={0}
      aria-valuenow={Number.isFinite(transport.meter.peakDbfs) ? transport.meter.peakDbfs : FLOOR_DB}
    >
      <div class="rms" style:width="{percent(transport.meter.rmsDbfs)}%"></div>
      <div class="peak" class:clip={transport.meter.clip} style:left="{percent(transport.meter.peakDbfs)}%"></div>
    </div>
    <span class="readouts">
      <span class="readout" data-testid="meter-peak">{t("meter.peak", { value: label(transport.meter.peakDbfs) })}</span>
      <span class="readout" data-testid="meter-rms">{t("meter.rms", { value: label(transport.meter.rmsDbfs) })}</span>
    </span>
  </div>
</footer>

<style>
  /* H-25: the meter bridge stacks In and Out as labelled rows. The bars themselves — track,
     green RMS, yellow peak line, red on clip — keep the look the owner likes. */
  .meter-bridge {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--pv-space-3);
    min-width: 18rem;
    padding: var(--pv-space-3);
    background: var(--pv-bg-panel);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-xs);
  }

  .row {
    display: grid;
    grid-template-columns: 5.5rem minmax(4rem, 1fr);
    grid-template-rows: auto auto;
    align-items: center;
    gap: var(--pv-space-1) var(--pv-space-2);
  }

  .row-label {
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-weight: var(--pv-weight-medium);
  }

  .meter {
    position: relative;
    height: 10px;
    overflow: hidden;
    border-radius: 2px;
    background: var(--surface-inset);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border);
  }

  .rms {
    height: 100%;
    background: var(--meter-green);
  }

  .peak {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--meter-yellow);
  }

  .peak.clip {
    background: var(--meter-red);
  }

  .readouts {
    display: flex;
    grid-column: 2;
    gap: var(--pv-space-3);
  }

  .readout {
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
</style>
