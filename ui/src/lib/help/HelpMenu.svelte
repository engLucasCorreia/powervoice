<script lang="ts">
  import { t } from "../i18n";
  import MenuBarMenu from "../menu/MenuBarMenu.svelte";
  import { MENU_MNEMONICS } from "../menu/menubar.svelte";
  import type { MenuEntry } from "../ui/menuModel";
  import { openAbout } from "./about.svelte";

  /** Help menu (H-19): a single "About PowerVoice…" item, with the app version (SPEC-000-adjacent
   * `app_info` — the ticket's "About with version"). H-26: on the shared menu. */
  const items = $derived<MenuEntry[]>([
    { kind: "item", id: "about", label: t("menu.help.about"), testid: "menu-about", onselect: openAbout },
  ]);
</script>

<MenuBarMenu
  id="help"
  label={t("menu.help")}
  mnemonic={MENU_MNEMONICS.help}
  {items}
  triggerTestid="menu-trigger-help"
  menuTestid="help-menu"
/>
