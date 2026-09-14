<script lang="ts">
  import { t } from "../i18n";
  import { documentState, resolveClipPrompt } from "./document.svelte";

  /**
   * T-209 (SPEC-005 §2.8): "12 samples are above 0 dBFS (peak +1.8 dBFS) and will be clipped in
   * this format." Shown by `document_save`/`document_save_as`'s `dialog.overs` refusal — never
   * for a 32-bit float target (float overs are kept, never a clip).
   */
  const doc = documentState();

  function peakLabel(peakDbfs: number): string {
    const sign = peakDbfs >= 0 ? "+" : "";
    return `${sign}${peakDbfs.toFixed(1)}`;
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveClipPrompt("cancel");
    }
  }
</script>

{#if doc.clipPrompt}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="clip-prompt-title"
      data-testid="clip-prompt-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="clip-prompt-title">{t("dialog.overs.title")}</h2>
      <p>
        {t("dialog.overs.message", {
          count: doc.clipPrompt.count,
          peak: peakLabel(doc.clipPrompt.peakDbfs),
        })}
      </p>
      <div class="actions">
        <button type="button" data-testid="clip-prompt-cancel" onclick={() => resolveClipPrompt("cancel")}>
          {t("dialog.overs.cancel")}
        </button>
        <button
          type="button"
          data-testid="clip-prompt-float"
          onclick={() => resolveClipPrompt("float")}
        >
          {t("dialog.overs.float")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="clip-prompt-clip"
          onclick={() => resolveClipPrompt("clip")}
        >
          {t("dialog.overs.clip")}
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
