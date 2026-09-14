<script lang="ts">
  import { t } from "../i18n";
  import {
    clearRecentFiles,
    openRecentFile,
    recentFilesState,
    refreshRecentFiles,
    removeRecentFile,
  } from "./recentFiles.svelte";

  /**
   * File → Open Recent (T-306, SPEC-018 §2.12): a small dropdown listing the 10 most recent
   * files, most-recent first. A missing file is greyed with "(missing)" and a Remove action;
   * picking one runs the normal Open flow (unsaved-changes prompt included).
   */
  let open = $state(false);
  const recent = recentFilesState();

  function toggle(): void {
    open = !open;
    if (open) {
      void refreshRecentFiles();
    }
  }

  function close(): void {
    open = false;
  }

  async function pick(path: string, exists: boolean | null): Promise<void> {
    close();
    if (exists === false) {
      return;
    }
    await openRecentFile(path);
  }

  async function remove(path: string, event: MouseEvent): Promise<void> {
    event.stopPropagation();
    await removeRecentFile(path);
  }

  async function clear(): Promise<void> {
    close();
    await clearRecentFiles();
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      close();
    }
  }
</script>

<svelte:window onclick={close} />

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="recent-menu" onkeydown={onKeydown}>
  <button
    type="button"
    data-testid="menu-open-recent"
    onclick={(e) => {
      e.stopPropagation();
      toggle();
    }}
  >
    {t("menu.file.open_recent")}
  </button>
  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="dropdown" data-testid="open-recent-dropdown" onclick={(e) => e.stopPropagation()}>
      {#if recent.entries.length === 0}
        <div class="empty">{t("menu.file.no_document")}</div>
      {:else}
        {#each recent.entries as entry (entry.path)}
          <div class="entry" class:missing={entry.exists === false} data-testid="recent-entry">
            <button
              type="button"
              class="entry-button"
              data-testid="recent-entry-open"
              onclick={() => void pick(entry.path, entry.exists)}
            >
              <span class="name">{entry.name}</span>
              <span class="folder">{entry.folder}</span>
              {#if entry.exists === false}
                <span class="missing-label">{t("recent.missing")}</span>
              {/if}
            </button>
            <button
              type="button"
              class="remove"
              data-testid="recent-entry-remove"
              title={t("recent.remove")}
              onclick={(e) => void remove(entry.path, e)}
            >
              ×
            </button>
          </div>
        {/each}
        <hr />
        <button
          type="button"
          class="clear"
          data-testid="menu-clear-recent"
          onclick={() => void clear()}
        >
          {t("menu.file.clear_recent")}
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .recent-menu {
    position: relative;
    display: inline-block;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.2rem 0.6rem;
  }

  button:hover {
    border-color: var(--accent);
  }

  .dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: 100;
    display: flex;
    flex-direction: column;
    min-width: 18rem;
    max-width: 26rem;
    margin-top: 0.25rem;
    padding: 0.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);
  }

  .empty {
    padding: 0.35rem 0.5rem;
    color: var(--text-disabled);
  }

  .entry {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .entry-button {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
    overflow: hidden;
    border: none;
    background: none;
    text-align: left;
    padding: 0.3rem 0.4rem;
  }

  .entry.missing .name,
  .entry.missing .folder {
    color: var(--text-disabled);
  }

  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .folder {
    flex: 1;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-secondary);
    font-size: 0.85em;
  }

  .missing-label {
    color: var(--text-disabled);
    font-size: 0.85em;
  }

  .remove {
    border: none;
    background: none;
    padding: 0.2rem 0.4rem;
    color: var(--text-secondary);
  }

  hr {
    border: none;
    border-top: 1px solid var(--surface-border);
    margin: 0.25rem 0;
  }

  .clear {
    border: none;
    background: none;
    text-align: left;
    padding: 0.3rem 0.4rem;
  }

  .clear:disabled {
    color: var(--text-disabled);
  }
</style>
