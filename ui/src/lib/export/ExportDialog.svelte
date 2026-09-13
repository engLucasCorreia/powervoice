<script lang="ts">
  import { documentState } from "../document/document.svelte";
  import { t } from "../i18n";
  import type { BitDepth, ExportFormatDto, ExportRangeDto, Mp3SettingsDto } from "../ipc/bindings";
  import { hasSelection, selectionState } from "../state/selection.svelte";
  import {
    cancelExportDialog,
    cancelExportJob,
    cancelNoiseOnlyExport,
    confirmExport,
    continueNoiseOnlyExport,
    dismissExportJob,
    exportState,
  } from "./export.svelte";

  /**
   * Export dialog (S4-04, H-08): format (WAV/FLAC/MP3), rate, bit depth/bitrate (or MP3
   * CBR/VBR), an ACX preset button, a Whole file / Selection range choice, and destination via
   * the native save dialog. The "Output noise only" confirmation (SPEC-014 §2.6) is a second,
   * separate dialog driven by `exp.noiseOnlyConfirm` (see `export.svelte.ts`).
   */
  const doc = documentState();
  const exp = exportState();
  const sel = selectionState();

  type Kind = "wav" | "flac" | "mp3";
  type Mp3Mode = "cbr" | "vbr";
  type Range = "whole_file" | "selection";
  const RATES = [44_100, 48_000, 88_200, 96_000];
  const WAV_BITS: BitDepth[] = ["16", "24", "32f"];
  const FLAC_BITS: BitDepth[] = ["16", "24"];
  const MP3_KBPS = [128, 160, 192, 224, 256, 320];
  const VBR_QUALITIES = [0, 1, 2, 3, 4];

  let kind = $state<Kind>("wav");
  let bits = $state<BitDepth>("24");
  let mp3Mode = $state<Mp3Mode>("cbr");
  let kbps = $state(192);
  let vbrQuality = $state(2);
  let rateHz = $state(48_000);
  let range = $state<Range>("whole_file");

  $effect(() => {
    if (exp.prompt) {
      kind = "wav";
      bits = "24";
      mp3Mode = "cbr";
      kbps = 192;
      vbrQuality = 2;
      rateHz = doc.current.sample_rate_hz || 48_000;
      range = "whole_file";
    }
  });

  const bitsForKind = $derived(kind === "flac" ? FLAC_BITS : WAV_BITS);
  const canExportSelection = $derived(hasSelection());

  function applyAcxPreset(): void {
    kind = "mp3";
    mp3Mode = "cbr";
    kbps = 192;
    rateHz = 44_100;
  }

  function currentFormat(): ExportFormatDto {
    if (kind === "wav") {
      return { kind: "wav", bits };
    }
    if (kind === "flac") {
      return { kind: "flac", bits: bits === "32f" ? "24" : bits };
    }
    const settings: Mp3SettingsDto =
      mp3Mode === "cbr" ? { kind: "cbr", kbps } : { kind: "vbr", quality: vbrQuality };
    return { kind: "mp3", settings };
  }

  function currentRange(): ExportRangeDto | null {
    if (range !== "selection") {
      return null;
    }
    const current = sel.current;
    if (!current) {
      return null;
    }
    return { start_sample: current.startSample, end_sample: current.endSample };
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      cancelExportDialog();
    }
  }
</script>

{#if exp.prompt}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="export-title"
      data-testid="export-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="export-title">{t("dialog.export.title")}</h2>

      <fieldset>
        <legend>{t("dialog.export.format")}</legend>
        {#each (["wav", "flac", "mp3"] as const) as k (k)}
          <label>
            <input
              type="radio"
              name="export-kind"
              value={k}
              checked={kind === k}
              disabled={k === "mp3" && !exp.mp3Available}
              onchange={() => (kind = k)}
            />
            {t(`dialog.export.format.${k}` as const)}
            {#if k === "mp3" && !exp.mp3Available}
              <span class="hint">{t("dialog.export.mp3_unavailable")}</span>
            {/if}
          </label>
        {/each}
      </fieldset>

      <fieldset>
        <legend>{t("dialog.export.sample_rate")}</legend>
        <select data-testid="export-rate" bind:value={rateHz}>
          {#each RATES as rate (rate)}
            <option value={rate}>{t("devices.rate_hz", { rate })}</option>
          {/each}
        </select>
      </fieldset>

      {#if kind === "mp3"}
        <fieldset>
          <legend>{t("dialog.export.bitrate_mode")}</legend>
          {#each (["cbr", "vbr"] as const) as m (m)}
            <label>
              <input
                type="radio"
                name="export-mp3-mode"
                value={m}
                checked={mp3Mode === m}
                onchange={() => (mp3Mode = m)}
              />
              {t(`dialog.export.bitrate_mode.${m}` as const)}
            </label>
          {/each}
        </fieldset>
        {#if mp3Mode === "cbr"}
          <fieldset>
            <legend>{t("dialog.export.bitrate")}</legend>
            <select data-testid="export-bitrate" bind:value={kbps}>
              {#each MP3_KBPS as rate (rate)}
                <option value={rate}>{rate} kbps</option>
              {/each}
            </select>
          </fieldset>
        {:else}
          <fieldset>
            <legend>{t("dialog.export.vbr_quality")}</legend>
            <select data-testid="export-vbr-quality" bind:value={vbrQuality}>
              {#each VBR_QUALITIES as q (q)}
                <option value={q}>V{q}</option>
              {/each}
            </select>
          </fieldset>
        {/if}
      {:else}
        <fieldset>
          <legend>{t("dialog.export.bit_depth")}</legend>
          {#each bitsForKind as depth (depth)}
            <label>
              <input
                type="radio"
                name="export-bits"
                value={depth}
                checked={bits === depth}
                onchange={() => (bits = depth)}
              />
              {t(`dialog.save_as.bit_depth.${depth}` as const)}
            </label>
          {/each}
        </fieldset>
      {/if}

      <fieldset>
        <legend>{t("dialog.export.range")}</legend>
        {#each (["whole_file", "selection"] as const) as r (r)}
          <label>
            <input
              type="radio"
              name="export-range"
              value={r}
              checked={range === r}
              disabled={r === "selection" && !canExportSelection}
              onchange={() => (range = r)}
            />
            {t(`dialog.export.range.${r}` as const)}
          </label>
        {/each}
      </fieldset>

      <div class="actions">
        <button type="button" data-testid="export-acx" onclick={applyAcxPreset} disabled={!exp.mp3Available}>
          {t("dialog.export.acx_preset")}
        </button>
        <span class="spacer"></span>
        <button type="button" data-testid="export-cancel" onclick={cancelExportDialog}>
          {t("dialog.export.cancel")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="export-choose"
          onclick={() => void confirmExport(currentFormat(), rateHz, currentRange())}
        >
          {t("dialog.export.choose_location")}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if exp.noiseOnlyConfirm}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="export-noise-only-title"
      data-testid="export-noise-only-confirm"
      tabindex="-1"
      onkeydown={(event) => {
        event.stopPropagation();
        if (event.key === "Escape") {
          cancelNoiseOnlyExport();
        }
      }}
    >
      <h2 id="export-noise-only-title">{t("dialog.export.noise_only_confirm.title")}</h2>
      <p>{t("dialog.export.noise_only_confirm.message")}</p>
      <div class="actions">
        <span class="spacer"></span>
        <button type="button" data-testid="export-noise-only-cancel" onclick={cancelNoiseOnlyExport}>
          {t("dialog.export.noise_only_confirm.cancel")}
        </button>
        <button
          type="button"
          class="primary"
          data-testid="export-noise-only-continue"
          onclick={() => void continueNoiseOnlyExport()}
        >
          {t("dialog.export.noise_only_confirm.continue")}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if exp.job}
  <div class="backdrop">
    <div class="dialog" data-testid="export-progress" role="status">
      <h2>{t("dialog.export.progress_title")}</h2>
      <progress data-testid="export-progress-bar" value={exp.job.fraction} max="1"></progress>
      {#if exp.job.state === "running"}
        <div class="actions">
          <button type="button" data-testid="export-progress-cancel" onclick={cancelExportJob}>
            {t("dialog.export.cancel")}
          </button>
        </div>
      {:else}
        <p data-testid="export-progress-state">
          {exp.job.state === "done"
            ? t("dialog.export.done")
            : exp.job.state === "cancelled"
              ? t("dialog.export.job_cancelled")
              : t("dialog.export.failed")}
        </p>
        <div class="actions">
          <button type="button" data-testid="export-progress-close" onclick={dismissExportJob}>
            {t("dialog.export.close")}
          </button>
        </div>
      {/if}
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

  .hint {
    color: var(--text-disabled);
    font-size: 0.85em;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  .spacer {
    flex: 1;
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

  progress {
    width: 100%;
  }
</style>
