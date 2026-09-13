<script lang="ts">
  import { t } from "../i18n";
  import type { MonitorMode } from "../ipc/bindings";
  import {
    clearClip,
    recordState,
    setMonitor,
    toggleArm,
    toggleRecord,
  } from "../state/record.svelte";
  import { formatElapsed } from "./format";

  /**
   * Record panel controls (S1-04, SPEC-002 §2.1–§2.2, §2.7): Input (arm) toggle, Record/Stop
   * button (Shift+R), elapsed time, clip lamp (click to clear), monitoring Off/Dry.
   */
  const rec = recordState();
  const noInput = $derived(rec.state.input_device === null);
  const busy = $derived(rec.state.recording || rec.state.finishing);
  const elapsed = $derived(formatElapsed(rec.elapsedSamples, rec.state.input_rate_hz ?? 0));
  const monitor = $derived<MonitorMode>(rec.state.monitor === "off" ? "off" : "dry");
</script>

<div class="record" role="group" aria-label={t("record.group")} data-testid="record-controls">
  <button
    type="button"
    data-testid="record-arm"
    class:active={rec.state.armed}
    aria-pressed={rec.state.armed}
    disabled={noInput || busy}
    title={noInput ? t("record.no_input_hint") : t("record.arm_title")}
    onclick={() => void toggleArm()}
  >
    {t("record.arm")}
  </button>
  <button
    type="button"
    data-testid="record-button"
    class="rec"
    class:recording={rec.state.recording}
    disabled={noInput || rec.state.finishing}
    title={noInput ? t("record.no_input_hint") : t("record.record_title")}
    onclick={() => void toggleRecord()}
  >
    {rec.state.recording ? t("record.stop") : t("record.record")}
  </button>
  <span class="elapsed" data-testid="record-elapsed" title={t("record.elapsed_title")}>{elapsed}</span>
  {#if rec.state.dropout_count > 0}
    <span
      class="dropouts"
      data-testid="record-dropouts"
      title={t("record.dropouts_title")}
    >
      {t("record.dropouts", { count: String(rec.state.dropout_count) })}
    </span>
  {/if}
  <button
    type="button"
    data-testid="record-clip"
    class="clip"
    class:lit={rec.clipLatched}
    title={t("record.clip_title")}
    onclick={clearClip}
  >
    {t("record.clip")}
  </button>
  <label class="monitor">
    <span>{t("record.monitor")}</span>
    <select
      data-testid="record-monitor"
      value={monitor}
      onchange={(e) => void setMonitor(e.currentTarget.value as MonitorMode)}
    >
      <option value="off">{t("record.monitor.off")}</option>
      <option value="dry">{t("record.monitor.dry")}</option>
    </select>
  </label>
</div>

<style>
  .record {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }

  button.active {
    border-color: var(--accent);
    color: var(--accent);
  }

  button.rec.recording {
    background: var(--meter-red);
    border-color: var(--meter-red);
    color: var(--text-primary);
  }

  .elapsed {
    min-width: 5.5rem;
    padding: 0.2rem 0.5rem;
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    font-variant-numeric: tabular-nums;
    text-align: right;
  }

  button.clip {
    padding: 0.25rem 0.4rem;
    font-size: 0.7rem;
    color: var(--text-disabled);
  }

  button.clip.lit {
    background: var(--meter-red);
    border-color: var(--meter-red);
    color: var(--text-primary);
  }

  .dropouts {
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    font-size: 0.75rem;
    background: var(--meter-yellow);
    color: var(--surface-panel);
  }

  .monitor {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-secondary);
    font-size: 0.8rem;
  }

  select {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.15rem 0.3rem;
  }
</style>
