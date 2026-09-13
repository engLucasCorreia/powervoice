<script lang="ts">
  import { t, tDynamic } from "../i18n";
  import { recordState } from "../state/record.svelte";
  import {
    copy,
    cut,
    deleteSelection,
    editState,
    hasClipboard,
    paste,
    redo,
    silence,
    trim,
    undo,
  } from "../state/edit.svelte";
  import { hasSelection } from "../state/selection.svelte";

  /**
   * Edit menu/toolbar (S2-01, SPEC-008 §2.11): Undo/Redo (with the history's i18n label) then
   * Cut, Copy, Paste, Delete, Trim to Selection, Silence, in that order under Undo/Redo. All
   * seven destructive/clipboard commands are disabled without a (non-empty) selection — Paste
   * only needs a non-empty clipboard — and while recording (SPEC-008 §2.2).
   */

  const edit = editState();
  const rec = recordState();

  const recording = $derived(rec.state.recording);
  const selected = $derived(hasSelection() && !recording);
  const pasteEnabled = $derived(hasClipboard() && !recording);

  const undoLabel = $derived(
    edit.history.undo_label
      ? t("menu.edit.undo", { label: tDynamic(edit.history.undo_label) })
      : t("menu.edit.undo_none"),
  );
  const redoLabel = $derived(
    edit.history.redo_label
      ? t("menu.edit.redo", { label: tDynamic(edit.history.redo_label) })
      : t("menu.edit.redo_none"),
  );
</script>

<div class="edit-menu" data-testid="edit-menu">
  <button
    type="button"
    data-testid="menu-undo"
    disabled={!edit.history.can_undo || recording}
    onclick={() => void undo()}
  >
    {undoLabel}
  </button>
  <button
    type="button"
    data-testid="menu-redo"
    disabled={!edit.history.can_redo || recording}
    onclick={() => void redo()}
  >
    {redoLabel}
  </button>
  <span class="divider" aria-hidden="true"></span>
  <button type="button" data-testid="menu-cut" disabled={!selected} onclick={() => void cut()}>
    {t("edit.cut")}
  </button>
  <button type="button" data-testid="menu-copy" disabled={!selected} onclick={() => void copy()}>
    {t("edit.copy")}
  </button>
  <button
    type="button"
    data-testid="menu-paste"
    disabled={!pasteEnabled}
    onclick={() => void paste()}
  >
    {t("edit.paste")}
  </button>
  <button
    type="button"
    data-testid="menu-delete"
    disabled={!selected}
    onclick={() => void deleteSelection()}
  >
    {t("edit.delete")}
  </button>
  <button type="button" data-testid="menu-trim" disabled={!selected} onclick={() => void trim()}>
    {t("edit.trim")}
  </button>
  <button
    type="button"
    data-testid="menu-silence"
    disabled={!selected}
    onclick={() => void silence()}
  >
    {t("edit.silence")}
  </button>
</div>

<style>
  .edit-menu {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.75rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
    font-size: 0.85em;
  }

  .divider {
    width: 1px;
    align-self: stretch;
    background: var(--surface-border);
    margin: 0 0.15rem;
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
