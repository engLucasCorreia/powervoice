<script lang="ts">
  import { t, tDynamic } from "../i18n";
  import type { PluginEntryDto } from "../ipc/bindings";
  import {
    Badge,
    Button,
    Dialog,
    EmptyState,
    formatNumber,
    Icon,
    IconButton,
    Menu,
    currentPlatform,
    type TabItem,
  } from "../ui";
  import type { MenuEntry } from "../ui/menuModel";
  import Toggle from "../ui/Toggle.svelte";
  import PluginFolders from "./PluginFolders.svelte";
  import {
    canToggle,
    countPlugins,
    fileName,
    filterPlugins,
    formatLabel,
    isInInstallFolder,
    portsKey,
    rowKey,
    sortPlugins,
    statusInfo,
    type SortKey,
  } from "./pluginList";
  import {
    addPluginFolder,
    blockPlugin,
    clearPluginFlag,
    closePluginManager,
    pluginsState,
    refreshPlugins,
    requestUninstall,
    rescanPlugins,
    revealPlugin,
    setManagerTab,
    setPluginEnabled,
    setPluginQuery,
    sortPluginsBy,
    startInstall,
    unblockPlugin,
    type ManagerTab,
  } from "./plugins.svelte";
  import Tabs from "../ui/Tabs.svelte";
  import TourButton from "../tour/TourButton.svelte";

  /**
   * The plugin manager (T-809): Effects → Manage Plugins…, Preferences → Plugins, a flagged rack
   * slot, or "Show in plugin manager" after an install. Two tabs:
   * - **Plugins** — search, a sortable table (name + path, vendor + version, format badge,
   *   channels, parameters, status with its reason), a switch per row for Add module, a row menu
   *   (block/unblock, clear a crash warning, show in the file manager), Rescan ▾ (new and
   *   changed / everything) with live progress, and Install module…;
   * - **Folders** — the scanned folders and the user's own (add with the native picker, remove).
   */
  const ps = pluginsState();
  const platform = currentPlatform();

  function statusWords(entry: PluginEntryDto): string {
    const info = statusInfo(entry.status);
    return `${t(info.labelKey)} ${info.detailKey ? tDynamic(info.detailKey, info.detailParams) : ""}`;
  }

  const list = $derived(ps.list ?? []);
  const shown = $derived(sortPlugins(filterPlugins(list, ps.query, statusWords), ps.sortKey, ps.sortDir));
  const counts = $derived(countPlugins(list));
  const tabs = $derived<TabItem<ManagerTab>[]>([
    { id: "plugins", label: t("plugins.tab.plugins"), badge: ps.list ? String(counts.total) : undefined },
    { id: "folders", label: t("plugins.tab.folders") },
  ]);
  const countsLine = $derived(
    [
      counts.total === 1 ? t("plugins.count.one") : t("plugins.count.many", { count: counts.total }),
      counts.disabled > 0 ? t("plugins.count.disabled", { count: counts.disabled }) : null,
      counts.blocklisted > 0 ? t("plugins.count.blocklisted", { count: counts.blocklisted }) : null,
      counts.flagged > 0 ? t("plugins.count.flagged", { count: counts.flagged }) : null,
    ]
      .filter((part): part is string => part !== null)
      .join(" · "),
  );
  const revealLabel = $derived(
    platform === "mac"
      ? t("plugins.action.reveal_mac")
      : platform === "windows"
        ? t("plugins.action.reveal_windows")
        : t("plugins.action.reveal_linux"),
  );

  // --- Rescan ▾ ----------------------------------------------------------------------------
  let rescanOpen = $state(false);
  let rescanButton: HTMLButtonElement | undefined = $state();
  const rescanItems = $derived<MenuEntry[]>([
    {
      kind: "item",
      id: "quick",
      label: t("plugins.rescan.quick"),
      testid: "plugins-rescan-quick",
      onselect: () => void rescanPlugins(false),
    },
    {
      kind: "item",
      id: "full",
      label: t("plugins.rescan.full"),
      title: t("plugins.rescan.full_title"),
      testid: "plugins-rescan-full",
      onselect: () => void rescanPlugins(true),
    },
  ]);

  // --- Row menu (one menu, anchored to the row's ⋯) ------------------------------------------
  let rowMenu = $state<{ entry: PluginEntryDto; anchor: HTMLElement } | null>(null);
  const rowMenuItems = $derived.by((): MenuEntry[] => {
    const entry = rowMenu?.entry;
    if (!entry) {
      return [];
    }
    // "Uninstall…" (H-29) replaces "Block" for a file PowerVoice itself installed; a file found
    // anywhere else is never removed here, only blocked.
    const installed = isInInstallFolder(entry.path, ps.folders?.install ?? null);
    const items: MenuEntry[] = [];
    if (entry.status.kind === "blocklisted") {
      items.push({
        kind: "item",
        id: "unblock",
        label: t("plugins.action.unblock"),
        title: t("plugins.action.unblock_title"),
        testid: "plugin-action-unblock",
        onselect: () => void unblockPlugin(entry),
      });
    } else {
      if (entry.status.kind === "flagged") {
        items.push({
          kind: "item",
          id: "clear-flag",
          label: t("plugins.action.clear_flag"),
          testid: "plugin-action-clear-flag",
          onselect: () => void clearPluginFlag(entry),
        });
      }
      if (!installed) {
        items.push({
          kind: "item",
          id: "block",
          label: t("plugins.action.block"),
          title: t("plugins.action.block_title"),
          icon: "blocked",
          testid: "plugin-action-block",
          onselect: () => void blockPlugin(entry),
        });
      }
    }
    if (installed) {
      items.push({
        kind: "item",
        id: "uninstall",
        label: t("plugins.action.uninstall"),
        title: t("plugins.action.uninstall_title"),
        icon: "delete",
        testid: "plugin-action-uninstall",
        onselect: () => requestUninstall(entry),
      });
    }
    items.push(
      { kind: "separator", id: "sep-reveal" },
      {
        kind: "item",
        id: "reveal",
        label: revealLabel,
        icon: "reveal",
        testid: "plugin-action-reveal",
        onselect: () => void revealPlugin(entry),
      },
    );
    return items;
  });

  function openRowMenu(entry: PluginEntryDto, anchor: HTMLElement): void {
    rowMenu = rowMenu?.entry === entry ? null : { entry, anchor };
  }

  function displayName(entry: PluginEntryDto): string {
    return entry.name || fileName(entry.path);
  }

  // --- Focus a row (from a flagged rack slot, or after an install) ---------------------------
  let tableWrap: HTMLElement | undefined = $state();
  $effect(() => {
    const key = ps.focusKey;
    const count = shown.length;
    if (!key || !tableWrap || count === 0 || ps.tab !== "plugins") {
      return;
    }
    const row = [...tableWrap.querySelectorAll<HTMLElement>("[data-key]")].find((el) => el.dataset.key === key);
    row?.scrollIntoView?.({ block: "center" });
  });

  function ariaSort(key: SortKey): "ascending" | "descending" | "none" {
    if (ps.sortKey !== key) {
      return "none";
    }
    return ps.sortDir === "asc" ? "ascending" : "descending";
  }

  function onKeydown(event: KeyboardEvent): void {
    // Typing in the search field must never reach the editor's shortcuts (Space = play…).
    event.stopPropagation();
    if (event.key === "Escape" && !rowMenu && !rescanOpen) {
      closePluginManager();
    }
  }

  function onSearchKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape" && ps.query) {
      event.stopPropagation();
      setPluginQuery("");
    }
  }
</script>

{#snippet sortHeader(key: SortKey, label: string, className: string)}
  <th scope="col" class={className} aria-sort={ariaSort(key)}>
    <button type="button" class="sort" data-testid="plugins-sort-{key}" onclick={() => sortPluginsBy(key)}>
      <span>{label}</span>
      {#if ps.sortKey === key}
        <Icon name={ps.sortDir === "asc" ? "chevronUp" : "chevronDown"} size="sm" />
      {/if}
    </button>
  </th>
{/snippet}

{#snippet pluginsPanel()}
  <div class="toolbar" data-tour="plugins-toolbar">
    <label class="search">
      <Icon name="search" size="sm" />
      <input
        type="search"
        placeholder={t("plugins.search")}
        aria-label={t("plugins.search")}
        data-testid="plugins-search"
        value={ps.query}
        oninput={(e) => setPluginQuery(e.currentTarget.value)}
        onkeydown={onSearchKeydown}
      />
      {#if ps.query}
        <IconButton
          icon="close"
          size="sm"
          label={t("plugins.search_clear")}
          testid="plugins-search-clear"
          onclick={() => setPluginQuery("")}
        />
      {/if}
    </label>
    <span class="grow"></span>
    <Button
      icon="refresh"
      iconEnd="chevronDown"
      loading={ps.rescanning}
      testid="plugins-rescan"
      aria-haspopup="menu"
      aria-expanded={rescanOpen}
      bind:element={rescanButton}
      onclick={() => (rescanOpen = !rescanOpen)}
    >
      {t("plugins.rescan")}
    </Button>
    <Menu
      open={rescanOpen}
      anchor={rescanButton}
      items={rescanItems}
      label={t("plugins.rescan.menu")}
      testid="plugins-rescan-menu"
      placement="bottom-end"
      onclose={() => (rescanOpen = false)}
    />
    <Button
      variant="primary"
      icon="install"
      testid="plugins-install"
      data-tour="plugins-install"
      title={t("plugins.install_title")}
      onclick={() => void startInstall()}
    >
      {t("plugins.install")}
    </Button>
  </div>

  {#if ps.scan}
    <div class="scan" role="status" data-testid="plugins-scan-progress">
      {#if ps.scan.total > 0}
        <progress max={ps.scan.total} value={ps.scan.done} aria-label={t("plugins.scan.label")}></progress>
        <span class="scan-text">
          <span class="count" data-testid="plugins-scan-count">
            {t("plugins.scan.progress", { done: formatNumber(ps.scan.done, 0), total: formatNumber(ps.scan.total, 0) })}
          </span>
          {#if ps.scan.currentPath}
            <span class="file" title={ps.scan.currentPath}>{fileName(ps.scan.currentPath)}</span>
          {/if}
        </span>
      {:else}
        <progress aria-label={t("plugins.scan.label")}></progress>
        <span class="scan-text">{t("plugins.scan.starting")}</span>
      {/if}
    </div>
  {/if}

  <div class="table-wrap" bind:this={tableWrap}>
    {#if ps.list === null}
      {#if ps.error}
        <EmptyState
          size="sm"
          level={3}
          icon="error"
          title={t("plugins.error.title")}
          description={t("plugins.error.description")}
          testid="plugins-error"
        >
          {#snippet actions()}
            <Button testid="plugins-retry" onclick={() => void refreshPlugins()}>{t("plugins.retry")}</Button>
          {/snippet}
        </EmptyState>
      {:else}
        <div class="loading" role="status" data-testid="plugins-loading">
          <span class="spin"><Icon name="loading" size="sm" /></span>
          {t("plugins.loading")}
        </div>
      {/if}
    {:else if list.length === 0}
      <EmptyState
        size="sm"
        level={3}
        icon="plugin"
        title={t("plugins.empty.title")}
        description={t("plugins.empty.description")}
        testid="plugins-empty"
      >
        {#snippet actions()}
          <Button icon="install" onclick={() => void startInstall()}>{t("plugins.install")}</Button>
          <Button icon="folderAdd" onclick={() => void addPluginFolder()}>{t("plugins.folders.add")}</Button>
        {/snippet}
      </EmptyState>
    {:else if shown.length === 0}
      <EmptyState
        size="sm"
        level={3}
        icon="search"
        title={t("plugins.no_match.title", { query: ps.query })}
        testid="plugins-no-match"
      >
        {#snippet actions()}
          <Button onclick={() => setPluginQuery("")}>{t("plugins.search_clear")}</Button>
        {/snippet}
      </EmptyState>
    {:else}
      <table class="plugins" data-testid="plugins-table">
        <colgroup>
          <col />
          <col class="w-vendor" />
          <col class="w-format" />
          <col class="w-io" />
          <col class="w-params" />
          <col class="w-status" />
          <col class="w-enabled" />
          <col class="w-actions" />
        </colgroup>
        <thead>
          <tr>
            {@render sortHeader("name", t("plugins.col.name"), "")}
            {@render sortHeader("vendor", t("plugins.col.vendor"), "")}
            {@render sortHeader("format", t("plugins.col.format"), "")}
            <th scope="col">{t("plugins.col.io")}</th>
            <th scope="col" class="num">{t("plugins.col.params")}</th>
            {@render sortHeader("status", t("plugins.col.status"), "")}
            <th scope="col">{t("plugins.col.enabled")}</th>
            <th scope="col"><span class="visually-hidden">{t("plugins.col.actions")}</span></th>
          </tr>
        </thead>
        <tbody>
          {#each shown as entry (rowKey(entry))}
            {@const key = rowKey(entry)}
            {@const info = statusInfo(entry.status)}
            {@const ports = portsKey(entry.ports)}
            {@const name = displayName(entry)}
            <tr
              data-testid="plugin-row"
              data-key={key}
              data-status={entry.status.kind}
              class:focused={ps.focusKey === key}
              class:off={entry.status.kind === "disabled" || entry.status.kind === "blocklisted"}
            >
              <td>
                <div class="name" title={name} data-testid="plugin-name">{name}</div>
                <div class="path" title={entry.path} data-testid="plugin-path">{entry.path}</div>
              </td>
              <td>
                <div class="vendor">{entry.vendor || t("plugins.unknown")}</div>
                {#if entry.version}<div class="meta">{t("plugins.version", { version: entry.version })}</div>{/if}
              </td>
              <td><span class="format" data-testid="plugin-format">{formatLabel(entry.format)}</span></td>
              <td class="meta-cell" data-testid="plugin-io">
                {ports ? tDynamic(ports.key, ports.params) : t("plugins.unknown")}
              </td>
              <td class="num meta-cell" data-testid="plugin-params">
                {entry.ports ? formatNumber(entry.param_count, 0) : t("plugins.unknown")}
              </td>
              <td>
                <Badge tone={info.tone} icon={info.icon} testid="plugin-status">{t(info.labelKey)}</Badge>
                {#if info.detailKey}
                  {@const detail = tDynamic(info.detailKey, info.detailParams)}
                  <div class="meta" title={detail} data-testid="plugin-status-detail">{detail}</div>
                {/if}
              </td>
              <td>
                {#if canToggle(entry)}
                  <Toggle
                    size="sm"
                    hideLabel
                    label={t("plugins.enabled_for", { name })}
                    checked={entry.status.kind !== "disabled"}
                    disabled={ps.pending === key}
                    testid="plugin-enabled"
                    onchange={(on) => void setPluginEnabled(entry, on)}
                  />
                {:else if entry.status.kind === "blocklisted"}
                  <Button
                    size="sm"
                    testid="plugin-unblock"
                    title={t("plugins.action.unblock_title")}
                    disabled={ps.pending === key}
                    onclick={() => void unblockPlugin(entry)}
                  >
                    {t("plugins.action.unblock_short")}
                  </Button>
                {/if}
              </td>
              <td class="actions-cell">
                <IconButton
                  icon="more"
                  size="sm"
                  label={t("plugins.actions_for", { name })}
                  testid="plugin-actions"
                  aria-haspopup="menu"
                  aria-expanded={rowMenu?.entry === entry}
                  onclick={(e) => openRowMenu(entry, e.currentTarget as HTMLElement)}
                />
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </div>
  <Menu
    open={rowMenu !== null}
    anchor={rowMenu?.anchor}
    items={rowMenuItems}
    label={rowMenu ? t("plugins.actions_for", { name: displayName(rowMenu.entry) }) : ""}
    testid="plugin-row-menu"
    placement="bottom-end"
    onclose={() => (rowMenu = null)}
  />

  <div class="status-line">
    {#if ps.list !== null && list.length > 0}
      <span data-testid="plugins-counts">{countsLine}</span>
    {/if}
    {#if !ps.scan && ps.lastScan}
      <span class="last-scan" data-testid="plugins-last-scan">
        {t("plugins.scan.done", {
          effects: formatNumber(ps.lastScan.effects, 0),
          new: formatNumber(ps.lastScan.newly_registered.length, 0),
          blocked: formatNumber(ps.lastScan.blocklisted_now, 0),
        })}
      </span>
    {/if}
  </div>
{/snippet}

{#if ps.open}
  <Dialog
    size="xl"
    title={t("plugins.title")}
    titleId="plugin-manager-title"
    testid="plugin-manager"
    onkeydown={onKeydown}
    actions={[{ label: t("plugins.close"), role: "primary", testid: "plugin-manager-close", onclick: closePluginManager }]}
  >
    {#snippet headerActions()}
      <TourButton tour="plugins" size="md" />
    {/snippet}
    <div class="manager">
      <div class="tabs-anchor" data-tour="plugins-tabs">
        <Tabs
          {tabs}
          selected={ps.tab}
          label={t("plugins.tabs")}
          idPrefix="plugin-manager"
          testid="plugin-manager-tabs"
          onchange={setManagerTab}
        />
      </div>
      <div
        class="panel"
        data-tour="plugins-list"
        role="tabpanel"
        id="plugin-manager-panel-{ps.tab}"
        aria-labelledby="plugin-manager-tab-{ps.tab}"
        data-testid="plugin-manager-panel-{ps.tab}"
      >
        {#if ps.tab === "plugins"}
          {@render pluginsPanel()}
        {:else}
          <PluginFolders />
        {/if}
      </div>
    </div>
  </Dialog>
{/if}

<style>
  .manager {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    /* The tab strip runs edge to edge under the title, like a panel header. */
    margin-inline: calc(-1 * var(--pv-space-5));
  }

  .manager :global(.pv-tabs),
  .tabs-anchor {
    flex: none;
  }

  .panel {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--pv-space-3);
    min-height: 0;
    padding: var(--pv-space-3) var(--pv-space-5) 0;
    overflow-y: auto;
  }

  .toolbar {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2);
  }

  .grow {
    flex: 1;
  }

  .search {
    display: flex;
    flex: 1 1 15rem;
    align-items: center;
    gap: var(--pv-space-2);
    max-width: 22rem;
    height: var(--pv-control-h-md);
    padding-inline: var(--pv-control-px-sm) var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-md);
    background: var(--pv-field-bg);
    color: var(--pv-text-tertiary);
  }

  .search:focus-within {
    border-color: var(--pv-accent);
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
    outline: none;
  }

  .search input:focus-visible {
    border: none;
    outline: none;
  }

  .search input::-webkit-search-cancel-button {
    display: none;
  }

  .search input::placeholder {
    color: var(--pv-text-tertiary);
  }

  .scan {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-3);
    min-height: var(--pv-hit-min);
  }

  .scan progress {
    flex: 0 0 12rem;
  }

  .scan-text {
    display: flex;
    min-width: 0;
    gap: var(--pv-space-2);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .scan-text .file {
    overflow: hidden;
    color: var(--pv-text-tertiary);
    font-family: var(--pv-font-mono);
    font-size: var(--pv-text-xs);
    text-overflow: ellipsis;
  }

  /* The list is a recessed well; its header sticks while rows scroll. */
  .table-wrap {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    overflow: auto;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-panel);
  }

  .plugins {
    width: 100%;
    border-collapse: collapse;
    table-layout: fixed;
    font-size: var(--pv-text-md);
  }

  .w-vendor {
    width: 15%;
  }
  .w-format {
    width: 4.5rem;
  }
  .w-io {
    width: 6rem;
  }
  .w-params {
    width: 4.25rem;
  }
  .w-status {
    width: 10.5rem;
  }
  .w-enabled {
    width: 5.25rem;
  }
  .w-actions {
    width: 2.5rem;
  }

  thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    height: var(--pv-panel-header-h);
    padding: 0 var(--pv-space-2);
    border-bottom: var(--pv-border-width) solid var(--pv-border);
    background: var(--pv-bg-raised);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-weight: var(--pv-weight-semibold);
    text-align: left;
    white-space: nowrap;
  }

  th.num,
  td.num {
    text-align: right;
  }

  .sort {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
    height: var(--pv-hit-min);
    margin-left: calc(-1 * var(--pv-space-1));
    padding: 0 var(--pv-space-1);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: default;
  }

  .sort:hover {
    color: var(--pv-text-primary);
    background: var(--pv-control-bg-hover);
  }

  .sort:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  th[aria-sort="ascending"] .sort,
  th[aria-sort="descending"] .sort {
    color: var(--pv-text-primary);
  }

  tbody tr {
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
    transition: background-color var(--pv-duration-fast) var(--pv-ease-standard);
  }

  tbody tr:hover {
    background: var(--pv-bg-raised);
  }

  tbody tr.focused {
    background: var(--pv-accent-soft);
    box-shadow: inset 2px 0 0 var(--pv-accent);
  }

  td {
    padding: var(--pv-space-2);
    vertical-align: middle;
    overflow: hidden;
  }

  .name,
  .vendor {
    overflow: hidden;
    color: var(--pv-text-primary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .name {
    font-weight: var(--pv-weight-medium);
  }

  tr.off .name {
    color: var(--pv-text-secondary);
  }

  .path {
    overflow: hidden;
    margin-top: var(--pv-space-half);
    color: var(--pv-text-tertiary);
    font-family: var(--pv-font-mono);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    margin-top: var(--pv-space-half);
    overflow: hidden;
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    font-variant-numeric: tabular-nums;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta-cell {
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  /* Format: a quiet outlined chip, so CLAP / VST3 / LV2 / JSFX line up as a column of tags. */
  .format {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding-inline: var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border-strong);
    border-radius: var(--pv-radius-sm);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-xs);
    font-weight: var(--pv-weight-medium);
    line-height: 1;
  }

  .actions-cell {
    text-align: right;
  }

  .loading {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--pv-space-2);
    margin: auto;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }

  .spin {
    display: inline-flex;
    animation: pv-spin 1s linear infinite;
  }

  @keyframes pv-spin {
    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation: none;
    }
  }

  .status-line {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: var(--pv-space-1) var(--pv-space-4);
    min-height: var(--pv-leading-xs);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    font-variant-numeric: tabular-nums;
  }

  .visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
