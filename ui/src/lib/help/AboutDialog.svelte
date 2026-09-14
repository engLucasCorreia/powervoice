<script lang="ts">
  import { t } from "../i18n";
  import { aboutState, closeAbout } from "./about.svelte";
  // T-705: `scripts/notices/generate.py` (`just notices`) writes this alongside the root
  // `THIRD_PARTY_NOTICES` file — see that script's docstring. Regenerate, don't hand-edit.
  import thirdPartyNotices from "./thirdPartyNotices.generated.txt?raw";

  /** H-19: Help → About with version (`app_info`'s `version`, already fetched once by
   * `App.svelte` at startup — passed in rather than re-fetched here). Same modal shape as
   * `ConfirmDialog.svelte`. T-705 adds a collapsible third-party notices panel (ADR-007). */
  let { version = "" }: { version?: string } = $props();

  let noticesOpen = $state(false);

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closeAbout();
    }
  }

  function toggleNotices(): void {
    noticesOpen = !noticesOpen;
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
      <button
        type="button"
        class="notices-toggle"
        data-testid="about-notices-toggle"
        aria-expanded={noticesOpen}
        onclick={toggleNotices}
      >
        {noticesOpen ? t("about.notices.hide") : t("about.notices.show")}
      </button>
      {#if noticesOpen}
        <pre class="notices" data-testid="about-notices">{thirdPartyNotices}</pre>
      {/if}
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

  .notices-toggle {
    align-self: flex-start;
  }

  .notices {
    max-width: 60vw;
    max-height: 50vh;
    margin: 0;
    padding: 0.5rem;
    overflow: auto;
    white-space: pre-wrap;
    font-family: monospace;
    font-size: 0.75rem;
    background: var(--surface-panel-raised);
    border: 1px solid var(--surface-border);
    border-radius: 4px;
  }
</style>
