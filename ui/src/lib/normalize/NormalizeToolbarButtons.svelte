<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "../i18n";
  import { Button, formatWithUnit } from "../ui";
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

  // H-25: the six favourites live in one "Normalize" menu instead of six toolbar buttons. The
  // menu stays in the DOM while closed (`hidden`), so every favourite keeps its id and state.
  let open = $state(false);
  let root: HTMLElement | undefined = $state();
  const anyEnabled = $derived(enabled || lufsEnabled);

  function items(): HTMLButtonElement[] {
    return root ? [...root.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)')] : [];
  }

  function onDocPointerDown(event: PointerEvent): void {
    if (root && !root.contains(event.target as Node)) {
      close(false);
    }
  }

  function openMenu(): void {
    open = true;
    document.addEventListener("pointerdown", onDocPointerDown, true);
    queueMicrotask(() => items()[0]?.focus());
  }

  function close(refocus: boolean): void {
    open = false;
    document.removeEventListener("pointerdown", onDocPointerDown, true);
    if (refocus) {
      root?.querySelector<HTMLButtonElement>('[data-testid="toolbar-normalize-menu"]')?.focus();
    }
  }

  function onMenuKeydown(event: KeyboardEvent): void {
    const list = items();
    const index = list.indexOf(document.activeElement as HTMLButtonElement);
    if (event.key === "Escape") {
      event.stopPropagation();
      close(true);
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      list[(index + step + list.length) % list.length]?.focus();
    } else if (event.key === "Home" || event.key === "End") {
      event.preventDefault();
      (event.key === "Home" ? list[0] : list[list.length - 1])?.focus();
    }
  }

  function run(action: () => Promise<void> | void): void {
    close(false);
    void action();
  }

  onDestroy(() => document.removeEventListener("pointerdown", onDocPointerDown, true));
</script>

<div class="normalize" bind:this={root}>
  <Button
    variant="ghost"
    iconEnd="chevronDown"
    testid="toolbar-normalize-menu"
    title={t("toolbar.normalize.menu_title")}
    aria-haspopup="menu"
    aria-expanded={open}
    disabled={!anyEnabled}
    onclick={() => (open ? close(false) : openMenu())}
  >
    {t("toolbar.normalize.menu")}
  </Button>
  <!-- svelte-ignore a11y_interactive_supports_focus -->
  <div
    class="menu"
    role="menu"
    aria-label={t("favorites.menu")}
    hidden={!open}
    onkeydown={onMenuKeydown}
  >
    <div class="heading" role="presentation">{t("toolbar.normalize.peak_heading")}</div>
    {#each FAVORITE_TARGETS_DB as targetDb (targetDb)}
      <button
        type="button"
        role="menuitem"
        tabindex="-1"
        data-testid={testId(targetDb)}
        disabled={!enabled}
        title={t("toolbar.normalize.tooltip", { target: targetDb.toFixed(1) })}
        onclick={() => run(() => normalizeFavorite(targetDb))}
      >
        {formatWithUnit(targetDb, "dBFS", 1)}
      </button>
    {/each}
    <div class="heading" role="presentation">{t("toolbar.normalize.lufs_heading")}</div>
    {#each FAVORITE_TARGETS_LUFS as targetLufs (targetLufs)}
      <button
        type="button"
        role="menuitem"
        tabindex="-1"
        data-testid={lufsTestId(targetLufs)}
        disabled={!lufsEnabled}
        title={t("toolbar.normalize_lufs.tooltip", { target: targetLufs.toFixed(1) })}
        onclick={() => run(() => normalizeLufsFavorite(targetLufs))}
      >
        {formatWithUnit(targetLufs, "LUFS", 1)}
      </button>
    {/each}
  </div>
</div>

<style>
  .normalize {
    position: relative;
    display: inline-flex;
  }

  .menu {
    position: absolute;
    top: calc(100% + var(--pv-space-1));
    right: 0;
    z-index: var(--pv-z-dropdown);
    display: flex;
    flex-direction: column;
    min-width: 11rem;
    padding: var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-2);
  }

  .menu[hidden] {
    display: none;
  }

  .heading {
    padding: var(--pv-space-2) var(--pv-space-2) var(--pv-space-1);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-weight: var(--pv-weight-semibold);
  }

  .menu button {
    display: flex;
    align-items: center;
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    font-variant-numeric: tabular-nums;
    text-align: left;
    white-space: nowrap;
    cursor: default;
  }

  .menu button:hover:not(:disabled),
  .menu button:focus-visible {
    background: var(--pv-control-bg-active);
    outline: none;
  }

  .menu button:disabled {
    color: var(--pv-text-disabled);
  }
</style>
