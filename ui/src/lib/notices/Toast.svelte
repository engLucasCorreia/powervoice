<script lang="ts">
  import { tDynamic, t } from "../i18n";
  import { dismissToast } from "../state/notices.svelte";
  import type { ActiveNotice } from "../state/notices.svelte";

  let { notice }: { notice: ActiveNotice } = $props();
</script>

<div class="toast" data-testid="toast" data-level={notice.level}>
  <span class="message">{tDynamic(notice.key, notice.params)}</span>
  <button
    type="button"
    class="dismiss"
    aria-label={t("notice.dismiss")}
    onclick={() => dismissToast(notice.localId)}
  >
    &times;
  </button>
</div>

<style>
  .toast {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.5rem 0.75rem;
    border-radius: 4px;
    background: var(--surface-panel-raised);
    border: 1px solid var(--surface-border);
    color: var(--text-primary);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35);
  }

  .toast[data-level="error"] {
    border-color: var(--meter-red);
  }

  .toast[data-level="warning"] {
    border-color: var(--meter-yellow);
  }

  .message {
    flex: 1;
  }

  .dismiss {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 1rem;
    line-height: 1;
    padding: 0 0.25rem;
  }

  .dismiss:hover {
    color: var(--text-primary);
  }
</style>
