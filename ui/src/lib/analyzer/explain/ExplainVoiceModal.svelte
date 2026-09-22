<script lang="ts">
  import { t } from "../../i18n";
  import { Button, Dialog, IconButton, Menu, StatusDot, ToggleButton } from "../../ui";
  import type { MenuEntry } from "../../ui/menuModel";
  import { formatNumber } from "../../ui/units";
  import type { IpcError } from "../../ipc/bindings";
  import { noticeFromIpcError } from "../../notices/fromIpcError";
  import { pushNotice } from "../../state/notices.svelte";
  import TourButton from "../../tour/TourButton.svelte";
  import { closeExplainVoice, explainModalState } from "./explainModal.svelte";
  import { explainVoiceState } from "./explainVoice.svelte";
  import { explainDialogStyle } from "./explainDialogSize";
  import { buildExplainExportData, type ExplainGraphExportFrame } from "./explainExport";
  import { exportExplainImage, exportExplainReport } from "./explainExportIo";
  import { loadShowAnnotationsPref, saveShowAnnotationsPref } from "./explainPrefs";
  import { explainFindings } from "./prose";
  import { buildVoiceSummary, type ProfileId } from "./summary";
  import ExplainGraph from "./ExplainGraph.svelte";
  import ExplainFindingCard from "./ExplainFindingCard.svelte";
  import ExplainActionButton from "./ExplainActionButton.svelte";
  import type { FindingSeverity, VoiceFinding } from "./findings";

  /**
   * The "Explain My Voice" modal (H-92 ticket §2): title, the real analysed span (never a
   * hardcoded figure — `VoiceReport::span_s`, H-91), the five toggles, the annotated graph, and
   * the findings the graph had no room (or no anchor) for, as plain cards underneath. Built on
   * the shared `Dialog` (size `xl`): large, responsive, themed, Escape-closable, focus-managed —
   * all of that is the kit's, not reimplemented here.
   *
   * Closing drops the frozen snapshot (`explainModal.svelte.ts`) and returns to plain live
   * analysis, which never stopped running underneath.
   *
   * **H-102**: the graph was a fifth of the dialog's height, with the summary and its two wide,
   * sparsely-filled columns above it — the opposite of the reference ("the annotated spectrum
   * *is* the page"). The dialog now opens taller than the shared `Dialog` kit's default `xl`
   * (a `style` override scoped to this one instance, not a change to `Dialog.svelte` itself,
   * which other `xl` dialogs — e.g. the plugin manager, T-809 — still use unchanged), the summary
   * is capped to a compact, independently-scrolling strip, and the graph gets the rest — by far
   * the largest region in the dialog, as the reference is. `DESKTOP_MAX_LABELS` rose with it: a
   * taller plot has room to keep most findings on the chart itself, which is what the reference
   * shows, rather than routing them to the "also measured" list below by default.
   *
   * **H-115**: four more owner requests. (1) The dialog now sizes itself from the viewport
   * (`explainDialogSize.ts`) instead of the `min(840px, 90vh)` pixel cap H-102 left behind — a
   * small box in the middle of a 4K display — plus a maximise/restore control next to the tour
   * button. (2) An "Annotations" toggle hides `ExplainGraph`'s cards/leader lines only (curves,
   * bands and markers keep drawing), remembered across sessions (`explainPrefs.ts`). (3) When
   * EQ Advice is on and there is nothing to draw, a line beside the toggles says so, with the
   * reason (reusing the summary's own "Suggested focus" prose — never a second explanation of the
   * same numbers) available as its tooltip. (4) The export menu renders a PNG and a self-contained
   * HTML report of exactly what is on screen (`explainExport*.ts`) through the native save dialog.
   */

  const MOBILE_MAX_LABELS = 3;
  const DESKTOP_MAX_LABELS = 9;
  const MOBILE_BREAKPOINT_PX = 640;

  let showRaw = $state(true);
  let showSmoothed = $state(true);
  let showHarmonics = $state(true);
  let showBands = $state(true);
  let showEqAdvice = $state(true);
  /** H-115 ticket §2: hides the graph's annotation cards/leader lines only, remembered across
   * sessions the same way the theme preference is (`explainPrefs.ts`). */
  let showAnnotations = $state(loadShowAnnotationsPref());
  /** H-115 ticket §1: the maximise/restore control next to the tour button. */
  let maximized = $state(false);
  let isNarrow = $state(false);
  let beneath: VoiceFinding[] = $state([]);
  /** H-115: `ExplainGraph`'s bound-out canvas + current placed-card layout, for the export. */
  let exportFrame: ExplainGraphExportFrame | null = $state(null);
  let exportMenuOpen = $state(false);
  let exportTrigger: HTMLButtonElement | undefined = $state();
  let exporting = $state(false);

  const open = $derived(explainModalState().open);
  const snapshot = $derived(explainVoiceState().snapshot);
  const maxLabels = $derived(isNarrow ? MOBILE_MAX_LABELS : DESKTOP_MAX_LABELS);
  const subtitle = $derived(snapshot ? t("explain.subtitle", { seconds: formatNumber(snapshot.spanS, 1) }) : "");
  const summary = $derived(snapshot ? buildVoiceSummary(snapshot) : null);
  const prose = $derived(snapshot ? explainFindings(snapshot) : []);
  const dialogStyle = $derived(explainDialogStyle(maximized));

  /** H-115 ticket §3: "EQ Advice must never be a silent no-op." `summary.eqBands` is what
   * `ExplainGraph` draws the dashed overlay from — empty means the toggle has nothing to show. */
  const eqAdviceEmpty = $derived(!!summary && summary.eqBands.length === 0);
  /** The reason, reusing the summary's own "Suggested focus" sentences (which already spell out
   * *why* nothing crossed far enough to warrant a change, e.g. "close enough to the line to
   * listen before anything is changed") rather than writing a second explanation of the same
   * numbers. The processed-floor observation is not about EQ, so it is left out. */
  const eqAdviceEmptyReason = $derived(
    summary
      ? summary.focus
          .filter((item) => item.id !== "noise_processing")
          .map((item) => item.text)
          .join(" ")
      : "",
  );

  const exportMenuItems = $derived<MenuEntry[]>([
    {
      kind: "item",
      id: "image",
      label: t("explain.export.image"),
      disabled: !exportFrame || exporting,
      testid: "explain-export-image",
      onselect: () => void runExport("image"),
    },
    {
      kind: "item",
      id: "report",
      label: t("explain.export.report"),
      disabled: !exportFrame || exporting,
      testid: "explain-export-report",
      onselect: () => void runExport("report"),
    },
  ]);

  $effect(() => {
    saveShowAnnotationsPref(showAnnotations);
  });

  function isIpcError(value: unknown): value is IpcError {
    return typeof value === "object" && value !== null && "code" in value && "key" in value;
  }

  function notify(level: "info" | "warning" | "error", key: string, params: Record<string, string> = {}): void {
    pushNotice({ level, key, params, persistent: false, id: null, cleared: false, auto_dismiss_ms: null, action: null });
  }

  /** Builds the export data from exactly what is already on screen and hands it to the native
   * save dialog (`explainExportIo.ts`); `false` back from that (the dialog was cancelled) shows
   * no notice — cancelling is not a failure. */
  async function runExport(kind: "image" | "report"): Promise<void> {
    exportMenuOpen = false;
    if (!snapshot || !summary || !exportFrame || exporting) {
      return;
    }
    exporting = true;
    try {
      const data = buildExplainExportData({
        snapshot,
        summary,
        prose,
        subtitle,
        graph: exportFrame,
        showAnnotations,
        showEqAdvice,
        beneathFindings: beneath,
      });
      const saved = kind === "image" ? await exportExplainImage(data) : await exportExplainReport(data);
      if (saved) {
        notify("info", kind === "image" ? "notice.explain.export_saved_image" : "notice.explain.export_saved_report");
      }
    } catch (err) {
      if (isIpcError(err)) {
        pushNotice(noticeFromIpcError(err));
      } else {
        notify("error", "notice.explain.export_failed");
      }
    } finally {
      exporting = false;
    }
  }

  const SEVERITY_TONE: Record<FindingSeverity, "success" | "accent" | "warning" | "danger"> = {
    good: "success",
    info: "accent",
    attention: "warning",
    significant: "danger",
  };

  /** A near-threshold `attention` reads as `info`: the words already say "barely across, no
   * correction", so the dot must not still shout `warning` (the orchestrator's instruction). */
  function profileTone(id: ProfileId, severity: FindingSeverity | null): "success" | "accent" | "warning" | "danger" | "neutral" {
    if (severity === null) {
      return "neutral";
    }
    const finding = snapshot?.findings.find((f) => f.id === id);
    if (severity === "attention" && finding && prose.find((p) => p.id === id)?.nearThreshold) {
      return "accent";
    }
    return SEVERITY_TONE[severity];
  }

  $effect(() => {
    if (!open || typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return;
    }
    const mq = window.matchMedia(`(max-width: ${MOBILE_BREAKPOINT_PX}px)`);
    isNarrow = mq.matches;
    const onChange = (e: MediaQueryListEvent): void => {
      isNarrow = e.matches;
    };
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  });

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closeExplainVoice();
    }
  }
</script>

{#if open && snapshot}
  <Dialog
    actions={[{ label: t("explain.close"), role: "primary", testid: "explain-close", onclick: closeExplainVoice }]}
    size="xl"
    title={t("explain.title")}
    titleId="explain-voice-title"
    testid="explain-voice-modal"
    onkeydown={onKeydown}
    style={dialogStyle}
  >
    {#snippet headerActions()}
      <IconButton
        icon={maximized ? "collapse" : "expand"}
        label={t(maximized ? "explain.restore" : "explain.maximize")}
        size="md"
        pressed={maximized}
        testid="explain-maximize"
        onclick={() => (maximized = !maximized)}
      />
      <div class="export-menu">
        <Button
          variant="ghost"
          size="md"
          icon="save"
          iconEnd="chevronDown"
          title={t("explain.export.menu_title")}
          aria-haspopup="menu"
          aria-expanded={exportMenuOpen}
          disabled={!exportFrame}
          loading={exporting}
          bind:element={exportTrigger}
          testid="explain-export-menu-trigger"
          onclick={() => (exportMenuOpen = !exportMenuOpen)}
        >
          {t("explain.export.menu")}
        </Button>
        <Menu
          open={exportMenuOpen}
          anchor={exportTrigger}
          items={exportMenuItems}
          label={t("explain.export.menu_title")}
          testid="explain-export-menu"
          placement="bottom-end"
          minWidth={200}
          onclose={() => (exportMenuOpen = false)}
        />
      </div>
      <TourButton tour="explain" size="md" />
    {/snippet}
    <p class="subtitle" data-testid="explain-subtitle" data-tour="explain-subtitle">{subtitle}</p>
    <p class="scope-note" data-testid="explain-scope-note">{t("explain.scope_note")}</p>

    {#if summary}
      <section class="summary" data-testid="explain-summary" data-tour="explain-summary">
        <h3>{t("explain.summary.title")}</h3>
        <p class="headline" data-testid="explain-summary-headline">{summary.headline}</p>
        <p class="basis">{summary.basis}</p>
        <div class="summary-columns">
          <div class="summary-col" data-testid="explain-summary-profile">
            <h4>{t("explain.summary.profile.title")}</h4>
            <ul>
              {#each summary.profile as row (row.id)}
                <li title={row.reading}>
                  <StatusDot tone={profileTone(row.id, row.severity)} label={row.label} />
                  <span class="profile-label">{row.label}</span>
                  <span class="profile-value">{row.value}</span>
                  <span class="profile-reading">{row.reading}</span>
                </li>
              {/each}
            </ul>
          </div>
          <div class="summary-col" data-testid="explain-summary-focus">
            <h4>{t("explain.summary.focus.title")}</h4>
            <ul>
              {#each summary.focus as item (item.id)}
                <li data-observation={item.id === "noise_processing"}>
                  <span class="focus-text">{item.text}</span>
                  {#if showEqAdvice && item.id !== "noise_processing"}
                    <ExplainActionButton action={item.action} testid={`explain-focus-${item.id}`} />
                  {/if}
                </li>
              {/each}
            </ul>
          </div>
        </div>
      </section>
    {/if}

    <div class="toggles" data-testid="explain-toggles" data-tour="explain-toggles">
      <ToggleButton size="sm" bind:pressed={showRaw} testid="explain-toggle-raw">{t("explain.toggle.raw")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showSmoothed} testid="explain-toggle-smoothed">{t("explain.toggle.smoothed")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showHarmonics} testid="explain-toggle-harmonics">{t("explain.toggle.harmonics")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showBands} testid="explain-toggle-bands">{t("explain.toggle.bands")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showEqAdvice} testid="explain-toggle-eq">{t("explain.toggle.eq_advice")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showAnnotations} testid="explain-toggle-annotations">{t("explain.toggle.annotations")}</ToggleButton>
      {#if showEqAdvice && eqAdviceEmpty}
        <span class="eq-advice-note" data-testid="explain-eq-advice-empty" title={eqAdviceEmptyReason}>
          {t("explain.eq_advice.no_change")}
        </span>
      {/if}
    </div>

    <div class="graph-area" data-testid="explain-graph-area" data-tour="explain-graph-area">
      <ExplainGraph
        {snapshot}
        {showRaw}
        {showSmoothed}
        {showHarmonics}
        {showBands}
        {showEqAdvice}
        {showAnnotations}
        eqBands={summary?.eqBands ?? []}
        {maxLabels}
        bind:beneath
        bind:exportFrame
        testid="explain-graph"
      />
    </div>

    {#if beneath.length > 0}
      <div class="beneath" data-testid="explain-beneath">
        <h3>{t("explain.more_title")}</h3>
        <div class="beneath-cards">
          {#each beneath as finding (finding.id)}
            {@const findingProse = prose.find((p) => p.id === finding.id)}
            {#if findingProse}
              <ExplainFindingCard prose={findingProse} {showEqAdvice} testid={`explain-beneath-${finding.id}`} />
            {/if}
          {/each}
        </div>
      </div>
    {/if}
  </Dialog>
{/if}

<style>
  .subtitle {
    margin: 0;
    color: var(--pv-text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  .scope-note {
    margin: 0;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-sm);
  }

  .summary {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--pv-space-1);
    padding: var(--pv-space-2) var(--pv-space-3);
    border: var(--pv-border-width) solid var(--pv-border-subtle);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-inset);
    /* H-102: the summary supports the graph, it doesn't compete with it — capped to a compact
       strip with its own scrollbar so a long finding list can never push the graph down to a
       sliver, whatever the take's findings. The bottom edge fades rather than cutting hard, so a
       scrollable list reads as "more below", never as a truncated line. */
    /* H-104: 13rem, and mind the unit — this app's root font is 13px, so the old `11rem` cap was
       143px, not the 176px it reads like, and the three-row profile needs 160px. That arithmetic
       is why several attempts to "fit six rows" failed: the cap, not the content, was the limit.
       Measured in the DOM rather than guessed. `flex: none` stops the graph's min-height
       squeezing the strip below the cap from the other direction. */
    max-height: 13rem;
    flex: none;
    overflow-y: auto;
    -webkit-mask-image: linear-gradient(to bottom, black calc(100% - 14px), transparent);
    mask-image: linear-gradient(to bottom, black calc(100% - 14px), transparent);
  }

  .summary h3 {
    margin: 0;
    font-size: var(--pv-text-sm);
  }

  .headline {
    margin: 0;
    color: var(--pv-text-primary);
  }

  .basis {
    margin: 0;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .summary-columns {
    display: flex;
    flex-wrap: wrap;
    gap: var(--pv-space-4);
    margin-top: var(--pv-space-1);
  }

  .summary-col {
    flex: 1 1 0;
    min-width: 200px;
  }

  .summary-col h4 {
    margin: 0 0 var(--pv-space-1);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-xs);
    font-weight: var(--pv-weight-semibold);
    text-transform: uppercase;
    letter-spacing: 0.02em;
  }

  .summary-col ul {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* H-104: the profile column displays its six items as a 2-column grid (3 rows),
     so all items fit without scrolling at desktop height. The focus column stays as
     a vertical list. */
  .summary-col:first-child ul {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0 2px;
  }

  .summary-col li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-1) var(--pv-space-2);
    font-size: var(--pv-text-xs);
  }

  /* H-104: profile rows are a scannable single-line grid: dot + label + value (right-aligned).
     Reading text is hidden; shown only as a tooltip on hover (cursor: help indicates this).
     At normal type size (~16px) plus dot and small gaps, each row is ~20px; six rows = ~120px.
     This fits comfortably in the 11rem budget beside headers and other elements. */
  .summary-col:first-child li {
    display: grid;
    grid-template-columns: auto 1fr auto;
    gap: 0 var(--pv-space-1);
    align-items: center;
    cursor: help;
  }

  .summary-col:first-child li > :first-child {
    grid-column: 1;
  }

  .summary-col:first-child li > .profile-label {
    grid-column: 2;
  }

  .summary-col:first-child li > .profile-value {
    grid-column: 3;
    text-align: right;
  }

  .summary-col:first-child li > .profile-reading {
    display: none;
  }

  .profile-label {
    color: var(--pv-text-primary);
    font-weight: var(--pv-weight-semibold);
  }

  .profile-value {
    color: var(--pv-text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .profile-reading {
    color: var(--pv-text-tertiary);
  }

  .focus-text {
    flex: 1 1 auto;
    min-width: 0;
    color: var(--pv-text-secondary);
  }

  /* An observation (the noise floor "looks processed" note), not an action: quieter than the
     actionable focus items beside it, and never given a button. */
  .summary-col li[data-observation="true"] .focus-text {
    color: var(--pv-text-tertiary);
    font-style: italic;
  }

  .toggles {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2);
  }

  /* H-115 ticket §3: "EQ Advice must never be a silent no-op" — said in words, right beside the
     toggle it describes, with the reason a tooltip away (`title`) rather than repeated in full. */
  .eq-advice-note {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-style: italic;
    cursor: help;
  }

  .export-menu {
    display: inline-flex;
    position: relative;
  }

  /* H-102: this is the dominant element of the dialog, not a fifth of it — everything else above
     is capped or sized to content so this `flex: 1` claims the rest. */
  .graph-area {
    display: flex;
    flex: 1;
    /* H-104: 28rem, not 30. The summary's own `max-height` never applied at desktop height —
       this min-height won the flex negotiation and squeezed the strip to whatever was left, so
       two of the six profile rows fell under the fade. 2rem back is invisible on the plot (the
       frequency axis still draws) and is exactly what the third row needs. */
    min-height: 28rem;
  }

  .beneath {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    flex: none;
    max-height: 9rem;
    overflow-y: auto;
    -webkit-mask-image: linear-gradient(to bottom, black calc(100% - 14px), transparent);
    mask-image: linear-gradient(to bottom, black calc(100% - 14px), transparent);
  }

  .beneath h3 {
    margin: 0;
  }

  .beneath-cards {
    display: flex;
    flex-wrap: wrap;
    gap: var(--pv-space-2);
  }

  .beneath-cards :global(.card) {
    flex: 1 1 220px;
    min-width: 200px;
  }

  /* H-102: below MOBILE_BREAKPOINT_PX the two summary columns stack instead of sitting
     side-by-side, so the same content needs more height — the desktop cap left almost nothing
     of the first finding visible before the fade. A taller (but still capped, still scrollable)
     allowance fits a typical short summary without clipping mid-item, while the graph keeps a
     smaller floor than desktop so it isn't fighting a much shorter viewport for space. */
  @media (max-width: 640px) {
    .summary {
      max-height: 15rem;
    }

    .beneath {
      max-height: 11rem;
    }

    .graph-area {
      min-height: 16rem;
    }
  }
</style>
