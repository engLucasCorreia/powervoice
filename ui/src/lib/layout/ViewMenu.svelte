<script lang="ts">
  import { t } from "../i18n";
  import { analyzerState, setAnalyzerVisible } from "../analyzer/analyzer.svelte";

  /**
   * View menu (H-16, SPEC-007 §2.9: "shown by default and can be hidden with View → Analyzer").
   * A single toggle for now — the same flat-toolbar pattern as `EditMenu`/`DocumentMenu` rather
   * than a real dropdown (this codebase has no menu-popover component yet).
   */

  const analyzer = analyzerState();
</script>

<div class="view-menu" data-testid="view-menu">
  <button
    type="button"
    data-testid="menu-view-analyzer"
    aria-pressed={analyzer.visible}
    class:active={analyzer.visible}
    onclick={() => setAnalyzerVisible(!analyzer.visible)}
  >
    {t("menu.view.analyzer")}
  </button>
</div>

<style>
  .view-menu {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.75rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
    font-size: 0.85em;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.2rem 0.6rem;
  }

  button:hover {
    border-color: var(--accent);
  }

  button.active {
    background: var(--accent);
    color: var(--text-on-accent);
    border-color: var(--accent);
  }
</style>
