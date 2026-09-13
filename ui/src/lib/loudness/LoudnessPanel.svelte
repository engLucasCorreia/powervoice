<script lang="ts">
  import { t } from "../i18n";
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
    return value.toFixed(1);
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
    return t("panel.acx.value.db", { value: rule.measured_db.toFixed(1) });
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
  <span class="title">{t("panel.loudness.title")}</span>

  <div class="source-toggle" role="group" aria-label={t("panel.loudness.title")}>
    <button
      type="button"
      data-testid="loudness-source-processed"
      class:active={state.source === "processed"}
      onclick={() => setLoudnessSource("processed")}
    >
      {t("panel.loudness.source.processed")}
    </button>
    <button
      type="button"
      data-testid="loudness-source-source"
      class:active={state.source === "source"}
      onclick={() => setLoudnessSource("source")}
    >
      {t("panel.loudness.source.source")}
    </button>
  </div>

  {#if running}
    <button type="button" data-testid="loudness-cancel" onclick={cancelLoudnessAnalyze}>
      {t("panel.loudness.analyzing")}
      {Math.round((state.job?.fraction ?? 0) * 100)}%
    </button>
  {:else}
    <button
      type="button"
      data-testid="loudness-analyze"
      disabled={!enabled}
      onclick={() => void startLoudnessAnalyze()}
    >
      {t("panel.loudness.analyze")}
    </button>
  {/if}

  {#if state.report}
    <span class="readout" data-testid="loudness-integrated">
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
  {:else}
    <span class="readout muted" data-testid="loudness-not-analyzed">
      {t("panel.loudness.not_analyzed")}
    </span>
  {/if}

  <div class="acx" data-testid="acx-section">
    <button
      type="button"
      data-testid="acx-check"
      disabled={!acxEnabled}
      onclick={() => void runAcxCheck()}
    >
      {acx.running ? t("panel.acx.checking") : t("panel.acx.check")}
    </button>

    {#if acx.report}
      {@const report = acx.report}
      <span
        class="acx-result"
        data-testid="acx-result"
        class:pass={report.passes}
        class:fail={!report.passes}
      >
        {report.passes ? t("panel.acx.pass") : t("panel.acx.fail")}
      </span>

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
  .loudness-panel {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.6rem;
    padding: 0.4rem 0.75rem;
    background: var(--surface-panel);
    border-top: 1px solid var(--surface-border);
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .title {
    color: var(--text-secondary);
  }

  .source-toggle {
    display: flex;
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    overflow: hidden;
  }

  .source-toggle button {
    border: none;
    border-radius: 0;
  }

  .source-toggle button.active {
    background: var(--accent, #4a90d9);
    color: var(--text-on-accent, #fff);
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.2rem 0.6rem;
    font-variant-numeric: tabular-nums;
  }

  button:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }

  .readout {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .readout.muted {
    color: var(--text-disabled);
  }

  .acx {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 0.4rem;
    flex-basis: 100%;
    margin-top: 0.2rem;
    padding-top: 0.4rem;
    border-top: 1px solid var(--surface-border);
  }

  .acx-result {
    font-weight: 600;
  }

  .acx-result.pass {
    color: var(--meter-green);
  }

  .acx-result.fail {
    color: var(--meter-red);
  }

  .acx-table {
    border-collapse: collapse;
    font-size: 0.75rem;
  }

  .acx-table th,
  .acx-table td {
    text-align: left;
    padding: 0.1rem 0.6rem 0.1rem 0;
    font-variant-numeric: tabular-nums;
  }

  .acx-table th {
    color: var(--text-secondary);
    font-weight: 500;
  }

  .acx-table td.pass {
    color: var(--meter-green);
  }

  .acx-table td.fail {
    color: var(--meter-red);
  }

  .acx-hint {
    margin: 0;
    color: var(--text-secondary);
    max-width: 40rem;
  }
</style>
