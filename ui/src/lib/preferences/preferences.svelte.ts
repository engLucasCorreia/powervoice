import { loadSettings, settingsState } from "../state/settings.svelte";

/**
 * H-17 item 5: the Preferences dialog's open/closed state. A small, reusable dialog — later
 * tickets (H-21's Recording page) add their own section to `PreferencesDialog.svelte` rather than
 * building a separate dialog; this store only owns whether it's shown.
 */

let open = $state(false);

/** Read-only accessor for the dialog. */
export function preferencesState(): { readonly open: boolean } {
  return {
    get open() {
      return open;
    },
  };
}

/** Edit → Preferences… (File → Preferences… on macOS, `menu/mnemonic` picks the menu). */
export function openPreferences(): void {
  open = true;
  if (!settingsState().current) {
    void loadSettings();
  }
}

export function closePreferences(): void {
  open = false;
}

/** Test/teardown helper. */
export function resetPreferencesForTest(): void {
  open = false;
}
