<script lang="ts">
  import type { RecoverableSessionDto, RecoveredTakeActionDto } from "../ipc/bindings";
  import { t } from "../i18n";
  import { Button, Dialog, Icon } from "../ui";
  import { formatBytes, formatDuration, formatTimestamp } from "./format";
  import {
    cancelDiscard,
    closeRecovery,
    confirmDiscard,
    decideLater,
    isUnrecoverable,
    recover,
    recoveryState,
    requestDiscard,
    setTakeAction,
    takeActionFor,
  } from "./recovery.svelte";

  /**
   * T-301 (SPEC-004 §2.7/§2.8): the start-up recovery dialog and File → Recovery & Storage….
   * One component for both: the start-up mode ends with "Decide later", the storage mode shows
   * the session storage figures and "Close".
   */
  const rec = recoveryState();

  const TAKE_ACTIONS: RecoveredTakeActionDto[] = ["apply", "new_document", "discard"];

  function nameOf(s: RecoverableSessionDto): string {
    return s.name ?? t("recovery.untitled");
  }

  function unsavedText(s: RecoverableSessionDto): string {
    return s.unsaved_changes === 1
      ? t("recovery.unsaved_changes_one")
      : t("recovery.unsaved_changes", { count: s.unsaved_changes });
  }

  const storageLine = $derived.by(() => {
    const info = rec.storage;
    if (!info || info.session_bytes === null) {
      return t("recovery.storage.no_session");
    }
    return t("recovery.storage.session", {
      total: formatBytes(info.session_bytes),
      history: formatBytes(info.history_bytes ?? 0),
    });
  });

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      if (rec.pendingDiscard) {
        cancelDiscard();
      } else if (rec.mode === "storage") {
        closeRecovery();
      }
    }
  }
</script>

{#if rec.mode}
  <Dialog
    actions={rec.mode === "startup"
      ? [{ label: t("recovery.decide_later"), role: "cancel", testid: "recovery-decide-later", onclick: decideLater }]
      : [{ label: t("recovery.close"), role: "primary", testid: "recovery-close", onclick: closeRecovery }]}
    size="lg"
    title={rec.mode === "startup" ? t("recovery.title") : t("recovery.storage_title")}
    titleId="recovery-title"
    testid="recovery-dialog"
    data-mode={rec.mode}
    onkeydown={onKeydown}
  >
    {#if rec.mode === "storage"}
      <p data-testid="recovery-storage">{storageLine}</p>
      <p>
        {t("recovery.storage.recovery_data", { size: formatBytes(rec.storage?.recovery_bytes ?? 0) })}
      </p>
    {:else}
      <p>{t("recovery.intro")}</p>
    {/if}
    {#if rec.sessions.length === 0}
      <p data-testid="recovery-empty">{t("recovery.empty")}</p>
    {/if}
    <ul class="sessions">
      {#each rec.sessions as s (s.id)}
        <li data-testid="recovery-session" data-id={s.id}>
          <div class="head">
            <div class="who">
              <div class="name">{nameOf(s)}</div>
              <div class="path">{s.path ?? t("recovery.no_path")}</div>
            </div>
            <div class="actions">
              <Button variant="ghost" size="sm" testid="recovery-discard" onclick={() => requestDiscard(s)}>
                {t("recovery.discard")}
              </Button>
              <Button
                variant="primary"
                size="sm"
                testid="recovery-recover"
                disabled={rec.busy || isUnrecoverable(s.id)}
                onclick={() => void recover(s.id)}
              >
                {t("recovery.recover")}
              </Button>
            </div>
          </div>
          <div class="facts">
            {#if s.last_modified_unix_ms !== null}
              <span>{t("recovery.last_edit", { time: formatTimestamp(s.last_modified_unix_ms) })}</span>
            {/if}
            <span data-testid="recovery-unsaved">{unsavedText(s)}</span>
            <span>{formatBytes(s.size_bytes)}</span>
          </div>
          {#if s.recording_samples !== null}
            <div class="take">
              <span class="take-label" data-testid="recovery-recording">
                <Icon name="record" size="sm" filled />
                {t("recovery.recording", {
                  duration: formatDuration(s.recording_samples, s.sample_rate_hz),
                })}
              </span>
              <select
                data-testid="recovery-take-action"
                aria-label={t("recovery.take_action")}
                value={takeActionFor(s.id)}
                onchange={(e) => setTakeAction(s.id, e.currentTarget.value as RecoveredTakeActionDto)}
              >
                {#each TAKE_ACTIONS as action (action)}
                  <option value={action}>{t(`recovery.take_action.${action}`)}</option>
                {/each}
              </select>
            </div>
          {/if}
          {#if s.source_changed}
            <p class="note warning" data-testid="recovery-changed">
              <Icon name="warning" size="sm" />{t("recovery.source_changed", { name: nameOf(s) })}
            </p>
          {/if}
          {#if s.source_missing}
            <p class="note warning"><Icon name="warning" size="sm" />{t("recovery.source_missing", { name: nameOf(s) })}</p>
          {/if}
          {#if s.damaged}
            <p class="note warning"><Icon name="warning" size="sm" />{t("recovery.damaged")}</p>
          {/if}
          {#if isUnrecoverable(s.id)}
            <p class="note error" data-testid="recovery-unrecoverable">
              <Icon name="error" size="sm" />{t("recovery.unrecoverable")}
            </p>
          {/if}
        </li>
      {/each}
    </ul>
    {#if rec.pendingDiscard}
      <div class="confirm" role="alertdialog" aria-modal="true" data-testid="recovery-discard-confirm">
        <h3>{t("recovery.discard_confirm.title")}</h3>
        <p>{t("recovery.discard_confirm.message", { name: nameOf(rec.pendingDiscard) })}</p>
        <div class="confirm-actions">
          <Button testid="recovery-discard-cancel" onclick={cancelDiscard}>
            {t("recovery.discard_confirm.cancel")}
          </Button>
          <Button variant="danger" testid="recovery-discard-proceed" onclick={() => void confirmDiscard()}>
            {t("recovery.discard_confirm.confirm")}
          </Button>
        </div>
      </div>
    {/if}
  </Dialog>
{/if}

<style>
  .sessions {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    padding: var(--pv-space-3);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-raised);
  }

  .head {
    display: flex;
    align-items: flex-start;
    gap: var(--pv-space-3);
  }

  .who {
    flex: 1;
    min-width: 0;
  }

  .name {
    font-weight: var(--pv-weight-semibold);
  }

  .path {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    overflow-wrap: anywhere;
  }

  .actions,
  .confirm-actions {
    display: flex;
    flex: none;
    gap: var(--pv-space-2);
  }

  .facts,
  .take {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-1) var(--pv-space-3);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
  }

  .take-label {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-1);
    color: var(--pv-record-text);
  }

  .note {
    display: flex;
    align-items: flex-start;
    gap: var(--pv-space-2);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .note.error {
    color: var(--pv-danger-text);
  }

  .confirm {
    position: absolute;
    inset: auto var(--pv-space-4) var(--pv-space-4);
    z-index: 1;
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    padding: var(--pv-space-4);
    border: var(--pv-border-width) solid var(--pv-border-strong);
    border-radius: var(--pv-radius-lg);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-3);
  }

  .confirm-actions {
    justify-content: flex-end;
  }
</style>
