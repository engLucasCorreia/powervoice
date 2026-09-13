<script lang="ts">
  import { t } from "../i18n";
  import { recordState } from "../state/record.svelte";
  import { canNormalize, FAVORITE_TARGETS_DB, normalizeFavorite } from "../state/normalize.svelte";
  import {
    canNormalizeLufs,
    FAVORITE_TARGETS_LUFS,
    normalizeLufsFavorite,
  } from "../state/normalizeLufs.svelte";

  /**
   * Toolbar "Normalize" button group (S2-02, SPEC-010 §2.5): three compact one-click favorites,
   * in PROMPT §3.3 order. Same enablement as the Favorites menu. S4-01 adds the LUFS favorites
   * (−16/−19/−23 LUFS) in the same group.
   */
  const rec = recordState();
  const enabled = $derived(canNormalize() && !rec.state.recording);
  const lufsEnabled = $derived(canNormalizeLufs() && !rec.state.recording);

  function testId(targetDb: number): string {
    return `toolbar-normalize-${Math.abs(targetDb).toFixed(1).replace(".", "-")}db`;
  }

  function lufsTestId(targetLufs: number): string {
    return `toolbar-normalize-lufs-${Math.abs(targetLufs).toFixed(0)}`;
  }
</script>

<div class="normalize-group" role="group" aria-label={t("favorites.menu")}>
  {#each FAVORITE_TARGETS_DB as targetDb (targetDb)}
    <button
      type="button"
      data-testid={testId(targetDb)}
      disabled={!enabled}
      title={t("toolbar.normalize.tooltip", { target: targetDb.toFixed(1) })}
      onclick={() => void normalizeFavorite(targetDb)}
    >
      {t("toolbar.normalize.button", { target: targetDb.toFixed(1) })}
    </button>
  {/each}
  {#each FAVORITE_TARGETS_LUFS as targetLufs (targetLufs)}
    <button
      type="button"
      data-testid={lufsTestId(targetLufs)}
      disabled={!lufsEnabled}
      title={t("toolbar.normalize_lufs.tooltip", { target: targetLufs.toFixed(1) })}
      onclick={() => void normalizeLufsFavorite(targetLufs)}
    >
      {t("toolbar.normalize_lufs.button", { target: targetLufs.toFixed(1) })}
    </button>
  {/each}
</div>

<style>
  .normalize-group {
    display: flex;
    align-items: center;
    gap: 0.3rem;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.6rem;
    font-variant-numeric: tabular-nums;
  }

  button:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }
</style>
