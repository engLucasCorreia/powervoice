<script lang="ts">
  import { tDynamic, t } from "../i18n";
  import { dismissBanner } from "../state/notices.svelte";
  import type { ActiveNotice } from "../state/notices.svelte";
  import { Button, Icon, type IconName } from "../ui";
  import { dispatchNoticeAction } from "./noticeActions";

  /** H-25: Persistent banner with a status icon per level (the word carries the meaning, the icon and colour
   * reinforce it) and a small dismiss key. H-67: an optional action button (`notice.action`) —
   * absent on every notice that doesn't set one, so this looks exactly as it did before. */
  let { notice }: { notice: ActiveNotice } = $props();

  const glyph = $derived<IconName>(
    notice.level === "error" ? "error" : notice.level === "warning" ? "warning" : "info",
  );
  const action = $derived(notice.action);
</script>

<div class="banner" data-testid="banner" data-level={notice.level}>
  <span class="glyph"><Icon name={glyph} /></span>
  <span class="message">{tDynamic(notice.key, notice.params)}</span>
  {#if action}
    <Button
      size="sm"
      variant="secondary"
      testid="notice-action"
      onclick={() => dispatchNoticeAction(action.id)}
    >
      {tDynamic(action.label_key)}
    </Button>
  {/if}
  <button
    type="button"
    class="dismiss"
    aria-label={t("notice.dismiss")}
    onclick={() => dismissBanner(notice.localId)}
  >
    <Icon name="close" size="sm" />
  </button>
</div>

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-height: 36px;
    padding: var(--pv-space-1) var(--pv-space-2) var(--pv-space-1) var(--pv-space-3);
    border-bottom: var(--pv-border-width) solid var(--pv-border);
    box-shadow: inset 3px 0 0 var(--pv-accent);
    background: var(--pv-bg-raised);
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
  }

  .banner[data-level="warning"] {
    box-shadow: inset 3px 0 0 var(--pv-warning);
  }

  .banner[data-level="error"] {
    box-shadow: inset 3px 0 0 var(--pv-danger-text);
  }

  .glyph {
    display: inline-flex;
    flex: none;
    color: var(--pv-accent-text);
  }

  .banner[data-level="warning"] .glyph {
    color: var(--pv-warning-text);
  }

  .banner[data-level="error"] .glyph {
    color: var(--pv-danger-text);
  }

  .message {
    flex: 1;
  }

  .dismiss {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--pv-control-h-sm);
    height: var(--pv-control-h-sm);
    padding: 0;
    border: none;
    border-radius: var(--pv-radius-sm);
    background: transparent;
    color: var(--pv-text-secondary);
    cursor: default;
  }

  .dismiss:hover {
    background: var(--pv-control-bg-active);
    color: var(--pv-text-primary);
  }

  .dismiss:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }
</style>
