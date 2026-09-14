<script lang="ts">
  import type { DefaultFormatDto } from "../ipc/bindings";
  import { openExportDialog } from "../export/export.svelte";
  import { t } from "../i18n";
  import { dispatchAction } from "../keymap";
  import { isPlatformMac } from "../keymap/registry";
  import { shortcutLabelForAction } from "../keymap/shortcutLabel";
  import { closeAllMenus, menubarState, moveToAdjacentMenu, toggleMenu } from "../menu/menubar.svelte";
  import { focusFirstItem, handleMenuKeydown } from "../menu/menuKeyboard";
  import MenuItemRow from "../menu/MenuItemRow.svelte";
  import MenuSeparatorRow from "../menu/MenuSeparatorRow.svelte";
  import { splitMnemonic } from "../menu/mnemonic";
  import { openPreferences } from "../preferences/preferences.svelte";
  import { openRecoveryStorage } from "../recovery/recovery.svelte";
  import { openNewRecordingPrompt, recordState } from "../state/record.svelte";
  import { settingsState } from "../state/settings.svelte";
  import {
    clearRecentFiles,
    pickRecentFile,
    recentFilesState,
    refreshRecentFiles,
  } from "./recentFiles.svelte";
  import { displayName, documentState, hasDocument, requestClose } from "./document.svelte";

  /**
   * File menu (H-19): New Recording…, Open…, Recent Files ▸, Save, Save As…, Export…,
   * Recovery & Storage…, Close — a real dropdown replacing S1-03/S4-04/H-06/T-306/T-209's flat
   * always-visible row. "Recent Files ▸" is `RecentFilesMenu`'s old dropdown-in-a-dropdown logic,
   * folded in here since it's a File-menu submenu now, not a standalone toolbar widget.
   *
   * H-17: Preferences… lives here on macOS (there's no native app menu to put it in instead);
   * everywhere else it's in `EditMenu.svelte`.
   */
  const MENU_ID = "file" as const;
  const showPreferencesHere = isPlatformMac();
  const doc = documentState();
  const rec = recordState();
  const recent = recentFilesState();
  const bar = menubarState();
  const open = $derived(bar.openMenuId === MENU_ID);
  const name = $derived(displayName(doc.current));
  const label = $derived(name ? `${name}${doc.current.dirty ? " *" : ""}` : t("menu.file.no_document"));
  const mnemonic = $derived(splitMnemonic(t("menu.file"), "f"));
  // Factory default (SPEC-002 §3) — used only if settings haven't loaded yet.
  const FALLBACK_FORMAT: DefaultFormatDto = { sample_rate_hz: 48_000, bit_depth: "24" };

  let buttonEl: HTMLButtonElement | undefined = $state();
  let popupEl: HTMLDivElement | undefined = $state();
  let recentOpen = $state(false);
  let recentPopupEl: HTMLDivElement | undefined = $state();

  function openExport(): void {
    const base = doc.current.name?.replace(/\.[^./\\]+$/, "") ?? "untitled";
    openExportDialog(base);
  }

  function openNewRecording(): void {
    openNewRecordingPrompt(settingsState().current?.default_format ?? FALLBACK_FORMAT);
  }

  function select(action: () => void): void {
    action();
    closeAllMenus();
  }

  function onTriggerClick(event: MouseEvent): void {
    event.stopPropagation();
    toggleMenu(MENU_ID);
  }

  function onTriggerKeydown(event: KeyboardEvent): void {
    if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      toggleMenu(MENU_ID);
      queueMicrotask(() => focusFirstItem(popupEl));
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      moveToAdjacentMenu(MENU_ID, 1);
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      moveToAdjacentMenu(MENU_ID, -1);
    }
  }

  function closeSelf(): void {
    closeAllMenus();
    buttonEl?.focus();
  }

  function openRecentSubmenu(): void {
    recentOpen = true;
    void refreshRecentFiles();
    queueMicrotask(() => focusFirstItem(recentPopupEl));
  }

  function closeRecentSubmenu(focusTrigger: boolean): void {
    recentOpen = false;
    if (focusTrigger) {
      queueMicrotask(() => {
        popupEl?.querySelector<HTMLElement>('[data-testid="menu-open-recent"]')?.focus();
      });
    }
  }

  function onPopupKeydown(event: KeyboardEvent): void {
    const current = document.activeElement as HTMLElement | null;
    if (event.key === "ArrowRight" && current?.getAttribute("data-testid") === "menu-open-recent") {
      event.preventDefault();
      event.stopPropagation();
      openRecentSubmenu();
      return;
    }
    handleMenuKeydown(popupEl!, event, {
      onEscape: closeSelf,
      onArrowLeft: () => moveToAdjacentMenu(MENU_ID, -1),
      onArrowRight: () => moveToAdjacentMenu(MENU_ID, 1),
    });
  }

  function onRecentPopupKeydown(event: KeyboardEvent): void {
    handleMenuKeydown(recentPopupEl!, event, {
      onCloseSubmenu: () => closeRecentSubmenu(true),
      onEscape: () => closeRecentSubmenu(true),
    });
  }

  /**
   * H-15 (SPEC-018 §2.12): a missing entry no longer bails out silently — `pickRecentFile` shows
   * the dedicated dialog (Locate…/Remove from List/Cancel). The menu closes either way: a missing
   * entry's dialog is its own modal (`RecentMissingDialog`, mounted in `App.svelte`), not part of
   * this popup.
   */
  async function pickRecent(path: string, exists: boolean | null): Promise<void> {
    closeAllMenus();
    await pickRecentFile(path, exists);
  }
</script>

<div class="menu">
  <button
    bind:this={buttonEl}
    type="button"
    role="menuitem"
    aria-haspopup="menu"
    aria-expanded={open}
    data-menu-trigger={MENU_ID}
    data-testid="menu-trigger-file"
    onclick={onTriggerClick}
    onkeydown={onTriggerKeydown}
  >
    {mnemonic.before}<u>{mnemonic.letter}</u>{mnemonic.after}
  </button>
  {#if open}
    <div
      bind:this={popupEl}
      role="menu"
      tabindex="-1"
      aria-label={t("menu.file")}
      class="menu-popup"
      data-menu-popup={MENU_ID}
      data-testid="document-menu"
      onkeydown={onPopupKeydown}
      onclick={(e) => e.stopPropagation()}
    >
      <MenuItemRow
        label={t("menu.file.new_recording")}
        disabled={rec.state.recording || rec.state.finishing}
        testid="menu-new-recording"
        onSelect={() => select(openNewRecording)}
      />
      <MenuItemRow
        label={t("menu.file.open")}
        shortcut={shortcutLabelForAction("file.open")}
        testid="menu-open"
        onSelect={() => select(() => dispatchAction("file.open"))}
      />
      <MenuItemRow
        label={t("menu.file.open_recent")}
        testid="menu-open-recent"
        isSubmenuTrigger
        expanded={recentOpen}
        onSelect={() => (recentOpen ? closeRecentSubmenu(false) : openRecentSubmenu())}
      />
      {#if recentOpen}
        <div
          bind:this={recentPopupEl}
          role="menu"
          tabindex="-1"
          aria-label={t("menu.file.open_recent")}
          class="menu-popup submenu-popup"
          onkeydown={onRecentPopupKeydown}
        >
          {#if recent.entries.length === 0}
            <div class="empty">{t("menu.file.no_document")}</div>
          {:else}
            {#each recent.entries as entry (entry.path)}
              <MenuItemRow
                label={entry.exists === false ? `${entry.name} ${t("recent.missing")}` : entry.name}
                muted={entry.exists === false}
                testid="recent-entry-open"
                onSelect={() => void pickRecent(entry.path, entry.exists)}
              />
            {/each}
            <MenuSeparatorRow />
            <MenuItemRow
              label={t("menu.file.clear_recent")}
              testid="menu-clear-recent"
              onSelect={() => select(() => void clearRecentFiles())}
            />
          {/if}
        </div>
      {/if}
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("menu.file.save")}
        shortcut={shortcutLabelForAction("file.save")}
        disabled={!hasDocument(doc.current)}
        testid="menu-save"
        onSelect={() => select(() => dispatchAction("file.save"))}
      />
      <MenuItemRow
        label={t("menu.file.save_as")}
        shortcut={shortcutLabelForAction("file.save_as")}
        disabled={!hasDocument(doc.current)}
        testid="menu-save-as"
        onSelect={() => select(() => dispatchAction("file.save_as"))}
      />
      <MenuItemRow
        label={t("menu.file.export")}
        disabled={!hasDocument(doc.current)}
        testid="menu-export"
        onSelect={() => select(openExport)}
      />
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("menu.file.recovery")}
        testid="menu-recovery"
        onSelect={() => select(() => void openRecoveryStorage())}
      />
      {#if showPreferencesHere}
        <MenuItemRow
          label={t("menu.preferences")}
          testid="menu-preferences"
          onSelect={() => select(openPreferences)}
        />
      {/if}
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("menu.file.close")}
        disabled={!hasDocument(doc.current)}
        testid="menu-close"
        onSelect={() => select(() => void requestClose())}
      />
    </div>
  {/if}
</div>
<span class="document-name" data-testid="document-name">{label}</span>

<style>
  .menu {
    position: relative;
  }

  button[data-menu-trigger] {
    background: none;
    color: var(--text-primary);
    border: none;
    border-radius: 4px;
    padding: 0.3rem 0.6rem;
  }

  button[data-menu-trigger]:hover,
  button[data-menu-trigger][aria-expanded="true"] {
    background: var(--surface-panel-raised);
  }

  u {
    text-decoration: underline;
  }

  .menu-popup {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: 100;
    display: flex;
    flex-direction: column;
    min-width: 14rem;
    margin-top: 0.15rem;
    padding: 0.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);
  }

  .submenu-popup {
    top: 0;
    left: 100%;
    margin-top: 0;
    margin-left: 0.15rem;
    min-width: 18rem;
    max-width: 26rem;
  }

  .empty {
    padding: 0.35rem 0.5rem;
    color: var(--text-disabled);
  }

  .document-name {
    margin-left: 0.5rem;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
