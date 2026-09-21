<script lang="ts">
  import { StatusDot } from "../../ui";
  import ExplainActionButton from "./ExplainActionButton.svelte";
  import type { FindingProse } from "./prose";

  /**
   * One finding, rendered from H-94's words (H-92): title, the measured sentence, the
   * interpretation and — only where the measurement justifies one — the recommendation, each its
   * own styled block. Nothing here writes a sentence; `prose` already carries every word through
   * i18n (H-94's module doc).
   *
   * **Emphasis follows the words, not just the severity.** `nearThreshold` findings are `attention`
   * severity that crossed by less than a dB — H-94's prose already downgrades to "barely across,
   * no correction" for these, so the border/dot must not still shout `warning`: a near-threshold
   * `attention` reads as `info`-level here. Everything else maps its own severity directly.
   * `compact` (the graph's floating cards) shows title + measured only, so the chart stays
   * readable; the full four blocks show in the "also measured" list, where space is not scarce.
   */
  let {
    prose,
    showEqAdvice,
    compact = false,
    testid,
  }: {
    prose: FindingProse;
    showEqAdvice: boolean;
    /** `true` on the graph's floating cards (title + measured only). */
    compact?: boolean;
    testid: string;
  } = $props();

  const TONE: Record<"good" | "info" | "attention" | "significant", "success" | "accent" | "warning" | "danger"> = {
    good: "success",
    info: "accent",
    attention: "warning",
    significant: "danger",
  };

  const tone = $derived(
    prose.severity === "attention" && prose.nearThreshold ? TONE.info : TONE[prose.severity],
  );
</script>

<div class="card" data-severity={prose.severity} data-near={prose.nearThreshold} data-compact={compact} data-testid={testid}>
  <div class="line">
    <StatusDot {tone} label={prose.title} />
    <span class="title">{prose.title}</span>
  </div>
  <p class="measured">{prose.measured}</p>
  {#if !compact}
    <p class="interpretation">{prose.interpretation}</p>
    {#if showEqAdvice && prose.recommendation}
      <div class="recommendation">
        <p>{prose.recommendation}</p>
        <ExplainActionButton action={prose.action} testid={`${testid}-action`} />
      </div>
    {/if}
  {/if}
</div>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
    padding: 6px 8px;
    border: var(--pv-border-width) solid var(--pv-border-subtle);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-overlay);
    font-size: var(--pv-text-xs);
  }

  .card[data-compact="true"] {
    box-shadow: var(--pv-shadow-1);
  }

  .line {
    display: flex;
    align-items: center;
    gap: var(--pv-space-1);
    min-width: 0;
  }

  .title {
    overflow: hidden;
    color: var(--pv-text-primary);
    font-weight: var(--pv-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .measured {
    margin: 0;
    color: var(--pv-text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .card[data-compact="true"] .measured {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .interpretation {
    margin: 0;
    color: var(--pv-text-secondary);
    line-height: var(--pv-leading-sm);
  }

  .recommendation {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--pv-space-1);
    padding-top: 2px;
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .recommendation p {
    margin: 0;
    color: var(--pv-text-primary);
    line-height: var(--pv-leading-sm);
  }

  /* Severity is never colour alone (the dot carries `label`) — and a near-threshold `attention`
     reads as `info`, not `warning`: the border must not shout where the words don't. */
  .card[data-severity="attention"]:not([data-near="true"]) {
    border-color: var(--pv-warning-soft);
  }

  .card[data-severity="significant"] {
    border-color: var(--pv-danger-soft);
  }
</style>
