import type { DockTabPref, LayoutPrefsDto } from "../ipc/bindings";
import { saveSettings } from "../state/settings.svelte";

/**
 * The app shell's persisted layout (H-24 item 3: column widths, dock height, collapsed panels,
 * the active dock tab — `Settings.layout`, debounced). Same shape as `state/waveformView.svelte.ts`
 * / `state/spectral.svelte.ts`: a module-level `$state` singleton seeded once from `Settings` at
 * startup (`applyLayoutPrefs`, called from `App.svelte`'s `onMount` after `loadSettings()`, same
 * pattern as `applySpectralDefaults`/`applyAnalyzerPrefs`), read/written by `App.svelte`'s
 * splitters and collapse buttons, and saved back through `saveSettings` on a debounce so a drag
 * gesture doesn't spam `settings_set`.
 *
 * These are the *raw* persisted values — `App.svelte` still clamps them against the window's
 * current size on every render (`splitterMath.ts`), so a value saved on a large monitor never
 * wedges a smaller one, and a stale/out-of-range value on disk never needs a migration.
 */

export const DEFAULT_LAYOUT_PREFS: LayoutPrefsDto = {
  markers_width_px: 240,
  rack_width_px: 280,
  dock_height_px: 240,
  markers_collapsed: false,
  rack_collapsed: false,
  dock_tab: "meters",
};

let state = $state<LayoutPrefsDto>({ ...DEFAULT_LAYOUT_PREFS });

export interface LayoutState {
  readonly markersWidthPx: number;
  readonly rackWidthPx: number;
  readonly dockHeightPx: number;
  readonly markersCollapsed: boolean;
  readonly rackCollapsed: boolean;
  readonly dockTab: DockTabPref;
}

export function layoutState(): LayoutState {
  return {
    get markersWidthPx() {
      return state.markers_width_px;
    },
    get rackWidthPx() {
      return state.rack_width_px;
    },
    get dockHeightPx() {
      return state.dock_height_px;
    },
    get markersCollapsed() {
      return state.markers_collapsed;
    },
    get rackCollapsed() {
      return state.rack_collapsed;
    },
    get dockTab() {
      return state.dock_tab;
    },
  };
}

/** Seeds the store from `Settings.layout` at startup (App.svelte, once settings load; `?? ` in
 * the caller tolerates a mocked/pre-H-24 settings object in tests, same convention as H-19's
 * `renderer_preference ?? "auto"`). */
export function applyLayoutPrefs(prefs: LayoutPrefsDto): void {
  state = { ...prefs };
}

const PERSIST_DEBOUNCE_MS = 250;
let persistTimer: ReturnType<typeof setTimeout> | null = null;

function schedulePersist(): void {
  if (persistTimer !== null) {
    clearTimeout(persistTimer);
  }
  persistTimer = setTimeout(() => {
    persistTimer = null;
    void saveSettings({ layout: { ...state } });
  }, PERSIST_DEBOUNCE_MS);
}

export function setMarkersWidthPx(px: number, persist = true): void {
  state = { ...state, markers_width_px: px };
  if (persist) {
    schedulePersist();
  }
}

export function setRackWidthPx(px: number, persist = true): void {
  state = { ...state, rack_width_px: px };
  if (persist) {
    schedulePersist();
  }
}

export function setDockHeightPx(px: number, persist = true): void {
  state = { ...state, dock_height_px: px };
  if (persist) {
    schedulePersist();
  }
}

export function setMarkersCollapsed(collapsed: boolean): void {
  state = { ...state, markers_collapsed: collapsed };
  schedulePersist();
}

export function setRackCollapsed(collapsed: boolean): void {
  state = { ...state, rack_collapsed: collapsed };
  schedulePersist();
}

export function setDockTab(tab: DockTabPref): void {
  state = { ...state, dock_tab: tab };
  schedulePersist();
}

/** Test/teardown helper (same convention as every other feature store's `reset*ForTest`). */
export function resetLayoutForTest(): void {
  if (persistTimer !== null) {
    clearTimeout(persistTimer);
    persistTimer = null;
  }
  state = { ...DEFAULT_LAYOUT_PREFS };
}
