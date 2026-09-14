<script lang="ts">
  import { t } from "../i18n";
  import { Button, Dialog } from "../ui";
  import { documentState, resolveClipPrompt } from "./document.svelte";

  /**
   * T-209 (SPEC-005 §2.8): "12 samples are above 0 dBFS (peak +1.8 dBFS) and will be clipped in
   * this format." Shown by `document_save`/`document_save_as`'s `dialog.overs` refusal — never
   * for a 32-bit float target (float overs are kept, never a clip). H-25: Dialog shell.
   */
  const doc = documentState();

  function peakLabel(peakDbfs: number): string {
    const sign = peakDbfs >= 0 ? "+" : "";
    return `${sign}${peakDbfs.toFixed(1)}`;
  }

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveClipPrompt("cancel");
    }
  }
</script>

{#if doc.clipPrompt}
  <Dialog
    role="alertdialog"
    title={t("dialog.overs.title")}
    titleId="clip-prompt-title"
    testid="clip-prompt-dialog"
    onkeydown={onKeydown}
  >
    <p>
      {t("dialog.overs.message", {
        count: doc.clipPrompt.count,
        peak: peakLabel(doc.clipPrompt.peakDbfs),
      })}
    </p>
    {#snippet footer()}
      <Button testid="clip-prompt-cancel" onclick={() => resolveClipPrompt("cancel")}>
        {t("dialog.overs.cancel")}
      </Button>
      <Button testid="clip-prompt-float" onclick={() => resolveClipPrompt("float")}>
        {t("dialog.overs.float")}
      </Button>
      <Button variant="primary" testid="clip-prompt-clip" onclick={() => resolveClipPrompt("clip")}>
        {t("dialog.overs.clip")}
      </Button>
    {/snippet}
  </Dialog>
{/if}
