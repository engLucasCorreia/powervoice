<script lang="ts">
  import { t } from "../i18n";
  import { recordState } from "../state/record.svelte";
  import { canNormalize, FAVORITE_TARGETS_DB, normalizeFavorite, openNormalizeDialog } from "../state/normalize.svelte";
  import NormalizeDialog from "./NormalizeDialog.svelte";

  /**
   * Favorites menu (S2-02, SPEC-010 §2.5): Normalize to −1/−0.1/−3 dB, a separator, then
   * Normalize… — enabled with a document open (`L > 0`) and not while recording.
   */
  const rec = recordState();
  const enabled = $derived(canNormalize() && !rec.state.recording);

  function testId(targetDb: number): string {
    return `favorites-normalize-${Math.abs(targetDb).toFixed(1).replace(".", "-")}db`;
  }
</script>

<div class="favorites-menu" data-testid="favorites-menu">
  <span class="label">{t("favorites.menu")}</span>
  {#each FAVORITE_TARGETS_DB as targetDb (targetDb)}
    <button
      type="button"
      data-testid={testId(targetDb)}
      disabled={!enabled}
      title={t("favorites.normalize_peak", { target: targetDb.toFixed(1) })}
      onclick={() => void normalizeFavorite(targetDb)}
    >
      {t("favorites.normalize_peak", { target: targetDb.toFixed(1) })}
    </button>
  {/each}
  <span class="divider" aria-hidden="true"></span>
  <button
    type="button"
    data-testid="favorites-normalize-custom"
    disabled={!enabled}
    onclick={openNormalizeDialog}
  >
    {t("effects.normalize_dialog")}
  </button>
</div>
<NormalizeDialog />

<style>
  .favorites-menu {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.75rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
    font-size: 0.85em;
  }

  .label {
    color: var(--text-secondary);
    margin-right: 0.25rem;
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
