<script lang="ts">
  import { t, type MessageKey } from "../i18n";

  /**
   * Shared normalize job progress dialog (H-09, SPEC-010 §2.8): a determinate bar and Cancel,
   * shown only once the job is still running after 250 ms ("short selections finish before the
   * dialog would appear, so a favorite on a phrase feels instant"). Reused for both peak
   * (`NormalizeDialog.svelte`) and LUFS (`NormalizeLufsDialog.svelte`) jobs — `titleKey` picks
   * "Normalizing…" vs "Normalizing loudness…".
   */
  interface JobState {
    jobId: number;
    fraction: number;
    state: "running" | "done" | "cancelled" | "failed";
  }

  let {
    job,
    titleKey,
    onCancel,
    onDismiss,
  }: {
    job: JobState | null;
    titleKey: MessageKey;
    onCancel: () => void;
    onDismiss: () => void;
  } = $props();

  let visible = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (job?.state === "running") {
      if (!visible && timer === null) {
        timer = setTimeout(() => {
          visible = true;
          timer = null;
        }, 250);
      }
    } else {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
      visible = false;
      if (job) {
        // A terminal state (done/cancelled/failed): clear the job so a later normalize starts
        // fresh, whether or not the dialog ever became visible.
        onDismiss();
      }
    }
  });
</script>

{#if visible && job}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="normalize-progress-title"
      data-testid="normalize-progress-dialog"
    >
      <h2 id="normalize-progress-title">{t(titleKey)}</h2>
      <progress data-testid="normalize-progress-bar" value={job.fraction} max="1"></progress>
      <div class="actions">
        <button type="button" data-testid="normalize-progress-cancel" onclick={onCancel}>
          {t("dialog.progress.cancel")}
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
    min-width: 16rem;
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

  progress {
    width: 100%;
  }

  .actions {
    display: flex;
    justify-content: flex-end;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }
</style>
