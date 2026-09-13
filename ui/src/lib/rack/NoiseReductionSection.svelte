<script lang="ts">
  import { t } from "../i18n";
  import type { NoiseProfileStatusDto } from "../ipc/bindings";
  import { canCapture, cancelCapture, isCapturing, startCapture } from "./nrCapture.svelte";
  import { hasSelection } from "../state/selection.svelte";
  import { recordState } from "../state/record.svelte";

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

<div class="nr-capture" data-testid="nr-capture">
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
  </div>
  <p class="status" data-testid="nr-capture-status">{t(statusKey)}</p>
</div>

<style>
  .nr-capture {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    margin: 0.3rem 0;
    padding: 0.3rem;
    background: var(--surface-inset);
    border-radius: 4px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .capture {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.2rem 0.6rem;
  }

  .capture:hover:not(:disabled) {
    border-color: var(--accent);
  }

  .capture:disabled {
    color: var(--text-disabled);
  }

  .spinner {
    width: 0.9rem;
    height: 0.9rem;
    border: 2px solid var(--surface-border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  .status {
    margin: 0;
    font-size: 0.78rem;
    color: var(--text-secondary);
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
