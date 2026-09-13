<script lang="ts">
  import { onMount } from "svelte";
  import { getAppInfo } from "./lib/ipc/commands";
  import { attachKeymap } from "./lib/keymap";
  import EditorView from "./lib/layout/EditorView.svelte";
  import MarkersProperties from "./lib/layout/MarkersProperties.svelte";
  import MeterBridge from "./lib/layout/MeterBridge.svelte";
  import RackPanel from "./lib/layout/RackPanel.svelte";
  import Toolbar from "./lib/layout/Toolbar.svelte";
  import NoticeHost from "./lib/notices/NoticeHost.svelte";
  import { loadSettings } from "./lib/state/settings.svelte";

  let version = $state("");

  onMount(async () => {
    const info = await getAppInfo();
    version = info.version;
  });

  onMount(() => {
    // No handlers are registered for any action yet (T-104): transport/recording/marker/history
    // features register their own in later tickets. Until then every key resolves to a no-op.
    return attachKeymap();
  });

  onMount(() => {
    void loadSettings();
  });
</script>

<div class="shell">
  <Toolbar {version} />
  <div class="workspace">
    <MarkersProperties />
    <EditorView />
    <RackPanel />
  </div>
  <MeterBridge />
</div>
<NoticeHost />

<style>
  .shell {
    display: grid;
    grid-template-rows: auto 1fr auto;
    height: 100vh;
  }

  .workspace {
    display: grid;
    grid-template-columns: minmax(200px, 240px) 1fr minmax(240px, 300px);
    min-height: 0;
  }
</style>
