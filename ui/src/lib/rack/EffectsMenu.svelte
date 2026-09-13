<script lang="ts">
  import { t } from "../i18n";
  import { canCapture, startCapture } from "./nrCapture.svelte";

  /**
   * Effects menu/toolbar (S3-06, SPEC-014 §2.3): "Effects → Noise Reduction / Restoration →
   * Capture Noise Print", flattened to one toolbar button (no other Effects command exists yet).
   * Targets the last-focused rack slot, like the Shift+P keymap action — clicking a specific
   * slot's own Capture button (`NoiseReductionSection.svelte`) is unambiguous and doesn't go
   * through here.
   */

  const enabled = $derived(canCapture());
</script>

<div class="effects-menu" data-testid="effects-menu">
  <button
    type="button"
    data-testid="menu-capture-noise-print"
    disabled={!enabled}
    title={t("module.noise_reduction.capture.hint")}
    onclick={() => void startCapture(null)}
  >
    {t("module.noise_reduction.capture")}
  </button>
</div>

<style>
  .effects-menu {
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

  button:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }
</style>
