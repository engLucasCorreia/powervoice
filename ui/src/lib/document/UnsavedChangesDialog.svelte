<script lang="ts">
  import { t } from "../i18n";
  import { documentState, resolveUnsavedPrompt } from "./document.svelte";

  /**
   * Save / Don't Save / Cancel prompt (SPEC-004 §2.8 "simple version"), shown by the document
   * store before Open or quit would discard unsaved changes.
   */
  const doc = documentState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveUnsavedPrompt("cancel");
    }
  }
</script>

{#if doc.unsavedPrompt}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="unsaved-changes-title"
      data-testid="unsaved-changes-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="unsaved-changes-title">{t("dialog.unsaved.title")}</h2>
      <p>{t("dialog.unsaved.message", { name: doc.unsavedPrompt.name })}</p>
      <div class="actions">
        <button
          type="button"
          data-testid="unsaved-cancel"
          onclick={() => resolveUnsavedPrompt("cancel")}
        >
          {t("dialog.unsaved.cancel")}
        </button>
        <button
          type="button"
          data-testid="unsaved-discard"
          onclick={() => resolveUnsavedPrompt("discard")}
        >
          {t("dialog.unsaved.discard")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="unsaved-save"
          onclick={() => resolveUnsavedPrompt("save")}
        >
          {t("dialog.unsaved.save")}
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
    min-width: 24rem;
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

  p {
    margin: 0;
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
</style>
