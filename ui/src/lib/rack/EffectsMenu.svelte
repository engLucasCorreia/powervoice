<script lang="ts">
  import { t } from "../i18n";
  import type { PresetEntryDto, PresetRefDto } from "../ipc/bindings";
  import { dispatchAction } from "../keymap";
  import { shortcutLabelForAction } from "../keymap/shortcutLabel";
  import { closeAllMenus, menubarState, moveToAdjacentMenu, toggleMenu } from "../menu/menubar.svelte";
  import { focusFirstItem, handleMenuKeydown } from "../menu/menuKeyboard";
  import MenuItemRow from "../menu/MenuItemRow.svelte";
  import MenuSeparatorRow from "../menu/MenuSeparatorRow.svelte";
  import { splitMnemonic } from "../menu/mnemonic";
  import {
    canNormalize,
    FAVORITE_TARGETS_DB,
    normalizeFavorite,
    openNormalizeDialog,
  } from "../state/normalize.svelte";
  import {
    canNormalizeLufs,
    FAVORITE_TARGETS_LUFS,
    normalizeLufsFavorite,
    openNormalizeLufsDialog,
  } from "../state/normalizeLufs.svelte";
  import { localized } from "./localized";
  import { canCapture } from "./nrCapture.svelte";
  import {
    deleteRackPreset,
    listRackPresets,
    loadRackPreset,
    rackState,
    saveRackPreset,
  } from "./rack.svelte";

  /**
   * Effects menu (H-19): Normalize…, Normalize (LUFS)…, Capture Noise Print, then Favorites ▸ (the
   * six one-click presets, S2-02/S4-01's old flat `FavoritesMenu` row folded in here as a
   * submenu — its "Normalize…"/"Normalize (LUFS)…" entries are dropped as duplicates of the two
   * items right above). The toolbar keeps its own compact favorite buttons
   * (`NormalizeToolbarButtons`, ticket: "keep the toolbar for ... the favorite normalize
   * buttons") — this is the menu-bar path to the same actions, plus the full dialogs.
   */
  const MENU_ID = "effects" as const;
  const bar = menubarState();
  const open = $derived(bar.openMenuId === MENU_ID);
  const mnemonic = $derived(splitMnemonic(t("menu.effects"), "c"));

  const captureEnabled = $derived(canCapture());
  const normalizeEnabled = $derived(canNormalize());
  const normalizeLufsEnabled = $derived(canNormalizeLufs());

  let buttonEl: HTMLButtonElement | undefined = $state();
  let popupEl: HTMLDivElement | undefined = $state();
  let favoritesOpen = $state(false);
  let favoritesPopupEl: HTMLDivElement | undefined = $state();

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

  function openFavoritesSubmenu(): void {
    favoritesOpen = true;
    queueMicrotask(() => focusFirstItem(favoritesPopupEl));
  }

  function closeFavoritesSubmenu(focusTrigger: boolean): void {
    favoritesOpen = false;
    if (focusTrigger) {
      queueMicrotask(() => {
        popupEl?.querySelector<HTMLElement>('[data-testid="menu-favorites"]')?.focus();
      });
    }
  }

  function onPopupKeydown(event: KeyboardEvent): void {
    const current = document.activeElement as HTMLElement | null;
    if (event.key === "ArrowRight" && current?.getAttribute("data-testid") === "menu-favorites") {
      event.preventDefault();
      event.stopPropagation();
      openFavoritesSubmenu();
      return;
    }
    if (
      event.key === "ArrowRight" &&
      current?.getAttribute("data-testid") === "menu-rack-presets"
    ) {
      event.preventDefault();
      event.stopPropagation();
      openRackPresetsSubmenu();
      return;
    }
    handleMenuKeydown(popupEl!, event, {
      onEscape: closeSelf,
      onArrowLeft: () => moveToAdjacentMenu(MENU_ID, -1),
      onArrowRight: () => moveToAdjacentMenu(MENU_ID, 1),
    });
  }

  function onFavoritesPopupKeydown(event: KeyboardEvent): void {
    handleMenuKeydown(favoritesPopupEl!, event, {
      onCloseSubmenu: () => closeFavoritesSubmenu(true),
      onEscape: () => closeFavoritesSubmenu(true),
    });
  }

  function favoritesDbTestId(targetDb: number): string {
    return `menu-favorites-normalize-${Math.abs(targetDb).toFixed(1).replace(".", "-")}db`;
  }

  function favoritesLufsTestId(targetLufs: number): string {
    return `menu-favorites-normalize-lufs-${Math.abs(targetLufs).toFixed(0)}`;
  }

  // --- Rack Presets (T-406, SPEC-012 "the rack-preset menu") ------------------------------
  let rackPresetsOpen = $state(false);
  let rackPresetsPopupEl: HTMLDivElement | undefined = $state();
  let rackPresetEntries = $state<PresetEntryDto[] | null>(null);
  let savingRackPreset = $state(false);
  let rackPresetName = $state("");
  let confirmingRackPreset = $state<PresetRefDto | null>(null);

  function refToKey(ref: PresetRefDto): string {
    return ref.kind === "factory" ? `factory:${ref.key}` : `user:${ref.name}`;
  }

  function openRackPresetsSubmenu(): void {
    rackPresetsOpen = true;
    savingRackPreset = false;
    confirmingRackPreset = null;
    queueMicrotask(() => focusFirstItem(rackPresetsPopupEl));
    void refreshRackPresets();
  }

  function closeRackPresetsSubmenu(focusTrigger: boolean): void {
    rackPresetsOpen = false;
    savingRackPreset = false;
    confirmingRackPreset = null;
    if (focusTrigger) {
      queueMicrotask(() => {
        popupEl?.querySelector<HTMLElement>('[data-testid="menu-rack-presets"]')?.focus();
      });
    }
  }

  async function refreshRackPresets(): Promise<void> {
    rackPresetEntries = await listRackPresets();
  }

  function startSaveRackPreset(): void {
    savingRackPreset = true;
    rackPresetName = "";
  }

  async function confirmSaveRackPreset(): Promise<void> {
    const name = rackPresetName.trim();
    if (!name) {
      return;
    }
    const saved = await saveRackPreset(name);
    if (saved) {
      savingRackPreset = false;
      rackPresetName = "";
      await refreshRackPresets();
    }
  }

  async function applyRackPreset(ref: PresetRefDto): Promise<void> {
    confirmingRackPreset = null;
    closeSelf();
    await loadRackPreset(ref);
  }

  function pickRackPreset(entry: PresetEntryDto): void {
    const ref: PresetRefDto = entry.is_factory
      ? { kind: "factory", key: entry.key }
      : { kind: "user", name: entry.key };
    if (rackState().state.slots.length > 0) {
      confirmingRackPreset = ref;
    } else {
      void applyRackPreset(ref);
    }
  }

  async function deleteRackPresetEntry(entry: PresetEntryDto, event: MouseEvent): Promise<void> {
    event.stopPropagation();
    if (await deleteRackPreset(entry.key)) {
      await refreshRackPresets();
    }
  }

  function onRackPresetsPopupKeydown(event: KeyboardEvent): void {
    handleMenuKeydown(rackPresetsPopupEl!, event, {
      onCloseSubmenu: () => closeRackPresetsSubmenu(true),
      onEscape: () => closeRackPresetsSubmenu(true),
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
    data-testid="menu-trigger-effects"
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
      aria-label={t("menu.effects")}
      class="menu-popup"
      data-menu-popup={MENU_ID}
      data-testid="effects-menu"
      onkeydown={onPopupKeydown}
      onclick={(e) => e.stopPropagation()}
    >
      <MenuItemRow
        label={t("effects.normalize_dialog")}
        disabled={!normalizeEnabled}
        testid="menu-normalize-dialog"
        onSelect={() => select(openNormalizeDialog)}
      />
      <MenuItemRow
        label={t("effects.normalize_lufs_dialog")}
        disabled={!normalizeLufsEnabled}
        testid="menu-normalize-lufs-dialog"
        onSelect={() => select(openNormalizeLufsDialog)}
      />
      <MenuItemRow
        label={t("module.noise_reduction.capture")}
        shortcut={shortcutLabelForAction("nr.capture_noise_print")}
        disabled={!captureEnabled}
        testid="menu-capture-noise-print"
        onSelect={() => select(() => dispatchAction("nr.capture_noise_print"))}
      />
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("favorites.menu")}
        testid="menu-favorites"
        isSubmenuTrigger
        expanded={favoritesOpen}
        onSelect={() => (favoritesOpen ? closeFavoritesSubmenu(false) : openFavoritesSubmenu())}
      />
      {#if favoritesOpen}
        <div
          bind:this={favoritesPopupEl}
          role="menu"
          tabindex="-1"
          aria-label={t("favorites.menu")}
          class="menu-popup submenu-popup"
          onkeydown={onFavoritesPopupKeydown}
        >
          {#each FAVORITE_TARGETS_DB as targetDb (targetDb)}
            <MenuItemRow
              label={t("favorites.normalize_peak", { target: targetDb.toFixed(1) })}
              disabled={!normalizeEnabled}
              testid={favoritesDbTestId(targetDb)}
              onSelect={() => select(() => void normalizeFavorite(targetDb))}
            />
          {/each}
          <MenuSeparatorRow />
          {#each FAVORITE_TARGETS_LUFS as targetLufs (targetLufs)}
            <MenuItemRow
              label={t("favorites.normalize_lufs", { target: targetLufs.toFixed(1) })}
              disabled={!normalizeLufsEnabled}
              testid={favoritesLufsTestId(targetLufs)}
              onSelect={() => select(() => void normalizeLufsFavorite(targetLufs))}
            />
          {/each}
        </div>
      {/if}
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("rack_preset.menu")}
        testid="menu-rack-presets"
        isSubmenuTrigger
        expanded={rackPresetsOpen}
        onSelect={() => (rackPresetsOpen ? closeRackPresetsSubmenu(false) : openRackPresetsSubmenu())}
      />
      {#if rackPresetsOpen}
        <div
          bind:this={rackPresetsPopupEl}
          role="menu"
          tabindex="-1"
          aria-label={t("rack_preset.menu")}
          class="menu-popup submenu-popup"
          data-testid="rack-presets-submenu"
          onkeydown={onRackPresetsPopupKeydown}
        >
          {#if confirmingRackPreset}
            <div class="confirm-replace">
              <p>{t("rack_preset.confirm_replace")}</p>
              <div class="confirm-actions">
                <button type="button" onclick={() => (confirmingRackPreset = null)}>
                  {t("rack_preset.confirm_replace_cancel")}
                </button>
                <button
                  type="button"
                  class="primary"
                  data-testid="rack-preset-confirm-replace"
                  onclick={() => void applyRackPreset(confirmingRackPreset!)}
                >
                  {t("rack_preset.confirm_replace_confirm")}
                </button>
              </div>
            </div>
          {:else if rackPresetEntries === null}
            <span class="preset-empty">…</span>
          {:else if rackPresetEntries.length === 0 && !savingRackPreset}
            <span class="preset-empty">{t("rack_preset.none")}</span>
          {:else}
            {#each rackPresetEntries as entry (refToKey(entry.is_factory ? { kind: "factory", key: entry.key } : { kind: "user", name: entry.key }))}
              <div class="preset-row">
                <button
                  type="button"
                  role="menuitem"
                  class="preset-name"
                  data-testid="rack-preset-{entry.key}"
                  onclick={() => pickRackPreset(entry)}
                >
                  {localized(entry.name)}
                </button>
                {#if !entry.is_factory}
                  <button
                    type="button"
                    class="preset-delete"
                    title={t("rack_preset.delete")}
                    data-testid="rack-preset-delete-{entry.key}"
                    onclick={(e) => void deleteRackPresetEntry(entry, e)}
                  >
                    ×
                  </button>
                {/if}
              </div>
            {/each}
          {/if}
          {#if !confirmingRackPreset}
            <MenuSeparatorRow />
            {#if savingRackPreset}
              <div class="save-form">
                <input
                  type="text"
                  placeholder={t("rack_preset.name_placeholder")}
                  data-testid="rack-preset-name"
                  bind:value={rackPresetName}
                  onkeydown={(e) => {
                    if (e.key === "Enter") void confirmSaveRackPreset();
                  }}
                />
                <div class="save-actions">
                  <button
                    type="button"
                    data-testid="rack-preset-save-confirm"
                    onclick={() => void confirmSaveRackPreset()}
                  >
                    {t("rack_preset.save_button")}
                  </button>
                  <button type="button" onclick={() => (savingRackPreset = false)}>
                    {t("rack_preset.cancel_button")}
                  </button>
                </div>
              </div>
            {:else}
              <MenuItemRow
                label={t("rack_preset.save_as")}
                testid="rack-preset-save"
                onSelect={startSaveRackPreset}
              />
            {/if}
          {/if}
        </div>
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
    min-width: 15rem;
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
    min-width: 12rem;
  }

  .preset-empty {
    color: var(--text-secondary);
    font-size: 0.8rem;
    padding: 0.3rem 0.6rem;
  }

  .preset-row {
    display: flex;
    align-items: center;
  }

  .preset-name {
    flex: 1;
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text-primary);
    padding: 0.3rem 0.6rem;
  }

  .preset-name:hover {
    background: var(--surface-panel-raised);
  }

  .preset-delete {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    padding: 0 0.4rem;
  }

  .preset-delete:hover {
    color: var(--meter-yellow);
  }

  .save-form {
    display: flex;
    flex-direction: column;
    gap: 0.3rem;
    padding: 0.3rem 0.6rem;
  }

  .save-form input[type="text"] {
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    color: var(--text-primary);
    border-radius: 3px;
    padding: 0.2rem 0.4rem;
  }

  .save-actions,
  .confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.4rem;
  }

  .confirm-replace {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.4rem 0.6rem;
    max-width: 14rem;
  }

  .confirm-replace p {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.8rem;
  }

  .confirm-replace button.primary {
    border-color: var(--accent);
    color: var(--accent);
  }
</style>
