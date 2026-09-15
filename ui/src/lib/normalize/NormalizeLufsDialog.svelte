<script lang="ts">
  import { t } from "../i18n";
  import {
    applyNormalizeLufsDialog,
    cancelNormalizeLufsJob,
    closeNormalizeLufsDialog,
    dismissNormalizeLufsJob,
    normalizeLufsState,
    setNormalizeLufsDialogText,
  } from "../state/normalizeLufs.svelte";
  import { Dialog } from "../ui";
  import NormalizeProgressDialog from "./NormalizeProgressDialog.svelte";

  /**
   * Effects → Normalize (LUFS)… dialog (S4-01/H-09). Custom integrated-loudness target
   * (−60.0…0.0 LUFS). Enter applies, Esc cancels; Apply is disabled while the field is invalid.
   * Runs as a job (`NormalizeProgressDialog`), mirroring `NormalizeDialog.svelte`.
   * H-25: Dialog shell.
   */
  const state = normalizeLufsState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closeNormalizeLufsDialog();
    } else if (event.key === "Enter" && state.dialogValid) {
      void applyNormalizeLufsDialog();
    }
  }
</script>

{#if state.dialogOpen}
  <Dialog
    actions={[
      { label: t("dialog.normalize_lufs.cancel"), role: "cancel", testid: "normalize-lufs-dialog-cancel", onclick: closeNormalizeLufsDialog },
      {
        label: t("dialog.normalize_lufs.apply"),
        role: "primary",
        testid: "normalize-lufs-dialog-apply",
        disabled: !state.dialogValid,
        onclick: () => void applyNormalizeLufsDialog(),
      },
    ]}
    size="sm"
    title={t("dialog.normalize_lufs.title")}
    titleId="normalize-lufs-dialog-title"
    testid="normalize-lufs-dialog"
    onkeydown={onKeydown}
  >
    <div class="field-row">
      <label for="normalize-lufs-dialog-target-input">{t("dialog.normalize_lufs.target_label")}</label>
      <input
        id="normalize-lufs-dialog-target-input"
        class="target"
        type="text"
        inputmode="decimal"
        autocomplete="off"
        data-testid="normalize-lufs-dialog-target"
        class:invalid={!state.dialogValid}
        aria-invalid={!state.dialogValid ? "true" : undefined}
        value={state.dialogText}
        oninput={(e) => setNormalizeLufsDialogText(e.currentTarget.value)}
      />
      <span class="unit">{t("dialog.normalize_lufs.unit_lufs")}</span>
    </div>
  </Dialog>
{/if}

<NormalizeProgressDialog
  job={state.job}
  titleKey="job.normalize_lufs"
  onCancel={cancelNormalizeLufsJob}
  onDismiss={dismissNormalizeLufsJob}
/>

<style>
  .target {
    width: 6rem;
    text-align: right;
  }
</style>
