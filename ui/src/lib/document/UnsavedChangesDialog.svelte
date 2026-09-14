<script lang="ts">
  import { t } from "../i18n";
  import { Button, Dialog } from "../ui";
  import { documentState, resolveUnsavedPrompt } from "./document.svelte";

  /**
   * Save / Don't Save / Cancel prompt (SPEC-004 §2.8 "simple version"), shown by the document
   * store before Open or quit would discard unsaved changes. H-25: Dialog shell; "Don't save"
   * sits apart on the left so it's never hit by accident next to Save.
   */
  const doc = documentState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveUnsavedPrompt("cancel");
    }
  }
</script>

{#if doc.unsavedPrompt}
  <Dialog
    role="alertdialog"
    size="sm"
    title={t("dialog.unsaved.title")}
    titleId="unsaved-changes-title"
    testid="unsaved-changes-dialog"
    onkeydown={onKeydown}
  >
    <p>{t("dialog.unsaved.message", { name: doc.unsavedPrompt.name })}</p>
    {#if doc.unsavedPrompt.effectSettingsOnly}
      <p data-testid="unsaved-effect-settings-changed">
        {t("dialog.unsaved.effect_settings_changed")}
      </p>
    {/if}
    {#snippet footer()}
      <Button variant="ghost" testid="unsaved-discard" onclick={() => resolveUnsavedPrompt("discard")}>
        {t("dialog.unsaved.discard")}
      </Button>
      <span class="spacer"></span>
      <Button testid="unsaved-cancel" onclick={() => resolveUnsavedPrompt("cancel")}>
        {t("dialog.unsaved.cancel")}
      </Button>
      <Button variant="primary" testid="unsaved-save" onclick={() => resolveUnsavedPrompt("save")}>
        {t("dialog.unsaved.save")}
      </Button>
    {/snippet}
  </Dialog>
{/if}
