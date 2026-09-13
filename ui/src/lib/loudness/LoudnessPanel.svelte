<script lang="ts">
  import { t } from "../i18n";
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
   */
  const state = loudnessState();
  const running = $derived(state.job?.state === "running");
  const enabled = $derived(canAnalyzeLoudness() && !running);

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
</style>
