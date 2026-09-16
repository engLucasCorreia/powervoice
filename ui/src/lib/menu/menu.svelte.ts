/**
 * Menu bar coordination (H-19): which of the five top-level menus (File/Edit/View/Effects/Help)
 * is open — same module-scoped `$state` store pattern as every other feature store in this
 * codebase (`edit.svelte.ts`, `spectral.svelte.ts`, …), not a component context, so it stays
 * consistent with the rest of the UI and needs no Svelte Snippet plumbing.
 *
 * Each top-level menu component (`DocumentMenu`, `EditMenu`, `ViewMenu`, `EffectsMenu`,
 * `HelpMenu`) is otherwise self-contained: it owns its own popup markup/keyboard handling (via
 * `menuKeyboard.ts`) and only calls into this module to open/close itself and its siblings, and to
 * move focus to an adjacent trigger button (`ArrowLeft`/`ArrowRight`, WAI-ARIA menubar pattern).
 * Cross-component focus uses plain `data-menu-trigger`/`data-menu-popup` attribute lookups rather
 * than component refs — the simplest thing that works when every menu is its own component with
 * no shared parent besides `MenuBar.svelte`'s plain wrapper `<div role="menubar">`.
 */

export type MenuId = "file" | "edit" | "view" | "effects" | "help";

/** Left-to-right order for `ArrowLeft`/`ArrowRight` (also `MenuBar.svelte`'s render order). */
export const MENU_ORDER: readonly MenuId[] = ["file", "edit", "view", "effects", "help"];

/** Alt+letter mnemonics. "Effects" can't use "e" (Edit already does) — "c" mirrors Audacity's own
 * "effeCt" mnemonic, the closest prior art for an audio editor's menu bar. */
export const MENU_MNEMONICS: Record<MenuId, string> = {
  file: "f",
  edit: "e",
  view: "v",
  effects: "c",
  help: "h",
};

let openMenuId = $state<MenuId | null>(null);

export function menubarState(): { readonly openMenuId: MenuId | null } {
  return {
    get openMenuId() {
      return openMenuId;
    },
  };
}

export function openMenu(id: MenuId): void {
  openMenuId = id;
}

export function closeAllMenus(): void {
  openMenuId = null;
}

export function toggleMenu(id: MenuId): void {
  openMenuId = openMenuId === id ? null : id;
}

/** Moves DOM focus to `id`'s trigger button, if it's currently mounted. */
export function focusMenuTrigger(id: MenuId): void {
  document.querySelector<HTMLElement>(`[data-menu-trigger="${id}"]`)?.focus();
}

/**
 * `ArrowLeft`/`ArrowRight` from `current`'s trigger or popup (WAI-ARIA menubar pattern): moves
 * focus to the adjacent top-level trigger, wrapping around. If a menu was already open, the
 * adjacent one opens too (cascading through the bar); otherwise focus just moves without opening
 * anything (simplification: focus lands on the new trigger button, not inside its popup — pressing
 * `ArrowDown`/`Enter` there opens it and focuses its first item, same as any other trigger).
 */
export function moveToAdjacentMenu(current: MenuId, direction: 1 | -1): void {
  const index = MENU_ORDER.indexOf(current);
  const next = MENU_ORDER[(index + direction + MENU_ORDER.length) % MENU_ORDER.length]!;
  const wasOpen = openMenuId !== null;
  if (wasOpen) {
    openMenuId = next;
  }
  focusMenuTrigger(next);
}

/**
 * Alt+letter mnemonics (ticket: "Alt+letter mnemonics"). Attached once from `App.svelte` (mirrors
 * `attachKeymap`): opens the matching menu and focuses its trigger, from anywhere in the window.
 * Ignored while Ctrl/⌘ is also held (keeps e.g. Ctrl+Alt+arrow marker navigation unaffected).
 */
export function attachMenuBarMnemonics(target: Window = window): () => void {
  const onKeydown = (event: KeyboardEvent): void => {
    if (!event.altKey || event.ctrlKey || event.metaKey) {
      return;
    }
    const key = event.key.toLowerCase();
    const entry = (Object.entries(MENU_MNEMONICS) as [MenuId, string][]).find(
      ([, letter]) => letter === key,
    );
    if (!entry) {
      return;
    }
    event.preventDefault();
    openMenu(entry[0]);
    focusMenuTrigger(entry[0]);
  };
  target.addEventListener("keydown", onKeydown);
  return () => target.removeEventListener("keydown", onKeydown);
}

/** Test/teardown helper. */
export function resetMenuBarForTest(): void {
  openMenuId = null;
}
