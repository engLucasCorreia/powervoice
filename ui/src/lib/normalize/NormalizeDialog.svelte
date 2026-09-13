<script lang="ts">
  import { t } from "../i18n";
  import {
    applyNormalizeDialog,
    closeNormalizeDialog,
    normalizeState,
    setNormalizeDialogText,
  } from "../state/normalize.svelte";

  /**
   * Effects → Normalize… dialog (S2-02, SPEC-010 §2.4). dB mode only (−60.00…0.00, ticket scope —
   * the % toggle is deferred). Enter applies, Esc cancels; Apply is disabled while the field is
   * invalid.
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
        <span class="unit">{t("dialog.normalize.unit_db")}</span>
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

  .unit {
    color: var(--text-secondary);
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
