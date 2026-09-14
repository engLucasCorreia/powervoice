<script lang="ts">
  import { analyzerState, setAnalyzerVisible } from "../analyzer/analyzer.svelte";
  import { t, tDynamic } from "../i18n";
  import { dispatchAction } from "../keymap";
  import { shortcutLabelForAction } from "../keymap/shortcutLabel";
  import { closeAllMenus, menubarState, moveToAdjacentMenu, toggleMenu } from "../menu/menubar.svelte";
  import MenuCheckboxRow from "../menu/MenuCheckboxRow.svelte";
  import { focusFirstItem, handleMenuKeydown } from "../menu/menuKeyboard";
  import MenuItemRow from "../menu/MenuItemRow.svelte";
  import MenuRadioRow from "../menu/MenuRadioRow.svelte";
  import MenuSeparatorRow from "../menu/MenuSeparatorRow.svelte";
  import { splitMnemonic } from "../menu/mnemonic";
  import type { RendererPreference } from "../render/rendererMode";
  import { rendererPref, setRendererPreference } from "../state/rendererPref.svelte";
  import { saveSettings } from "../state/settings.svelte";
  import { spectralState } from "../state/spectral.svelte";

  /**
   * View menu (H-19): Spectral/Analyzer toggles, waveform zoom, and the H-13 renderer override
   * (View → Renderer, backed by `Settings.renderer_preference`) — a real dropdown replacing
   * H-16's flat single-toggle row.
   */
  const MENU_ID = "view" as const;
  const analyzer = analyzerState();
  const spectral = spectralState();
  const bar = menubarState();
  const open = $derived(bar.openMenuId === MENU_ID);
  const mnemonic = $derived(splitMnemonic(t("menu.view"), "v"));
  const renderer = rendererPref();

  const RENDERER_OPTIONS: readonly { value: RendererPreference; labelKey: string }[] = [
    { value: "auto", labelKey: "menu.view.renderer_auto" },
    { value: "webgl2", labelKey: "menu.view.renderer_webgl2" },
    { value: "canvas2d", labelKey: "menu.view.renderer_canvas2d" },
  ];

  let buttonEl: HTMLButtonElement | undefined = $state();
  let popupEl: HTMLDivElement | undefined = $state();
  let rendererOpen = $state(false);
  let rendererPopupEl: HTMLDivElement | undefined = $state();

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

  function openRendererSubmenu(): void {
    rendererOpen = true;
    queueMicrotask(() => focusFirstItem(rendererPopupEl));
  }

  function closeRendererSubmenu(focusTrigger: boolean): void {
    rendererOpen = false;
    if (focusTrigger) {
      queueMicrotask(() => {
        popupEl?.querySelector<HTMLElement>('[data-testid="menu-renderer"]')?.focus();
      });
    }
  }

  /** Applies the choice to the live renderers (H-13's store, no reload needed) and persists it
   * (H-19: `Settings.renderer_preference`, `just gen-types`) so it survives a restart. */
  function pickRenderer(value: RendererPreference): void {
    setRendererPreference(value);
    void saveSettings({ renderer_preference: value });
  }

  function onPopupKeydown(event: KeyboardEvent): void {
    const current = document.activeElement as HTMLElement | null;
    if (event.key === "ArrowRight" && current?.getAttribute("data-testid") === "menu-renderer") {
      event.preventDefault();
      event.stopPropagation();
      openRendererSubmenu();
      return;
    }
    handleMenuKeydown(popupEl!, event, {
      onEscape: closeSelf,
      onArrowLeft: () => moveToAdjacentMenu(MENU_ID, -1),
      onArrowRight: () => moveToAdjacentMenu(MENU_ID, 1),
    });
  }

  function onRendererPopupKeydown(event: KeyboardEvent): void {
    handleMenuKeydown(rendererPopupEl!, event, {
      onCloseSubmenu: () => closeRendererSubmenu(true),
      onEscape: () => closeRendererSubmenu(true),
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
    data-testid="menu-trigger-view"
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
      aria-label={t("menu.view")}
      class="menu-popup"
      data-menu-popup={MENU_ID}
      data-testid="view-menu"
      onkeydown={onPopupKeydown}
      onclick={(e) => e.stopPropagation()}
    >
      <MenuCheckboxRow
        label={t("spectral.toggle")}
        checked={spectral.visible}
        shortcut={shortcutLabelForAction("spectral.toggle")}
        testid="menu-view-spectral"
        onToggle={() => select(() => dispatchAction("spectral.toggle"))}
      />
      <MenuCheckboxRow
        label={t("menu.view.analyzer")}
        checked={analyzer.visible}
        testid="menu-view-analyzer"
        onToggle={() => select(() => setAnalyzerVisible(!analyzer.visible))}
      />
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("menu.view.zoom_in")}
        shortcut={shortcutLabelForAction("waveform.zoom_in")}
        testid="menu-zoom-in"
        onSelect={() => select(() => dispatchAction("waveform.zoom_in"))}
      />
      <MenuItemRow
        label={t("menu.view.zoom_out")}
        shortcut={shortcutLabelForAction("waveform.zoom_out")}
        testid="menu-zoom-out"
        onSelect={() => select(() => dispatchAction("waveform.zoom_out"))}
      />
      <MenuSeparatorRow />
      <MenuItemRow
        label={t("menu.view.renderer")}
        testid="menu-renderer"
        isSubmenuTrigger
        expanded={rendererOpen}
        onSelect={() => (rendererOpen ? closeRendererSubmenu(false) : openRendererSubmenu())}
      />
      {#if rendererOpen}
        <div
          bind:this={rendererPopupEl}
          role="menu"
          tabindex="-1"
          aria-label={t("menu.view.renderer")}
          class="menu-popup submenu-popup"
          onkeydown={onRendererPopupKeydown}
        >
          {#each RENDERER_OPTIONS as option (option.value)}
            <MenuRadioRow
              label={tDynamic(option.labelKey)}
              checked={renderer.value === option.value}
              testid={`menu-renderer-${option.value}`}
              onSelect={() => select(() => pickRenderer(option.value))}
            />
          {/each}
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
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: none;
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    cursor: default;
  }

  button[data-menu-trigger]:hover,
  button[data-menu-trigger][aria-expanded="true"] {
    background: var(--pv-control-bg-active);
  }

  u {
    text-decoration: underline;
  }

  .menu-popup {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: var(--pv-z-dropdown);
    display: flex;
    flex-direction: column;
    min-width: 14rem;
    margin-top: var(--pv-space-half);
    padding: var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-2);
  }

  .submenu-popup {
    top: 0;
    left: 100%;
    margin-top: 0;
    margin-left: 0.15rem;
    min-width: 10rem;
  }
</style>
