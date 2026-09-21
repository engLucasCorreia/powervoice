<script lang="ts">
  import { t, type MessageKey } from "../i18n";
  import type { Notice, NoticeLevel, VoiceReportDto } from "../ipc/bindings";
  import { Button, IconButton, StatusDot } from "../ui";
  import { formatNumber } from "../ui/units";
  import { pushNotice } from "../state/notices.svelte";
  import {
    assessReport,
    formatFreqShort,
    NOTABLE_OCTAVE_CORRECTION,
    TONE_ZONES,
    type EqAction,
    type Finding,
    type FindingId,
    type Severity,
  } from "./diagnosticsHints";
  import { applyEqAction } from "./eqApply";
  import { formatNote } from "./notes";

  /**
   * Voice diagnostics (H-42, SPEC-007 §8.6–§8.10): pitch, tone balance, sibilance with its
   * de-esser target, hum, rumble, noise floor and SNR — each with a plain-language hint, a
   * status dot (never colour alone: the hint says the same), and where an EQ move helps, "Add EQ
   * band here" (through the rack's own commands; undo it in the rack). Used in the dock beside the
   * analyzer and in the Spectrum Inspector.
   */

  let {
    report,
    scopeText,
    emptyText,
    testid = "diagnostics",
    onclose,
  }: {
    report: VoiceReportDto | null;
    /** "Live · 9.8 s of audio" / "Average · …". */
    scopeText: string;
    /** Shown with no report yet. */
    emptyText: string;
    testid?: string;
    onclose?: () => void;
  } = $props();

  const findings = $derived(report ? assessReport(report) : []);

  function finding(id: FindingId): Finding | undefined {
    return findings.find((f) => f.id === id);
  }

  const SEVERITY_TONE: Record<Severity, "success" | "accent" | "warning"> = {
    ok: "success",
    info: "accent",
    warn: "warning",
  };

  function hintText(f: Finding | undefined): string {
    return f ? t(f.hintKey, f.params) : "";
  }

  function relative(db: number | null | undefined): string {
    if (db === null || db === undefined || !Number.isFinite(db)) {
      return "—";
    }
    return t("analyzer.diag.relative", { db: db > 0 ? `+${formatNumber(db, 1)}` : formatNumber(db, 1) });
  }

  interface ToneRow {
    id: "mud" | "presence" | "air";
    labelKey: MessageKey;
    value: number | null;
    domain: [number, number];
    zone: readonly [number, number];
  }

  const toneRows = $derived.by((): ToneRow[] => {
    const tone = report?.tone;
    if (!tone) {
      return [];
    }
    const rows: ToneRow[] = [
      { id: "mud", labelKey: "analyzer.diag.mud", value: tone.mud_db, domain: [-20, 20], zone: [TONE_ZONES.mud.low, TONE_ZONES.mud.high] },
    ];
    if (tone.presence_db !== null) {
      rows.push({
        id: "presence",
        labelKey: "analyzer.diag.presence",
        value: tone.presence_db,
        domain: [-30, 10],
        zone: [TONE_ZONES.presence.low, TONE_ZONES.presence.high],
      });
    }
    if (tone.air_db !== null) {
      rows.push({ id: "air", labelKey: "analyzer.diag.air", value: tone.air_db, domain: [-50, 0], zone: [TONE_ZONES.air.low, TONE_ZONES.air.high] });
    }
    return rows;
  });

  function pct(v: number, [lo, hi]: [number, number]): number {
    return Math.max(0, Math.min(100, ((v - lo) / (hi - lo)) * 100));
  }

  function notice(level: NoticeLevel, key: string, params: Record<string, string> = {}): Notice {
    return { level, key, params, persistent: false, id: null, cleared: false, auto_dismiss_ms: null, action: null };
  }

  function eqWhat(action: EqAction): string {
    return t(`analyzer.eq_kind.${action.kind}` as `analyzer.eq_kind.${EqAction["kind"]}`);
  }

  let busy = $state(false);

  async function addEq(action: EqAction): Promise<void> {
    if (busy) {
      return;
    }
    busy = true;
    try {
      const result = await applyEqAction(action);
      const freq = formatFreqShort(action.freqHz);
      if (result.outcome === "applied") {
        pushNotice(
          result.band === "hp"
            ? notice("info", "notice.analyzer.eq_hp_added", { freq })
            : notice("info", "notice.analyzer.eq_added", { band: String(result.band), what: eqWhat(action), freq }),
        );
      } else if (result.outcome === "no_free_band") {
        pushNotice(notice("warning", "notice.analyzer.eq_no_free_band"));
      } else {
        pushNotice(notice("error", "notice.analyzer.eq_failed"));
      }
    } finally {
      busy = false;
    }
  }

  async function copyFreq(freqHz: number): Promise<void> {
    try {
      await navigator.clipboard.writeText(String(Math.round(freqHz)));
    } catch {
      // No clipboard permission: the notice still names the frequency.
    }
    pushNotice(notice("info", "notice.analyzer.freq_copied", { freq: formatFreqShort(freqHz) }));
  }

  const f0 = $derived(report?.f0 ?? null);
  const f0Main = $derived(f0 ? (f0.current_hz ?? f0.median_hz) : null);
  // H-97: low pitch confidence (the tracker had little to lock onto) is a caveat on the number,
  // not a defect in the voice — it earns the "info" reading `assessReport` produces, never "warn".
  const f0IsEstimate = $derived(finding("f0")?.severity === "info");
  // Octave correction is a diagnostic of *our* tracker, never shown as a raw percentage in the
  // main readout (H-97 scope) — only, human-worded, in the hover detail, and independently of
  // confidence (a firm reading can still have needed some folding).
  const f0Tooltip = $derived.by((): string | undefined => {
    if (!f0) {
      return undefined;
    }
    const parts: string[] = [];
    if (f0IsEstimate) {
      parts.push(t("analyzer.diag.f0_confidence_tooltip", { confidence: formatNumber(f0.confidence, 2) }));
    }
    if (f0.octave_corrected > NOTABLE_OCTAVE_CORRECTION) {
      parts.push(t("analyzer.diag.f0_octave_tooltip"));
    }
    return parts.length > 0 ? parts.join(" ") : undefined;
  });
</script>

{#snippet status(f: Finding | undefined)}
  {#if f}
    <p class="hint" data-severity={f.severity}>
      <StatusDot tone={SEVERITY_TONE[f.severity]} label={hintText(f)} />
      <span>{hintText(f)}</span>
    </p>
  {/if}
{/snippet}

{#snippet action(f: Finding | undefined)}
  {#if f?.action?.type === "eq"}
    {@const eq = f.action.eq}
    <Button
      size="sm"
      variant="ghost"
      icon="add"
      disabled={busy}
      testid={`${testid}-add-eq-${f.id}`}
      title={t("analyzer.diag.add_eq_tooltip", { what: eqWhat(eq), freq: formatFreqShort(eq.freqHz) })}
      onclick={() => void addEq(eq)}
    >
      {t("analyzer.diag.add_eq")}
    </Button>
  {:else if f?.action?.type === "copy"}
    {@const hz = f.action.freqHz}
    <Button
      size="sm"
      variant="ghost"
      icon="copy"
      testid={`${testid}-copy-${f.id}`}
      title={t("analyzer.diag.copy_tooltip")}
      onclick={() => void copyFreq(hz)}
    >
      {t("analyzer.diag.copy_freq", { freq: formatFreqShort(hz) })}
    </Button>
  {/if}
{/snippet}

<aside class="diagnostics" data-testid={testid} aria-label={t("analyzer.diag.title")}>
  <header>
    <div class="heading">
      <h3>{t("analyzer.diag.title")}</h3>
      <span class="scope">{scopeText}</span>
    </div>
    {#if onclose}
      <IconButton icon="close" size="sm" label={t("analyzer.diag.close")} testid={`${testid}-close`} onclick={onclose} />
    {/if}
  </header>

  {#if !report || findings.length === 0}
    <p class="empty" data-testid={`${testid}-empty`}>{emptyText}</p>
  {:else}
    <div class="rows">
      {#if f0 && f0Main !== null}
        <section class="row f0" data-testid={`${testid}-f0`}>
          <div class="line">
            <span class="label">{t("analyzer.diag.f0")}</span>
            <span class="value strong" data-estimate={f0IsEstimate ? "true" : undefined} title={f0Tooltip}
              >{f0IsEstimate ? "≈" : ""}{formatNote(f0Main)}</span
            >
          </div>
          <div class="line">
            <span class="sub">
              {#if f0.current_hz !== null}{t("analyzer.diag.f0_now")} {formatFreqShort(f0.current_hz)}{/if}
            </span>
            <span class="sub">{t("analyzer.diag.f0_voiced", { pct: formatNumber(f0.voiced_fraction * 100, 0) })}</span>
          </div>
          <p class="sub">
            {t("analyzer.diag.f0_range", {
              median: formatFreqShort(f0.median_hz),
              low: formatFreqShort(f0.low_hz),
              high: formatFreqShort(f0.high_hz),
            })}
          </p>
          {#if f0IsEstimate}
            <div class="line status-line">
              {@render status(finding("f0"))}
              {@render action(finding("f0"))}
            </div>
          {/if}
        </section>
      {/if}

      {#if toneRows.length > 0}
        <section class="row" data-testid={`${testid}-tone`}>
          <div class="line">
            <span class="label">{t("analyzer.diag.tone")}</span>
            <span class="sub">{t("analyzer.diag.tone_ref")}</span>
          </div>
          {#each toneRows as row (row.id)}
            {@const f = finding(row.id)}
            <div class="tone-row" data-testid={`${testid}-tone-${row.id}`}>
              <div class="line">
                <span class="band">{t(row.labelKey)}</span>
                <span class="value">{relative(row.value)}</span>
              </div>
              <div class="bar" aria-hidden="true">
                <span
                  class="zone"
                  style:left="{pct(row.zone[0], row.domain)}%"
                  style:width="{pct(row.zone[1], row.domain) - pct(row.zone[0], row.domain)}%"
                ></span>
                {#if row.value !== null}
                  <span class="mark" data-severity={f?.severity ?? "ok"} style:left="{pct(row.value, row.domain)}%"></span>
                {/if}
              </div>
              <div class="line status-line">
                {@render status(f)}
                {@render action(f)}
              </div>
            </div>
          {/each}
        </section>
      {/if}

      {#if report.sibilance}
        {@const f = finding("sibilance")}
        <section class="row" data-testid={`${testid}-sibilance`}>
          <div class="line">
            <span class="label">{t("analyzer.diag.sibilance")}</span>
            <span class="value">{relative(report.sibilance.ratio_db)}</span>
          </div>
          <div class="line status-line">
            {@render status(f)}
            {@render action(f)}
          </div>
        </section>
      {/if}

      {#if finding("hum")}
        {@const f = finding("hum")}
        <section class="row" data-testid={`${testid}-hum`}>
          <div class="line">
            <span class="label">{t("analyzer.diag.hum")}</span>
            {#if report.hum}
              <span class="value">{formatNumber(report.hum.level_db, 1)} dB</span>
            {/if}
          </div>
          <div class="line status-line">
            {@render status(f)}
            {@render action(f)}
          </div>
        </section>
      {/if}

      {#if report.rumble_db !== null}
        {@const f = finding("rumble")}
        <section class="row" data-testid={`${testid}-rumble`}>
          <div class="line">
            <span class="label">{t("analyzer.diag.rumble")}</span>
            <span class="value">{relative(report.rumble_db)}</span>
          </div>
          <div class="line status-line">
            {@render status(f)}
            {@render action(f)}
          </div>
        </section>
      {/if}

      {#if report.noise_floor_dbfs !== null}
        <section class="row" data-testid={`${testid}-noise`}>
          <div class="line">
            <span class="label">{t("analyzer.diag.noise")}</span>
            <span class="value">{formatNumber(report.noise_floor_dbfs, 1)} dBFS</span>
          </div>
          {@render status(finding("noise"))}
          {#if report.snr_db !== null}
            <div class="line">
              <span class="label">{t("analyzer.diag.snr")}</span>
              <span class="value">{formatNumber(report.snr_db, 1)} dB</span>
            </div>
            {@render status(finding("snr"))}
          {/if}
        </section>
      {/if}
    </div>
  {/if}
</aside>

<style>
  .diagnostics {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    background: var(--pv-bg-panel);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-xs);
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    flex: none;
    padding: var(--pv-space-1) var(--pv-space-2) var(--pv-space-1) var(--pv-space-3);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .heading {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }

  h3 {
    margin: 0;
    color: var(--pv-text-primary);
    font-size: var(--pv-text-sm);
    font-weight: var(--pv-weight-semibold);
  }

  .scope {
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .empty {
    margin: 0;
    padding: var(--pv-space-3);
    color: var(--pv-text-tertiary);
  }

  .rows {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--pv-space-3) var(--pv-space-2);
  }

  .row {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    padding: var(--pv-space-2) 0;
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .row:last-child {
    border-bottom: 0;
  }

  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--pv-space-2);
    min-width: 0;
  }

  .status-line {
    flex-wrap: wrap;
    row-gap: 2px;
  }

  .status-line > :global(button) {
    margin-left: auto;
  }

  .label {
    color: var(--pv-text-primary);
    font-weight: var(--pv-weight-semibold);
  }

  .band {
    color: var(--pv-text-secondary);
  }

  .value {
    color: var(--pv-text-primary);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .value.strong {
    font-size: var(--pv-text-md);
    font-weight: var(--pv-weight-semibold);
  }

  /* H-97: an estimate never looks as solid as a clean reading. */
  .value.strong[data-estimate="true"] {
    color: var(--pv-text-secondary);
  }

  .sub {
    margin: 0;
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .hint {
    display: flex;
    align-items: center;
    gap: var(--pv-space-1);
    min-width: 0;
    margin: 0;
    color: var(--pv-text-secondary);
  }

  .hint span:last-child {
    min-width: 0;
  }

  .hint[data-severity="warn"] {
    color: var(--pv-warning-text);
  }

  .tone-row {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding-top: var(--pv-space-1);
  }

  /* A track with the "sounds fine" zone shaded and a mark at the measured value. */
  .bar {
    position: relative;
    height: 6px;
    border-radius: 3px;
    background: var(--pv-bg-inset);
  }

  .zone {
    position: absolute;
    top: 0;
    bottom: 0;
    border-radius: 3px;
    background: var(--pv-success-soft);
  }

  .mark {
    position: absolute;
    top: -2px;
    width: 3px;
    height: 10px;
    margin-left: -1.5px;
    border-radius: 1px;
    background: var(--pv-success);
  }

  .mark[data-severity="info"] {
    background: var(--pv-accent);
  }

  .mark[data-severity="warn"] {
    background: var(--pv-warning);
  }
</style>
