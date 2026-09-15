<script lang="ts">
  import { t } from "../i18n";
  import { Dialog } from "../ui";
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
    actions={[
      { label: t("dialog.unsaved.discard"), role: "destructive", testid: "unsaved-discard", onclick: () => resolveUnsavedPrompt("discard") },
      { label: t("dialog.unsaved.cancel"), role: "cancel", testid: "unsaved-cancel", onclick: () => resolveUnsavedPrompt("cancel") },
      { label: t("dialog.unsaved.save"), role: "primary", testid: "unsaved-save", onclick: () => resolveUnsavedPrompt("save") },
    ]}
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
  </Dialog>
{/if}
