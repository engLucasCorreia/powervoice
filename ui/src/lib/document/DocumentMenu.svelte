<script lang="ts">
  import type { DefaultFormatDto } from "../ipc/bindings";
  import { openExportDialog } from "../export/export.svelte";
  import { t } from "../i18n";
  import { dispatchAction } from "../shortcuts";
  import { isPlatformMac } from "../shortcuts/registry";
  import { shortcutLabelForAction } from "../shortcuts/shortcutLabel";
  import MenuBarMenu from "../menu/MenuBarMenu.svelte";
  import { closeAllMenus, MENU_MNEMONICS } from "../menu/menubar.svelte";
  import { openPreferences } from "../preferences/preferences.svelte";
  import { openRecoveryStorage } from "../recovery/recovery.svelte";
  import { openNewRecordingPrompt, recordState } from "../state/record.svelte";
  import { settingsState } from "../state/settings.svelte";
  import type { MenuEntry } from "../ui/menuModel";
  import {
    clearRecentFiles,
    pickRecentFile,
    recentFilesState,
    refreshRecentFiles,
  } from "./recentFiles.svelte";
  import { displayName, documentState, hasDocument, requestClose } from "./document.svelte";

  /**
   * File menu (H-19): New Recording…, Open…, Recent Files ▸, Save, Save As…, Export…,
   * Recovery & Storage…, Close. H-26: on the shared menu (`MenuBarMenu` + `Menu`).
   *
   * H-17: Preferences… lives here on macOS (there's no native app menu to put it in instead);
   * everywhere else it's in `EditMenu.svelte`.
   */
  const showPreferencesHere = isPlatformMac();
  const doc = documentState();
  const rec = recordState();
  const recent = recentFilesState();
  const name = $derived(displayName(doc.current));
  const label = $derived(name ? `${name}${doc.current.dirty ? " *" : ""}` : t("menu.file.no_document"));
  // Factory default (SPEC-002 §3) — used only if settings haven't loaded yet.
  const FALLBACK_FORMAT: DefaultFormatDto = { sample_rate_hz: 48_000, bit_depth: "24" };
  const hasDoc = $derived(hasDocument(doc.current));

  function openExport(): void {
    const base = doc.current.name?.replace(/\.[^./\\]+$/, "") ?? "untitled";
    openExportDialog(base);
  }

  function openNewRecording(): void {
    openNewRecordingPrompt(settingsState().current?.default_format ?? FALLBACK_FORMAT);
  }

  /**
   * H-15 (SPEC-018 §2.12): a missing entry shows the dedicated dialog (Locate…/Remove from
   * List/Cancel) — its own modal (`RecentMissingDialog`, mounted in `App.svelte`).
   */
  async function pickRecent(path: string, exists: boolean | null): Promise<void> {
    closeAllMenus();
    await pickRecentFile(path, exists);
  }

  const recentItems = $derived<MenuEntry[]>(
    recent.entries.length === 0
      ? [{ kind: "note", id: "empty", label: t("menu.file.no_recent") }]
      : [
          ...recent.entries.map(
            (entry): MenuEntry => ({
              kind: "item",
              id: entry.path,
              label: entry.exists === false ? `${entry.name} ${t("recent.missing")}` : entry.name,
              title: entry.path,
              muted: entry.exists === false,
              testid: "recent-entry-open",
              onselect: () => void pickRecent(entry.path, entry.exists),
            }),
          ),
          { kind: "separator", id: "sep-clear" },
          {
            kind: "item",
            id: "clear",
            label: t("menu.file.clear_recent"),
            testid: "menu-clear-recent",
            onselect: () => void clearRecentFiles(),
          },
        ],
  );

  const items = $derived<MenuEntry[]>([
    {
      kind: "item",
      id: "new-recording",
      label: t("menu.file.new_recording"),
      disabled: rec.state.recording || rec.state.finishing,
      testid: "menu-new-recording",
      onselect: openNewRecording,
    },
    {
      kind: "item",
      id: "open",
      label: t("menu.file.open"),
      shortcut: shortcutLabelForAction("file.open"),
      testid: "menu-open",
      onselect: () => dispatchAction("file.open"),
    },
    {
      kind: "submenu",
      id: "open-recent",
      label: t("menu.file.open_recent"),
      testid: "menu-open-recent",
      minWidth: 288,
      onopen: () => void refreshRecentFiles(),
      items: recentItems,
    },
    { kind: "separator", id: "sep-save" },
    {
      kind: "item",
      id: "save",
      label: t("menu.file.save"),
      shortcut: shortcutLabelForAction("file.save"),
      disabled: !hasDoc,
      testid: "menu-save",
      onselect: () => dispatchAction("file.save"),
    },
    {
      kind: "item",
      id: "save-as",
      label: t("menu.file.save_as"),
      shortcut: shortcutLabelForAction("file.save_as"),
      disabled: !hasDoc,
      testid: "menu-save-as",
      onselect: () => dispatchAction("file.save_as"),
    },
    {
      kind: "item",
      id: "export",
      label: t("menu.file.export"),
      disabled: !hasDoc,
      testid: "menu-export",
      onselect: openExport,
    },
    { kind: "separator", id: "sep-recovery" },
    {
      kind: "item",
      id: "recovery",
      label: t("menu.file.recovery"),
      testid: "menu-recovery",
      onselect: () => void openRecoveryStorage(),
    },
    ...(showPreferencesHere
      ? ([
          {
            kind: "item",
            id: "preferences",
            label: t("menu.preferences"),
            testid: "menu-preferences",
            onselect: openPreferences,
          },
        ] satisfies MenuEntry[])
      : []),
    { kind: "separator", id: "sep-close" },
    {
      kind: "item",
      id: "close",
      label: t("menu.file.close"),
      disabled: !hasDoc,
      testid: "menu-close",
      onselect: () => void requestClose(),
    },
  ]);
</script>

<MenuBarMenu
  id="file"
  label={t("menu.file")}
  mnemonic={MENU_MNEMONICS.file}
  {items}
  triggerTestid="menu-trigger-file"
  menuTestid="document-menu"
  minWidth={224}
/>
<span class="document-name" data-testid="document-name">{label}</span>

<style>
  /* H-25: the document's name is state, not a menu — centred in the menu bar like a window title. */
  .document-name {
    position: absolute;
    left: 50%;
    max-width: 40%;
    overflow: hidden;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
    transform: translateX(-50%);
    pointer-events: none;
  }
</style>
