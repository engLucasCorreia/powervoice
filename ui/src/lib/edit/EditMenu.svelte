<script lang="ts">
  import { hasDocument, documentState } from "../document/document.svelte";
  import { t, tDynamic } from "../i18n";
  import { dispatchAction } from "../keymap";
  import { isPlatformMac } from "../keymap/registry";
  import { shortcutLabelForAction } from "../keymap/shortcutLabel";
  import { closeAllMenus, menubarState, moveToAdjacentMenu, toggleMenu } from "../menu/menubar.svelte";
  import { focusFirstItem, handleMenuKeydown } from "../menu/menuKeyboard";
  import MenuItemRow from "../menu/MenuItemRow.svelte";
  import MenuSeparatorRow from "../menu/MenuSeparatorRow.svelte";
  import { splitMnemonic } from "../menu/mnemonic";
  import { markersState } from "../markers/markers.svelte";
  import { openPreferences } from "../preferences/preferences.svelte";
  import { recordState } from "../state/record.svelte";
  import { hasClipboard, editState, silence } from "../state/edit.svelte";
  import { hasSelection } from "../state/selection.svelte";

  /**
   * Edit menu (H-19): Undo/Redo (with the history's i18n label), Cut/Copy/Paste/Delete/Trim/
   * Silence, Select All, Markers ▸ — a real dropdown replacing S2-01/S2-03's flat always-visible
   * row. Every item with a keymap binding dispatches through the same `dispatchAction` path the
   * shortcut itself uses (Silence and the normalize favorites have no default binding — menu
   * only, `bindings.ts`).
   *
   * H-17: Preferences… lives here on non-macOS platforms (File → Preferences… on macOS,
   * `DocumentMenu.svelte`) — there's no native app menu to put it in instead.
   */
  const MENU_ID = "edit" as const;
  const showPreferencesHere = !isPlatformMac();
  const doc = documentState();
  const edit = editState();
  const rec = recordState();
  const markers = markersState();
  const bar = menubarState();
  const open = $derived(bar.openMenuId === MENU_ID);
  const mnemonic = $derived(splitMnemonic(t("menu.edit"), "e"));

  const recording = $derived(rec.state.recording);
  const selected = $derived(hasSelection() && !recording);
  const pasteEnabled = $derived(hasClipboard() && !recording);
  const hasDoc = $derived(hasDocument(doc.current));

  /** T-301 (ADR-004 Amendment 3): a label's placeholder values ("Normalize to {target} dB"). */
  function labelParams(params: Partial<Record<string, string>>): Record<string, string> {
    return Object.fromEntries(
      Object.entries(params).filter((e): e is [string, string] => e[1] !== undefined),
    );
  }

  const undoLabel = $derived(
    edit.history.undo_label
      ? t("menu.edit.undo", {
          label: tDynamic(edit.history.undo_label, labelParams(edit.history.undo_label_params)),
        })
      : t("menu.edit.undo_none"),
  );
  const redoLabel = $derived(
    edit.history.redo_label
      ? t("menu.edit.redo", {
          label: tDynamic(edit.history.redo_label, labelParams(edit.history.redo_label_params)),
        })
      : t("menu.edit.redo_none"),
  );

  let buttonEl: HTMLButtonElement | undefined = $state();
  let popupEl: HTMLDivElement | undefined = $state();
  let markersOpen = $state(false);
  let markersPopupEl: HTMLDivElement | undefined = $state();

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

  function openMarkersSubmenu(): void {
    markersOpen = true;
    queueMicrotask(() => focusFirstItem(markersPopupEl));
  }

  function closeMarkersSubmenu(focusTrigger: boolean): void {
    markersOpen = false;
    if (focusTrigger) {
      queueMicrotask(() => {
        popupEl?.querySelector<HTMLElement>('[data-testid="menu-markers"]')?.focus();
      });
    }
  }

  function onPopupKeydown(event: KeyboardEvent): void {
    const current = document.activeElement as HTMLElement | null;
    if (event.key === "ArrowRight" && current?.getAttribute("data-testid") === "menu-markers") {
      event.preventDefault();
      event.stopPropagation();
      openMarkersSubmenu();
      return;
    }
    handleMenuKeydown(popupEl!, event, {
      onEscape: closeSelf,
      onArrowLeft: () => moveToAdjacentMenu(MENU_ID, -1),
      onArrowRight: () => moveToAdjacentMenu(MENU_ID, 1),
    });
  }

  function onMarkersPopupKeydown(event: KeyboardEvent): void {
    handleMenuKeydown(markersPopupEl!, event, {
      onCloseSubmenu: () => closeMarkersSubmenu(true),
      onEscape: () => closeMarkersSubmenu(true),
    });
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
    data-testid="menu-trigger-edit"
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
      aria-label={t("menu.edit")}
      class="menu-popup"
      data-menu-popup={MENU_ID}
      data-testid="edit-menu"
      onkeydown={onPopupKeydown}
      onclick={(e) => e.stopPropagation()}
    >
      <MenuItemRow
        label={undoLabel}
        shortcut={shortcutLabelForAction("history.undo")}
        disabled={!edit.history.can_undo || recording}
        testid="menu-undo"
        onSelect={() => select(() => dispatchAction("history.undo"))}
      />
      <MenuItemRow
        label={redoLabel}
        shortcut={shortcutLabelForAction("history.redo")}
        disabled={!edit.history.can_redo || recording}
        testid="menu-redo"
        onSelect={() => select(() => dispatchAction("history.redo"))}
      />
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("edit.cut")}
        shortcut={shortcutLabelForAction("edit.cut")}
        disabled={!selected}
        testid="menu-cut"
        onSelect={() => select(() => dispatchAction("edit.cut"))}
      />
      <MenuItemRow
        label={t("edit.copy")}
        shortcut={shortcutLabelForAction("edit.copy")}
        disabled={!selected}
        testid="menu-copy"
        onSelect={() => select(() => dispatchAction("edit.copy"))}
      />
      <MenuItemRow
        label={t("edit.paste")}
        shortcut={shortcutLabelForAction("edit.paste")}
        disabled={!pasteEnabled}
        testid="menu-paste"
        onSelect={() => select(() => dispatchAction("edit.paste"))}
      />
      <MenuItemRow
        label={t("edit.delete")}
        shortcut={shortcutLabelForAction("edit.delete")}
        disabled={!selected}
        testid="menu-delete"
        onSelect={() => select(() => dispatchAction("edit.delete"))}
      />
      <MenuItemRow
        label={t("edit.trim")}
        shortcut={shortcutLabelForAction("edit.trim")}
        disabled={!selected}
        testid="menu-trim"
        onSelect={() => select(() => dispatchAction("edit.trim"))}
      />
      <MenuItemRow
        label={t("edit.silence")}
        disabled={!selected}
        testid="menu-silence"
        onSelect={() => select(() => void silence())}
      />
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("menu.edit.select_all")}
        shortcut={shortcutLabelForAction("waveform.select_all")}
        disabled={!hasDoc || recording}
        testid="menu-select-all"
        onSelect={() => select(() => dispatchAction("waveform.select_all"))}
      />
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("menu.edit.markers")}
        testid="menu-markers"
        isSubmenuTrigger
        expanded={markersOpen}
        onSelect={() => (markersOpen ? closeMarkersSubmenu(false) : openMarkersSubmenu())}
      />
      {#if markersOpen}
        <div
          bind:this={markersPopupEl}
          role="menu"
          tabindex="-1"
          aria-label={t("menu.edit.markers")}
          class="menu-popup submenu-popup"
          onkeydown={onMarkersPopupKeydown}
        >
          <MenuItemRow
            label={t("menu.edit.marker_add")}
            shortcut={shortcutLabelForAction("marker.add")}
            disabled={!hasDoc}
            testid="menu-marker-add"
            onSelect={() => select(() => dispatchAction("marker.add"))}
          />
          <MenuItemRow
            label={t("menu.edit.marker_delete_selected")}
            shortcut={shortcutLabelForAction("marker.delete_selected")}
            disabled={markers.selectedId === null}
            testid="menu-marker-delete"
            onSelect={() => select(() => dispatchAction("marker.delete_selected"))}
          />
          <MenuItemRow
            label={t("menu.edit.marker_next")}
            shortcut={shortcutLabelForAction("marker.next")}
            disabled={markers.list.length === 0}
            testid="menu-marker-next"
            onSelect={() => select(() => dispatchAction("marker.next"))}
          />
          <MenuItemRow
            label={t("menu.edit.marker_prev")}
            shortcut={shortcutLabelForAction("marker.prev")}
            disabled={markers.list.length === 0}
            testid="menu-marker-prev"
            onSelect={() => select(() => dispatchAction("marker.prev"))}
          />
        </div>
      {/if}
      {#if showPreferencesHere}
        <MenuSeparatorRow />
        <MenuItemRow
          label={t("menu.preferences")}
          testid="menu-preferences"
          onSelect={() => select(openPreferences)}
        />
      {/if}
    </div>
  {/if}
</div>

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
    min-width: 16rem;
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
    min-width: 14rem;
  }
</style>
