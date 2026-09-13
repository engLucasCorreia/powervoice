<script lang="ts">
  import { t } from "../i18n";
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
    return Number.isFinite(db) ? db.toFixed(1) : t("meter.silence");
  }
</script>

{#if rec.state.input_open}
  <div class="input-meter" data-testid="input-meter">
    <span>{t("meter.input")}</span>
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
  </div>
{/if}

<style>
  .input-meter {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }

  .meter {
    position: relative;
    width: 20rem;
    max-width: 30vw;
    height: 0.6rem;
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    border-radius: 2px;
    overflow: hidden;
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

  .readout {
    min-width: 7rem;
    font-variant-numeric: tabular-nums;
    background: none;
    border: none;
    padding: 0;
    color: inherit;
    font: inherit;
    text-align: left;
  }

  button.readout {
    cursor: pointer;
  }
</style>
