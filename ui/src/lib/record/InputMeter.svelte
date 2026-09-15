<script lang="ts">
  import { t } from "../i18n";
  import { formatNumber } from "../ui/units";
  import { recordState, resetMaxPeak } from "../state/record.svelte";

  /**
   * Input meter (SPEC-002 §2.1), visible while the input is open: peak bar with ballistics,
   * 300 ms RMS bar, peak-hold tick; −60…0 dBFS; the max readout resets on click.
   */
  const FLOOR_DB = -60;
  const rec = recordState();

  function percent(db: number): number {
    if (!Number.isFinite(db)) {
      return 0;
    }
    return Math.min(100, Math.max(0, ((db - FLOOR_DB) / -FLOOR_DB) * 100));
  }

  function label(db: number): string {
    return Number.isFinite(db) ? formatNumber(db, 1) : t("meter.silence");
  }
</script>

{#if rec.state.input_open}
  <div class="input-meter" data-testid="input-meter">
    <span class="row-label">{t("meter.input")}</span>
    <div
      class="meter"
      role="meter"
      aria-label={t("meter.input")}
      aria-valuemin={FLOOR_DB}
      aria-valuemax={0}
      aria-valuenow={Number.isFinite(rec.meter.peakDbfs) ? rec.meter.peakDbfs : FLOOR_DB}
    >
      <div class="peak-bar" style:width="{percent(rec.meter.peakDbfs)}%"></div>
      <div class="rms" style:width="{percent(rec.meter.rmsDbfs)}%"></div>
      <div class="hold" class:clip={rec.clipLatched} style:left="{percent(rec.meter.holdDbfs)}%"></div>
    </div>
    <span class="readouts">
    <button
      type="button"
      class="readout"
      data-testid="input-meter-max"
      title={t("meter.reset_max")}
      onclick={resetMaxPeak}
    >
      {t("meter.max", { value: label(rec.meter.maxDbfs) })}
    </button>
    <span class="readout" data-testid="input-meter-rms">{t("meter.rms", { value: label(rec.meter.rmsDbfs) })}</span>
    </span>
  </div>
{/if}

<style>
  /* H-25: same row anatomy as the output meter in MeterBridge; bar colours unchanged. */
  .input-meter {
    display: grid;
    grid-template-columns: 5.5rem minmax(4rem, 1fr);
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

  .peak-bar,
  .rms {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 0;
  }

  .peak-bar {
    background: var(--meter-yellow);
    opacity: 0.45;
  }

  .rms {
    background: var(--meter-green);
  }

  .hold {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--meter-yellow);
  }

  .hold.clip {
    background: var(--meter-red);
  }

  .readouts {
    display: flex;
    grid-column: 2;
    gap: var(--pv-space-3);
  }

  .readout {
    padding: 0;
    border: none;
    background: none;
    color: var(--pv-text-tertiary);
    font: inherit;
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    text-align: left;
    white-space: nowrap;
  }

  button.readout {
    cursor: pointer;
  }

  button.readout:hover {
    color: var(--pv-text-primary);
  }
</style>
