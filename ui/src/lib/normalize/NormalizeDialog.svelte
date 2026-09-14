<script lang="ts">
  import { t } from "../i18n";
  import {
    applyNormalizeDialog,
    cancelNormalizeJob,
    closeNormalizeDialog,
    dismissNormalizeJob,
    normalizeState,
    setNormalizeDialogText,
    setNormalizeDialogUnit,
  } from "../state/normalize.svelte";
  import { Button, Dialog, SegmentedControl, type SegmentOption } from "../ui";
  import NormalizeProgressDialog from "./NormalizeProgressDialog.svelte";

  /**
   * Effects → Normalize… dialog (S2-02/H-09, SPEC-010 §2.4). dB or % mode (a two-way toggle);
   * Enter applies, Esc cancels; Apply is disabled while the field is invalid. Runs as a job
   * (`NormalizeProgressDialog`): the dialog itself closes immediately on Apply, per SPEC-010's
   * "one click, no confirmation" — the progress modal is a separate concern for long files.
   * H-25: Dialog shell; the unit toggle is a SegmentedControl.
   */
  const state = normalizeState();

  const UNITS: SegmentOption<"db" | "pct">[] = [
    { value: "db", label: t("dialog.normalize.unit_db"), testid: "normalize-dialog-unit-db" },
    { value: "pct", label: t("dialog.normalize.unit_pct"), testid: "normalize-dialog-unit-pct" },
  ];

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closeNormalizeDialog();
    } else if (event.key === "Enter" && state.dialogValid) {
      void applyNormalizeDialog();
    }
  }
</script>

{#if state.dialogOpen}
  <Dialog
    size="sm"
    title={t("dialog.normalize.title")}
    titleId="normalize-dialog-title"
    testid="normalize-dialog"
    onkeydown={onKeydown}
  >
    <div class="field-row">
      <label for="normalize-dialog-target-input">{t("dialog.normalize.target_label")}</label>
      <input
        id="normalize-dialog-target-input"
        class="target"
        type="text"
        inputmode="decimal"
        autocomplete="off"
        data-testid="normalize-dialog-target"
        class:invalid={!state.dialogValid}
        aria-invalid={!state.dialogValid ? "true" : undefined}
        value={state.dialogText}
        oninput={(e) => setNormalizeDialogText(e.currentTarget.value)}
      />
      <SegmentedControl
        options={UNITS}
        value={state.dialogUnit}
        label={t("dialog.normalize.target_label")}
        size="sm"
        onchange={setNormalizeDialogUnit}
      />
    </div>
    {#snippet footer()}
      <Button testid="normalize-dialog-cancel" onclick={closeNormalizeDialog}>
        {t("dialog.normalize.cancel")}
      </Button>
      <Button
        variant="primary"
        testid="normalize-dialog-apply"
        disabled={!state.dialogValid}
        onclick={() => void applyNormalizeDialog()}
      >
        {t("dialog.normalize.apply")}
      </Button>
    {/snippet}
  </Dialog>
{/if}

<NormalizeProgressDialog
  job={state.job}
  titleKey="job.normalize"
  onCancel={cancelNormalizeJob}
  onDismiss={dismissNormalizeJob}
/>

<style>
  .target {
    width: 6rem;
    text-align: right;
  }
</style>
