<script lang="ts">
  import { t } from "../i18n";
  import { Button, Dialog } from "../ui";
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
  <Dialog title={t("calibration.title")} titleId="calibration-title" testid="calibration-dialog" onkeydown={onKeydown}>
    {#if cal.stage === "connect"}
      <p>{t("calibration.connect")}</p>
    {:else if cal.stage === "measuring"}
      <p>{t("calibration.measuring")}</p>
      <progress data-testid="calibration-progress" max="1" value={cal.progress}></progress>
    {:else if cal.stage === "failed"}
      <p class="warning">{t("calibration.failed")}</p>
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
          <p class="hint">{t("calibration.peak", { db: result.peak_dbfs.toFixed(1) })}</p>
        {/if}
        {#if result.clipped}
          <p class="warning" data-testid="calibration-warn-clipped">{t("calibration.warn_clipped")}</p>
        {/if}
        {#if result.weak}
          <p class="warning" data-testid="calibration-warn-weak">{t("calibration.warn_weak")}</p>
        {/if}
      {:else}
        <p class="warning" data-testid="calibration-rejected">
          {result.reason === "no_signal"
            ? t("calibration.no_signal")
            : t("calibration.rejected", { agree: String(result.reps_agreeing) })}
        </p>
      {/if}
    {/if}
    {#snippet footer()}
      {#if cal?.stage === "connect"}
        <Button testid="calibration-cancel" onclick={() => void closeCalibration()}>
          {t("calibration.cancel")}
        </Button>
        <Button variant="primary" testid="calibration-start" onclick={() => void startCalibration(false)}>
          {t("calibration.start")}
        </Button>
      {:else if cal?.stage === "measuring"}
        <Button testid="calibration-cancel" onclick={() => void closeCalibration()}>
          {t("calibration.cancel")}
        </Button>
      {:else if cal?.stage === "failed"}
        <Button testid="calibration-close" onclick={() => void closeCalibration()}>
          {t("calibration.close")}
        </Button>
        <Button variant="primary" testid="calibration-retry" onclick={() => void startCalibration(cal?.verify ?? false)}>
          {t("calibration.retry")}
        </Button>
      {:else if result}
        <Button testid="calibration-close" onclick={() => void closeCalibration()}>
          {cal?.applied || result.verify ? t("calibration.close") : t("calibration.cancel")}
        </Button>
        <Button testid="calibration-retry" onclick={() => void startCalibration(false)}>
          {t("calibration.retry")}
        </Button>
        {#if cal?.applied || result.verify}
          <Button variant="primary" testid="calibration-verify" onclick={() => void startCalibration(true)}>
            {t("calibration.verify")}
          </Button>
        {:else}
          <Button
            variant="primary"
            testid="calibration-apply"
            disabled={!result.accepted}
            onclick={() => void applyCalibration()}
          >
            {t("calibration.apply")}
          </Button>
        {/if}
      {/if}
    {/snippet}
  </Dialog>
{/if}
