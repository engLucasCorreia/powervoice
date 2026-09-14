<script lang="ts">
  import { tDynamic } from "../i18n";
  import { documentState, resolveConfirmPrompt } from "./document.svelte";

  /**
   * T-306 (SPEC-018 §2.9/§2.11): "already open in another instance" and "changed on disk"
   * confirmations, shown by the document store before Open/Save proceeds with something that
   * could overwrite someone else's changes. Same shape for both — only the copy differs.
   */
  const doc = documentState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveConfirmPrompt(false);
    }
  }
</script>

{#if doc.confirmPrompt}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="confirm-dialog-title"
      data-testid="confirm-dialog"
      data-kind={doc.confirmPrompt.kind}
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="confirm-dialog-title">{tDynamic(`dialog.${doc.confirmPrompt.kind}.title`)}</h2>
      <p>{tDynamic(`dialog.${doc.confirmPrompt.kind}.message`, { name: doc.confirmPrompt.name })}</p>
      <div class="actions">
        <button
          type="button"
          data-testid="confirm-cancel"
          onclick={() => resolveConfirmPrompt(false)}
        >
          {tDynamic(`dialog.${doc.confirmPrompt.kind}.cancel`)}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="confirm-proceed"
          onclick={() => resolveConfirmPrompt(true)}
        >
          {tDynamic(`dialog.${doc.confirmPrompt.kind}.confirm`)}
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
