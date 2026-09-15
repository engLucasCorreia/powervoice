<script lang="ts">
  import Menu from "../ui/Menu.svelte";
  import type { MenuEntry } from "../ui/menuModel";
  import {
    closeAllMenus,
    menubarState,
    moveToAdjacentMenu,
    openMenu,
    toggleMenu,
    type MenuId,
  } from "./menubar.svelte";
  import { splitMnemonic } from "./mnemonic";

  /**
   * One menu-bar menu (H-26): the trigger (`role="menuitem"` in the `menubar`, mnemonic letter
   * underlined) plus the shared `Menu`. Which menu is open lives in `menubar.svelte.ts`; this
   * wires the WAI-ARIA menubar keys — ↓/Enter/Space open on the first item, ↑ on the last, ←/→
   * move between the bar's menus (from the trigger or from inside an open menu) — and, while one
   * menu is open, hovering another trigger switches to it. The five feature menus only build
   * their `MenuEntry[]`.
   */
  let {
    id,
    label,
    mnemonic,
    items,
    triggerTestid,
    menuTestid,
    minWidth,
  }: {
    id: MenuId;
    label: string;
    /** The Alt+letter mnemonic (`MENU_MNEMONICS`), underlined in the trigger. */
    mnemonic: string;
    items: MenuEntry[];
    triggerTestid: string;
    menuTestid: string;
    minWidth?: number;
  } = $props();

  const bar = menubarState();
  const open = $derived(bar.openMenuId === id);
  const parts = $derived(splitMnemonic(label, mnemonic));
  let trigger: HTMLButtonElement | undefined = $state();
  let initialFocus = $state<"first" | "last" | "menu">("first");

  function onTriggerClick(event: MouseEvent): void {
    event.stopPropagation();
    // A real pointer click (detail ≥ 1) focuses the menu itself; a keyboard or scripted click
    // lands on the first item.
    initialFocus = event.detail > 0 ? "menu" : "first";
    toggleMenu(id);
  }

  function onTriggerKeydown(event: KeyboardEvent): void {
    switch (event.key) {
      case "ArrowDown":
      case "Enter":
      case " ":
        event.preventDefault();
        initialFocus = "first";
        openMenu(id);
        break;
      case "ArrowUp":
        event.preventDefault();
        initialFocus = "last";
        openMenu(id);
        break;
      case "ArrowRight":
        event.preventDefault();
        moveToAdjacentMenu(id, 1);
        break;
      case "ArrowLeft":
        event.preventDefault();
        moveToAdjacentMenu(id, -1);
        break;
      default:
        break;
    }
  }

  function onTriggerPointerEnter(): void {
    if (bar.openMenuId !== null && bar.openMenuId !== id) {
      initialFocus = "menu";
      openMenu(id);
    }
  }
</script>

<div class="menubar-menu">
  <button
    bind:this={trigger}
    type="button"
    role="menuitem"
    aria-haspopup="menu"
    aria-expanded={open}
    data-menu-trigger={id}
    data-testid={triggerTestid}
    onclick={onTriggerClick}
    onkeydown={onTriggerKeydown}
    onpointerenter={onTriggerPointerEnter}
  >
    {parts.before}<u>{parts.letter}</u>{parts.after}
  </button>
  <Menu
    {open}
    anchor={trigger}
    {items}
    {label}
    testid={menuTestid}
    {minWidth}
    {initialFocus}
    onclose={() => closeAllMenus()}
    onnavigate={(direction) => moveToAdjacentMenu(id, direction)}
    data-menu-popup={id}
  />
</div>

<style>
  .menubar-menu {
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

  button[data-menu-trigger]:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  u {
    text-decoration: underline;
  }
</style>
