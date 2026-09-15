<script lang="ts">
  import { onMount } from "svelte";
  import AnalyzerPanel from "./lib/analyzer/AnalyzerPanel.svelte";
  import { analyzerState, applyAnalyzerPrefs } from "./lib/analyzer/analyzer.svelte";
  import { applyDiagnosticsPrefs } from "./lib/analyzer/diagnostics.svelte";
  import SpectrumInspector from "./lib/analyzer/SpectrumInspector.svelte";
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
  import ShortcutsDialog from "./lib/help/ShortcutsDialog.svelte";
  import HelpMenu from "./lib/help/HelpMenu.svelte";
  import { t } from "./lib/i18n";
  import { getAppInfo, settingsStartupNoticeTake } from "./lib/ipc/commands";
  import { attachKeymap } from "./lib/shortcuts";
  import EditorView from "./lib/layout/EditorView.svelte";
  import {
    applyLayoutPrefs,
    DEFAULT_LAYOUT_PREFS,
    layoutState,
    setDockHeightPx,
    setDockTab,
    setMarkersCollapsed,
    setMarkersWidthPx,
    setRackCollapsed,
    setRackWidthPx,
  } from "./lib/layout/layoutSettings.svelte";
  import MarkersProperties from "./lib/layout/MarkersProperties.svelte";
  import MeterBridge from "./lib/layout/MeterBridge.svelte";
  import Splitter from "./lib/layout/Splitter.svelte";
  import { clampColumnWidthPx, clampDockHeightPx, stepSizePx } from "./lib/layout/splitterMath";
  import { fitSideColumns } from "./lib/layout/fitColumns";
  import { applyThemePref } from "./lib/theme/theme.svelte";
  import { Icon } from "./lib/ui";
  import Toolbar from "./lib/layout/Toolbar.svelte";
  import ViewMenu from "./lib/layout/ViewMenu.svelte";
  import LoudnessPanel from "./lib/loudness/LoudnessPanel.svelte";
  import { initLoudness } from "./lib/loudness/loudness.svelte";
  import { attachMenuBarMnemonics } from "./lib/menu/menubar.svelte";
  import MenuBar from "./lib/menu/MenuBar.svelte";
  import NormalizeDialog from "./lib/normalize/NormalizeDialog.svelte";
  import NormalizeLufsDialog from "./lib/normalize/NormalizeLufsDialog.svelte";
  import EffectsMenu from "./lib/rack/EffectsMenu.svelte";
  import BakeDialogs from "./lib/rack/BakeDialogs.svelte";
  import ManagePresetsDialog from "./lib/rack/ManagePresetsDialog.svelte";
  import RackPanel from "./lib/rack/RackPanel.svelte";
  import { initNrCapture } from "./lib/rack/nrCapture.svelte";
  import NoticeHost from "./lib/notices/NoticeHost.svelte";
  import { initMarkers } from "./lib/markers/markers.svelte";
  import PreferencesDialog from "./lib/preferences/PreferencesDialog.svelte";
  import InstallPluginDialog from "./lib/plugins/InstallPluginDialog.svelte";
  import PluginManagerDialog from "./lib/plugins/PluginManagerDialog.svelte";
  import UninstallPluginDialog from "./lib/plugins/UninstallPluginDialog.svelte";
  import { initPlugins } from "./lib/plugins/plugins.svelte";
  import RecoveryDialog from "./lib/recovery/RecoveryDialog.svelte";
  import { initRecovery } from "./lib/recovery/recovery.svelte";
  import CalibrationDialog from "./lib/record/CalibrationDialog.svelte";
  import LowDiskDialog from "./lib/record/LowDiskDialog.svelte";
  import NewRecordingDialog from "./lib/record/NewRecordingDialog.svelte";
  import { setRendererPreference } from "./lib/state/rendererPref.svelte";
  import { initEdit } from "./lib/state/edit.svelte";
  import { initNormalize } from "./lib/state/normalize.svelte";
  import { initNormalizeLufs } from "./lib/state/normalizeLufs.svelte";
  import { initNotices, pushNotice } from "./lib/state/notices.svelte";
  import { initRecord } from "./lib/state/record.svelte";
  import { applySpectralDefaults, initSpectral } from "./lib/state/spectral.svelte";
  import { loadSettings, settingsState } from "./lib/state/settings.svelte";
  import { initTransport } from "./lib/state/transport.svelte";
  import TourOverlay from "./lib/tour/TourOverlay.svelte";
  import { armWelcomeOffer } from "./lib/tour/tour.svelte";
  import WelcomeOffer from "./lib/tour/WelcomeOffer.svelte";

  let version = $state("");

  onMount(async () => {
    const info = await getAppInfo();
    version = info.version;
  });

  /**
   * H-24: the resizable app shell. `layout` mirrors `Settings.layout` (item 3, debounced
   * persistence lives in `layoutSettings.svelte.ts`); `mainAreaWidthPx`/`mainAreaHeightPx` are
   * this component's own measurement of the space available below the toolbar (item 1's "main
   * area is a vertical split: workspace ... above the bottom dock"), used to clamp every
   * splitter's value against the *current* window size on every render — a value saved on a
   * large monitor never wedges a smaller one (item 3's own note in `settings.rs`).
   */
  const layout = layoutState();

  let mainAreaEl: HTMLElement | undefined = $state();
  let mainAreaWidthPx = $state(0);
  let mainAreaHeightPx = $state(0);

  $effect(() => {
    const el = mainAreaEl;
    if (!el) {
      mainAreaWidthPx = 0;
      mainAreaHeightPx = 0;
      return;
    }
    mainAreaWidthPx = el.clientWidth;
    mainAreaHeightPx = el.clientHeight;
    if (typeof ResizeObserver === "undefined") {
      return;
    }
    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        mainAreaWidthPx = Math.max(0, Math.round(entry.contentRect.width));
        mainAreaHeightPx = Math.max(0, Math.round(entry.contentRect.height));
      }
    });
    ro.observe(el);
    return () => ro.disconnect();
  });

  const MARKERS_MIN_PX = 180;
  const RACK_MIN_PX = 200;
  const SIDE_COLUMN_MAX_PX = 480;

  // A column never eats more than 35% of the main area, so the editor always keeps most of the
  // width even on a narrow window (SPEC-007 §2.1-adjacent "nothing overlaps or clips" concern,
  // item 9).
  const sideColumnMaxPx = $derived(
    mainAreaWidthPx > 0 ? Math.min(SIDE_COLUMN_MAX_PX, mainAreaWidthPx * 0.35) : SIDE_COLUMN_MAX_PX,
  );
  const markersWidthPx = $derived(
    clampColumnWidthPx(layout.markersWidthPx, MARKERS_MIN_PX, sideColumnMaxPx),
  );
  const rackWidthPx = $derived(clampColumnWidthPx(layout.rackWidthPx, RACK_MIN_PX, sideColumnMaxPx));
  // H-25: the rendered widths — the editor keeps a minimum and the side panels shrink (Markers
  // drops out first) so the Rack never falls off the right edge of a narrow window.
  const fit = $derived(
    fitSideColumns({
      mainPx: mainAreaWidthPx,
      markersPx: layout.markersCollapsed ? 0 : markersWidthPx,
      rackPx: layout.rackCollapsed ? 0 : rackWidthPx,
    }),
  );
  const dockHeightPx = $derived(clampDockHeightPx(mainAreaHeightPx, layout.dockHeightPx));

  function onMarkersDrag(deltaPx: number): void {
    if (layout.markersCollapsed) {
      setMarkersCollapsed(false);
    }
    setMarkersWidthPx(clampColumnWidthPx(markersWidthPx + deltaPx, MARKERS_MIN_PX, sideColumnMaxPx));
  }

  function onMarkersStep(direction: 1 | -1): void {
    setMarkersWidthPx(stepSizePx(markersWidthPx, direction, MARKERS_MIN_PX, sideColumnMaxPx));
  }

  // The Rack splitter sits on Rack's *left* edge: dragging right shrinks it, dragging left grows
  // it (splitterMath.ts's "reverse" convention) — both the drag delta and the keyboard direction
  // are negated so "ArrowRight"/dragging right always visually moves the divider right.
  function onRackDrag(deltaPx: number): void {
    if (layout.rackCollapsed) {
      setRackCollapsed(false);
    }
    setRackWidthPx(clampColumnWidthPx(rackWidthPx - deltaPx, RACK_MIN_PX, sideColumnMaxPx));
  }

  function onRackStep(direction: 1 | -1): void {
    setRackWidthPx(stepSizePx(rackWidthPx, direction === 1 ? -1 : 1, RACK_MIN_PX, sideColumnMaxPx));
  }

  // The dock splitter sits above the dock: dragging/stepping "down" shrinks the dock (grows the
  // workspace above it), matching the physical direction of the drag.
  function onDockDrag(deltaPx: number): void {
    setDockHeightPx(clampDockHeightPx(mainAreaHeightPx, dockHeightPx - deltaPx));
  }

  function onDockStep(direction: 1 | -1): void {
    setDockHeightPx(clampDockHeightPx(mainAreaHeightPx, dockHeightPx - direction * 16));
  }

  onMount(() => {
    // Features register their action handlers (S1-01: transport); unhandled keys are no-ops.
    return attachKeymap();
  });

  // H-19: Alt+letter mnemonics for the menu bar (File/Edit/View/Effects/Help), independent of the
  // per-action keymap above.
  onMount(() => attachMenuBarMnemonics());

  // T-809: the plugin manager's scan progress (the start-up scan too) and the crash flags the
  // rack's flagged-slot affordance reads.
  onMount(() => {
    let disposed = false;
    let teardown: (() => void) | null = null;
    void initPlugins().then((cleanup) => {
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
      // H-42 (SPEC-007 §8): peak labels, diagnostics panel, Spectrum Inspector settings.
      applyDiagnosticsPrefs(current.analyzer_diagnostics);
      // H-19 (ADR-009 §4): View → Renderer's persisted choice, seeded before any waveform/
      // spectral view mounts. `?? "auto"` tolerates a mocked/pre-H-19 settings object in tests.
      setRendererPreference(current.renderer_preference ?? "auto");
      // H-25: Preferences → Appearance (dark by default; `?? "dark"` tolerates mocked settings).
      applyThemePref(current.theme ?? "dark");
      // H-24 (item 3): the app-shell layout (column widths, dock height, collapsed panels, dock
      // tab), seeded before the splitters' first render. `?? DEFAULT_LAYOUT_PREFS` tolerates a
      // mocked/pre-H-24 settings object in tests, same convention as `renderer_preference`.
      applyLayoutPrefs(current.layout ?? DEFAULT_LAYOUT_PREFS);
      // T-709: the first-run Welcome tour offer (it waits for the crash-recovery check itself).
      armWelcomeOffer(current.tours);
    });
  });

  // T-703 (settings file robustness): one-shot — `Some` only right after a corrupt settings
  // file was replaced by defaults at startup.
  onMount(() => {
    void settingsStartupNoticeTake().then((notice) => {
      if (notice) {
        pushNotice(notice);
      }
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
  <!--
    H-24 items 1/2/9: an explicit `main-area` row (menu, toolbar, main-area — three grid rows for
    three children, fixing the old "5 children on 4 row tracks" mismatch) holding a vertical
    split: the workspace (Markers | editor | Rack, each column individually resizable) above the
    bottom dock (Meters/Analyzer or Loudness, tabbed so it can't squeeze the editor). Every
    splitter clamps against `mainAreaWidthPx`/`mainAreaHeightPx` (measured here), so a size saved
    on a larger window never wedges a smaller one.
  -->
  <div class="main-area" data-testid="main-area" bind:this={mainAreaEl}>
    <div class="workspace" data-testid="workspace">
      {#if !layout.markersCollapsed && fit.markersPx > 0}
        <div class="col col-markers" data-testid="col-markers" style={`width: ${fit.markersPx}px`}>
          <MarkersProperties />
        </div>
      {/if}
      <Splitter
        orientation="vertical"
        ariaLabel={t("layout.splitter.markers")}
        testid="splitter-markers"
        collapsible
        collapsed={layout.markersCollapsed}
        onDrag={onMarkersDrag}
        onReset={() => setMarkersWidthPx(DEFAULT_LAYOUT_PREFS.markers_width_px)}
        onStep={onMarkersStep}
        onToggleCollapse={() => setMarkersCollapsed(!layout.markersCollapsed)}
      />
      <EditorView />
      <Splitter
        orientation="vertical"
        ariaLabel={t("layout.splitter.rack")}
        testid="splitter-rack"
        collapsible
        collapsed={layout.rackCollapsed}
        onDrag={onRackDrag}
        onReset={() => setRackWidthPx(DEFAULT_LAYOUT_PREFS.rack_width_px)}
        onStep={onRackStep}
        onToggleCollapse={() => setRackCollapsed(!layout.rackCollapsed)}
      />
      {#if !layout.rackCollapsed && fit.rackPx > 0}
        <div class="col col-rack" data-testid="col-rack" style={`width: ${fit.rackPx}px`}>
          <RackPanel />
        </div>
      {/if}
    </div>
    <Splitter
      orientation="horizontal"
      ariaLabel={t("layout.splitter.dock")}
      testid="splitter-dock"
      onDrag={onDockDrag}
      onReset={() => setDockHeightPx(DEFAULT_LAYOUT_PREFS.dock_height_px)}
      onStep={onDockStep}
    />
    <div class="dock" data-testid="bottom-dock" data-tour="dock" style={`height: ${dockHeightPx}px`}>
      <div class="dock-tabs" role="tablist" aria-label={t("panel.meters.title")}>
        <button
          type="button"
          role="tab"
          aria-selected={layout.dockTab === "meters"}
          class:active={layout.dockTab === "meters"}
          data-testid="dock-tab-meters"
          onclick={() => setDockTab("meters")}
        >
          <Icon name="meters" size="sm" />
          {t("layout.dock.tab_meters")}
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={layout.dockTab === "loudness"}
          class:active={layout.dockTab === "loudness"}
          data-testid="dock-tab-loudness"
          onclick={() => setDockTab("loudness")}
        >
          <Icon name="loudness" size="sm" />
          {t("layout.dock.tab_loudness")}
        </button>
      </div>
      <div class="dock-body">
        <!-- Both tabs stay mounted (H-24 item 10: never remounted just by switching tabs, so the
             meter bridge/analyzer's own state and subscriptions are never disturbed) — only
             hidden with the `hidden` attribute. -->
        <div class="meters-row" data-testid="dock-tab-panel-meters" hidden={layout.dockTab !== "meters"}>
          <MeterBridge />
          {#if analyzerState().visible}
            <AnalyzerPanel />
          {/if}
        </div>
        <div class="loudness-tab" data-testid="dock-tab-panel-loudness" hidden={layout.dockTab !== "loudness"}>
          <LoudnessPanel />
        </div>
      </div>
    </div>
  </div>
</div>
<NoticeHost />
<RecoveryDialog />
<PreferencesDialog />
<PluginManagerDialog />
<InstallPluginDialog />
<UninstallPluginDialog />
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
<BakeDialogs />
<ManagePresetsDialog />
<AboutDialog {version} />
<ShortcutsDialog />
<CalibrationDialog />
<SpectrumInspector />
<WelcomeOffer />
<TourOverlay />

<style>
  .shell {
    display: grid;
    /* H-24: exactly three row tracks for exactly three children (menu bar, toolbar, main-area) —
       the old `auto auto 1fr auto` had four tracks for what became five children (the Loudness
       panel and the bottom dock both landed in/after the `1fr` row), so the dock's height was
       never actually bounded and could grow to fill the window (item 1's diagnosis). */
    grid-template-rows: auto auto 1fr;
    /* H-25: one column that may shrink below its content's min-content — without this the
       toolbar's width became the grid's width and pushed the Rack off-screen. */
    grid-template-columns: minmax(0, 1fr);
    height: 100vh;
    overflow: hidden;
    background: var(--pv-bg-app);
  }

  /* H-24 items 1/2: the main area is itself a vertical split — the workspace (item 1: "flex,
     min 40% of the window") above the bottom dock (item 1: "default ~240px, min 120, max 60%"),
     with a draggable horizontal splitter between them. Both are always given an explicit,
     definite size (`workspace` via flex, `dock` via its own `height` from `clampDockHeightPx`)
     — never sized from their content (item 4: no more feedback loop through the analyzer's
     canvas). */
  .main-area {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .workspace {
    display: flex;
    flex: 1;
    min-height: 40%;
    min-width: 0;
  }

  .col {
    flex: none;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* The Markers/Rack panels' own root elements (`<aside class="markers-properties">`/`<aside
     class="rack">`) used to be direct CSS Grid children, stretched to the row's full height by
     Grid's own default `align-items: stretch`. Wrapping them in a sized `.col` for the resizable
     layout (item 2) loses that for free, so it's restored explicitly here rather than editing
     either panel's own file (kept out of scope: T-802 touches `RackPanel.svelte` concurrently). */
  .col-markers :global(.markers-properties),
  .col-rack :global(.rack) {
    flex: 1;
    min-height: 0;
  }

  /* H-24 item 1: a fixed height from the layout store (`clampDockHeightPx`), never `auto` —
     this is what actually fixes the "bottom dock fills the window" bug; every canvas inside it
     (the analyzer's) now sits in a container with a definite size. */
  .dock {
    flex: none;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--pv-bg-panel);
  }

  /* H-24 item 1 / H-25: dock tabs in the system's underline style, 32 px, aligned with every
     panel header. */
  .dock-tabs {
    display: flex;
    flex: none;
    align-items: stretch;
    gap: var(--pv-space-1);
    height: var(--pv-panel-header-h);
    padding-inline: var(--pv-space-2);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
  }

  .dock-tabs button {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: calc(var(--pv-space-1) + var(--pv-space-half));
    padding-inline: var(--pv-space-2);
    border: none;
    background: transparent;
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
    font-weight: var(--pv-weight-medium);
    cursor: default;
  }

  .dock-tabs button::after {
    content: "";
    position: absolute;
    inset: auto var(--pv-space-2) -1px;
    height: 2px;
    border-radius: 1px;
    background: transparent;
  }

  .dock-tabs button:hover {
    color: var(--pv-text-primary);
  }

  .dock-tabs button.active {
    color: var(--pv-text-primary);
  }

  .dock-tabs button.active::after {
    background: var(--pv-accent);
  }

  .dock-tabs button:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: -4px;
    border-radius: var(--pv-radius-sm);
  }

  /* H-26: the inactive tab panel is `hidden`, but `.meters-row`/`.loudness-tab` set their own
     `display`, which beats the attribute's UA style — the Loudness tab used to show the meters. */
  .dock-body > [hidden] {
    display: none;
  }

  .dock-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  /* T-208: the analyzer panel sits to the right of the meter bridge (SPEC-007 §2.9). */
  .meters-row {
    display: flex;
    min-height: 0;
    height: 100%;
  }
</style>
