<script lang="ts">
  import { t } from "../i18n";
  import { Dialog, formatNumber, type DialogAction } from "../ui";
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

  const signedMs = (ms: number): string => formatNumber(ms, 2, { signed: true });

  // H-26: the footer by stage, as role-described actions (the Dialog orders them per platform).
  const actions = $derived.by((): DialogAction[] => {
    const cancel: DialogAction = {
      label: t("calibration.cancel"),
      role: "cancel",
      testid: "calibration-cancel",
      onclick: () => void closeCalibration(),
    };
    if (cal?.stage === "connect") {
      return [
        cancel,
        { label: t("calibration.start"), role: "primary", testid: "calibration-start", onclick: () => void startCalibration(false) },
      ];
    }
    if (cal?.stage === "measuring") {
      return [cancel];
    }
    if (cal?.stage === "failed") {
      return [
        { label: t("calibration.close"), role: "cancel", testid: "calibration-close", onclick: () => void closeCalibration() },
        {
          label: t("calibration.retry"),
          role: "primary",
          testid: "calibration-retry",
          onclick: () => void startCalibration(cal?.verify ?? false),
        },
      ];
    }
    if (result) {
      const done = cal?.applied || result.verify;
      return [
        {
          label: done ? t("calibration.close") : t("calibration.cancel"),
          role: "cancel",
          testid: "calibration-close",
          onclick: () => void closeCalibration(),
        },
        { label: t("calibration.retry"), role: "alternate", testid: "calibration-retry", onclick: () => void startCalibration(false) },
        done
          ? { label: t("calibration.verify"), role: "primary", testid: "calibration-verify", onclick: () => void startCalibration(true) }
          : {
              label: t("calibration.apply"),
              role: "primary",
              testid: "calibration-apply",
              disabled: !result.accepted,
              onclick: () => void applyCalibration(),
            },
      ];
    }
    return [];
  });
</script>

{#if cal}
  <Dialog
    actions={actions} title={t("calibration.title")} titleId="calibration-title" testid="calibration-dialog" onkeydown={onKeydown}>
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
          {t("calibration.residual", { ms: formatNumber(result.offset_ms, 2) })}
        </p>
      {:else if result.accepted}
        <p data-testid="calibration-accepted">
          {t("calibration.accepted", {
            ms: signedMs(result.offset_ms),
            samples: formatNumber(Math.round(result.offset_samples), 0),
            rate: String(result.device_rate_hz / 1000),
            agree: String(result.reps_agreeing),
          })}
        </p>
        {#if result.peak_dbfs !== null}
          <p class="hint">{t("calibration.peak", { db: formatNumber(result.peak_dbfs, 1) })}</p>
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
  </Dialog>
{/if}
