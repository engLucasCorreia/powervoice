import type { ThemePref } from "../ipc/bindings";
import { saveSettings } from "../state/settings.svelte";
import { applyThemePref } from "./theme.svelte";

/**
 * The one way a user picks a theme (Preferences → Appearance, View → Theme ▸): apply it at once —
 * every surface and renderer repaints — and persist it in `Settings.theme`.
 */
export function chooseTheme(pref: ThemePref): void {
  applyThemePref(pref);
  void saveSettings({ theme: pref });
}
