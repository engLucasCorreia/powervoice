<script lang="ts">
  import type { BitDepth, DefaultFormatDto } from "../ipc/bindings";
  import { t } from "../i18n";
  import { cancelNewRecordingPrompt, confirmNewRecordingPrompt, recordState } from "../state/record.svelte";
  import { saveSettings } from "../state/settings.svelte";

  /**
   * New Recording dialog (H-06, File → New Recording…, SPEC-002 §2.2): sample rate and save bit
   * depth for a fresh recording, prefilled from the current default format. Confirming saves the
   * choice as the new default (`saveSettings`), then replaces the document (after the standard
   * unsaved-changes prompt) and starts recording at the chosen format.
   */
  const rec = recordState();
  let sampleRateHz = $state(48_000);
  let bitDepth = $state<BitDepth>("24");
  let busy = $state(false);

  $effect(() => {
    if (rec.newRecordingPrompt) {
      sampleRateHz = rec.newRecordingPrompt.sample_rate_hz;
      bitDepth = rec.newRecordingPrompt.bit_depth;
    }
  });

  // SPEC-002 §2.2: {44 100, 48 000, 88 200, 96 000} Hz.
  const SAMPLE_RATES = [44_100, 48_000, 88_200, 96_000] as const;
  const BIT_DEPTHS: BitDepth[] = ["16", "24", "32f"];

  async function confirm(): Promise<void> {
    busy = true;
    try {
      const format: DefaultFormatDto = { sample_rate_hz: sampleRateHz, bit_depth: bitDepth };
      await saveSettings({ default_format: format });
      await confirmNewRecordingPrompt(format);
    } finally {
      busy = false;
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      cancelNewRecordingPrompt();
    }
  }
</script>

{#if rec.newRecordingPrompt}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="new-recording-title"
      data-testid="new-recording-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="new-recording-title">{t("dialog.new_recording.title")}</h2>
      <fieldset>
        <legend>{t("dialog.new_recording.sample_rate")}</legend>
        {#each SAMPLE_RATES as rate (rate)}
          <label>
            <input
              type="radio"
              name="new-recording-rate"
              value={rate}
              checked={sampleRateHz === rate}
              onchange={() => (sampleRateHz = rate)}
            />
            {t(`dialog.new_recording.sample_rate.${rate}` as const)}
          </label>
        {/each}
      </fieldset>
      <fieldset>
        <legend>{t("dialog.new_recording.bit_depth")}</legend>
        {#each BIT_DEPTHS as depth (depth)}
          <label>
            <input
              type="radio"
              name="new-recording-bits"
              value={depth}
              checked={bitDepth === depth}
              onchange={() => (bitDepth = depth)}
            />
            {t(`dialog.new_recording.bit_depth.${depth}` as const)}
          </label>
        {/each}
      </fieldset>
      <div class="actions">
        <button type="button" data-testid="new-recording-cancel" onclick={cancelNewRecordingPrompt}>
          {t("dialog.new_recording.cancel")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="new-recording-confirm"
          disabled={busy}
          onclick={() => void confirm()}
        >
          {t("dialog.new_recording.record")}
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

  fieldset {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.5rem 0.75rem;
  }

  legend {
    color: var(--text-secondary);
    padding: 0 0.25rem;
  }

  label {
    display: flex;
    align-items: center;
    gap: 0.4rem;
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

  button:disabled {
    color: var(--text-disabled);
  }
</style>
