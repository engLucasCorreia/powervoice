<script lang="ts">
  import type { MeterSpeedPref } from "../ipc/bindings";
  import { t } from "../i18n";
  import InputMeter from "../record/InputMeter.svelte";
  import { saveSettings, settingsState } from "../state/settings.svelte";
  import { SegmentedControl, type SegmentOption } from "../ui";
  import OutputMeter from "./OutputMeter.svelte";

  /**
   * H-41/H-112 (owner request): the whole bridge is a fixed-width row — `width`/`flex: none`,
   * never `min-width` (which still lets a flex item grow to fit its content) — so neither meter's
   * numeric readouts can ever change the analyzer's width next door, no matter how the levels
   * swing. H-112: the input meter is now the same vertical form as the output meter (the owner's
   * request — "input and output side by side is the natural arrangement for setting a recording
   * level"), so the two sit next to each other, each filling the dock's height and resizing with
   * it (H-24's dock splitter). `InputMeter` renders nothing (no DOM at all) while disarmed
   * (`rec.state.input_open` is false), so the bridge is output-meter-only width until armed.
   *
   * H-123 (owner: "can i setup the speed? between fast and slow?"): a Fast/Medium/Slow ballistics
   * speed, shared by both meters (one setting, `Settings.meter_speed`, not per-meter — the owner
   * asked to set "the speed", not two independent speeds) — reuses the analyzer's own
   * Fast/Medium/Slow segmented-control look (`AnalyzerPanel.svelte`) since it's the same familiar
   * choice, just applied to a different pair of meters.
   */

  const SPEEDS: MeterSpeedPref[] = ["fast", "medium", "slow"];
  const speedOptions: SegmentOption<MeterSpeedPref>[] = SPEEDS.map((s) => ({
    value: s,
    label: t(`meter.speed.${s}` as `meter.speed.${MeterSpeedPref}`),
  }));

  const speed = $derived<MeterSpeedPref>(settingsState().current?.meter_speed ?? "medium");

  function chooseSpeed(next: MeterSpeedPref): void {
    void saveSettings({ meter_speed: next });
  }
</script>

<footer class="meter-bridge" data-testid="meter-bridge">
  <div class="speed-row">
    <span class="speed-label">{t("meter.speed_label")}</span>
    <SegmentedControl
      options={speedOptions}
      value={speed}
      label={t("meter.speed_title")}
      size="sm"
      testid="meter-speed"
      onchange={chooseSpeed}
    />
  </div>
  <div class="meters-row">
    <InputMeter />
    <OutputMeter />
  </div>
</footer>

<style>
  .meter-bridge {
    display: flex;
    flex-direction: column;
    flex: none;
    width: 21rem;
    min-height: 0;
    gap: var(--pv-space-2);
    padding: var(--pv-space-3);
    background: var(--pv-bg-panel);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-xs);
  }

  .speed-row {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-2);
  }

  .speed-label {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .meters-row {
    display: flex;
    flex-direction: row;
    align-items: stretch;
    flex: 1;
    min-height: 0;
    gap: var(--pv-space-4);
  }
</style>
