<script lang="ts">
  import type { DefaultFormatDto } from "../ipc/bindings";
  import { openExportDialog } from "../export/export.svelte";
  import { t } from "../i18n";
  import { openNewRecordingPrompt, recordState } from "../state/record.svelte";
  import { settingsState } from "../state/settings.svelte";
  import {
    displayName,
    documentState,
    hasDocument,
    requestOpen,
    requestSave,
    requestSaveAs,
  } from "./document.svelte";
  import RecentFilesMenu from "./RecentFilesMenu.svelte";

  /**
   * File menu / toolbar (S1-03: Open, Save, Save As with bit-depth choice; S4-04: Export…;
   * H-06: New Recording…) plus the current document's name with a `*` while modified (SPEC-004
   * §2.6; the window title carries the same information, `document.svelte.ts`'s `titleFor`).
   */
  const doc = documentState();
  const rec = recordState();
  const name = $derived(displayName(doc.current));
  const label = $derived(name ? `${name}${doc.current.dirty ? " *" : ""}` : t("menu.file.no_document"));
  // Factory default (SPEC-002 §3) — used only if settings haven't loaded yet.
  const FALLBACK_FORMAT: DefaultFormatDto = { sample_rate_hz: 48_000, bit_depth: "24" };

  function openExport(): void {
    const base = doc.current.name?.replace(/\.[^./\\]+$/, "") ?? "untitled";
    openExportDialog(base);
  }

  function openNewRecording(): void {
    openNewRecordingPrompt(settingsState().current?.default_format ?? FALLBACK_FORMAT);
  }
</script>

<div class="document-menu" data-testid="document-menu">
  <button type="button" data-testid="menu-open" onclick={() => void requestOpen()}>
    {t("menu.file.open")}
  </button>
  <RecentFilesMenu />
  <button
    type="button"
    data-testid="menu-new-recording"
    disabled={rec.state.recording || rec.state.finishing}
    onclick={openNewRecording}
  >
    {t("menu.file.new_recording")}
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
