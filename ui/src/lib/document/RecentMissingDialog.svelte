<script lang="ts">
  import { t } from "../i18n";
  import { recentFilesState, resolveRecentMissingPrompt } from "./recentFiles.svelte";

  /**
   * H-15 (SPEC-018 §2.12, ticket-added "Locate…"): picking a missing `File → Open Recent` entry
   * shows this instead of failing through the normal open flow's error toast. "Locate…" opens the
   * native picker and re-points the entry to whatever the user picks (`recentFiles.svelte.ts`);
   * "Remove from List" drops the dead entry; "Cancel" leaves the list untouched.
   */
  const recent = recentFilesState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveRecentMissingPrompt("cancel");
    }
  }
</script>

{#if recent.missingPrompt}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="recent-missing-title"
      data-testid="recent-missing-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="recent-missing-title">{t("dialog.recent_missing.title")}</h2>
      <p>{t("dialog.recent_missing.message", { name: recent.missingPrompt.name })}</p>
      <div class="actions">
        <button
          type="button"
          data-testid="recent-missing-cancel"
          onclick={() => resolveRecentMissingPrompt("cancel")}
        >
          {t("dialog.recent_missing.cancel")}
        </button>
        <button
          type="button"
          data-testid="recent-missing-remove"
          onclick={() => resolveRecentMissingPrompt("remove")}
        >
          {t("dialog.recent_missing.remove")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="recent-missing-locate"
          onclick={() => resolveRecentMissingPrompt("locate")}
        >
          {t("dialog.recent_missing.locate")}
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
