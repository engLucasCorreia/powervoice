<script lang="ts">
  import { t } from "../i18n";
  import { Dialog } from "../ui";
  import { fileName } from "./pluginList";
  import { cancelUninstall, confirmUninstall, pluginsState } from "./plugins.svelte";

  /**
   * H-29: "Uninstall…" confirmation, from the Plugin Manager's row ⋯ menu (only offered for a
   * file inside the per-user install folder — files found elsewhere offer "Block" instead). H-26
   * Dialog `actions`, destructive role for the removal itself.
   */
  const ps = pluginsState();
  const prompt = $derived(ps.uninstallPrompt);
  const name = $derived(prompt ? prompt.entry.name || fileName(prompt.entry.path) : "");

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      cancelUninstall();
    }
  }
</script>

{#if prompt}
  <Dialog
    size="sm"
    role="alertdialog"
    title={t("plugins.uninstall.title", { name })}
    testid="plugin-uninstall-dialog"
    onkeydown={onKeydown}
    actions={[
      {
        label: t("plugins.uninstall.cancel"),
        role: "cancel",
        testid: "plugin-uninstall-cancel",
        disabled: prompt.busy,
        onclick: cancelUninstall,
      },
      {
        label: t("plugins.uninstall.confirm"),
        role: "primary",
        variant: "danger",
        testid: "plugin-uninstall-confirm",
        loading: prompt.busy,
        onclick: () => void confirmUninstall(),
      },
    ]}
  >
    <p data-testid="plugin-uninstall-message">{t("plugins.uninstall.message", { name })}</p>
    <p class="hint">{t("plugins.uninstall.hint")}</p>
  </Dialog>
{/if}

<style>
  .hint {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-sm);
  }
</style>
