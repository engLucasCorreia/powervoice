<script lang="ts">
  import { onMount } from "svelte";
  import AnalyzerPanel from "./lib/analyzer/AnalyzerPanel.svelte";
  import { analyzerState, applyAnalyzerPrefs } from "./lib/analyzer/analyzer.svelte";
  import ChannelChoiceDialog from "./lib/document/ChannelChoiceDialog.svelte";
  import ClipPromptDialog from "./lib/document/ClipPromptDialog.svelte";
  import ConfirmDialog from "./lib/document/ConfirmDialog.svelte";
  import DocumentMenu from "./lib/document/DocumentMenu.svelte";
  import ImportProgressBar from "./lib/document/ImportProgressBar.svelte";
  import RecentMissingDialog from "./lib/document/RecentMissingDialog.svelte";
  import SaveAsDialog from "./lib/document/SaveAsDialog.svelte";
  import UnsavedChangesDialog from "./lib/document/UnsavedChangesDialog.svelte";
  import { initDocument } from "./lib/document/document.svelte";
  import { initRecentFiles } from "./lib/document/recentFiles.svelte";
  import EditMenu from "./lib/edit/EditMenu.svelte";
  import ExportDialog from "./lib/export/ExportDialog.svelte";
  import AboutDialog from "./lib/help/AboutDialog.svelte";
  import HelpMenu from "./lib/help/HelpMenu.svelte";
  import { t } from "./lib/i18n";
  import { getAppInfo } from "./lib/ipc/commands";
  import { attachKeymap } from "./lib/keymap";
  import EditorView from "./lib/layout/EditorView.svelte";
  import MarkersProperties from "./lib/layout/MarkersProperties.svelte";
  import MeterBridge from "./lib/layout/MeterBridge.svelte";
  import Toolbar from "./lib/layout/Toolbar.svelte";
  import ViewMenu from "./lib/layout/ViewMenu.svelte";
  import LoudnessPanel from "./lib/loudness/LoudnessPanel.svelte";
  import { initLoudness } from "./lib/loudness/loudness.svelte";
  import { attachMenuBarMnemonics } from "./lib/menu/menubar.svelte";
  import MenuBar from "./lib/menu/MenuBar.svelte";
  import NormalizeDialog from "./lib/normalize/NormalizeDialog.svelte";
  import NormalizeLufsDialog from "./lib/normalize/NormalizeLufsDialog.svelte";
  import EffectsMenu from "./lib/rack/EffectsMenu.svelte";
  import RackPanel from "./lib/rack/RackPanel.svelte";
  import { initNrCapture } from "./lib/rack/nrCapture.svelte";
  import NoticeHost from "./lib/notices/NoticeHost.svelte";
  import { initMarkers } from "./lib/markers/markers.svelte";
  import PreferencesDialog from "./lib/preferences/PreferencesDialog.svelte";
  import RecoveryDialog from "./lib/recovery/RecoveryDialog.svelte";
  import { initRecovery } from "./lib/recovery/recovery.svelte";
  import CalibrationDialog from "./lib/record/CalibrationDialog.svelte";
  import LowDiskDialog from "./lib/record/LowDiskDialog.svelte";
  import NewRecordingDialog from "./lib/record/NewRecordingDialog.svelte";
  import { setRendererPreference } from "./lib/state/rendererPref.svelte";
  import { initEdit } from "./lib/state/edit.svelte";
  import { initNormalize } from "./lib/state/normalize.svelte";
  import { initNormalizeLufs } from "./lib/state/normalizeLufs.svelte";
  import { initNotices } from "./lib/state/notices.svelte";
  import { initRecord } from "./lib/state/record.svelte";
  import { applySpectralDefaults, initSpectral } from "./lib/state/spectral.svelte";
  import { loadSettings, settingsState } from "./lib/state/settings.svelte";
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

  // H-19: Alt+letter mnemonics for the menu bar (File/Edit/View/Effects/Help), independent of the
  // per-action keymap above.
  onMount(() => attachMenuBarMnemonics());

  // H-12 (A-014): once settings load, seed the spectral pane's display settings from the app's
  // last-used defaults (a document's own sidecar `spectral_view`, applied later by
  // `document.svelte.ts` on open, overrides this).
  onMount(() => {
    void loadSettings().then(() => {
      const current = settingsState().current;
      if (!current) {
        return;
      }
      applySpectralDefaults(current.spectral_defaults);
      // H-16 (SPEC-007 §2.9): visibility/response/peak-hold seeded before the panel ever mounts.
      applyAnalyzerPrefs({
        visible: current.analyzer_visible,
        response: current.analyzer_response,
        peakHold: current.analyzer_peak_hold,
      });
      // H-19 (ADR-009 §4): View → Renderer's persisted choice, seeded before any waveform/
      // spectral view mounts. `?? "auto"` tolerates a mocked/pre-H-19 settings object in tests.
      setRendererPreference(current.renderer_preference ?? "auto");
    });
  });

  // T-301 (SPEC-004 §2.7): offer recoverable sessions before any document opens.
  onMount(() => {
    void initRecovery();
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

  // T-306: File → Open Recent (SPEC-018 §2.12).
  onMount(() => {
    let disposed = false;
    let teardown: (() => void) | null = null;
    void initRecentFiles().then((cleanup) => {
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

  // S2-02: peak normalize favorites (toolbar buttons, Effects → Favorites ▸, Normalize… dialog).
  onMount(() => initNormalize());

  // S4-01: LUFS normalize favorites (toolbar buttons, Effects → Favorites ▸, Normalize (LUFS)…
  // dialog).
  onMount(() => initNormalizeLufs());

  // S4-01: the Loudness panel's analysis job (job_progress/loudness_report events).
  onMount(() => initLoudness());

  // S2-03: markers (M, Ctrl+0, Ctrl+Alt+→/←, and the Markers panel's list/add/rename/delete).
  onMount(() => {
    let disposed = false;
    let teardown: (() => void) | null = null;
    void initMarkers().then((cleanup) => {
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
  // S3-06: Capture Noise Print (Shift+P) and its job-progress listener.
  onMount(() => initNrCapture());

  // T-207: the spectral pane's Shift+D toggle.
  onMount(() => initSpectral());
</script>

<div class="shell">
  <MenuBar label={t("menu.bar")}>
    <DocumentMenu />
    <EditMenu />
    <ViewMenu />
    <EffectsMenu />
    <HelpMenu />
  </MenuBar>
  <Toolbar {version} />
  <div class="workspace">
    <MarkersProperties />
    <EditorView />
    <RackPanel />
  </div>
  <LoudnessPanel />
  <div class="bottom-dock">
    <MeterBridge />
    {#if analyzerState().visible}
      <AnalyzerPanel />
    {/if}
  </div>
</div>
<NoticeHost />
<RecoveryDialog />
<PreferencesDialog />
<UnsavedChangesDialog />
<ConfirmDialog />
<RecentMissingDialog />
<SaveAsDialog />
<ChannelChoiceDialog />
<ClipPromptDialog />
<ImportProgressBar />
<ExportDialog />
<NewRecordingDialog />
<LowDiskDialog />
<NormalizeDialog />
<NormalizeLufsDialog />
<AboutDialog {version} />
<CalibrationDialog />

<style>
  .shell {
    display: grid;
    /* H-19: menu bar, toolbar, the flexible workspace, then the bottom dock (was 5 separate flat
       menu rows + toolbar before, with the tracks below no longer lined up with the right
       children — down to one real menu bar row now). */
    grid-template-rows: auto auto 1fr auto;
    height: 100vh;
  }

  .workspace {
    display: grid;
    grid-template-columns: minmax(200px, 240px) 1fr minmax(240px, 300px);
    min-height: 0;
  }

  /* T-208: the analyzer panel sits to the right of the meter bridge (SPEC-007 §2.9). */
  .bottom-dock {
    display: flex;
    min-height: 0;
  }
</style>
