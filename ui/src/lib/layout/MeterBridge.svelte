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
  <span>{t("panel.meters.title")}</span>
  <InputMeter />
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
  <span class="readout" data-testid="meter-peak">{t("meter.peak", { value: label(transport.meter.peakDbfs) })}</span>
  <span class="readout" data-testid="meter-rms">{t("meter.rms", { value: label(transport.meter.rmsDbfs) })}</span>
</footer>

<style>
  .meter-bridge {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.4rem 0.75rem;
    background: var(--surface-panel);
    border-top: 1px solid var(--surface-border);
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .meter {
    position: relative;
    flex: 1;
    max-width: 32rem;
    height: 0.6rem;
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    border-radius: 2px;
    overflow: hidden;
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

  .readout {
    min-width: 7rem;
    font-variant-numeric: tabular-nums;
  }
</style>
