/** About dialog visibility (H-19: Help → About with version). Trivial module-scoped state, same
 * pattern as every other dialog-open store in this codebase (e.g. `document.svelte.ts`'s
 * `saveAsPrompt`, simplified here since About has no fields of its own). */
let open = $state(false);

export function aboutState(): { readonly open: boolean } {
  return {
    get open() {
      return open;
    },
  };
}

export function openAbout(): void {
  open = true;
}

export function closeAbout(): void {
  open = false;
}

/** Test/teardown helper. */
export function resetAboutForTest(): void {
  open = false;
}
