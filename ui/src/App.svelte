<script lang="ts">
  import { onMount } from "svelte";
  import { getAppInfo } from "./lib/ipc/commands";
  import EditorView from "./lib/layout/EditorView.svelte";
  import MarkersProperties from "./lib/layout/MarkersProperties.svelte";
  import MeterBridge from "./lib/layout/MeterBridge.svelte";
  import RackPanel from "./lib/layout/RackPanel.svelte";
  import Toolbar from "./lib/layout/Toolbar.svelte";

  let version = $state("");

  onMount(async () => {
    const info = await getAppInfo();
    version = info.version;
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
