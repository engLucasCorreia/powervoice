<script lang="ts">
  import { t } from "../i18n";
  import { Dialog } from "../ui";
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
    actions={[
      { label: t("dialog.recent_missing.remove"), role: "destructive", testid: "recent-missing-remove", onclick: () => resolveRecentMissingPrompt("remove") },
      { label: t("dialog.recent_missing.cancel"), role: "cancel", testid: "recent-missing-cancel", onclick: () => resolveRecentMissingPrompt("cancel") },
      { label: t("dialog.recent_missing.locate"), role: "primary", testid: "recent-missing-locate", onclick: () => resolveRecentMissingPrompt("locate") },
    ]}
    role="alertdialog"
    title={t("dialog.recent_missing.title")}
    titleId="recent-missing-title"
    testid="recent-missing-dialog"
    onkeydown={onKeydown}
  >
    <p>{t("dialog.recent_missing.message", { name: recent.missingPrompt.name })}</p>
  </Dialog>
{/if}
