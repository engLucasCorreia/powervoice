<script lang="ts">
  import { t } from "../i18n";
  import { formatTime } from "../transport/playhead";
  import { cancelImportJob, dismissImportJob, documentState } from "./document.svelte";

  /**
   * H-20 (SPEC-005 §2.3): the import job's progress, shown as a slim **non-modal** bar (unlike
   * `NormalizeProgressDialog`'s full-screen backdrop) — "while importing: zoom, scroll and
   * selection work" requires the rest of the editor to stay visible and interactive underneath.
   * The document shell (file name, rate and — when the container states one — length) comes from
   * `import_started`, applied well before the import completes (`documentState().importJob`,
   * populated by `applyImportStarted`). This is the "simpler acceptable design" of SPEC-005
   * §2.3's progressive waveform: the shell's name/length/progress bar, without live partial peaks
   * (see the ticket report for why). The document underneath — if one was already open — is left
   * completely alone until the import commits, so Cancel/error need no restore step of their own.
   */
  const doc = documentState();

  const lengthLabel = $derived.by(() => {
    const job = doc.importJob;
    if (!job || job.lenSamples === null) {
      return null;
    }
    return formatTime(job.lenSamples, job.sampleRateHz);
  });

  $effect(() => {
    const job = doc.importJob;
    if (job && job.state !== "running") {
      // The bar has nothing left to show once the job is no longer running — a successful import
      // is already reflected by `document_changed`; a cancelled/failed one already restored the
      // previous document's title (`applyImportJobProgress`).
      dismissImportJob();
    }
  });
</script>

{#if doc.importJob && doc.importJob.state === "running"}
  <div class="bar" data-testid="import-progress-bar" role="status">
    <span class="label">
      {t("dialog.import.opening", {
        name: doc.importJob.name,
        percent: String(Math.round(doc.importJob.fraction * 100)),
      })}
    </span>
    {#if lengthLabel}
      <span class="length" data-testid="import-progress-length">{lengthLabel}</span>
    {/if}
    <progress data-testid="import-progress-value" value={doc.importJob.fraction} max="1"></progress>
    <button type="button" data-testid="import-progress-cancel" onclick={cancelImportJob}>
      {t("dialog.progress.cancel")}
    </button>
  </div>
{/if}

<style>
  .bar {
    position: fixed;
    top: 0;
    left: 0;
    right: 0;
    z-index: 900;
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.35rem 0.75rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
    color: var(--text-primary);
    font-size: 0.85rem;
  }

  .label {
    flex: 0 0 auto;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .length {
    flex: 0 0 auto;
    white-space: nowrap;
    opacity: 0.75;
    font-variant-numeric: tabular-nums;
  }

  progress {
    flex: 1 1 auto;
    min-width: 6rem;
  }

  button {
    flex: 0 0 auto;
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.15rem 0.6rem;
  }
</style>
