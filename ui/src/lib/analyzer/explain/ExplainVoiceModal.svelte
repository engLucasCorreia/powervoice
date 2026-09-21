<script lang="ts">
  import { t } from "../../i18n";
  import { Dialog, StatusDot, ToggleButton } from "../../ui";
  import { formatNumber } from "../../ui/units";
  import { closeExplainVoice, explainModalState } from "./explainModal.svelte";
  import { explainVoiceState } from "./explainVoice.svelte";
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
   */

  const MOBILE_MAX_LABELS = 3;
  const DESKTOP_MAX_LABELS = 6;
  const MOBILE_BREAKPOINT_PX = 640;

  let showRaw = $state(true);
  let showSmoothed = $state(true);
  let showHarmonics = $state(true);
  let showBands = $state(true);
  let showEqAdvice = $state(true);
  let isNarrow = $state(false);
  let beneath: VoiceFinding[] = $state([]);

  const open = $derived(explainModalState().open);
  const snapshot = $derived(explainVoiceState().snapshot);
  const maxLabels = $derived(isNarrow ? MOBILE_MAX_LABELS : DESKTOP_MAX_LABELS);
  const subtitle = $derived(snapshot ? t("explain.subtitle", { seconds: formatNumber(snapshot.spanS, 1) }) : "");
  const summary = $derived(snapshot ? buildVoiceSummary(snapshot) : null);
  const prose = $derived(snapshot ? explainFindings(snapshot) : []);

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
  >
    <p class="subtitle" data-testid="explain-subtitle">{subtitle}</p>
    <p class="scope-note" data-testid="explain-scope-note">{t("explain.scope_note")}</p>

    {#if summary}
      <section class="summary" data-testid="explain-summary">
        <h3>{t("explain.summary.title")}</h3>
        <p class="headline" data-testid="explain-summary-headline">{summary.headline}</p>
        <p class="basis">{summary.basis}</p>
        <div class="summary-columns">
          <div class="summary-col" data-testid="explain-summary-profile">
            <h4>{t("explain.summary.profile.title")}</h4>
            <ul>
              {#each summary.profile as row (row.id)}
                <li>
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

    <div class="toggles" data-testid="explain-toggles">
      <ToggleButton size="sm" bind:pressed={showRaw} testid="explain-toggle-raw">{t("explain.toggle.raw")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showSmoothed} testid="explain-toggle-smoothed">{t("explain.toggle.smoothed")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showHarmonics} testid="explain-toggle-harmonics">{t("explain.toggle.harmonics")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showBands} testid="explain-toggle-bands">{t("explain.toggle.bands")}</ToggleButton>
      <ToggleButton size="sm" bind:pressed={showEqAdvice} testid="explain-toggle-eq">{t("explain.toggle.eq_advice")}</ToggleButton>
    </div>

    <div class="graph-area" data-testid="explain-graph-area">
      <ExplainGraph
        {snapshot}
        {showRaw}
        {showSmoothed}
        {showHarmonics}
        {showBands}
        {showEqAdvice}
        {maxLabels}
        bind:beneath
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
    flex-direction: column;
    gap: var(--pv-space-1);
    padding: var(--pv-space-3);
    border: var(--pv-border-width) solid var(--pv-border-subtle);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-inset);
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
    flex: 1 1 260px;
    min-width: 220px;
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

  .summary-col li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-1) var(--pv-space-2);
    font-size: var(--pv-text-xs);
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
    flex-wrap: wrap;
    gap: var(--pv-space-2);
  }

  .graph-area {
    display: flex;
    flex: 1;
    min-height: 22rem;
  }

  .beneath {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    flex: none;
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
</style>
