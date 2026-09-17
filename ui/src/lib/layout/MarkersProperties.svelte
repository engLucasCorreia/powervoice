<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { documentState, hasDocument } from "../document/document.svelte";
  import { t, type MessageKey } from "../i18n";
  import type { MarkerDto } from "../ipc/bindings";
  import { registerAction } from "../shortcuts";
  import { shortcutLabelForAction } from "../shortcuts/shortcutLabel";
  import { EmptyState, IconButton, Menu, PanelHeader, Select } from "../ui";
  import type { MenuEntry } from "../ui/menuModel";
  import type { SelectOption } from "../ui/types";
  import {
    activateMarker,
    addMarker,
    deleteAllMarkers,
    deleteFilteredMarkers,
    deleteSelectedMarker,
    goToNextMarker,
    goToPreviousMarker,
    markersState,
    renameMarker,
    selectMarker,
    setMarkerFilterText,
    setMarkerSort,
    setMarkerTypeFilter,
  } from "../markers/markers.svelte";
  import {
    PANEL_OVERSCAN_ROWS,
    PANEL_ROW_PX,
    virtualRowRange,
    type MarkerSortColumn,
    type MarkerTypeFilter,
  } from "../markers/markerListView";
  import { timeRulerFormatState } from "../state/waveformView.svelte";
  import { formatDocumentTime } from "../waveform/timeFormat";

  /**
   * Markers panel (S2-03/H-64, SPEC-009 §2.8) — a virtualized, filterable, sortable list: click a
   * row **activates** it (H-57, §2.8: a region sets the time selection to its range; a point or
   * dropout clears the selection — both move the cursor/seek to `pos`), double-click (or the
   * rename input's Enter, or `/` on the single selection) to rename, the header's + adds at the
   * heard position/cursor/selection (same as M), Delete removes the selection. The panel menu
   * (⋯) holds Delete All/Filtered Markers and Go to Next/Previous Marker.
   *
   * Deferred (see the ticket report): per-cell typed Start/End/Duration editing (§2.5's panel
   * subset), multi-row selection (§2.8), and persisting the sort/type-filter to the sidecar's view
   * state (§2.8 — kept as plain UI state here instead, like the text filter already is).
   */

  const doc = documentState();
  const markers = markersState();
  const isOpen = $derived(hasDocument(doc.current));
  const rateHz = $derived(doc.current.sample_rate_hz);
  // T-206 (SPEC-006 §2.5): the marker list's times follow the same time_ruler_format as the
  // ruler, toolbar clock and selection readouts.
  const timeFormat = timeRulerFormatState();

  let renamingId = $state<number | null>(null);
  let renameValue = $state("");

  function startRename(id: number, currentName: string): void {
    renamingId = id;
    renameValue = currentName;
  }

  function commitRename(): void {
    const id = renamingId;
    if (id === null) {
      return;
    }
    const value = renameValue;
    renamingId = null;
    void renameMarker(id, value);
  }

  function cancelRename(): void {
    renamingId = null;
  }

  function onRenameKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      commitRename();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancelRename();
    }
  }

  function onRowClick(id: number): void {
    selectMarker(id);
    activateMarker(id);
  }

  /** SPEC-009 §2.1: the Type a marker shows — Dropout wins over Region/Point regardless of
   * `len_samples` (dropout markers are points in v1, but the rule is written this way in the
   * spec so a future non-point dropout still reads "Dropout"). */
  function markerTypeKey(
    marker: MarkerDto,
  ): "markers.panel.type_dropout" | "markers.panel.type_region" | "markers.panel.type_point" {
    if (marker.kind === "dropout") {
      return "markers.panel.type_dropout";
    }
    return marker.len_samples > 0 ? "markers.panel.type_region" : "markers.panel.type_point";
  }

  /** A "focus this element on mount" action (the rename input, so typing starts immediately). */
  function autofocus(node: HTMLElement): void {
    node.focus();
  }

  // --- H-64 (SPEC-009 §2.8): text/type filter -----------------------------------------------

  const TYPE_FILTER_OPTIONS = $derived<SelectOption<MarkerTypeFilter>[]>([
    { value: "all", label: t("markers.panel.filter_all") },
    { value: "points", label: t("markers.panel.filter_points") },
    { value: "regions", label: t("markers.panel.filter_regions") },
    { value: "dropouts", label: t("markers.panel.filter_dropouts") },
  ]);

  function onFilterTextInput(event: Event & { currentTarget: HTMLInputElement }): void {
    setMarkerFilterText(event.currentTarget.value);
  }

  function onTypeFilterChange(value: MarkerTypeFilter): void {
    setMarkerTypeFilter(value);
  }

  const visibleMarkers = $derived(markers.visibleList);
  const countLabel = $derived(
    markers.list.length === 0
      ? ""
      : markers.isFiltered
        ? t("markers.panel.count_filtered", { shown: visibleMarkers.length, total: markers.list.length })
        : t("markers.panel.count", { count: markers.list.length }),
  );

  // --- H-64 (SPEC-009 §2.8): sortable column headers ----------------------------------------

  const SORT_COLUMNS: Array<{ column: MarkerSortColumn; labelKey: MessageKey }> = [
    { column: "name", labelKey: "markers.panel.name_header" },
    { column: "start", labelKey: "markers.panel.start_header" },
    { column: "end", labelKey: "markers.panel.end_header" },
    { column: "duration", labelKey: "markers.panel.duration_header" },
    { column: "type", labelKey: "markers.panel.type_header" },
  ];

  function ariaSortFor(column: MarkerSortColumn): "ascending" | "descending" | "none" {
    if (markers.sortColumn !== column) {
      return "none";
    }
    return markers.sortDirection === "asc" ? "ascending" : "descending";
  }

  function sortIndicator(column: MarkerSortColumn): string {
    if (markers.sortColumn !== column) {
      return "";
    }
    return markers.sortDirection === "asc" ? "▲" : "▼";
  }

  // --- H-64 (SPEC-009 §2.6): the panel menu (Delete All/Filtered, Go to Next/Previous) ------

  let menuOpen = $state(false);
  let menuTrigger: HTMLButtonElement | undefined = $state();

  const panelMenuItems = $derived<MenuEntry[]>([
    {
      kind: "item",
      id: "delete-all",
      label: t("markers.panel.menu_delete_all"),
      shortcut: shortcutLabelForAction("marker.delete_all"),
      disabled: markers.list.length === 0,
      testid: "markers-menu-delete-all",
      onselect: () => void deleteAllMarkers(),
    },
    ...(markers.isFiltered
      ? ([
          {
            kind: "item",
            id: "delete-filtered",
            label: t("markers.panel.menu_delete_filtered", { count: visibleMarkers.length }),
            disabled: visibleMarkers.length === 0,
            testid: "markers-menu-delete-filtered",
            onselect: () => void deleteFilteredMarkers(),
          },
        ] satisfies MenuEntry[])
      : []),
    { kind: "separator", id: "sep-nav" },
    {
      kind: "item",
      id: "go-next",
      label: t("markers.panel.menu_go_next"),
      shortcut: shortcutLabelForAction("marker.next"),
      disabled: markers.list.length === 0,
      testid: "markers-menu-go-next",
      onselect: () => goToNextMarker(),
    },
    {
      kind: "item",
      id: "go-prev",
      label: t("markers.panel.menu_go_prev"),
      shortcut: shortcutLabelForAction("marker.prev"),
      disabled: markers.list.length === 0,
      testid: "markers-menu-go-prev",
      onselect: () => goToPreviousMarker(),
    },
  ]);

  // --- H-64 (SPEC-009 §2.8/AC-15): virtualized list -----------------------------------------

  let scrollTop = $state(0);
  let viewportHeightPx = $state(0);
  let listEl: HTMLDivElement | undefined = $state();

  function onListScroll(event: Event & { currentTarget: HTMLElement }): void {
    scrollTop = event.currentTarget.scrollTop;
  }

  // A plain, guarded `ResizeObserver` (not `bind:clientHeight`, which uses one internally with no
  // guard — jsdom/Vitest has no global `ResizeObserver`, matching `WaveformView.svelte`'s own
  // convention). The synchronous `clientHeight` read makes the initial height available
  // immediately (and testable by stubbing `HTMLElement.prototype.clientHeight`, `WaveformView.
  // test.ts`'s `stubWidth` pattern) even where the observer itself never fires.
  $effect(() => {
    const el = listEl;
    if (!el) {
      viewportHeightPx = 0;
      return;
    }
    untrack(() => {
      viewportHeightPx = el.clientHeight;
    });
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        viewportHeightPx = Math.max(0, Math.round(entry.contentRect.height));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  const rowRange = $derived(
    virtualRowRange(scrollTop, viewportHeightPx, PANEL_ROW_PX, PANEL_OVERSCAN_ROWS, visibleMarkers.length),
  );
  const renderedMarkers = $derived(
    visibleMarkers.slice(rowRange.start, rowRange.end).map((marker, i) => ({ marker, row: rowRange.start + i })),
  );
  const totalHeightPx = $derived(visibleMarkers.length * PANEL_ROW_PX);

  onMount(() =>
    // H-64 (SPEC-009 §2.4): `/` opens the rename editor on the single panel selection. Registered
    // only while this component is mounted — the panel's own visibility (App.svelte renders it
    // conditionally on View → Markers) — so it's a no-op while the panel is hidden, and with 0
    // selected (a real state today; ≥2 never happens yet — single-selection panel).
    registerAction("marker.rename", () => {
      const id = markers.selectedId;
      if (id === null) {
        return;
      }
      const marker = markers.list.find((m) => m.id === id);
      if (marker) {
        startRename(marker.id, marker.name);
      }
    }),
  );
</script>

<aside class="markers-properties" data-testid="markers-properties" data-tour="markers">
  <section class="markers-panel">
    <PanelHeader title={t("panel.markers.title")} meta={countLabel || undefined}>
      {#snippet actions()}
        <IconButton
          icon="add"
          label={t("markers.panel.add_label")}
          shortcut={shortcutLabelForAction("marker.add")}
          size="sm"
          testid="markers-add"
          disabled={!isOpen}
          onclick={() => void addMarker()}
        />
        <IconButton
          icon="delete"
          label={t("markers.panel.delete_label")}
          shortcut={shortcutLabelForAction("marker.delete_selected")}
          size="sm"
          testid="markers-delete"
          disabled={markers.selectedId === null}
          onclick={() => void deleteSelectedMarker()}
        />
        <IconButton
          icon="moreVertical"
          label={t("markers.panel.menu_label")}
          size="sm"
          testid="markers-menu-trigger"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          bind:element={menuTrigger}
          onclick={() => (menuOpen = !menuOpen)}
        />
        <Menu
          open={menuOpen}
          anchor={menuTrigger}
          items={panelMenuItems}
          label={t("markers.panel.menu_label")}
          testid="markers-panel-menu"
          onclose={() => (menuOpen = false)}
        />
      {/snippet}
    </PanelHeader>
    <div class="markers-toolbar">
      <input
        class="filter-input"
        type="search"
        data-testid="markers-filter-text"
        placeholder={t("markers.panel.filter_placeholder")}
        aria-label={t("markers.panel.filter_label")}
        value={markers.filterText}
        oninput={onFilterTextInput}
      />
      <Select
        options={TYPE_FILTER_OPTIONS}
        value={markers.filterType}
        label={t("markers.panel.type_filter_label")}
        hideLabel
        size="sm"
        testid="markers-filter-type"
        onchange={onTypeFilterChange}
      />
    </div>
    {#if markers.list.length === 0}
      <EmptyState icon="marker" title={t("markers.panel.empty")} size="sm" level={3} testid="markers-empty" />
    {:else if visibleMarkers.length === 0}
      <EmptyState title={t("markers.panel.empty_filtered")} size="sm" level={3} testid="markers-empty-filtered" />
    {:else}
      <div class="marker-headers" role="row">
        {#each SORT_COLUMNS as { column, labelKey } (column)}
          <button
            type="button"
            class="sort-header"
            data-column={column}
            role="columnheader"
            aria-sort={ariaSortFor(column)}
            data-testid={`markers-sort-${column}`}
            onclick={() => setMarkerSort(column)}
          >
            {t(labelKey)}<span class="sort-indicator">{sortIndicator(column)}</span>
          </button>
        {/each}
      </div>
      <div
        class="marker-list"
        data-testid="marker-list"
        bind:this={listEl}
        onscroll={onListScroll}
      >
        <div class="marker-list-inner" style={`height: ${totalHeightPx}px`}>
          {#each renderedMarkers as { marker, row } (marker.id)}
            <div class="marker-row-wrap" style={`top: ${row * PANEL_ROW_PX}px; height: ${PANEL_ROW_PX}px`}>
              {#if renamingId === marker.id}
                <div class="marker-row renaming">
                  <input
                    class="rename-input"
                    data-testid={`marker-rename-${marker.id}`}
                    aria-label={t("markers.panel.rename_title")}
                    bind:value={renameValue}
                    onkeydown={onRenameKeydown}
                    onblur={commitRename}
                    use:autofocus
                  />
                </div>
              {:else}
                <button
                  type="button"
                  class="marker-row"
                  class:selected={markers.selectedId === marker.id}
                  data-testid={`marker-row-${marker.id}`}
                  title={t("markers.panel.rename_title")}
                  onclick={() => onRowClick(marker.id)}
                  ondblclick={() => startRename(marker.id, marker.name)}
                >
                  <span class="marker-name">{marker.name}</span>
                  <span class="marker-time">
                    {formatDocumentTime(marker.pos_samples, rateHz, timeFormat.current)}
                  </span>
                  <span class="marker-end">
                    {marker.len_samples > 0
                      ? formatDocumentTime(marker.pos_samples + marker.len_samples, rateHz, timeFormat.current)
                      : "—"}
                  </span>
                  <span class="marker-duration">
                    {marker.len_samples > 0
                      ? formatDocumentTime(marker.len_samples, rateHz, timeFormat.current)
                      : "—"}
                  </span>
                  <span class="marker-type">
                    <span
                      class="marker-type-dot"
                      data-kind={marker.kind === "dropout" ? "dropout" : marker.len_samples > 0 ? "region" : "point"}
                    ></span>
                    {t(markerTypeKey(marker))}
                  </span>
                </button>
              {/if}
            </div>
          {/each}
        </div>
      </div>
    {/if}
  </section>
</aside>

<style>
  /* H-25/H-64: the Markers panel — panel header with count and add/delete/menu keys, a
     filter/type-filter toolbar row, sortable column headers, and a virtualized (fixed 22 px row)
     list. */
  .markers-properties {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--pv-bg-panel);
    font-family: var(--pv-font-sans);
  }

  .markers-panel {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
  }

  .markers-toolbar {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-1);
    padding: var(--pv-space-1) var(--pv-space-2);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .filter-input {
    flex: 1;
    min-width: 0;
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border-default);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-field-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-xs);
  }

  .filter-input:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .marker-headers {
    display: grid;
    grid-template-columns: 1fr 4.5rem 4.5rem 3.75rem 4.25rem;
    flex: none;
    gap: 1px;
    padding: 0 var(--pv-space-1);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .sort-header {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-1);
    border: none;
    background: transparent;
    color: var(--pv-text-tertiary);
    font-family: inherit;
    font-size: var(--pv-text-xs);
    font-weight: var(--pv-weight-semibold);
    text-align: right;
    cursor: default;
  }

  .sort-header[data-column="name"] {
    justify-content: flex-start;
    text-align: left;
  }

  .sort-header:hover {
    color: var(--pv-text-primary);
  }

  .sort-header:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: -2px;
  }

  .sort-indicator {
    display: inline-block;
    width: 0.7em;
    font-size: 0.65em;
  }

  .marker-list {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: var(--pv-space-1);
    overflow-y: auto;
  }

  .marker-list-inner {
    position: relative;
    width: 100%;
  }

  .marker-row-wrap {
    position: absolute;
    left: 0;
    right: 0;
  }

  .marker-row {
    display: grid;
    grid-template-columns: 1fr 4.5rem 4.5rem 3.75rem 4.25rem;
    align-items: center;
    gap: 1px;
    width: 100%;
    height: 100%;
    padding: 0 var(--pv-space-1);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    text-align: left;
    cursor: default;
  }

  .marker-row:hover {
    background: var(--pv-control-bg-hover);
  }

  .marker-row.selected {
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
  }

  .marker-row:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: -2px;
  }

  .marker-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .marker-time,
  .marker-end,
  .marker-duration {
    overflow: hidden;
    color: var(--pv-text-tertiary);
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .marker-row.selected .marker-time,
  .marker-row.selected .marker-end,
  .marker-row.selected .marker-duration {
    color: inherit;
  }

  .marker-type {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--pv-space-1);
    overflow: hidden;
    color: var(--pv-text-tertiary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .marker-row.selected .marker-type {
    color: inherit;
  }

  /* SPEC-009 §2.8: the Type column's colour chip (`--wave-marker`/`--wave-marker-region`/
     `--wave-marker-dropout`, SPEC-006 §2.12). */
  .marker-type-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: var(--pv-radius-full);
    background: var(--wave-marker);
  }

  .marker-type-dot[data-kind="region"] {
    background: var(--wave-marker-region);
    outline: 1px solid var(--wave-marker);
    outline-offset: -1px;
  }

  .marker-type-dot[data-kind="dropout"] {
    background: var(--wave-marker-dropout);
  }

  .marker-row.renaming,
  div.marker-row {
    padding: 0;
  }

  .rename-input {
    width: 100%;
    height: 100%;
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-accent);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-field-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
  }

  .rename-input:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }
</style>
