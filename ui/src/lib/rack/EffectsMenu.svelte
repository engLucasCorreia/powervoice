<script lang="ts">
  import { t } from "../i18n";
  import { canNormalize, openNormalizeDialog } from "../state/normalize.svelte";
  import { canNormalizeLufs, openNormalizeLufsDialog } from "../state/normalizeLufs.svelte";
  import { canCapture, startCapture } from "./nrCapture.svelte";

  /**
   * Effects menu/toolbar (S3-06 Capture Noise Print, SPEC-014 §2.3; H-09 adds Normalize…/
   * Normalize (LUFS)…, SPEC-010 §2.5: "Effects → Normalize…", mirroring Audition's Effects →
   * Amplitude → Normalize). The dialogs themselves are mounted once, in `FavoritesMenu.svelte`,
   * which also has these same two entries at the bottom of its own menu — both just open the
   * shared dialog state. Capture Noise Print targets the last-focused rack slot, like the
   * Shift+P keymap action — clicking a specific slot's own Capture button
   * (`NoiseReductionSection.svelte`) is unambiguous and doesn't go through here.
   */

  const enabled = $derived(canCapture());
  const normalizeEnabled = $derived(canNormalize());
  const normalizeLufsEnabled = $derived(canNormalizeLufs());
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
  <span class="divider" aria-hidden="true"></span>
  <button
    type="button"
    data-testid="menu-normalize-dialog"
    disabled={!normalizeEnabled}
    onclick={openNormalizeDialog}
  >
    {t("effects.normalize_dialog")}
  </button>
  <button
    type="button"
    data-testid="menu-normalize-lufs-dialog"
    disabled={!normalizeLufsEnabled}
    onclick={openNormalizeLufsDialog}
  >
    {t("effects.normalize_lufs_dialog")}
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

  .divider {
    width: 1px;
    align-self: stretch;
    background: var(--surface-border);
    margin: 0 0.15rem;
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
