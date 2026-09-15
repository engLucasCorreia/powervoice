<script lang="ts">
  import { onMount } from "svelte";
  import { t } from "../i18n";
  import { Badge, Button, Icon, IconButton } from "../ui";
  import { addPluginFolder, pluginsState, refreshFolders, removePluginFolder } from "./plugins.svelte";

  /**
   * The folders PowerVoice scans for plugins (T-809 item 2): the plugin manager's Folders tab
   * (everything) and Preferences → Plugins (`showStandard={false}`: only the user's own). Adding
   * uses the native folder picker; every change is followed by a rescan (the manager shows its
   * progress).
   */
  let { showStandard = true }: { showStandard?: boolean } = $props();

  const ps = pluginsState();
  const installs = $derived(ps.folders?.install_folders ?? []);
  const scanned = $derived.by(() => {
    const standard = ps.folders?.standard ?? [];
    return [...installs.filter((dir) => !standard.includes(dir)), ...standard];
  });
  const custom = $derived(ps.folders?.custom ?? []);

  onMount(() => {
    void refreshFolders();
  });
</script>

<div class="folders" data-testid="plugin-folders">
  {#if showStandard}
    <section>
      <h3>{t("plugins.folders.scanned")}</h3>
      <p class="hint">{t("plugins.folders.scanned_hint")}</p>
      <ul>
        {#each scanned as path (path)}
          <li data-testid="plugin-folder-standard">
            <Icon name="open" size="sm" />
            <span class="path" title={path}>{path}</span>
            {#if installs.includes(path)}
              <Badge tone="accent" testid="plugin-folder-install-badge">{t("plugins.folders.install_badge")}</Badge>
            {:else}
              <Badge>{t("plugins.folders.standard_badge")}</Badge>
            {/if}
          </li>
        {/each}
      </ul>
    </section>
  {/if}
  <section>
    <h3>{t("plugins.folders.custom")}</h3>
    {#if custom.length === 0}
      <p class="hint" data-testid="plugin-folders-empty">{t("plugins.folders.custom_empty")}</p>
    {:else}
      <ul>
        {#each custom as path (path)}
          <li data-testid="plugin-folder-custom">
            <Icon name="open" size="sm" />
            <span class="path" title={path}>{path}</span>
            <IconButton
              icon="delete"
              size="sm"
              label={t("plugins.folders.remove", { path })}
              testid="plugin-folder-remove"
              onclick={() => void removePluginFolder(path)}
            />
          </li>
        {/each}
      </ul>
    {/if}
    <div class="add">
      <Button icon="folderAdd" testid="plugin-folder-add" onclick={() => void addPluginFolder()}>
        {t("plugins.folders.add")}
      </Button>
      <span class="hint">{t("plugins.folders.rescan_hint")}</span>
    </div>
  </section>
</div>

<style>
  .folders {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-4);
  }

  section {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
  }

  section > h3 {
    margin: 0;
  }

  ul {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-md);
    background: var(--pv-bg-panel);
    list-style: none;
    overflow: hidden;
  }

  li {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-height: var(--pv-control-h-lg);
    padding: 0 var(--pv-space-1) 0 var(--pv-space-3);
    color: var(--pv-text-tertiary);
  }

  li + li {
    border-top: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--pv-text-primary);
    font-family: var(--pv-font-mono);
    font-size: var(--pv-text-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Badges end the row at the same inset as the remove button. */
  li > :global(.pv-badge) {
    margin-right: var(--pv-space-2);
  }

  .add {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-3);
  }
</style>
