<script lang="ts">
  import { t } from "../i18n";
  import MenuBarMenu from "../menu/MenuBarMenu.svelte";
  import { MENU_MNEMONICS } from "../menu/menu.svelte";
  import type { MenuEntry } from "../ui/menuModel";
  import { startTour } from "../tour/tour.svelte";
  import { TOUR_IDS, TOURS } from "../tour/tours";
  import { openAbout } from "./about.svelte";
  import { openShortcutsDialog } from "./shortcuts.svelte";

  /** Help menu (H-19; T-709 adds Take the Tour and Tours ▸): "About PowerVoice…", with the app version (SPEC-000-adjacent
   * `app_info` — the ticket's "About with version"). H-26: on the shared menu. */
  const items = $derived<MenuEntry[]>([
    // T-709: replay the Welcome tour, or any tour, at any time.
    { kind: "item", id: "tour", label: t("menu.help.tour"), testid: "menu-tour", onselect: () => startTour("welcome") },
    {
      kind: "submenu",
      id: "tours",
      label: t("menu.help.tours"),
      testid: "menu-tours",
      menuTestid: "menu-tours-list",
      items: TOUR_IDS.map(
        (id): MenuEntry => ({
          kind: "item",
          id,
          label: t(TOURS[id].nameKey),
          testid: `menu-tour-${id}`,
          onselect: () => startTour(id),
        }),
      ),
    },
    { kind: "separator", id: "sep-shortcuts" },
    {
      kind: "item",
      id: "shortcuts",
      label: t("menu.help.shortcuts"),
      testid: "menu-shortcuts",
      onselect: openShortcutsDialog,
    },
    { kind: "separator", id: "sep-about" },
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
