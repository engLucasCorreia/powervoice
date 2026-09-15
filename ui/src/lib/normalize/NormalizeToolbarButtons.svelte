<script lang="ts">
  import { t } from "../i18n";
  import { Button, formatNumber, formatWithUnit, Menu } from "../ui";
  import type { MenuEntry } from "../ui/menuModel";
  import { recordState } from "../state/record.svelte";
  import { canNormalize, FAVORITE_TARGETS_DB, normalizeFavorite } from "../state/normalize.svelte";
  import {
    canNormalizeLufs,
    FAVORITE_TARGETS_LUFS,
    normalizeLufsFavorite,
  } from "../state/normalizeLufs.svelte";

  /**
   * Toolbar "Normalize ▾" (S2-02, SPEC-010 §2.5; S4-01 LUFS favorites): the six one-click
   * favorites in PROMPT §3.3 order, grouped Peak / Loudness. Same enablement as the Favorites
   * menu. H-25 folded the six buttons into one menu; H-26 moves it onto the shared menu.
   */
  const rec = recordState();
  const enabled = $derived(canNormalize() && !rec.state.recording);
  const lufsEnabled = $derived(canNormalizeLufs() && !rec.state.recording);
  const anyEnabled = $derived(enabled || lufsEnabled);

  let open = $state(false);
  let trigger: HTMLButtonElement | undefined = $state();

  function testId(targetDb: number): string {
    return `toolbar-normalize-${Math.abs(targetDb).toFixed(1).replace(".", "-")}db`;
  }

  function lufsTestId(targetLufs: number): string {
    return `toolbar-normalize-lufs-${Math.abs(targetLufs).toFixed(0)}`;
  }

  const items = $derived<MenuEntry[]>([
    { kind: "heading", id: "peak", label: t("toolbar.normalize.peak_heading") },
    ...FAVORITE_TARGETS_DB.map(
      (targetDb): MenuEntry => ({
        kind: "item",
        id: `db-${targetDb}`,
        label: formatWithUnit(targetDb, "dBFS", 1),
        title: t("toolbar.normalize.tooltip", { target: formatNumber(targetDb, 1) }),
        disabled: !enabled,
        testid: testId(targetDb),
        onselect: () => void normalizeFavorite(targetDb),
      }),
    ),
    { kind: "heading", id: "lufs", label: t("toolbar.normalize.lufs_heading") },
    ...FAVORITE_TARGETS_LUFS.map(
      (targetLufs): MenuEntry => ({
        kind: "item",
        id: `lufs-${targetLufs}`,
        label: formatWithUnit(targetLufs, "LUFS", 1),
        title: t("toolbar.normalize_lufs.tooltip", { target: formatNumber(targetLufs, 1) }),
        disabled: !lufsEnabled,
        testid: lufsTestId(targetLufs),
        onselect: () => void normalizeLufsFavorite(targetLufs),
      }),
    ),
  ]);
</script>

<div class="normalize" data-tour="normalize">
  <Button
    variant="ghost"
    iconEnd="chevronDown"
    testid="toolbar-normalize-menu"
    title={t("toolbar.normalize.menu_title")}
    aria-haspopup="menu"
    aria-expanded={open}
    disabled={!anyEnabled}
    bind:element={trigger}
    onclick={() => (open = !open)}
  >
    {t("toolbar.normalize.menu")}
  </Button>
  <Menu
    {open}
    anchor={trigger}
    {items}
    label={t("favorites.menu")}
    testid="toolbar-normalize-popup"
    placement="bottom-end"
    minWidth={176}
    onclose={() => (open = false)}
  />
</div>

<style>
  .normalize {
    display: inline-flex;
  }
</style>
