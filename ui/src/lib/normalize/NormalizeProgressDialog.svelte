<script lang="ts">
  import { t, type MessageKey } from "../i18n";
  import { Button, Dialog } from "../ui";

  /**
   * Shared normalize job progress dialog (H-09, SPEC-010 §2.8): a determinate bar and Cancel,
   * shown only once the job is still running after 250 ms ("short selections finish before the
   * dialog would appear, so a favorite on a phrase feels instant"). Reused for both peak
   * (`NormalizeDialog.svelte`) and LUFS (`NormalizeLufsDialog.svelte`) jobs — `titleKey` picks
   * "Normalizing…" vs "Normalizing loudness…". H-25: Dialog shell, a slim accent bar and a
   * tabular percentage.
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
  <Dialog size="sm" title={t(titleKey)} titleId="normalize-progress-title" testid="normalize-progress-dialog">
    <div class="progress-row">
      <progress data-testid="normalize-progress-bar" value={job.fraction} max="1"></progress>
      <span class="percent">{Math.round(job.fraction * 100)}%</span>
    </div>
    {#snippet footer()}
      <Button testid="normalize-progress-cancel" onclick={onCancel}>
        {t("dialog.progress.cancel")}
      </Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .progress-row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-3);
  }

  .progress-row progress {
    flex: 1;
  }

  .percent {
    min-width: 3em;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
</style>
