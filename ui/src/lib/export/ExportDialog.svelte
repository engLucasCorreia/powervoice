<script lang="ts">
  import { documentState } from "../document/document.svelte";
  import { t } from "../i18n";
  import { Button, Dialog } from "../ui";
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
  <Dialog title={t("dialog.export.title")} titleId="export-title" testid="export-dialog" onkeydown={onKeydown}>
    <fieldset>
      <legend>{t("dialog.export.format")}</legend>
      <div class="options">
        {#each (["wav", "flac", "mp3"] as const) as k (k)}
          <label class="option">
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
      </div>
    </fieldset>
    <div class="columns">
      <fieldset>
        <legend>{t("dialog.export.sample_rate")}</legend>
        <select data-testid="export-rate" bind:value={rateHz}>
          {#each RATES as rate (rate)}
            <option value={rate}>{t("devices.rate_hz", { rate })}</option>
          {/each}
        </select>
      </fieldset>
      {#if kind === "mp3"}
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
      {/if}
    </div>
    {#if kind === "mp3"}
      <fieldset>
        <legend>{t("dialog.export.bitrate_mode")}</legend>
        <div class="options">
          {#each (["cbr", "vbr"] as const) as m (m)}
            <label class="option">
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
        </div>
      </fieldset>
    {:else}
      <fieldset>
        <legend>{t("dialog.export.bit_depth")}</legend>
        <div class="options">
          {#each bitsForKind as depth (depth)}
            <label class="option">
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
        </div>
      </fieldset>
    {/if}
    <fieldset>
      <legend>{t("dialog.export.range")}</legend>
      <div class="options">
        {#each (["whole_file", "selection"] as const) as r (r)}
          <label class="option">
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
      </div>
    </fieldset>
    {#snippet footer()}
      <Button variant="ghost" testid="export-acx" disabled={!exp.mp3Available} onclick={applyAcxPreset}>
        {t("dialog.export.acx_preset")}
      </Button>
      <span class="spacer"></span>
      <Button testid="export-cancel" onclick={cancelExportDialog}>
        {t("dialog.export.cancel")}
      </Button>
      <Button
        variant="primary"
        testid="export-choose"
        onclick={() => void confirmExport(currentFormat(), rateHz, currentRange())}
      >
        {t("dialog.export.choose_location")}
      </Button>
    {/snippet}
  </Dialog>
{/if}

{#if exp.noiseOnlyConfirm}
  <Dialog
    role="alertdialog"
    size="sm"
    title={t("dialog.export.noise_only_confirm.title")}
    titleId="export-noise-only-title"
    testid="export-noise-only-confirm"
    onkeydown={(event) => {
      event.stopPropagation();
      if (event.key === "Escape") {
        cancelNoiseOnlyExport();
      }
    }}
  >
    <p>{t("dialog.export.noise_only_confirm.message")}</p>
    {#snippet footer()}
      <Button testid="export-noise-only-cancel" onclick={cancelNoiseOnlyExport}>
        {t("dialog.export.noise_only_confirm.cancel")}
      </Button>
      <Button variant="primary" testid="export-noise-only-continue" onclick={() => void continueNoiseOnlyExport()}>
        {t("dialog.export.noise_only_confirm.continue")}
      </Button>
    {/snippet}
  </Dialog>
{/if}

{#if exp.job}
  <Dialog size="sm" title={t("dialog.export.progress_title")} testid="export-progress">
    <progress data-testid="export-progress-bar" value={exp.job.fraction} max="1"></progress>
    {#if exp.job.state !== "running"}
      <p data-testid="export-progress-state">
        {exp.job.state === "done"
          ? t("dialog.export.done")
          : exp.job.state === "cancelled"
            ? t("dialog.export.job_cancelled")
            : t("dialog.export.failed")}
      </p>
    {/if}
    {#snippet footer()}
      {#if exp.job?.state === "running"}
        <Button testid="export-progress-cancel" onclick={cancelExportJob}>
          {t("dialog.export.cancel")}
        </Button>
      {:else}
        <Button variant="primary" testid="export-progress-close" onclick={dismissExportJob}>
          {t("dialog.export.close")}
        </Button>
      {/if}
    {/snippet}
  </Dialog>
{/if}

<style>
  .columns {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: var(--pv-space-3);
  }
</style>
