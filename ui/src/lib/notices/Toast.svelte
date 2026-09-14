<script lang="ts">
  import { tDynamic, t } from "../i18n";
  import { dismissToast } from "../state/notices.svelte";
  import type { ActiveNotice } from "../state/notices.svelte";
  import { Icon, type IconName } from "../ui";

  /** H-25: Toast with a status icon per level (the word carries the meaning, the icon and colour
   * reinforce it) and a small dismiss key. */
  let { notice }: { notice: ActiveNotice } = $props();

  const glyph = $derived<IconName>(
    notice.level === "error" ? "error" : notice.level === "warning" ? "warning" : "info",
  );
</script>

<div class="toast" data-testid="toast" data-level={notice.level}>
  <span class="glyph"><Icon name={glyph} /></span>
  <span class="message">{tDynamic(notice.key, notice.params)}</span>
  <button
    type="button"
    class="dismiss"
    aria-label={t("notice.dismiss")}
    onclick={() => dismissToast(notice.localId)}
  >
    <Icon name="close" size="sm" />
  </button>
</div>

<style>
  .toast {
    display: flex;
    align-items: flex-start;
    gap: var(--pv-space-2);
    min-width: 16rem;
    padding: var(--pv-space-2) var(--pv-space-2) var(--pv-space-2) var(--pv-space-3);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-2);
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    animation: pv-toast-in var(--pv-duration-slow) var(--pv-ease-standard);
  }

  .glyph {
    display: inline-flex;
    flex: none;
    margin-top: 1px;
    color: var(--pv-accent-text);
  }

  .toast[data-level="warning"] .glyph {
    color: var(--pv-warning-text);
  }

  .toast[data-level="error"] .glyph {
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
    margin: calc(-1 * var(--pv-space-half)) 0;
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

  @keyframes pv-toast-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
  }
</style>
