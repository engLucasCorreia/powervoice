<script lang="ts">
  import { documentState, hasDocument } from "../document/document.svelte";
  import { t } from "../i18n";
  import { shortcutLabelForAction } from "../keymap/shortcutLabel";
  import { EmptyState, IconButton, PanelHeader } from "../ui";
  import {
    addMarker,
    deleteSelectedMarker,
    jumpToMarker,
    markersState,
    renameMarker,
    selectMarker,
  } from "../markers/markers.svelte";
  import { formatTime } from "../transport/playhead";

  /**
   * Markers panel (S2-03, SPEC-009 §2.8 essential subset): a flat, position-sorted list — click
   * a row to select and jump, double-click (or the rename input's Enter) to rename, the header's
   * + adds at the heard position/cursor/selection (same as M), Delete removes the selection.
   * Sorting/filtering, multi-select, virtualization and marker `kind` are deferred (ticket "Out"
   * list) — this panel is not built for 10 000 markers yet.
   */

  const doc = documentState();
  const markers = markersState();
  const isOpen = $derived(hasDocument(doc.current));
  const rateHz = $derived(doc.current.sample_rate_hz);

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
    jumpToMarker(id);
  }

  /** A "focus this element on mount" action (the rename input, so typing starts immediately). */
  function autofocus(node: HTMLElement): void {
    node.focus();
  }
</script>

<aside class="markers-properties" data-testid="markers-properties" data-tour="markers">
  <section class="markers-panel">
    <PanelHeader title={t("panel.markers.title")}>
      {#snippet actions()}
        {#if markers.list.length > 0}
          <span class="count" data-testid="markers-count">
            {t("markers.panel.count", { count: markers.list.length })}
          </span>
        {/if}
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
      {/snippet}
    </PanelHeader>
    {#if markers.list.length === 0}
      <EmptyState icon="marker" title={t("markers.panel.empty")} size="sm" level={3} testid="markers-empty" />
    {:else}
      <ul class="marker-list" data-testid="marker-list">
        {#each markers.list as marker (marker.id)}
          <li>
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
                <span class="marker-time">{formatTime(marker.pos_samples, rateHz)}</span>
                {#if marker.len_samples > 0}
                  <span class="marker-duration">
                    {formatTime(marker.len_samples, rateHz)}
                  </span>
                {/if}
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
</aside>

<style>
  /* H-25: the Markers panel — panel header with count and add/delete keys, 28 px rows, the
     selection in the accent tint, times in tabular tertiary text. */
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

  .count {
    margin-right: var(--pv-space-1);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }

  .marker-list {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 1px;
    min-height: 0;
    margin: 0;
    padding: var(--pv-space-1);
    overflow-y: auto;
    list-style: none;
  }

  .marker-row {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    width: 100%;
    height: var(--pv-control-h-md);
    padding: 0 var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
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
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .marker-time,
  .marker-duration {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  .marker-row.selected .marker-time,
  .marker-row.selected .marker-duration {
    color: inherit;
  }

  .marker-row.renaming,
  div.marker-row {
    padding: 0;
  }

  .rename-input {
    width: 100%;
    height: var(--pv-control-h-md);
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
