<script lang="ts">
  import { t } from "../i18n";
  import { Dialog, formatNumber } from "../ui";
  import { documentState, resolveClipPrompt } from "./document.svelte";

  /**
   * T-209 (SPEC-005 §2.8): "12 samples are above 0 dBFS (peak +1.8 dBFS) and will be clipped in
   * this format." Shown by `document_save`/`document_save_as`'s `dialog.overs` refusal — never
   * for a 32-bit float target (float overs are kept, never a clip). H-25: Dialog shell.
   */
  const doc = documentState();

  function peakLabel(peakDbfs: number): string {
    return formatNumber(peakDbfs, 1, { signed: true });
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
    actions={[
      { label: t("dialog.overs.cancel"), role: "cancel", testid: "clip-prompt-cancel", onclick: () => resolveClipPrompt("cancel") },
      { label: t("dialog.overs.float"), role: "alternate", testid: "clip-prompt-float", onclick: () => resolveClipPrompt("float") },
      { label: t("dialog.overs.clip"), role: "primary", testid: "clip-prompt-clip", onclick: () => resolveClipPrompt("clip") },
    ]}
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
  </Dialog>
{/if}
