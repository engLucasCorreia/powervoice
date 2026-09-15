<script lang="ts">
  import { t } from "../i18n";
  import type { NoiseProfileStatusDto } from "../ipc/bindings";
  import { canCapture, cancelCapture, isCapturing, startCapture } from "./nrCapture.svelte";
  import { hasSelection } from "../state/selection.svelte";
  import { recordState } from "../state/record.svelte";
  import TourButton from "../tour/TourButton.svelte";

  /**
   * Capture Noise Print status line + button (S3-06, SPEC-014 §2.8, item 1). Rendered only for a
   * slot whose module exposes the `NoiseProfile` extension (`RackSlot.svelte` checks). The Capture
   * button always targets *this* slot directly (unambiguous — the "last-focused slot" resolution
   * SPEC-014 §2.3 describes only matters for the global Shift+P keymap action).
   */
  let { slotIndex, status }: { slotIndex: number; status: NoiseProfileStatusDto } = $props();

  const capturing = $derived(isCapturing(slotIndex));
  const enabled = $derived(canCapture() && !capturing);
  const rec = recordState();

  const disabledHint = $derived(
    rec.state.recording
      ? t("module.noise_reduction.capture.disabled_recording")
      : !hasSelection()
        ? t("module.noise_reduction.capture.disabled_no_selection")
        : "",
  );

  const statusKey = $derived.by(() => {
    if (capturing) {
      return "module.noise_reduction.status.capturing" as const;
    }
    switch (status) {
      case "loaded":
        return "module.noise_reduction.status.loaded" as const;
      case "unreadable":
        return "module.noise_reduction.status.unreadable" as const;
      case "too_new":
        return "module.noise_reduction.status.too_new" as const;
      default:
        return "module.noise_reduction.status.none" as const;
    }
  });
</script>

<div class="nr-capture" data-testid="nr-capture" data-tour="nr-capture">
  <div class="row">
    {#if capturing}
      <button
        type="button"
        class="capture"
        data-testid="nr-capture-cancel"
        onclick={() => cancelCapture()}
      >
        {t("module.noise_reduction.capture.cancel")}
      </button>
      <span class="spinner" aria-hidden="true" data-testid="nr-capture-spinner"></span>
    {:else}
      <button
        type="button"
        class="capture"
        data-testid="nr-capture-button"
        disabled={!enabled}
        title={enabled ? t("module.noise_reduction.capture.hint") : disabledHint}
        onclick={() => void startCapture(slotIndex)}
      >
        {t("module.noise_reduction.capture")}
      </button>
    {/if}
    <span class="tour-help"><TourButton tour="noise" /></span>
  </div>
  <p class="status" data-testid="nr-capture-status">{t(statusKey)}</p>
</div>

<style>
  .nr-capture {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    font-family: var(--pv-font-sans);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
  }

  /* T-709: the Noise tour's "?" at the end of the capture row. */
  .tour-help {
    margin-left: auto;
  }

  .capture {
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
    font-weight: var(--pv-weight-medium);
    cursor: default;
  }

  .capture:hover:not(:disabled) {
    background: var(--pv-control-bg-hover);
  }

  .capture:disabled {
    color: var(--pv-text-disabled);
    border-color: var(--pv-border-subtle);
  }

  .capture:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--pv-control-track);
    border-top-color: var(--pv-accent);
    border-radius: var(--pv-radius-full);
    animation: pv-nr-spin 0.9s linear infinite;
  }

  @keyframes pv-nr-spin {
    to {
      transform: rotate(360deg);
    }
  }

  .status {
    margin: 0;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }
</style>
