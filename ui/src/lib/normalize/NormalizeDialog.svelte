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
  import NormalizeProgressDialog from "./NormalizeProgressDialog.svelte";

  /**
   * Effects → Normalize… dialog (S2-02/H-09, SPEC-010 §2.4). dB or % mode (a two-way toggle);
   * Enter applies, Esc cancels; Apply is disabled while the field is invalid. Runs as a job
   * (`NormalizeProgressDialog`): the dialog itself closes immediately on Apply, per SPEC-010's
   * "one click, no confirmation" — the progress modal is a separate concern for long files.
   */
  const state = normalizeState();

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
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="normalize-dialog-title"
      data-testid="normalize-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="normalize-dialog-title">{t("dialog.normalize.title")}</h2>
      <label class="field">
        <span>{t("dialog.normalize.target_label")}</span>
        <input
          type="text"
          inputmode="decimal"
          data-testid="normalize-dialog-target"
          class:invalid={!state.dialogValid}
          value={state.dialogText}
          oninput={(e) => setNormalizeDialogText(e.currentTarget.value)}
        />
        <div class="unit-toggle" role="group" aria-label={t("dialog.normalize.target_label")}>
          <button
            type="button"
            data-testid="normalize-dialog-unit-db"
            class:active={state.dialogUnit === "db"}
            onclick={() => setNormalizeDialogUnit("db")}
          >
            {t("dialog.normalize.unit_db")}
          </button>
          <button
            type="button"
            data-testid="normalize-dialog-unit-pct"
            class:active={state.dialogUnit === "pct"}
            onclick={() => setNormalizeDialogUnit("pct")}
          >
            {t("dialog.normalize.unit_pct")}
          </button>
        </div>
      </label>
      <div class="actions">
        <button type="button" data-testid="normalize-dialog-cancel" onclick={closeNormalizeDialog}>
          {t("dialog.normalize.cancel")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="normalize-dialog-apply"
          disabled={!state.dialogValid}
          onclick={() => void applyNormalizeDialog()}
        >
          {t("dialog.normalize.apply")}
        </button>
      </div>
    </div>
  </div>
{/if}

<NormalizeProgressDialog
  job={state.job}
  titleKey="job.normalize"
  onCancel={cancelNormalizeJob}
  onDismiss={dismissNormalizeJob}
/>

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
    min-width: 20rem;
    max-width: 90vw;
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

  .field {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  input {
    width: 6rem;
    padding: 0.25rem 0.4rem;
    background: var(--surface-inset);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    font-variant-numeric: tabular-nums;
  }

  input.invalid {
    border-color: var(--error, #c0392b);
  }

  .unit-toggle {
    display: flex;
  }

  .unit-toggle button {
    padding: 0.2rem 0.5rem;
    color: var(--text-secondary);
  }

  .unit-toggle button:first-child {
    border-top-right-radius: 0;
    border-bottom-right-radius: 0;
  }

  .unit-toggle button:last-child {
    border-top-left-radius: 0;
    border-bottom-left-radius: 0;
    border-left: none;
  }

  .unit-toggle button.active {
    color: var(--accent);
    border-color: var(--accent);
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

  button.primary {
    border-color: var(--accent);
    color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }
</style>
