<script lang="ts">
  import { formatNumber } from "../ui/units";
  import { t } from "../i18n";
  import { Badge, Button, SegmentedControl, type SegmentOption } from "../ui";
  import type { AcxRuleDto, AcxRuleStatusDto } from "../ipc/bindings";
  import { acxState, runAcxCheck } from "./acx.svelte";
  import {
    canAnalyzeLoudness,
    cancelLoudnessAnalyze,
    loudnessState,
    setLoudnessSource,
    startLoudnessAnalyze,
  } from "./loudness.svelte";

  /**
   * Loudness panel (S4-01, bottom dock): integrated / short-term max / momentary max / LRA /
   * sample peak / true peak, for the current selection (or whole file). "Processed" (rack
   * rendered) is the default source; "Source" bypasses the rack.
   *
   * S4-03 adds "ACX Check" below: pass/fail per ACX rule (RMS, sample peak, noise floor) for the
   * whole document, reusing the same "Processed"/"Source" toggle above rather than a second one.
   */
  const state = loudnessState();
  const running = $derived(state.job?.state === "running");
  const enabled = $derived(canAnalyzeLoudness() && !running);

  const acx = acxState();
  // H-25: Processed / Source as the kit's small segmented control.
  const SOURCES: SegmentOption<"processed" | "source">[] = [
    { value: "processed", label: t("panel.loudness.source.processed"), testid: "loudness-source-processed" },
    { value: "source", label: t("panel.loudness.source.source"), testid: "loudness-source-source" },
  ];
  const acxEnabled = $derived(canAnalyzeLoudness() && !acx.running);

  function db(value: number | undefined | null): string {
    if (value === undefined || value === null) {
      return t("panel.loudness.na");
    }
    if (Number.isNaN(value)) {
      return t("panel.loudness.na");
    }
    if (!Number.isFinite(value)) {
      return t("panel.loudness.silence");
    }
    return formatNumber(value, 1);
  }

  /** One ACX rule row's "Measured" column. */
  function acxValue(rule: AcxRuleDto): string {
    if (rule.status === "too_short" || rule.measured_db === null) {
      return t("panel.acx.value.too_short");
    }
    if (Number.isNaN(rule.measured_db)) {
      return t("panel.acx.value.invalid");
    }
    if (!Number.isFinite(rule.measured_db)) {
      return t("panel.loudness.silence");
    }
    return t("panel.acx.value.db", { value: formatNumber(rule.measured_db, 1) });
  }

  function acxStatusGlyph(status: AcxRuleStatusDto): string {
    return status === "pass" ? t("panel.acx.status.pass") : t("panel.acx.status.fail");
  }

  type AcxHintKey =
    | "panel.acx.hint.rms_too_low"
    | "panel.acx.hint.rms_too_high"
    | "panel.acx.hint.peak_too_high"
    | "panel.acx.hint.noise_floor_too_high"
    | "panel.acx.hint.noise_floor_too_short"
    | "panel.acx.hint.invalid";

  function rmsHintKey(status: AcxRuleStatusDto): AcxHintKey | null {
    switch (status) {
      case "too_low":
        return "panel.acx.hint.rms_too_low";
      case "too_high":
        return "panel.acx.hint.rms_too_high";
      case "invalid":
        return "panel.acx.hint.invalid";
      default:
        return null;
    }
  }

  function peakHintKey(status: AcxRuleStatusDto): AcxHintKey | null {
    switch (status) {
      case "too_high":
        return "panel.acx.hint.peak_too_high";
      case "invalid":
        return "panel.acx.hint.invalid";
      default:
        return null;
    }
  }

  function noiseFloorHintKey(status: AcxRuleStatusDto): AcxHintKey | null {
    switch (status) {
      case "too_high":
        return "panel.acx.hint.noise_floor_too_high";
      case "too_short":
        return "panel.acx.hint.noise_floor_too_short";
      case "invalid":
        return "panel.acx.hint.invalid";
      default:
        return null;
    }
  }
</script>


<footer class="loudness-panel" data-testid="loudness-panel">
  <div class="controls">
    <span class="title">{t("panel.loudness.title")}</span>
    <SegmentedControl
      options={SOURCES}
      value={state.source}
      label={t("panel.loudness.title")}
      size="sm"
      onchange={setLoudnessSource}
    />
    {#if running}
      <Button size="sm" loading testid="loudness-cancel" onclick={cancelLoudnessAnalyze}>
        {t("panel.loudness.analyzing")}
        {Math.round((state.job?.fraction ?? 0) * 100)}%
      </Button>
    {:else}
      <Button
        size="sm"
        variant="primary"
        icon="loudness"
        testid="loudness-analyze"
        disabled={!enabled}
        onclick={() => void startLoudnessAnalyze()}
      >
        {t("panel.loudness.analyze")}
      </Button>
    {/if}
  </div>

  {#if state.report}
    <div class="readouts">
      <span class="readout lead" data-testid="loudness-integrated">
        {t("panel.loudness.integrated", { value: db(state.report.integrated_lufs) })}
      </span>
      <span class="readout" data-testid="loudness-short-term">
        {t("panel.loudness.short_term", { value: db(state.report.max_short_term_lufs) })}
      </span>
      <span class="readout" data-testid="loudness-momentary">
        {t("panel.loudness.momentary", { value: db(state.report.max_momentary_lufs) })}
      </span>
      <span class="readout" data-testid="loudness-lra">
        {t("panel.loudness.lra", { value: db(state.report.lra_lu) })}
      </span>
      <span class="readout" data-testid="loudness-sample-peak">
        {t("panel.loudness.sample_peak", { value: db(state.report.sample_peak_dbfs) })}
      </span>
      <span class="readout" data-testid="loudness-true-peak">
        {t("panel.loudness.true_peak", { value: db(state.report.true_peak_dbtp) })}
      </span>
    </div>
  {:else}
    <p class="readout muted" data-testid="loudness-not-analyzed">
      {t("panel.loudness.not_analyzed")}
    </p>
  {/if}

  <div class="acx" data-testid="acx-section">
    <div class="acx-head">
      <Button size="sm" icon="check" testid="acx-check" loading={acx.running} disabled={!acxEnabled} onclick={() => void runAcxCheck()}>
        {acx.running ? t("panel.acx.checking") : t("panel.acx.check")}
      </Button>
    </div>

    {#if acx.report}
      {@const report = acx.report}
      <Badge tone={report.passes ? "success" : "danger"} icon={report.passes ? "success" : "error"} testid="acx-result">
        {report.passes ? t("panel.acx.pass") : t("panel.acx.fail")}
      </Badge>

      <table class="acx-table" data-testid="acx-table">
        <thead>
          <tr>
            <th>{t("panel.acx.rule")}</th>
            <th>{t("panel.acx.measured")}</th>
            <th>{t("panel.acx.limit")}</th>
            <th></th>
          </tr>
        </thead>
        <tbody>
          <tr data-testid="acx-row-rms">
            <td>{t("panel.acx.rule.rms")}</td>
            <td>{acxValue(report.rms)}</td>
            <td>{t("panel.acx.limit.rms")}</td>
            <td class:pass={report.rms.status === "pass"} class:fail={report.rms.status !== "pass"}>
              {acxStatusGlyph(report.rms.status)}
            </td>
          </tr>
          <tr data-testid="acx-row-peak">
            <td>{t("panel.acx.rule.peak")}</td>
            <td>{acxValue(report.peak)}</td>
            <td>{t("panel.acx.limit.peak")}</td>
            <td
              class:pass={report.peak.status === "pass"}
              class:fail={report.peak.status !== "pass"}
            >
              {acxStatusGlyph(report.peak.status)}
            </td>
          </tr>
          <tr data-testid="acx-row-noise-floor">
            <td>{t("panel.acx.rule.noise_floor")}</td>
            <td>{acxValue(report.noise_floor)}</td>
            <td>{t("panel.acx.limit.noise_floor")}</td>
            <td
              class:pass={report.noise_floor.status === "pass"}
              class:fail={report.noise_floor.status !== "pass"}
            >
              {acxStatusGlyph(report.noise_floor.status)}
            </td>
          </tr>
        </tbody>
      </table>

      {#if rmsHintKey(report.rms.status)}
        <p class="acx-hint" data-testid="acx-hint-rms">
          {t(rmsHintKey(report.rms.status)!, { value: acxValue(report.rms) })}
        </p>
      {/if}
      {#if peakHintKey(report.peak.status)}
        <p class="acx-hint" data-testid="acx-hint-peak">
          {t(peakHintKey(report.peak.status)!, { value: acxValue(report.peak) })}
        </p>
      {/if}
      {#if noiseFloorHintKey(report.noise_floor.status)}
        <p class="acx-hint" data-testid="acx-hint-noise-floor">
          {t(noiseFloorHintKey(report.noise_floor.status)!, { value: acxValue(report.noise_floor) })}
        </p>
      {/if}
    {/if}
  </div>
</footer>

<style>
  /* H-25: the loudness tab — controls row, a readout grid led by Integrated, then ACX. */
  .loudness-panel {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-3);
    padding: var(--pv-space-3);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
  }

  .controls,
  .acx-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2) var(--pv-space-3);
  }

  .title {
    color: var(--pv-text-secondary);
    font-weight: var(--pv-weight-semibold);
  }

  .readouts {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(12rem, 1fr));
    gap: var(--pv-space-2) var(--pv-space-4);
  }

  .readout {
    margin: 0;
    color: var(--pv-text-primary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .readout.lead {
    font-size: var(--pv-text-lg);
    font-weight: var(--pv-weight-semibold);
  }

  .readout.muted {
    color: var(--pv-text-tertiary);
  }

  .acx {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--pv-space-2);
    padding-top: var(--pv-space-3);
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .acx-table {
    border-collapse: collapse;
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
  }

  .acx-table th,
  .acx-table td {
    padding: var(--pv-space-1) var(--pv-space-4) var(--pv-space-1) 0;
    text-align: left;
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .acx-table th {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-weight: var(--pv-weight-medium);
  }

  .acx-table td {
    color: var(--pv-text-primary);
  }

  .acx-table td.pass {
    color: var(--pv-success-text);
    font-weight: var(--pv-weight-semibold);
  }

  .acx-table td.fail {
    color: var(--pv-danger-text);
    font-weight: var(--pv-weight-semibold);
  }

  .acx-hint {
    max-width: 40rem;
    margin: 0;
    color: var(--pv-text-secondary);
    line-height: var(--pv-leading-sm);
  }
</style>
