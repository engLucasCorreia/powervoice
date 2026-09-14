<script lang="ts">
  import type { RecoverableSessionDto, RecoveredTakeActionDto } from "../ipc/bindings";
  import { t } from "../i18n";
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
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="recovery-title"
      data-testid="recovery-dialog"
      data-mode={rec.mode}
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="recovery-title">
        {rec.mode === "startup" ? t("recovery.title") : t("recovery.storage_title")}
      </h2>
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
      <ul>
        {#each rec.sessions as s (s.id)}
          <li data-testid="recovery-session" data-id={s.id}>
            <div class="name">{nameOf(s)}</div>
            <div class="path">{s.path ?? t("recovery.no_path")}</div>
            <div class="facts">
              {#if s.last_modified_unix_ms !== null}
                <span>{t("recovery.last_edit", { time: formatTimestamp(s.last_modified_unix_ms) })}</span>
              {/if}
              <span data-testid="recovery-unsaved">{unsavedText(s)}</span>
              <span>{formatBytes(s.size_bytes)}</span>
            </div>
            {#if s.recording_samples !== null}
              <div class="take">
                <span data-testid="recovery-recording">
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
              <p class="warning" data-testid="recovery-changed">
                {t("recovery.source_changed", { name: nameOf(s) })}
              </p>
            {/if}
            {#if s.source_missing}
              <p class="warning">{t("recovery.source_missing", { name: nameOf(s) })}</p>
            {/if}
            {#if s.damaged}
              <p class="warning">{t("recovery.damaged")}</p>
            {/if}
            {#if isUnrecoverable(s.id)}
              <p class="warning" data-testid="recovery-unrecoverable">{t("recovery.unrecoverable")}</p>
            {/if}
            <div class="actions">
              <button
                type="button"
                class="primary"
                data-testid="recovery-recover"
                disabled={rec.busy || isUnrecoverable(s.id)}
                onclick={() => void recover(s.id)}
              >
                {t("recovery.recover")}
              </button>
              <button type="button" data-testid="recovery-discard" onclick={() => requestDiscard(s)}>
                {t("recovery.discard")}
              </button>
            </div>
          </li>
        {/each}
      </ul>

      <div class="footer">
        {#if rec.mode === "startup"}
          <button type="button" data-testid="recovery-decide-later" onclick={decideLater}>
            {t("recovery.decide_later")}
          </button>
        {:else}
          <button type="button" data-testid="recovery-close" onclick={closeRecovery}>
            {t("recovery.close")}
          </button>
        {/if}
      </div>

      {#if rec.pendingDiscard}
        <div class="confirm" role="alertdialog" aria-modal="true" data-testid="recovery-discard-confirm">
          <h3>{t("recovery.discard_confirm.title")}</h3>
          <p>{t("recovery.discard_confirm.message", { name: nameOf(rec.pendingDiscard) })}</p>
          <div class="actions">
            <button type="button" data-testid="recovery-discard-cancel" onclick={cancelDiscard}>
              {t("recovery.discard_confirm.cancel")}
            </button>
            <button
              type="button"
              class="danger"
              data-testid="recovery-discard-proceed"
              onclick={() => void confirmDiscard()}
            >
              {t("recovery.discard_confirm.confirm")}
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    background: rgba(0, 0, 0, 0.45);
    /* Below the unsaved-changes prompt (1000), which Recover may raise on top. */
    z-index: 900;
  }

  .dialog {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    width: min(40rem, 90vw);
    max-height: 85vh;
    overflow: auto;
    padding: 1rem 1.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    color: var(--text-primary);
  }

  h2,
  h3 {
    margin: 0;
    font-size: 1rem;
  }

  p {
    margin: 0;
    color: var(--text-secondary);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  li {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--surface-border);
    border-radius: 4px;
  }

  .name {
    font-weight: 600;
  }

  .path,
  .facts {
    color: var(--text-secondary);
    font-size: 0.85em;
    overflow-wrap: anywhere;
  }

  .facts,
  .take {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    align-items: center;
  }

  .warning {
    color: var(--warning, #e0a040);
    font-size: 0.85em;
  }

  .actions,
  .footer {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  .confirm {
    position: absolute;
    inset: auto 1rem 1rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    background: var(--surface-panel-raised);
    border: 1px solid var(--accent);
    border-radius: 6px;
  }

  button,
  select {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
    padding: 0.25rem 0.75rem;
  }

  button.primary {
    border-color: var(--accent);
    color: var(--accent);
  }

  button.danger {
    border-color: var(--danger, #d05050);
    color: var(--danger, #d05050);
  }

  button:disabled {
    color: var(--text-disabled);
  }
</style>
