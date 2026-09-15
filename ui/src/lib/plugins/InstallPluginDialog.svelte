<script lang="ts">
  import { t, type MessageKey } from "../i18n";
  import type { BlockCauseDto, InstallFailureDto } from "../ipc/bindings";
  import { Dialog, Icon, type DialogAction } from "../ui";
  import { fileName } from "./pluginList";
  import { closeInstall, confirmReplace, pluginsState, showInstallInManager } from "./plugins.svelte";

  /**
   * "Install module…" (T-809 item 3): the steps after the native file picker — installing (copy
   * + sandboxed scan of just that file, up to 30 s), a name collision to confirm (Replace is
   * destructive; the installed file is kept if the new one can't be used), the result: the
   * effects it added (in Add module at once), or why it failed and whether it was blocklisted.
   * States the trust model (ADR-006 §7): plugins are native code with the user's permissions.
   */
  const ps = pluginsState();
  const st = $derived(ps.install);

  const FAILURE: Record<InstallFailureDto, MessageKey> = {
    not_found: "plugins.install.failure.not_found",
    not_a_plugin: "plugins.install.failure.not_a_plugin",
    already_installed: "plugins.install.failure.already_installed",
    blocklisted: "plugins.install.failure.blocklisted",
    no_effects: "plugins.install.failure.no_effects",
    scan_crashed: "plugins.install.failure.scan_crashed",
    scan_timed_out: "plugins.install.failure.scan_timed_out",
    scan_failed: "plugins.install.failure.scan_failed",
    no_install_dir: "plugins.install.failure.no_install_dir",
    io: "plugins.install.failure.io",
  };
  const CAUSE: Record<BlockCauseDto, MessageKey> = {
    crashed: "plugins.cause.crashed",
    timed_out: "plugins.cause.timed_out",
    manual: "plugins.cause.manual",
  };
  /** Codes whose underlying (OS/plugin) text helps: shown as a quiet detail line. */
  const WITH_DETAIL: readonly InstallFailureDto[] = ["scan_crashed", "scan_timed_out", "scan_failed", "io"];

  function folderOf(path: string): string {
    const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return cut > 0 ? path.slice(0, cut) : path;
  }

  const file = $derived(st.phase === "idle" ? "" : fileName(st.source));

  const actions = $derived.by((): DialogAction[] => {
    switch (st.phase) {
      case "collision":
        return [
          { label: t("plugins.install.cancel"), role: "cancel", testid: "plugin-install-cancel", onclick: closeInstall },
          {
            label: t("plugins.install.replace"),
            role: "primary",
            variant: "danger",
            testid: "plugin-install-replace",
            onclick: () => void confirmReplace(),
          },
        ];
      case "installed":
        return [
          {
            label: t("plugins.install.show_in_manager"),
            role: "alternate",
            testid: "plugin-install-show",
            onclick: showInstallInManager,
          },
          { label: t("plugins.install.done"), role: "primary", testid: "plugin-install-done", onclick: closeInstall },
        ];
      case "failed": {
        const list: DialogAction[] = [];
        if (st.blocklisted) {
          list.push({
            label: t("plugins.install.show_in_manager"),
            role: "alternate",
            testid: "plugin-install-show",
            onclick: showInstallInManager,
          });
        }
        list.push({ label: t("plugins.install.close"), role: "primary", testid: "plugin-install-close", onclick: closeInstall });
        return list;
      }
      default:
        return [];
    }
  });

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closeInstall();
    }
  }
</script>

{#if st.phase === "installing"}
  <Dialog
    size="sm"
    title={t("plugins.install.installing_title")}
    testid="plugin-install-dialog"
    data-phase="installing"
    aria-busy="true"
    onkeydown={onKeydown}
  >
    <p data-testid="plugin-install-message">{t("plugins.install.installing", { file })}</p>
    <progress aria-label={t("plugins.install.progress")}></progress>
    <p class="hint">{t("plugins.install.trust")}</p>
  </Dialog>
{:else if st.phase === "collision"}
  <Dialog
    size="sm"
    role="alertdialog"
    title={t("plugins.install.collision_title", { file })}
    testid="plugin-install-dialog"
    data-phase="collision"
    onkeydown={onKeydown}
    {actions}
  >
    <p data-testid="plugin-install-message">{t("plugins.install.collision_body", { folder: folderOf(st.target) })}</p>
  </Dialog>
{:else if st.phase === "installed"}
  <Dialog
    size="md"
    title={t("plugins.install.done_title")}
    testid="plugin-install-dialog"
    data-phase="installed"
    onkeydown={onKeydown}
    {actions}
  >
    <p class="lead success" data-testid="plugin-install-message">
      <Icon name="success" />
      <span>
        {t(st.replaced ? "plugins.install.replaced_in" : "plugins.install.installed_in", {
          file,
          folder: folderOf(st.target),
        })}
      </span>
    </p>
    <h3>
      {st.effects.length === 1
        ? t("plugins.install.added_one")
        : t("plugins.install.added_many", { count: st.effects.length })}
    </h3>
    <ul class="effects">
      {#each st.effects as effect (effect.id)}
        <li data-testid="plugin-install-effect"><Icon name="plugin" size="sm" />{effect.name}</li>
      {/each}
    </ul>
    <p class="hint">{t("plugins.install.where")}</p>
    <p class="hint">{t("plugins.install.trust")}</p>
  </Dialog>
{:else if st.phase === "failed"}
  <Dialog
    size="md"
    title={t("plugins.install.failed_title", { file })}
    testid="plugin-install-dialog"
    data-phase="failed"
    onkeydown={onKeydown}
    {actions}
  >
    <p class="lead failure" data-testid="plugin-install-reason">
      <Icon name="error" />
      <span>
        {t(FAILURE[st.code], { cause: st.cause ? t(CAUSE[st.cause]) : "" })}
      </span>
    </p>
    {#if WITH_DETAIL.includes(st.code) && st.detail}
      <p class="hint detail" data-testid="plugin-install-detail">{t("plugins.install.details", { detail: st.detail })}</p>
    {/if}
    {#if st.blocklisted && st.code !== "blocklisted"}
      <p data-testid="plugin-install-blocklisted">{t("plugins.install.blocklisted_note")}</p>
    {/if}
  </Dialog>
{/if}

<style>
  .lead {
    display: flex;
    align-items: flex-start;
    gap: var(--pv-space-2);
    color: var(--pv-text-primary);
  }

  .lead :global(svg) {
    flex: none;
    margin-top: 1px;
  }

  .lead.success :global(svg) {
    color: var(--pv-success-text);
  }

  .lead.failure :global(svg) {
    color: var(--pv-danger-text);
  }

  .effects {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-1);
    margin: 0;
    padding: var(--pv-space-2) var(--pv-space-3);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-panel);
    list-style: none;
  }

  .effects li {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-height: var(--pv-hit-min);
    color: var(--pv-text-primary);
  }

  .effects li :global(svg) {
    color: var(--pv-text-tertiary);
  }

  .detail {
    font-family: var(--pv-font-mono);
    font-size: var(--pv-text-xs);
    overflow-wrap: anywhere;
  }
</style>
