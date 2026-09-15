<script lang="ts">
  import { open as openFileDialog, save as saveFileDialog } from "@tauri-apps/plugin-dialog";
  import { t } from "../i18n";
  import type { PresetEntryDto } from "../ipc/bindings";
  import { Button, Dialog, EmptyState, Select, Tabs, type DialogAction, type TabItem } from "../ui";
  import { localized } from "./localized";
  import {
    closeManagePresets,
    managePresetsState,
    setManagePresetsModule,
    setManagePresetsTab,
    type ManagePresetsTab,
  } from "./managePresets.svelte";
  import {
    deleteModulePreset,
    deleteRackPreset,
    exportModulePreset,
    exportRackPreset,
    importModulePreset,
    importRackPreset,
    listModulePresets,
    listRackPresets,
    rackState,
    renameModulePreset,
    renameRackPreset,
    tryImportModulePreset,
    tryImportRackPreset,
  } from "./rack.svelte";

  /**
   * Manage Presets… (H-22, SPEC-012 §2.7 follow-up): reached from a rack slot's Presets ▸
   * submenu or Effects ▸ Rack Presets ▸ — a small dialog handles rename (also H-22's "via a small
   * name dialog from the menus": this dialog *is* the one small dialog every rename goes through)
   * and delete confirms destructively; factory presets are listed but carry no actions. Import/
   * export round-trip a `.json` file through the native file dialogs.
   */
  const st = managePresetsState();

  const JSON_FILTER = [{ name: t("manage_presets.file_filter"), extensions: ["json"] }];

  const modules = $derived(
    [...rackState().modules].sort((a, b) => localized(a.name).localeCompare(localized(b.name))),
  );
  const selectedModuleId = $derived(st.moduleId ?? modules[0]?.id ?? null);

  const tabs: TabItem<ManagePresetsTab>[] = [
    { id: "rack", label: t("manage_presets.tab_rack") },
    { id: "module", label: t("manage_presets.tab_module") },
  ];

  let entries = $state<PresetEntryDto[] | null>(null);
  let renaming = $state<{ name: string; value: string } | null>(null);
  let renameError = $state(false);
  let pendingDelete = $state<{ name: string } | null>(null);
  let importConflict = $state<{ path: string; name?: string } | null>(null);

  async function refresh(): Promise<void> {
    entries = null;
    if (st.tab === "rack") {
      entries = await listRackPresets();
    } else if (selectedModuleId) {
      entries = await listModulePresets(selectedModuleId);
    } else {
      entries = [];
    }
  }

  $effect(() => {
    // Re-list whenever the dialog opens, the tab changes, or (module tab) the module changes.
    // Every dependency is read unconditionally here so the effect reliably re-tracks them all,
    // rather than relying on which branch `refresh()`'s own body happens to take.
    const open = st.open;
    const tab = st.tab;
    const moduleId = selectedModuleId;
    void tab;
    void moduleId;
    if (open) {
      void refresh();
    }
  });

  function selectTab(tab: ManagePresetsTab): void {
    renaming = null;
    pendingDelete = null;
    setManagePresetsTab(tab);
  }

  function selectModule(moduleId: string): void {
    renaming = null;
    pendingDelete = null;
    setManagePresetsModule(moduleId);
  }

  function startRename(entry: PresetEntryDto): void {
    renaming = { name: entry.key, value: localized(entry.name) };
    renameError = false;
  }

  async function confirmRename(): Promise<void> {
    if (!renaming) {
      return;
    }
    const newName = renaming.value.trim();
    if (!newName) {
      return;
    }
    const result =
      st.tab === "rack"
        ? await renameRackPreset(renaming.name, newName)
        : selectedModuleId
          ? await renameModulePreset(selectedModuleId, renaming.name, newName)
          : null;
    if (result) {
      renaming = null;
      await refresh();
    } else {
      renameError = true;
    }
  }

  async function confirmDelete(): Promise<void> {
    if (!pendingDelete) {
      return;
    }
    const ok =
      st.tab === "rack"
        ? await deleteRackPreset(pendingDelete.name)
        : selectedModuleId
          ? await deleteModulePreset(selectedModuleId, pendingDelete.name)
          : false;
    pendingDelete = null;
    if (ok) {
      await refresh();
    }
  }

  async function exportEntry(entry: PresetEntryDto): Promise<void> {
    const path = await saveFileDialog({
      title: t("manage_presets.export_title"),
      defaultPath: `${entry.key}.json`,
      filters: JSON_FILTER,
    });
    if (!path) {
      return;
    }
    const ok =
      st.tab === "rack"
        ? await exportRackPreset(entry.key, path)
        : selectedModuleId
          ? await exportModulePreset(selectedModuleId, entry.key, path)
          : false;
    if (ok) {
      // The list itself doesn't change; nothing to refresh — export doesn't mutate the store.
    }
  }

  async function startImport(): Promise<void> {
    const picked = await openFileDialog({
      multiple: false,
      directory: false,
      title: t("manage_presets.import_title"),
      filters: JSON_FILTER,
    });
    if (typeof picked !== "string") {
      return;
    }
    if (st.tab === "rack") {
      const outcome = await tryImportRackPreset(picked);
      if (outcome.status === "conflict") {
        importConflict = { path: picked, name: outcome.name };
      } else if (outcome.status === "ok") {
        await refresh();
      }
    } else {
      const outcome = await tryImportModulePreset(picked);
      if (outcome.status === "conflict") {
        importConflict = { path: picked, name: outcome.name };
      } else if (outcome.status === "ok") {
        if (outcome.value.module_id !== selectedModuleId) {
          setManagePresetsModule(outcome.value.module_id);
        }
        await refresh();
      }
    }
  }

  async function confirmImportOverwrite(): Promise<void> {
    if (!importConflict) {
      return;
    }
    const { path } = importConflict;
    importConflict = null;
    if (st.tab === "rack") {
      if (await importRackPreset(path, true)) {
        await refresh();
      }
    } else {
      const imported = await importModulePreset(path, true);
      if (imported) {
        if (imported.module_id !== selectedModuleId) {
          setManagePresetsModule(imported.module_id);
        }
        await refresh();
      }
    }
  }

  const dialogActions: DialogAction[] = [
    { label: t("manage_presets.close"), role: "primary", testid: "manage-presets-close", onclick: closeManagePresets },
  ];

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape" && !renaming && !pendingDelete && !importConflict) {
      closeManagePresets();
    }
  }
</script>

{#snippet row(entry: PresetEntryDto)}
  <div class="row" data-testid="manage-presets-row" data-factory={entry.is_factory}>
    <span class="name">{localized(entry.name)}</span>
    {#if entry.is_factory}
      <span class="badge">{t("manage_presets.factory_badge")}</span>
    {:else}
      <div class="row-actions">
        <Button size="sm" testid="manage-presets-export-{entry.key}" onclick={() => void exportEntry(entry)}>
          {t("manage_presets.export")}
        </Button>
        <Button size="sm" testid="manage-presets-rename-{entry.key}" onclick={() => startRename(entry)}>
          {t("manage_presets.rename")}
        </Button>
        <Button
          size="sm"
          variant="danger"
          testid="manage-presets-delete-{entry.key}"
          onclick={() => (pendingDelete = { name: entry.key })}
        >
          {t("manage_presets.delete")}
        </Button>
      </div>
    {/if}
  </div>
{/snippet}

{#snippet list()}
  {#if entries === null}
    <p class="hint" data-testid="manage-presets-loading">{t("manage_presets.loading")}</p>
  {:else if entries.length === 0}
    <EmptyState size="sm" title={t("manage_presets.empty")} testid="manage-presets-empty" />
  {:else}
    <div class="list" role="list" data-testid="manage-presets-list">
      {#each entries as entry (entry.key)}
        {@render row(entry)}
      {/each}
    </div>
  {/if}
  <div class="import-row">
    <Button size="sm" testid="manage-presets-import" onclick={() => void startImport()}>
      {t("manage_presets.import")}
    </Button>
  </div>
{/snippet}

{#if st.open}
  <Dialog
    size="lg"
    title={t("manage_presets.title")}
    testid="manage-presets-dialog"
    onkeydown={onKeydown}
    actions={dialogActions}
  >
    <Tabs {tabs} selected={st.tab} label={t("manage_presets.title")} idPrefix="manage-presets" onchange={selectTab} />
    {#if st.tab === "module"}
      {#if modules.length === 0}
        <EmptyState size="sm" title={t("manage_presets.no_modules")} testid="manage-presets-no-modules" />
      {:else}
        <Select
          label={t("manage_presets.module_label")}
          layout="stacked"
          testid="manage-presets-module-select"
          options={modules.map((m) => ({ value: m.id, label: localized(m.name) }))}
          value={selectedModuleId ?? modules[0]!.id}
          onchange={selectModule}
        />
        {@render list()}
      {/if}
    {:else}
      {@render list()}
    {/if}
  </Dialog>
{/if}

{#if renaming}
  <Dialog
    size="sm"
    title={t("manage_presets.rename_title")}
    testid="manage-presets-rename-dialog"
    onkeydown={(e) => {
      e.stopPropagation();
      if (e.key === "Escape") renaming = null;
    }}
    actions={[
      { label: t("manage_presets.cancel"), role: "cancel", testid: "manage-presets-rename-cancel", onclick: () => (renaming = null) },
      { label: t("manage_presets.rename_confirm"), role: "primary", testid: "manage-presets-rename-confirm", onclick: () => void confirmRename() },
    ]}
  >
    <input
      type="text"
      aria-label={t("manage_presets.rename_label")}
      data-testid="manage-presets-rename-input"
      bind:value={renaming.value}
      onkeydown={(e) => {
        if (e.key === "Enter") void confirmRename();
      }}
      {@attach (node) => node.focus()}
    />
    {#if renameError}
      <p class="error" data-testid="manage-presets-rename-error">{t("manage_presets.rename_error")}</p>
    {/if}
  </Dialog>
{/if}

{#if pendingDelete}
  <Dialog
    size="sm"
    role="alertdialog"
    title={t("manage_presets.delete_title", { name: pendingDelete.name })}
    testid="manage-presets-delete-dialog"
    onkeydown={(e) => {
      e.stopPropagation();
      if (e.key === "Escape") pendingDelete = null;
    }}
    actions={[
      { label: t("manage_presets.cancel"), role: "cancel", testid: "manage-presets-delete-cancel", onclick: () => (pendingDelete = null) },
      {
        label: t("manage_presets.delete_confirm"),
        role: "primary",
        variant: "danger",
        testid: "manage-presets-delete-confirm",
        onclick: () => void confirmDelete(),
      },
    ]}
  >
    <p>{t("manage_presets.delete_body")}</p>
  </Dialog>
{/if}

{#if importConflict}
  <Dialog
    size="sm"
    role="alertdialog"
    title={t("manage_presets.overwrite_title", { name: importConflict.name ?? "" })}
    testid="manage-presets-import-conflict-dialog"
    onkeydown={(e) => {
      e.stopPropagation();
      if (e.key === "Escape") importConflict = null;
    }}
    actions={[
      { label: t("manage_presets.cancel"), role: "cancel", testid: "manage-presets-import-conflict-cancel", onclick: () => (importConflict = null) },
      {
        label: t("manage_presets.overwrite_confirm"),
        role: "primary",
        variant: "danger",
        testid: "manage-presets-import-conflict-confirm",
        onclick: () => void confirmImportOverwrite(),
      },
    ]}
  >
    <p>{t("manage_presets.overwrite_body", { name: importConflict.name ?? "" })}</p>
  </Dialog>
{/if}

<style>
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
  }

  .row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-height: var(--pv-control-h-md);
    padding: 0 var(--pv-space-2);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-bg-raised);
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--pv-text-primary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .badge {
    flex: none;
    padding: 0 var(--pv-space-1);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .row-actions {
    display: flex;
    flex: none;
    gap: var(--pv-space-1);
    padding: var(--pv-space-1) 0;
  }

  .import-row {
    display: flex;
    justify-content: flex-end;
    margin-top: var(--pv-space-2);
  }
</style>
