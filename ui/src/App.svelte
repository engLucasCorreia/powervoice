<script lang="ts">
  import { onMount } from "svelte";
  import DocumentMenu from "./lib/document/DocumentMenu.svelte";
  import SaveAsDialog from "./lib/document/SaveAsDialog.svelte";
  import UnsavedChangesDialog from "./lib/document/UnsavedChangesDialog.svelte";
  import { initDocument } from "./lib/document/document.svelte";
  import EditMenu from "./lib/edit/EditMenu.svelte";
  import ExportDialog from "./lib/export/ExportDialog.svelte";
  import { getAppInfo } from "./lib/ipc/commands";
  import { attachKeymap } from "./lib/keymap";
  import EditorView from "./lib/layout/EditorView.svelte";
  import MarkersProperties from "./lib/layout/MarkersProperties.svelte";
  import MeterBridge from "./lib/layout/MeterBridge.svelte";
  import RackPanel from "./lib/layout/RackPanel.svelte";
  import Toolbar from "./lib/layout/Toolbar.svelte";
  import FavoritesMenu from "./lib/normalize/FavoritesMenu.svelte";
  import NoticeHost from "./lib/notices/NoticeHost.svelte";
  import { initEdit } from "./lib/state/edit.svelte";
  import { initNormalize } from "./lib/state/normalize.svelte";
  import { initNotices } from "./lib/state/notices.svelte";
  import { initRecord } from "./lib/state/record.svelte";
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

  // S2-02: the backend's `notice` event (silent/already-normalized notices, and every other
  // notice the app already emits — device loss, recording).
  onMount(() => {
    let disposed = false;
    let teardown: (() => void) | null = null;
    void initNotices().then((cleanup) => {
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

  // S1-04: record panel (arm, Record + Shift+R, input meter).
  onMount(() => initRecord());

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

  // S2-01: cut/copy/paste/delete/trim/silence, undo/redo (Ctrl+X/C/V, Delete, Ctrl+T, Ctrl+Z,
  // Ctrl+Shift+Z) and the Edit menu's history/clipboard state.
  onMount(() => {
    let disposed = false;
    let teardown: (() => void) | null = null;
    void initEdit().then((cleanup) => {
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

  // S2-02: peak normalize favorites (toolbar buttons, Favorites menu, Normalize… dialog).
  onMount(() => initNormalize());
</script>

<div class="shell">
  <DocumentMenu />
  <EditMenu />
  <FavoritesMenu />
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
<ExportDialog />

<style>
  .shell {
    display: grid;
    grid-template-rows: auto auto auto auto 1fr auto;
    height: 100vh;
  }

  .workspace {
    display: grid;
    grid-template-columns: minmax(200px, 240px) 1fr minmax(240px, 300px);
    min-height: 0;
  }
</style>
