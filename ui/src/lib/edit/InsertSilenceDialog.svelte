<script lang="ts">
  import { t } from "../i18n";
  import {
    applyInsertSilenceDialog,
    closeInsertSilenceDialog,
    insertSilenceState,
    setInsertSilenceDialogText,
  } from "../state/insertSilence.svelte";
  import { Dialog } from "../ui";

  /**
   * Edit → Insert Silence… dialog (H-56, SPEC-008 §2.5): a Duration field that accepts plain
   * seconds, timecode or a `smp` sample count, plus the computed length and insertion point.
   * Enter = OK, Esc = Cancel; OK is disabled while the field is invalid or out of range.
   */
  const state = insertSilenceState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closeInsertSilenceDialog();
    } else if (event.key === "Enter" && state.dialogValid) {
      void applyInsertSilenceDialog();
    }
  }
</script>

{#if state.dialogOpen}
  <Dialog
    actions={[
      {
        label: t("dialog.insert_silence.cancel"),
        role: "cancel",
        testid: "insert-silence-dialog-cancel",
        onclick: closeInsertSilenceDialog,
      },
      {
        label: t("dialog.insert_silence.ok"),
        role: "primary",
        testid: "insert-silence-dialog-ok",
        disabled: !state.dialogValid,
        onclick: () => void applyInsertSilenceDialog(),
      },
    ]}
    size="sm"
    title={t("dialog.insert_silence.title")}
    titleId="insert-silence-dialog-title"
    testid="insert-silence-dialog"
    onkeydown={onKeydown}
  >
    <div class="field-row">
      <label for="insert-silence-dialog-duration">{t("dialog.insert_silence.duration")}</label>
      <input
        id="insert-silence-dialog-duration"
        class="duration"
        type="text"
        inputmode="text"
        autocomplete="off"
        data-testid="insert-silence-dialog-duration"
        class:invalid={!state.dialogValid}
        aria-invalid={!state.dialogValid ? "true" : undefined}
        value={state.dialogText}
        oninput={(e) => setInsertSilenceDialogText(e.currentTarget.value)}
      />
    </div>
    {#if state.dialogValid && state.lengthLabel}
      <p class="hint" data-testid="insert-silence-dialog-length">
        {t("dialog.insert_silence.length", { length: state.lengthLabel })}
      </p>
    {:else}
      <p class="hint error" data-testid="insert-silence-dialog-error">
        {t("dialog.insert_silence.range_error")}
      </p>
    {/if}
    <p class="hint" data-testid="insert-silence-dialog-at">
      {t("dialog.insert_silence.at", { position: state.positionLabel })}
    </p>
  </Dialog>
{/if}

<style>
  .field-row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-3);
  }

  .duration {
    width: 10rem;
    text-align: right;
  }

  .duration.invalid {
    border-color: var(--pv-danger-text);
  }

  .hint {
    margin: var(--pv-space-2) 0 0;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
  }

  .hint.error {
    color: var(--pv-danger-text);
  }
</style>
