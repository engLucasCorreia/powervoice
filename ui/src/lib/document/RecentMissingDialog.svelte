<script lang="ts">
  import { t } from "../i18n";
  import { Button, Dialog } from "../ui";
  import { recentFilesState, resolveRecentMissingPrompt } from "./recentFiles.svelte";

  /**
   * H-15 (SPEC-018 §2.12, ticket-added "Locate…"): picking a missing `File → Open Recent` entry
   * shows this instead of failing through the normal open flow's error toast. "Locate…" opens the
   * native picker and re-points the entry to whatever the user picks (`recentFiles.svelte.ts`);
   * "Remove from List" drops the dead entry; "Cancel" leaves the list untouched.
   * H-25: Dialog shell.
   */
  const recent = recentFilesState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveRecentMissingPrompt("cancel");
    }
  }
</script>

{#if recent.missingPrompt}
  <Dialog
    role="alertdialog"
    title={t("dialog.recent_missing.title")}
    titleId="recent-missing-title"
    testid="recent-missing-dialog"
    onkeydown={onKeydown}
  >
    <p>{t("dialog.recent_missing.message", { name: recent.missingPrompt.name })}</p>
    {#snippet footer()}
      <Button variant="ghost" testid="recent-missing-remove" onclick={() => resolveRecentMissingPrompt("remove")}>
        {t("dialog.recent_missing.remove")}
      </Button>
      <span class="spacer"></span>
      <Button testid="recent-missing-cancel" onclick={() => resolveRecentMissingPrompt("cancel")}>
        {t("dialog.recent_missing.cancel")}
      </Button>
      <Button variant="primary" testid="recent-missing-locate" onclick={() => resolveRecentMissingPrompt("locate")}>
        {t("dialog.recent_missing.locate")}
      </Button>
    {/snippet}
  </Dialog>
{/if}
