<script lang="ts">
  import { t } from "../i18n";
  import {
    applyCalibration,
    closeCalibration,
    recordState,
    startCalibration,
  } from "../state/record.svelte";

  /**
   * Loopback calibration wizard (T-304, SPEC-022 §2.14): Connect → Measure (5 sweeps, progress
   * from `job_progress`) → Result (accepted: Apply / Retry / Cancel; rejected: Apply disabled,
   * the stored offset untouched) → optional Verify (the residual after compensation).
   */
  const rec = recordState();
  const cal = $derived(rec.calibration);
  const result = $derived(cal?.result ?? null);

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      void closeCalibration();
    }
  }

  const signedMs = (ms: number): string => `${ms >= 0 ? "+" : ""}${ms.toFixed(2)}`;
</script>

{#if cal}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="calibration-title"
      data-testid="calibration-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="calibration-title">{t("calibration.title")}</h2>
      {#if cal.stage === "connect"}
        <p>{t("calibration.connect")}</p>
        <div class="actions">
          <button type="button" data-testid="calibration-cancel" onclick={() => void closeCalibration()}>
            {t("calibration.cancel")}
          </button>
          <button
            type="button"
            class="primary"
            data-testid="calibration-start"
            onclick={() => void startCalibration(false)}
          >
            {t("calibration.start")}
          </button>
        </div>
      {:else if cal.stage === "measuring"}
        <p>{t("calibration.measuring")}</p>
        <progress data-testid="calibration-progress" max="1" value={cal.progress}></progress>
        <div class="actions">
          <button type="button" data-testid="calibration-cancel" onclick={() => void closeCalibration()}>
            {t("calibration.cancel")}
          </button>
        </div>
      {:else if cal.stage === "failed"}
        <p>{t("calibration.failed")}</p>
        <div class="actions">
          <button type="button" data-testid="calibration-close" onclick={() => void closeCalibration()}>
            {t("calibration.close")}
          </button>
          <button
            type="button"
            data-testid="calibration-retry"
            onclick={() => void startCalibration(cal.verify)}
          >
            {t("calibration.retry")}
          </button>
        </div>
      {:else if result}
        {#if result.verify}
          <p data-testid="calibration-residual">
            {t("calibration.residual", { ms: result.offset_ms.toFixed(2) })}
          </p>
        {:else if result.accepted}
          <p data-testid="calibration-accepted">
            {t("calibration.accepted", {
              ms: signedMs(result.offset_ms),
              samples: String(Math.round(result.offset_samples)),
              rate: String(result.device_rate_hz / 1000),
              agree: String(result.reps_agreeing),
            })}
          </p>
          {#if result.peak_dbfs !== null}
            <p class="detail">{t("calibration.peak", { db: result.peak_dbfs.toFixed(1) })}</p>
          {/if}
          {#if result.clipped}
            <p class="warn" data-testid="calibration-warn-clipped">{t("calibration.warn_clipped")}</p>
          {/if}
          {#if result.weak}
            <p class="warn" data-testid="calibration-warn-weak">{t("calibration.warn_weak")}</p>
          {/if}
        {:else}
          <p class="warn" data-testid="calibration-rejected">
            {result.reason === "no_signal"
              ? t("calibration.no_signal")
              : t("calibration.rejected", { agree: String(result.reps_agreeing) })}
          </p>
        {/if}
        <div class="actions">
          <button type="button" data-testid="calibration-close" onclick={() => void closeCalibration()}>
            {cal.applied || result.verify ? t("calibration.close") : t("calibration.cancel")}
          </button>
          <button
            type="button"
            data-testid="calibration-retry"
            onclick={() => void startCalibration(false)}
          >
            {t("calibration.retry")}
          </button>
          {#if cal.applied || result.verify}
            <button
              type="button"
              data-testid="calibration-verify"
              onclick={() => void startCalibration(true)}
            >
              {t("calibration.verify")}
            </button>
          {:else}
            <button
              type="button"
              class="primary"
              data-testid="calibration-apply"
              disabled={!result.accepted}
              onclick={() => void applyCalibration()}
            >
              {t("calibration.apply")}
            </button>
          {/if}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.45);
    z-index: 1000;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-width: 26rem;
    max-width: 36rem;
    padding: 1rem 1.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    color: var(--text-primary);
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  p {
    margin: 0;
    color: var(--text-secondary);
  }

  p.warn {
    color: var(--meter-yellow);
  }

  progress {
    width: 100%;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button:disabled {
    color: var(--text-disabled);
  }

  button.primary:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
