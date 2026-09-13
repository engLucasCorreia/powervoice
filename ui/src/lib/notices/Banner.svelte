<script lang="ts">
  import { tDynamic, t } from "../i18n";
  import { dismissBanner } from "../state/notices.svelte";
  import type { ActiveNotice } from "../state/notices.svelte";

  let { notice }: { notice: ActiveNotice } = $props();
</script>

<div class="banner" data-testid="banner" data-level={notice.level}>
  <span class="message">{tDynamic(notice.key, notice.params)}</span>
  <button
    type="button"
    class="dismiss"
    aria-label={t("notice.dismiss")}
    onclick={() => dismissBanner(notice.localId)}
  >
    &times;
  </button>
</div>

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0.75rem;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--surface-border);
    color: var(--text-primary);
  }

  .banner[data-level="error"] {
    border-left: 3px solid var(--meter-red);
  }

  .banner[data-level="warning"] {
    border-left: 3px solid var(--meter-yellow);
  }

  .banner[data-level="info"] {
    border-left: 3px solid var(--accent);
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
