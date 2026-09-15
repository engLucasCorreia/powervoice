<script lang="ts">
  import { hasDocument, documentState } from "../document/document.svelte";
  import { t, tDynamic } from "../i18n";
  import { dispatchAction } from "../shortcuts";
  import { isPlatformMac } from "../shortcuts/registry";
  import { shortcutLabelForAction } from "../shortcuts/shortcutLabel";
  import MenuBarMenu from "../menu/MenuBarMenu.svelte";
  import { MENU_MNEMONICS } from "../menu/menubar.svelte";
  import { markersState } from "../markers/markers.svelte";
  import { openPreferences } from "../preferences/preferences.svelte";
  import { recordState } from "../state/record.svelte";
  import { hasClipboard, editState, silence } from "../state/edit.svelte";
  import { hasSelection } from "../state/selection.svelte";
  import type { MenuEntry } from "../ui/menuModel";

  /**
   * Edit menu (H-19): Undo/Redo (with the history's i18n label), Cut/Copy/Paste/Delete/Trim/
   * Silence, Select All, Markers ▸. Every item with a keymap binding dispatches through the same
   * `dispatchAction` path the shortcut itself uses (Silence and the normalize favorites have no
   * default binding — menu only, `bindings.ts`). H-26: on the shared menu.
   *
   * H-17: Preferences… lives here on non-macOS platforms (File → Preferences… on macOS,
   * `DocumentMenu.svelte`) — there's no native app menu to put it in instead.
   */
  const showPreferencesHere = !isPlatformMac();
  const doc = documentState();
  const edit = editState();
  const rec = recordState();
  const markers = markersState();

  const recording = $derived(rec.state.recording);
  const selected = $derived(hasSelection() && !recording);
  const pasteEnabled = $derived(hasClipboard() && !recording);
  const hasDoc = $derived(hasDocument(doc.current));

  /** T-301 (ADR-004 Amendment 3): a label's placeholder values ("Normalize to {target} dB"). */
  function labelParams(params: Partial<Record<string, string>>): Record<string, string> {
    return Object.fromEntries(
      Object.entries(params).filter((e): e is [string, string] => e[1] !== undefined),
    );
  }

  const undoLabel = $derived(
    edit.history.undo_label
      ? t("menu.edit.undo", {
          label: tDynamic(edit.history.undo_label, labelParams(edit.history.undo_label_params)),
        })
      : t("menu.edit.undo_none"),
  );
  const redoLabel = $derived(
    edit.history.redo_label
      ? t("menu.edit.redo", {
          label: tDynamic(edit.history.redo_label, labelParams(edit.history.redo_label_params)),
        })
      : t("menu.edit.redo_none"),
  );

  /** An item that runs a keymap action (and shows its shortcut). */
  function action(
    id: string,
    label: string,
    actionId: Parameters<typeof dispatchAction>[0],
    disabled: boolean,
    withShortcut = true,
  ): MenuEntry {
    return {
      kind: "item",
      id,
      label,
      shortcut: withShortcut ? shortcutLabelForAction(actionId) : undefined,
      disabled,
      testid: `menu-${id}`,
      onselect: () => dispatchAction(actionId),
    };
  }

  const items = $derived<MenuEntry[]>([
    action("undo", undoLabel, "history.undo", !edit.history.can_undo || recording),
    action("redo", redoLabel, "history.redo", !edit.history.can_redo || recording),
    { kind: "separator", id: "sep-clipboard" },
    action("cut", t("edit.cut"), "edit.cut", !selected),
    action("copy", t("edit.copy"), "edit.copy", !selected),
    action("paste", t("edit.paste"), "edit.paste", !pasteEnabled),
    action("delete", t("edit.delete"), "edit.delete", !selected),
    action("trim", t("edit.trim"), "edit.trim", !selected),
    {
      kind: "item",
      id: "silence",
      label: t("edit.silence"),
      disabled: !selected,
      testid: "menu-silence",
      onselect: () => void silence(),
    },
    { kind: "separator", id: "sep-select" },
    action("select-all", t("menu.edit.select_all"), "waveform.select_all", !hasDoc || recording),
    { kind: "separator", id: "sep-markers" },
    {
      kind: "submenu",
      id: "markers",
      label: t("menu.edit.markers"),
      testid: "menu-markers",
      minWidth: 224,
      items: [
        action("marker-add", t("menu.edit.marker_add"), "marker.add", !hasDoc),
        action(
          "marker-delete",
          t("menu.edit.marker_delete_selected"),
          "marker.delete_selected",
          markers.selectedId === null,
        ),
        action("marker-next", t("menu.edit.marker_next"), "marker.next", markers.list.length === 0),
        action("marker-prev", t("menu.edit.marker_prev"), "marker.prev", markers.list.length === 0),
      ],
    },
    ...(showPreferencesHere
      ? ([
          { kind: "separator", id: "sep-preferences" },
          {
            kind: "item",
            id: "preferences",
            label: t("menu.preferences"),
            testid: "menu-preferences",
            onselect: openPreferences,
          },
        ] satisfies MenuEntry[])
      : []),
  ]);
</script>

<MenuBarMenu
  id="edit"
  label={t("menu.edit")}
  mnemonic={MENU_MNEMONICS.edit}
  {items}
  triggerTestid="menu-trigger-edit"
  menuTestid="edit-menu"
  minWidth={256}
/>
