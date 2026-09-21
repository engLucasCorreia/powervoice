<script lang="ts">
  import { t } from "../i18n";
  import type { InputMeterFloorPref } from "../ipc/bindings";
  import VerticalMeter from "../meters/VerticalMeter.svelte";
  import { recordState, resetMaxPeak } from "../state/record.svelte";
  import { saveSettings, settingsState } from "../state/settings.svelte";
  import { formatNumber } from "../ui/units";

  /**
   * Input meter (H-112, owner request: "It should definitely look like the Output level" —
   * SPEC-002 §2.1 amended): the same vertical form as `layout/OutputMeter.svelte`, through the
   * shared `meters/VerticalMeter.svelte`, plus:
   *  - a selectable scale floor (−60/−80/−120 dBFS, persisted in `Settings.input_meter_floor`);
   *  - the max readout and its click-to-reset (kept from the pre-H-112 horizontal meter — the
   *    output meter has no equivalent).
   * The clip lamp stays on the transport bar (`RecordControls.svelte`, `record.clip`/
   * `record.clip_title`) — H-112 doesn't duplicate it here, only tints the hold tick red
   * (`VerticalMeter`'s `clip` prop), the same as the output meter's own hold tick.
   */

  const FLOOR_OPTIONS: ReadonlyArray<{ value: InputMeterFloorPref; label: string }> = [
    { value: "-60", label: t("meter.input_floor.60") },
    { value: "-80", label: t("meter.input_floor.80") },
    { value: "-120", label: t("meter.input_floor.120") },
  ];

  const rec = recordState();
  const floorPref = $derived<InputMeterFloorPref>(settingsState().current?.input_meter_floor ?? "-60");
  const floorDb = $derived(Number(floorPref));

  function label(db: number): string {
    return Number.isFinite(db) ? formatNumber(db, 1) : t("meter.silence");
  }

  function chooseFloor(next: string): void {
    void saveSettings({ input_meter_floor: next as InputMeterFloorPref });
  }
</script>

{#if rec.state.input_open}
  <div class="input-meter" data-testid="input-meter-wrap">
    <VerticalMeter
      label={t("meter.input")}
      testid="input-meter"
      floorDb={floorDb}
      peakDbfs={rec.meter.peakDbfs}
      rmsDbfs={rec.meter.rmsDbfs}
      holdDbfs={rec.meter.holdDbfs}
      clip={rec.clipLatched}
      peakReadoutDbfs={rec.meter.peakReadoutDbfs}
      rmsReadoutDbfs={rec.meter.rmsReadoutDbfs}
    >
      <button
        type="button"
        class="readout"
        data-testid="input-meter-max"
        title={t("meter.reset_max")}
        onclick={resetMaxPeak}
      >
        {t("meter.max", { value: label(rec.meter.maxDbfs) })}
      </button>
    </VerticalMeter>
    <label class="floor-picker" title={t("meter.input_floor_title")}>
      <span class="floor-picker-label">{t("meter.input_floor")}</span>
      <select
        data-testid="input-meter-floor"
        value={floorPref}
        onchange={(e) => chooseFloor(e.currentTarget.value)}
      >
        {#each FLOOR_OPTIONS as opt (opt.value)}
          <option value={opt.value}>{opt.label}</option>
        {/each}
      </select>
    </label>
  </div>
{/if}

<style>
  .input-meter {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    gap: var(--pv-space-1);
  }

  /* Matches `VerticalMeter`'s own `.readout` look (button content passed through `children` keeps
     the *caller's* scoped styles, not the child component's — so this is deliberately duplicated,
     not a second ballistics/meter-math implementation). */
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
    cursor: pointer;
  }

  .readout:hover {
    color: var(--pv-text-primary);
  }

  .floor-picker {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-1);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .floor-picker select {
    height: 22px;
    padding: 0 var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-xs);
  }

  .floor-picker select:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }
</style>
