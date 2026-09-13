<script lang="ts">
  import { t } from "../i18n";
  import { recordState, resolveLowDiskPrompt } from "../state/record.svelte";

  /**
   * "Only N min of disk space left. Record anyway?" confirm prompt (H-11, SPEC-002 §2.5), shown
   * by the record store before Record starts a take with less than `DISK_WARN_MINUTES` of
   * estimated recording time left on the session volume.
   */
  const rec = recordState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveLowDiskPrompt(false);
    }
  }
</script>

{#if rec.lowDiskPrompt}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="low-disk-title"
      data-testid="low-disk-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="low-disk-title">{t("dialog.low_disk.title")}</h2>
      <p>{t("dialog.low_disk.message", { minutes: String(rec.lowDiskPrompt.minutes) })}</p>
      <div class="actions">
        <button type="button" data-testid="low-disk-cancel" onclick={() => resolveLowDiskPrompt(false)}>
          {t("dialog.low_disk.cancel")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="low-disk-confirm"
          onclick={() => resolveLowDiskPrompt(true)}
        >
          {t("dialog.low_disk.confirm")}
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
