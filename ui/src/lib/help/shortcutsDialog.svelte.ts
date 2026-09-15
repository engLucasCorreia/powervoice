/**
 * Help ▸ Keyboard Shortcuts dialog visibility (T-701, item 2's "Help → Keyboard Shortcuts list ...
 * add a simple dialog built with the H-26 kit"). Same trivial module-scoped-state pattern as
 * `help/about.svelte.ts`.
 */
let open = $state(false);

export function shortcutsDialogState(): { readonly open: boolean } {
  return {
    get open() {
      return open;
    },
  };
}

export function openShortcutsDialog(): void {
  open = true;
}

export function closeShortcutsDialog(): void {
  open = false;
}

/** Test/teardown helper. */
export function resetShortcutsDialogForTest(): void {
  open = false;
}
