<script lang="ts">
  import { tDynamic } from "../i18n";
  import { Dialog } from "../ui";
  import { documentState, resolveConfirmPrompt } from "./document.svelte";

  /**
   * T-306 (SPEC-018 §2.9/§2.11): "already open in another instance" and "changed on disk"
   * confirmations, shown by the document store before Open/Save proceeds with something that
   * could overwrite someone else's changes. Same shape for both — only the copy differs.
   * H-25: on the kit's Dialog shell.
   */
  const doc = documentState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveConfirmPrompt(false);
    }
  }
</script>

{#if doc.confirmPrompt}
  <Dialog
    actions={[
      {
        label: tDynamic(`dialog.${doc.confirmPrompt?.kind}.cancel`),
        role: "cancel",
        testid: "confirm-cancel",
        onclick: () => resolveConfirmPrompt(false),
      },
      {
        label: tDynamic(`dialog.${doc.confirmPrompt?.kind}.confirm`),
        role: "primary",
        testid: "confirm-proceed",
        onclick: () => resolveConfirmPrompt(true),
      },
    ]}
    role="alertdialog"
    size="sm"
    title={tDynamic(`dialog.${doc.confirmPrompt.kind}.title`)}
    titleId="confirm-dialog-title"
    testid="confirm-dialog"
    data-kind={doc.confirmPrompt.kind}
    onkeydown={onKeydown}
  >
    <p>{tDynamic(`dialog.${doc.confirmPrompt.kind}.message`, { name: doc.confirmPrompt.name })}</p>
  </Dialog>
{/if}
