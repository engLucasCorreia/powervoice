<script lang="ts">
  import { t } from "../i18n";
  import type { PresetEntryDto, PresetRefDto } from "../ipc/bindings";
  import { dispatchAction } from "../shortcuts";
  import { shortcutLabelForAction } from "../shortcuts/shortcutLabel";
  import MenuBarMenu from "../menu/MenuBarMenu.svelte";
  import { closeAllMenus, focusMenuTrigger, MENU_MNEMONICS } from "../menu/menubar.svelte";
  import {
    canNormalize,
    FAVORITE_TARGETS_DB,
    normalizeFavorite,
    openNormalizeDialog,
  } from "../state/normalize.svelte";
  import {
    canNormalizeLufs,
    FAVORITE_TARGETS_LUFS,
    normalizeLufsFavorite,
    openNormalizeLufsDialog,
  } from "../state/normalizeLufs.svelte";
  import { openPluginManager, startInstall } from "../plugins/plugins.svelte";
  import { Button, formatWithUnit } from "../ui";
  import type { MenuEntry } from "../ui/menuModel";
  import { localized } from "./localized";
  import { canCapture } from "./nrCapture.svelte";
  import { canBake, startBake } from "../state/bake.svelte";
  import { openManagePresets } from "./managePresets.svelte";
  import {
    deleteRackPreset,
    listRackPresets,
    loadRackPreset,
    rackState,
    saveRackPreset,
    trySaveRackPreset,
  } from "./rack.svelte";

  /**
   * Effects menu (H-19): Normalize…, Normalize (LUFS)…, Capture Noise Print, Bake Rack (T-602:
   * renders the rack into the selection or the whole file, then resets it), Favorites ▸ (the six
   * one-click normalize presets) and Rack Presets ▸ (T-406: load, save the live rack, delete user
   * presets, confirm before replacing a non-empty rack). H-26: on the shared menu; the preset
   * name field and the replace confirmation are inline content of the submenu. T-809: Manage
   * Plugins… (the plugin manager) and Install Module… (native picker → copy + scan).
   */
  const captureEnabled = $derived(canCapture());
  const bakeEnabled = $derived(canBake());
  const normalizeEnabled = $derived(canNormalize());
  const normalizeLufsEnabled = $derived(canNormalizeLufs());

  function favoritesDbTestId(targetDb: number): string {
    return `menu-favorites-normalize-${Math.abs(targetDb).toFixed(1).replace(".", "-")}db`;
  }

  function favoritesLufsTestId(targetLufs: number): string {
    return `menu-favorites-normalize-lufs-${Math.abs(targetLufs).toFixed(0)}`;
  }

  // --- Rack Presets (T-406, SPEC-012 "the rack-preset menu") ------------------------------
  let rackPresetEntries = $state<PresetEntryDto[] | null>(null);
  let savingRackPreset = $state(false);
  let rackPresetName = $state("");
  let confirmingRackPreset = $state<PresetRefDto | null>(null);
  // H-22: a same-named user rack preset already exists — "Replace preset ‹name›?" before overwriting.
  let saveConflict = $state(false);

  function refOf(entry: PresetEntryDto): PresetRefDto {
    return entry.is_factory ? { kind: "factory", key: entry.key } : { kind: "user", name: entry.key };
  }

  function refToKey(ref: PresetRefDto): string {
    return ref.kind === "factory" ? `factory:${ref.key}` : `user:${ref.name}`;
  }

  function onRackPresetsOpen(): void {
    savingRackPreset = false;
    saveConflict = false;
    confirmingRackPreset = null;
    void refreshRackPresets();
  }

  async function refreshRackPresets(): Promise<void> {
    rackPresetEntries = await listRackPresets();
  }

  function startSaveRackPreset(): void {
    savingRackPreset = true;
    saveConflict = false;
    rackPresetName = "";
  }

  async function confirmSaveRackPreset(): Promise<void> {
    const name = rackPresetName.trim();
    if (!name) {
      return;
    }
    const outcome = await trySaveRackPreset(name);
    if (outcome.status === "ok") {
      savingRackPreset = false;
      saveConflict = false;
      rackPresetName = "";
      await refreshRackPresets();
    } else if (outcome.status === "conflict") {
      saveConflict = true;
    }
  }

  /** Replaces the existing rack preset once the user confirms "Replace preset ‹name›?". */
  async function confirmOverwriteRackPreset(): Promise<void> {
    const name = rackPresetName.trim();
    if (!name) {
      return;
    }
    const saved = await saveRackPreset(name, true);
    if (saved) {
      savingRackPreset = false;
      saveConflict = false;
      rackPresetName = "";
      await refreshRackPresets();
    }
  }

  async function applyRackPreset(ref: PresetRefDto): Promise<void> {
    confirmingRackPreset = null;
    closeAllMenus();
    focusMenuTrigger("effects");
    await loadRackPreset(ref);
  }

  function pickRackPreset(entry: PresetEntryDto): void {
    const ref = refOf(entry);
    if (rackState().state.slots.length > 0) {
      confirmingRackPreset = ref;
    } else {
      void applyRackPreset(ref);
    }
  }

  async function deleteRackPresetEntry(entry: PresetEntryDto): Promise<void> {
    if (await deleteRackPreset(entry.key)) {
      await refreshRackPresets();
    }
  }

  const rackPresetItems = $derived.by((): MenuEntry[] => {
    if (confirmingRackPreset) {
      return [{ kind: "custom", id: "confirm-replace", content: confirmReplace }];
    }
    const list: MenuEntry[] = [];
    if (rackPresetEntries === null) {
      list.push({ kind: "note", id: "loading", label: "…" });
    } else if (rackPresetEntries.length === 0 && !savingRackPreset) {
      list.push({ kind: "note", id: "none", label: t("rack_preset.none") });
    } else {
      for (const entry of rackPresetEntries) {
        list.push({
          kind: "item",
          id: refToKey(refOf(entry)),
          label: localized(entry.name),
          testid: `rack-preset-${entry.key}`,
          // Loading may first ask to replace the rack (inline) — the submenu stays open.
          keepOpen: true,
          onselect: () => pickRackPreset(entry),
          trailing: entry.is_factory
            ? undefined
            : {
                icon: "delete",
                label: t("rack_preset.delete"),
                testid: `rack-preset-delete-${entry.key}`,
                onselect: () => void deleteRackPresetEntry(entry),
              },
        });
      }
    }
    list.push({ kind: "separator", id: "sep-manage" });
    list.push({
      kind: "item",
      id: "manage",
      label: t("rack_preset.manage"),
      testid: "rack-preset-manage",
      onselect: () => openManagePresets({ tab: "rack" }),
    });
    list.push({ kind: "separator", id: "sep-save" });
    list.push(
      savingRackPreset
        ? { kind: "custom", id: "save-form", content: saveForm }
        : {
            kind: "item",
            id: "save-as",
            label: t("rack_preset.save_as"),
            testid: "rack-preset-save",
            keepOpen: true,
            onselect: startSaveRackPreset,
          },
    );
    return list;
  });

  const items = $derived<MenuEntry[]>([
    {
      kind: "item",
      id: "normalize",
      label: t("effects.normalize_dialog"),
      disabled: !normalizeEnabled,
      testid: "menu-normalize-dialog",
      onselect: openNormalizeDialog,
    },
    {
      kind: "item",
      id: "normalize-lufs",
      label: t("effects.normalize_lufs_dialog"),
      disabled: !normalizeLufsEnabled,
      testid: "menu-normalize-lufs-dialog",
      onselect: openNormalizeLufsDialog,
    },
    {
      kind: "item",
      id: "capture",
      label: t("module.noise_reduction.capture"),
      shortcut: shortcutLabelForAction("nr.capture_noise_print"),
      disabled: !captureEnabled,
      testid: "menu-capture-noise-print",
      onselect: () => dispatchAction("nr.capture_noise_print"),
    },
    {
      kind: "item",
      id: "bake-rack",
      label: t("effects.bake_rack"),
      disabled: !bakeEnabled,
      testid: "menu-bake-rack",
      onselect: () => void startBake(),
    },
    { kind: "separator", id: "sep-favorites" },
    {
      kind: "submenu",
      id: "favorites",
      label: t("favorites.menu"),
      testid: "menu-favorites",
      minWidth: 224,
      items: [
        { kind: "heading", id: "peak", label: t("toolbar.normalize.peak_heading") },
        ...FAVORITE_TARGETS_DB.map(
          (targetDb): MenuEntry => ({
            kind: "item",
            id: `db-${targetDb}`,
            label: t("favorites.normalize_peak", { target: formatWithUnit(targetDb, "", 1) }),
            disabled: !normalizeEnabled,
            testid: favoritesDbTestId(targetDb),
            onselect: () => void normalizeFavorite(targetDb),
          }),
        ),
        { kind: "separator", id: "sep-lufs" },
        { kind: "heading", id: "lufs", label: t("toolbar.normalize.lufs_heading") },
        ...FAVORITE_TARGETS_LUFS.map(
          (targetLufs): MenuEntry => ({
            kind: "item",
            id: `lufs-${targetLufs}`,
            label: t("favorites.normalize_lufs", { target: formatWithUnit(targetLufs, "", 1) }),
            disabled: !normalizeLufsEnabled,
            testid: favoritesLufsTestId(targetLufs),
            onselect: () => void normalizeLufsFavorite(targetLufs),
          }),
        ),
      ],
    },
    { kind: "separator", id: "sep-presets" },
    {
      kind: "submenu",
      id: "rack-presets",
      label: t("rack_preset.menu"),
      testid: "menu-rack-presets",
      menuTestid: "rack-presets-submenu",
      minWidth: 208,
      onopen: onRackPresetsOpen,
      items: rackPresetItems,
    },
    { kind: "separator", id: "sep-plugins" },
    {
      kind: "item",
      id: "manage-plugins",
      label: t("effects.manage_plugins"),
      icon: "plugin",
      testid: "menu-manage-plugins",
      onselect: () => openPluginManager(),
    },
    {
      kind: "item",
      id: "install-module",
      label: t("effects.install_module"),
      icon: "install",
      testid: "menu-install-module",
      onselect: () => void startInstall(),
    },
  ]);
</script>

{#snippet confirmReplace()}
  <p>{t("rack_preset.confirm_replace")}</p>
  <div class="actions">
    <Button size="sm" onclick={() => (confirmingRackPreset = null)}>
      {t("rack_preset.confirm_replace_cancel")}
    </Button>
    <Button
      size="sm"
      variant="primary"
      testid="rack-preset-confirm-replace"
      onclick={() => void applyRackPreset(confirmingRackPreset!)}
    >
      {t("rack_preset.confirm_replace_confirm")}
    </Button>
  </div>
{/snippet}

{#snippet saveForm()}
  {#if saveConflict}
    <p data-testid="rack-preset-overwrite-message">
      {t("rack_preset.confirm_overwrite", { name: rackPresetName.trim() })}
    </p>
    <div class="actions">
      <Button size="sm" testid="rack-preset-overwrite-cancel" onclick={() => (saveConflict = false)}>
        {t("rack_preset.cancel_button")}
      </Button>
      <Button
        size="sm"
        variant="primary"
        testid="rack-preset-overwrite-confirm"
        onclick={() => void confirmOverwriteRackPreset()}
      >
        {t("rack_preset.overwrite_button")}
      </Button>
    </div>
  {:else}
    <input
      type="text"
      placeholder={t("rack_preset.name_placeholder")}
      aria-label={t("rack_preset.name_placeholder")}
      data-testid="rack-preset-name"
      bind:value={rackPresetName}
      onkeydown={(e) => {
        if (e.key === "Enter") void confirmSaveRackPreset();
      }}
      {@attach (node) => node.focus()}
    />
    <div class="actions">
      <Button size="sm" onclick={() => (savingRackPreset = false)}>
        {t("rack_preset.cancel_button")}
      </Button>
      <Button size="sm" variant="primary" testid="rack-preset-save-confirm" onclick={() => void confirmSaveRackPreset()}>
        {t("rack_preset.save_button")}
      </Button>
    </div>
  {/if}
{/snippet}

<MenuBarMenu
  id="effects"
  label={t("menu.effects")}
  mnemonic={MENU_MNEMONICS.effects}
  {items}
  triggerTestid="menu-trigger-effects"
  menuTestid="effects-menu"
  minWidth={240}
/>
