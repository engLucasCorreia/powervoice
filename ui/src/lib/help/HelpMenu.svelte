<script lang="ts">
  import { t } from "../i18n";
  import { closeAllMenus, menubarState, moveToAdjacentMenu, toggleMenu } from "../menu/menubar.svelte";
  import { focusFirstItem, handleMenuKeydown } from "../menu/menuKeyboard";
  import MenuItemRow from "../menu/MenuItemRow.svelte";
  import { splitMnemonic } from "../menu/mnemonic";
  import { openAbout } from "./about.svelte";

  /** Help menu (H-19): a single "About PowerVoice…" item, with the app version (SPEC-000-adjacent
   * `app_info` — the ticket's "About with version"). */
  const MENU_ID = "help" as const;
  const bar = menubarState();
  const open = $derived(bar.openMenuId === MENU_ID);
  const mnemonic = $derived(splitMnemonic(t("menu.help"), "h"));

  let buttonEl: HTMLButtonElement | undefined = $state();
  let popupEl: HTMLDivElement | undefined = $state();

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

  function onPopupKeydown(event: KeyboardEvent): void {
    handleMenuKeydown(popupEl!, event, {
      onEscape: () => {
        closeAllMenus();
        buttonEl?.focus();
      },
      onArrowLeft: () => moveToAdjacentMenu(MENU_ID, -1),
      onArrowRight: () => moveToAdjacentMenu(MENU_ID, 1),
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
    data-testid="menu-trigger-help"
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
      aria-label={t("menu.help")}
      class="menu-popup"
      data-menu-popup={MENU_ID}
      data-testid="help-menu"
      onkeydown={onPopupKeydown}
      onclick={(e) => e.stopPropagation()}
    >
      <MenuItemRow
        label={t("menu.help.about")}
        testid="menu-about"
        onSelect={() => select(openAbout)}
      />
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
    min-width: 12rem;
    margin-top: 0.15rem;
    padding: 0.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);
  }
</style>
