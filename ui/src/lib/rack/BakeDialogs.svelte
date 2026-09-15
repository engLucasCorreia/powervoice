<script lang="ts">
  import { t } from "../i18n";
  import NormalizeProgressDialog from "../normalize/NormalizeProgressDialog.svelte";
  import {
    bakeState,
    cancelBakeConfirm,
    cancelBakeJob,
    continueBakeConfirm,
    dismissBakeJob,
  } from "../state/bake.svelte";
  import { Dialog } from "../ui";

  /**
   * Bake rack dialogs (T-602): SPEC-014 §2.6's "Output noise only" confirmation (Continue /
   * Cancel) and the job's progress dialog with Cancel (the shared job progress dialog: it shows
   * once the job is still running after 250 ms, so short bakes feel instant). The "done" toast
   * is the backend's `notice.bake.done`.
   */
  const state = bakeState();

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      cancelBakeConfirm();
    }
  }
</script>

{#if state.confirmOpen}
  <Dialog
    role="alertdialog"
    size="sm"
    title={t("dialog.bake.noise_only_confirm.title")}
    titleId="bake-noise-only-title"
    testid="bake-noise-only-confirm"
    onkeydown={onKeydown}
    actions={[
      {
        label: t("dialog.bake.noise_only_confirm.cancel"),
        role: "cancel",
        testid: "bake-noise-only-cancel",
        onclick: cancelBakeConfirm,
      },
      {
        label: t("dialog.bake.noise_only_confirm.continue"),
        role: "primary",
        testid: "bake-noise-only-continue",
        onclick: () => void continueBakeConfirm(),
      },
    ]}
  >
    <p>{t("dialog.bake.noise_only_confirm.message")}</p>
  </Dialog>
{/if}

<NormalizeProgressDialog
  job={state.job}
  titleKey="job.bake"
  testidPrefix="bake-progress"
  onCancel={cancelBakeJob}
  onDismiss={dismissBakeJob}
/>
