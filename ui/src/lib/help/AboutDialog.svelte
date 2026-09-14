<script lang="ts">
  import { t } from "../i18n";
  import { aboutState, closeAbout } from "./about.svelte";

  /** H-19: Help → About with version (`app_info`'s `version`, already fetched once by
   * `App.svelte` at startup — passed in rather than re-fetched here). Same modal shape as
   * `ConfirmDialog.svelte`. */
  let { version = "" }: { version?: string } = $props();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closeAbout();
    }
  }
</script>

{#if aboutState().open}
  <div class="backdrop">
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="about-dialog-title"
      data-testid="about-dialog"
      tabindex="-1"
      onkeydown={onKeydown}
    >
      <h2 id="about-dialog-title">{t("about.title")}</h2>
      <p data-testid="about-version">{t("about.version", { version })}</p>
      <div class="actions">
        <button type="button" class="primary" data-testid="about-close" onclick={closeAbout}>
          {t("about.close")}
        </button>
      </div>
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
    z-index: 1000;
  }

  .dialog {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    min-width: 20rem;
    max-width: 90vw;
    padding: 1rem 1.25rem;
    background: var(--surface-panel);
    border: 1px solid var(--surface-border);
    border-radius: 6px;
    color: var(--text-primary);
  }

  h2 {
    margin: 0;
    font-size: 1rem;
  }

  p {
    margin: 0;
    color: var(--text-secondary);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.5rem;
  }

  button {
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
</style>
