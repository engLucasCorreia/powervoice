<script lang="ts">
  import { t } from "../i18n";
  import { Button, Dialog } from "../ui";
  import { aboutState, closeAbout } from "./about.svelte";
  // T-705: `scripts/notices/generate.py` (`just notices`) writes this alongside the root
  // `THIRD_PARTY_NOTICES` file — see that script's docstring. Regenerate, don't hand-edit.
  import thirdPartyNotices from "./thirdPartyNotices.generated.txt?raw";

  /** H-19: Help → About with version (`app_info`'s `version`, already fetched once by
   * `App.svelte` at startup — passed in rather than re-fetched here). T-705 adds a collapsible
   * third-party notices panel (ADR-007). H-25: Dialog shell; it widens while notices are open. */
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
  <Dialog
    actions={[{ label: t("about.close"), role: "primary", testid: "about-close", onclick: closeAbout }]}
    size={noticesOpen ? "lg" : "sm"}
    title={t("about.title")}
    titleId="about-dialog-title"
    testid="about-dialog"
    onkeydown={onKeydown}
  >
    <p data-testid="about-version">{t("about.version", { version })}</p>
    <div class="notices-toggle">
      <Button
        variant="ghost"
        size="sm"
        iconEnd={noticesOpen ? "chevronUp" : "chevronDown"}
        testid="about-notices-toggle"
        aria-expanded={noticesOpen}
        onclick={toggleNotices}
      >
        {noticesOpen ? t("about.notices.hide") : t("about.notices.show")}
      </Button>
    </div>
    {#if noticesOpen}
      <pre class="notices" data-testid="about-notices">{thirdPartyNotices}</pre>
    {/if}
  </Dialog>
{/if}

<style>
  .notices-toggle {
    margin-left: calc(-1 * var(--pv-space-2));
  }

  .notices {
    max-height: 50vh;
    margin: 0;
    padding: var(--pv-space-3);
    overflow: auto;
    white-space: pre-wrap;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-inset);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-mono);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
  }
</style>
