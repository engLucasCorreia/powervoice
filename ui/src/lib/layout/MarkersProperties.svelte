<script lang="ts">
  import { documentState, hasDocument } from "../document/document.svelte";
  import { t } from "../i18n";
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

<aside class="markers-properties" data-testid="markers-properties">
  <section class="markers-panel">
    <div class="panel-header">
      <h2>{t("panel.markers.title")}</h2>
      <div class="panel-actions">
        <button
          type="button"
          data-testid="markers-add"
          title={t("markers.panel.add_title")}
          disabled={!isOpen}
          onclick={() => void addMarker()}
        >
          {t("markers.panel.add")}
        </button>
        <button
          type="button"
          data-testid="markers-delete"
          title={t("markers.panel.delete_title")}
          disabled={markers.selectedId === null}
          onclick={() => void deleteSelectedMarker()}
        >
          {t("markers.panel.delete")}
        </button>
      </div>
    </div>
    {#if markers.list.length === 0}
      <p class="empty" data-testid="markers-empty">{t("markers.panel.empty")}</p>
    {:else}
      <span class="count" data-testid="markers-count">
        {t("markers.panel.count", { count: markers.list.length })}
      </span>
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
  <h2>{t("panel.properties.title")}</h2>
</aside>

<style>
  .markers-properties {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    background: var(--surface-panel);
    border-right: 1px solid var(--surface-border);
    padding: 0.75rem;
    overflow-y: auto;
  }

  .markers-panel {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    min-height: 0;
  }

  h2 {
    font-size: 0.8rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-secondary);
    margin: 0;
  }

  .panel-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.5rem;
  }

  .panel-actions {
    display: flex;
    gap: 0.25rem;
  }

  .panel-actions button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.1rem 0.4rem;
    font-size: 0.75rem;
  }

  .panel-actions button:disabled {
    color: var(--text-disabled);
  }

  .count {
    font-size: 0.7rem;
    color: var(--text-secondary);
  }

  .empty {
    font-size: 0.8rem;
    color: var(--text-secondary);
    margin: 0;
  }

  .marker-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
    overflow-y: auto;
  }

  .marker-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    background: transparent;
    color: var(--text-primary);
    border: none;
    border-radius: 3px;
    padding: 0.2rem 0.3rem;
    text-align: left;
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
  }

  .marker-row:hover {
    background: var(--surface-panel-raised);
  }

  .marker-row.selected {
    background: var(--accent);
    color: var(--surface-panel);
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
    color: inherit;
    opacity: 0.8;
  }

  .marker-row.renaming,
  div.marker-row {
    padding: 0;
  }

  .rename-input {
    width: 100%;
    box-sizing: border-box;
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--accent);
    border-radius: 3px;
    padding: 0.2rem 0.3rem;
    font-size: 0.75rem;
  }
</style>
