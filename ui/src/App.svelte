<script lang="ts">
  import { onMount } from "svelte";
  import DocumentMenu from "./lib/document/DocumentMenu.svelte";
  import SaveAsDialog from "./lib/document/SaveAsDialog.svelte";
  import UnsavedChangesDialog from "./lib/document/UnsavedChangesDialog.svelte";
  import { initDocument } from "./lib/document/document.svelte";
  import { getAppInfo } from "./lib/ipc/commands";
  import { attachKeymap } from "./lib/keymap";
  import EditorView from "./lib/layout/EditorView.svelte";
  import MarkersProperties from "./lib/layout/MarkersProperties.svelte";
  import MeterBridge from "./lib/layout/MeterBridge.svelte";
  import RackPanel from "./lib/layout/RackPanel.svelte";
  import Toolbar from "./lib/layout/Toolbar.svelte";
  import NoticeHost from "./lib/notices/NoticeHost.svelte";
  import { loadSettings } from "./lib/state/settings.svelte";
  import { initTransport } from "./lib/state/transport.svelte";

  let version = $state("");

  onMount(async () => {
    const info = await getAppInfo();
    version = info.version;
  });

  onMount(() => {
    // Features register their action handlers (S1-01: transport); unhandled keys are no-ops.
    return attachKeymap();
  });

  onMount(() => {
    void loadSettings();
  });

  onMount(() => {
    let disposed = false;
    let teardown: (() => void) | null = null;
    void initTransport().then((cleanup) => {
      if (disposed) {
        cleanup();
      } else {
        teardown = cleanup;
      }
    });
    return () => {
      disposed = true;
      teardown?.();
    };
  });

  onMount(() => {
    let disposed = false;
    let teardown: (() => void) | null = null;
    void initDocument().then((cleanup) => {
      if (disposed) {
        cleanup();
      } else {
        teardown = cleanup;
      }
    });
    return () => {
      disposed = true;
      teardown?.();
    };
  });
</script>

<div class="shell">
  <DocumentMenu />
  <Toolbar {version} />
  <div class="workspace">
    <MarkersProperties />
    <EditorView />
    <RackPanel />
  </div>
  <MeterBridge />
</div>
<NoticeHost />
<UnsavedChangesDialog />
<SaveAsDialog />

<style>
  .shell {
    display: grid;
    grid-template-rows: auto auto 1fr auto;
    height: 100vh;
  }

  .workspace {
    display: grid;
    grid-template-columns: minmax(200px, 240px) 1fr minmax(240px, 300px);
    min-height: 0;
  }
</style>
