<script lang="ts">
  import type { BitDepth, DefaultFormatDto } from "../ipc/bindings";
  import { t } from "../i18n";
  import { Button, Dialog } from "../ui";
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
  <Dialog
    title={t("dialog.new_recording.title")}
    titleId="new-recording-title"
    testid="new-recording-dialog"
    onkeydown={onKeydown}
  >
    <fieldset>
      <legend>{t("dialog.new_recording.sample_rate")}</legend>
      <div class="options">
        {#each SAMPLE_RATES as rate (rate)}
          <label class="option">
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
      </div>
    </fieldset>
    <fieldset>
      <legend>{t("dialog.new_recording.bit_depth")}</legend>
      <div class="options">
        {#each BIT_DEPTHS as depth (depth)}
          <label class="option">
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
      </div>
    </fieldset>
    {#snippet footer()}
      <Button testid="new-recording-cancel" onclick={cancelNewRecordingPrompt}>
        {t("dialog.new_recording.cancel")}
      </Button>
      <Button variant="primary" icon="record" testid="new-recording-confirm" loading={busy} onclick={() => void confirm()}>
        {t("dialog.new_recording.record")}
      </Button>
    {/snippet}
  </Dialog>
{/if}
