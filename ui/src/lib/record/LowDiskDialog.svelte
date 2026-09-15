<script lang="ts">
  import { t } from "../i18n";
  import { recordState, resolveLowDiskPrompt } from "../state/record.svelte";
  import { Dialog } from "../ui";

  /**
   * "Only N min of disk space left. Record anyway?" confirm prompt (H-11, SPEC-002 §2.5), shown
   * by the record store before Record starts a take with less than `DISK_WARN_MINUTES` of
   * estimated recording time left on the session volume. H-25: Dialog shell.
   */
  const rec = recordState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      resolveLowDiskPrompt(false);
    }
  }
</script>

{#if rec.lowDiskPrompt}
  <Dialog
    actions={[
      { label: t("dialog.low_disk.cancel"), role: "cancel", testid: "low-disk-cancel", onclick: () => resolveLowDiskPrompt(false) },
      { label: t("dialog.low_disk.confirm"), role: "primary", icon: "record", testid: "low-disk-confirm", onclick: () => resolveLowDiskPrompt(true) },
    ]}
    role="alertdialog"
    size="sm"
    title={t("dialog.low_disk.title")}
    titleId="low-disk-title"
    testid="low-disk-dialog"
    onkeydown={onKeydown}
  >
    <p>{t("dialog.low_disk.message", { minutes: String(rec.lowDiskPrompt.minutes) })}</p>
  </Dialog>
{/if}
