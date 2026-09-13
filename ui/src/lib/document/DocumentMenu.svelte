<script lang="ts">
  import { openExportDialog } from "../export/export.svelte";
  import { t } from "../i18n";
  import {
    displayName,
    documentState,
    hasDocument,
    requestOpen,
    requestSave,
    requestSaveAs,
  } from "./document.svelte";

  /**
   * File menu / toolbar (S1-03: Open, Save, Save As with bit-depth choice; S4-04: Export…) plus
   * the current document's name with a `*` while modified (SPEC-004 §2.6; the window title
   * carries the same information, `document.svelte.ts`'s `titleFor`).
   */
  const doc = documentState();
  const name = $derived(displayName(doc.current));
  const label = $derived(name ? `${name}${doc.current.dirty ? " *" : ""}` : t("menu.file.no_document"));

  function openExport(): void {
    const base = doc.current.name?.replace(/\.[^./\\]+$/, "") ?? "untitled";
    openExportDialog(base);
  }
</script>

<div class="document-menu" data-testid="document-menu">
  <button type="button" data-testid="menu-open" onclick={() => void requestOpen()}>
    {t("menu.file.open")}
  </button>
  <button
    type="button"
    data-testid="menu-save"
    disabled={!hasDocument(doc.current)}
    onclick={() => void requestSave()}
  >
    {t("menu.file.save")}
  </button>
  <button
    type="button"
    data-testid="menu-save-as"
    disabled={!hasDocument(doc.current)}
    onclick={requestSaveAs}
  >
    {t("menu.file.save_as")}
  </button>
  <button
    type="button"
    data-testid="menu-export"
    disabled={!hasDocument(doc.current)}
    onclick={openExport}
  >
    {t("menu.file.export")}
  </button>
  <span class="document-name" data-testid="document-name">{label}</span>
</div>

<style>
  .document-menu {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.75rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
    font-size: 0.85em;
  }

  .document-name {
    margin-left: 0.5rem;
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  button {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.2rem 0.6rem;
  }

  button:hover:not(:disabled) {
    border-color: var(--accent);
  }

  button:disabled {
    color: var(--text-disabled);
  }
</style>
