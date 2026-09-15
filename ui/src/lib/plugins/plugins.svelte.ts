import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { t } from "../i18n";
import { currentPlatform } from "../ui/platform";
import type {
  BlockCauseDto,
  EventName,
  InstalledEffectDto,
  InstallFailureDto,
  IpcError,
  PluginEntryDto,
  PluginFoldersDto,
  PluginScanProgressDto,
  PluginScanSummaryDto,
} from "../ipc/bindings";
import {
  pluginsAddFolder,
  pluginsBlock,
  pluginsClearFlag,
  pluginsFolders,
  pluginsInstall,
  pluginsList,
  pluginsRemoveFolder,
  pluginsRescan,
  pluginsReveal,
  pluginsSetEnabled,
  pluginsUnblock,
  pluginsUninstall,
} from "../ipc/commands";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";
import { rowKey, type SortDir, type SortKey } from "./pluginList";

/**
 * Plugin manager store (T-809): the manager dialog (Effects → Manage Plugins…, Preferences →
 * Plugins, a flagged rack slot), its list with search/sort, the actions (enable/disable,
 * block/unblock, clear a crash flag, reveal), rescans with live progress from
 * `plugin_scan_progress` (also the start-up scan), custom folders, and the "Install module…"
 * flow (native picker → copy + scan → success, name collision to confirm, or failure).
 */

export type ManagerTab = "plugins" | "folders";

export interface ScanProgress {
  done: number;
  total: number;
  currentPath: string | null;
}

export type InstallState =
  | { phase: "idle" }
  | { phase: "installing"; source: string }
  | {
      phase: "collision";
      source: string;
      target: string;
      /** T-805: a module package's installed version and the package's (else `null`). */
      installedVersion: string | null;
      newVersion: string | null;
    }
  | { phase: "installed"; source: string; target: string; replaced: boolean; effects: InstalledEffectDto[] }
  | {
      phase: "failed";
      source: string;
      code: InstallFailureDto;
      detail: string;
      blocklisted: boolean;
      cause: BlockCauseDto | null;
    };

/**
 * What "Install module…" accepts: the backends that exist (T-803 CLAP, T-806 VST3, T-807 LV2,
 * T-808 JSFX). A VST3 or LV2 bundle is a folder on Linux and Windows: picking any file inside it
 * installs the whole bundle — `ttl` lets the picker show an LV2 bundle's `manifest.ttl`. A JSFX
 * script is a `.jsfx` file (its relative imports come along); REAPER's extensionless scripts are
 * found by scanning their folder instead (Preferences → Plugins → folders). T-805: a PowerVoice
 * module package (`voxmod`).
 */
export const INSTALL_EXTENSIONS: readonly string[] = ["clap", "vst3", "lv2", "ttl", "jsfx", "voxmod"];

let open = $state(false);
let tab = $state<ManagerTab>("plugins");
let focusKey = $state<string | null>(null);
let list = $state<PluginEntryDto[] | null>(null);
let loading = $state(false);
let error = $state(false);
let folders = $state<PluginFoldersDto | null>(null);
let scan = $state<ScanProgress | null>(null);
let lastScan = $state<PluginScanSummaryDto | null>(null);
let rescanning = $state(false);
let query = $state("");
let sortKey = $state<SortKey>("name");
let sortDir = $state<SortDir>("asc");
let pending = $state<string | null>(null);
let install = $state<InstallState>({ phase: "idle" });
/** "Uninstall…" (H-29): the row awaiting confirmation, or `null` when the confirm dialog is
 * closed. `busy`: the IPC call is in flight (the dialog's buttons disable). */
let uninstallPrompt = $state<{ entry: PluginEntryDto; busy: boolean } | null>(null);
/** Crash counts by module id, from the last list (the rack's flagged-slot affordance). */
let crashCounts = $state<Record<string, number>>({});

/** Read-only accessor for components. */
export function pluginsState(): {
  readonly open: boolean;
  readonly tab: ManagerTab;
  readonly focusKey: string | null;
  readonly list: PluginEntryDto[] | null;
  readonly loading: boolean;
  readonly error: boolean;
  readonly folders: PluginFoldersDto | null;
  readonly scan: ScanProgress | null;
  readonly lastScan: PluginScanSummaryDto | null;
  readonly rescanning: boolean;
  readonly query: string;
  readonly sortKey: SortKey;
  readonly sortDir: SortDir;
  readonly pending: string | null;
  readonly install: InstallState;
  readonly uninstallPrompt: { entry: PluginEntryDto; busy: boolean } | null;
} {
  return {
    get open() {
      return open;
    },
    get tab() {
      return tab;
    },
    get focusKey() {
      return focusKey;
    },
    get list() {
      return list;
    },
    get loading() {
      return loading;
    },
    get error() {
      return error;
    },
    get folders() {
      return folders;
    },
    get scan() {
      return scan;
    },
    get lastScan() {
      return lastScan;
    },
    get rescanning() {
      return rescanning;
    },
    get query() {
      return query;
    },
    get sortKey() {
      return sortKey;
    },
    get sortDir() {
      return sortDir;
    },
    get pending() {
      return pending;
    },
    get install() {
      return install;
    },
    get uninstallPrompt() {
      return uninstallPrompt;
    },
  };
}

/** How many times `moduleId` has crashed at runtime (0: not flagged, or unknown). */
export function pluginCrashCount(moduleId: string | null | undefined): number {
  return moduleId ? (crashCounts[moduleId] ?? 0) : 0;
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

function applyList(next: PluginEntryDto[]): void {
  list = next;
  const counts: Record<string, number> = {};
  for (const entry of next) {
    if (entry.status.kind === "flagged" && entry.id) {
      counts[entry.id] = entry.status.crash_count;
    }
  }
  crashCounts = counts;
}

/** Reloads the list (the manager's rows and the rack's crash flags). */
export async function refreshPlugins(): Promise<void> {
  loading = true;
  error = false;
  try {
    const next = await pluginsList();
    applyList(Array.isArray(next) ? next : []);
  } catch (err) {
    error = true;
    report(err);
  } finally {
    loading = false;
  }
}

export async function refreshFolders(): Promise<void> {
  try {
    const next = await pluginsFolders();
    folders = next ?? { install: null, standard: [], custom: [] };
  } catch (err) {
    report(err);
  }
}

/** Opens the manager — on `focus` (a module id, or a row key) when given, e.g. from a flagged
 * rack slot or after an install. */
export function openPluginManager(options: { focus?: string | null; tab?: ManagerTab } = {}): void {
  open = true;
  tab = options.tab ?? "plugins";
  focusKey = options.focus ?? null;
  if (focusKey) {
    query = "";
  }
  void refreshPlugins();
  void refreshFolders();
}

export function closePluginManager(): void {
  open = false;
  focusKey = null;
}

export function setManagerTab(next: ManagerTab): void {
  tab = next;
}

export function setPluginQuery(next: string): void {
  query = next;
}

/** Sorts by `key`; choosing the current key again flips the direction. */
export function sortPluginsBy(key: SortKey): void {
  if (sortKey === key) {
    sortDir = sortDir === "asc" ? "desc" : "asc";
  } else {
    sortKey = key;
    sortDir = "asc";
  }
}

function onScanProgress(payload: PluginScanProgressDto): void {
  if (payload.summary) {
    scan = null;
    lastScan = payload.summary;
    void refreshPlugins();
    return;
  }
  scan = { done: payload.done, total: payload.total, currentPath: payload.current_path };
}

/** Rescans (quick: changed files only; `full`: ignore the cache). Progress arrives as events. */
export async function rescanPlugins(full: boolean): Promise<void> {
  if (rescanning) {
    return;
  }
  rescanning = true;
  lastScan = null;
  scan = scan ?? { done: 0, total: 0, currentPath: null };
  try {
    await pluginsRescan(full);
  } catch (err) {
    report(err);
  } finally {
    rescanning = false;
    scan = null;
  }
  await refreshPlugins();
}

async function withPending(entry: PluginEntryDto, action: () => Promise<unknown>): Promise<boolean> {
  pending = rowKey(entry);
  try {
    await action();
    return true;
  } catch (err) {
    report(err);
    return false;
  } finally {
    pending = null;
  }
}

/** Shows/hides a plugin in Add Module (it still loads for documents that use it). */
export async function setPluginEnabled(entry: PluginEntryDto, enabled: boolean): Promise<void> {
  if (await withPending(entry, () => pluginsSetEnabled(entry.id, enabled))) {
    await refreshPlugins();
  }
}

/** Blocks the plugin's file (every plugin in it); scans skip it until it's unblocked. */
export async function blockPlugin(entry: PluginEntryDto): Promise<void> {
  if (await withPending(entry, () => pluginsBlock(entry.path))) {
    await refreshPlugins();
  }
}

/** Unblocks the file and rescans so it's picked up again. */
export async function unblockPlugin(entry: PluginEntryDto): Promise<void> {
  if (await withPending(entry, () => pluginsUnblock(entry.path))) {
    focusKey = rowKey(entry);
    await rescanPlugins(false);
  }
}

/** Clears a flagged plugin's crash count. */
export async function clearPluginFlag(entry: PluginEntryDto): Promise<void> {
  if (await withPending(entry, () => pluginsClearFlag(entry.id))) {
    await refreshPlugins();
  }
}

/** Shows the plugin's file in the system file manager. */
export async function revealPlugin(entry: PluginEntryDto): Promise<void> {
  await withPending(entry, () => pluginsReveal(entry.path));
}

// --- "Uninstall…" (H-29) ---------------------------------------------------------------------

/** Opens the confirm dialog for `entry` (the row menu only offers this for a file inside the
 * per-user install folder — see `isInInstallFolder`). */
export function requestUninstall(entry: PluginEntryDto): void {
  uninstallPrompt = { entry, busy: false };
}

/** Closes the confirm dialog without removing anything (ignored while the removal is in flight). */
export function cancelUninstall(): void {
  if (uninstallPrompt?.busy) {
    return;
  }
  uninstallPrompt = null;
}

/** The confirm dialog's destructive action: removes the file, then refreshes the list. */
export async function confirmUninstall(): Promise<void> {
  if (!uninstallPrompt || uninstallPrompt.busy) {
    return;
  }
  const { entry } = uninstallPrompt;
  uninstallPrompt = { entry, busy: true };
  try {
    await pluginsUninstall(entry.path);
    uninstallPrompt = null;
    await refreshPlugins();
  } catch (err) {
    uninstallPrompt = null;
    report(err);
  }
}

/** Adds a scan folder picked with the native folder dialog; a rescan follows (with progress). */
export async function addPluginFolder(): Promise<void> {
  const picked = await openDialog({ directory: true, multiple: false, title: t("plugins.folders.pick_title") });
  if (typeof picked !== "string") {
    return;
  }
  try {
    await pluginsAddFolder(picked);
    scan = scan ?? { done: 0, total: 0, currentPath: null };
  } catch (err) {
    report(err);
  }
  await refreshFolders();
}

export async function removePluginFolder(path: string): Promise<void> {
  try {
    await pluginsRemoveFolder(path);
    scan = scan ?? { done: 0, total: 0, currentPath: null };
  } catch (err) {
    report(err);
  }
  await refreshFolders();
}

// --- "Install module…" ----------------------------------------------------------------------

/** Effects → Install Module… / the manager's Install button: native picker, then install. */
export async function startInstall(): Promise<void> {
  if (install.phase === "installing") {
    return;
  }
  // Linux can't select a `.vst3` bundle directory in this picker (H-34): the title and filter
  // name explain that any file inside it (its binary, say) stands for the whole bundle.
  const linux = currentPlatform() === "linux";
  const picked = await openDialog({
    multiple: false,
    directory: false,
    title: t(linux ? "plugins.install.pick_title_linux" : "plugins.install.pick_title"),
    filters: [
      { name: t(linux ? "plugins.install.filter_linux" : "plugins.install.filter"), extensions: [...INSTALL_EXTENSIONS] },
    ],
  });
  if (typeof picked !== "string") {
    return;
  }
  await installFrom(picked, false);
}

/** Installs `source` (`replace`: the user confirmed replacing a same-named plugin). */
export async function installFrom(source: string, replace: boolean): Promise<void> {
  if (install.phase === "installing") {
    return;
  }
  install = { phase: "installing", source };
  try {
    const result = await pluginsInstall(source, replace);
    switch (result.kind) {
      case "installed":
        install = {
          phase: "installed",
          source,
          target: result.path,
          replaced: result.replaced,
          effects: result.effects,
        };
        break;
      case "collision":
        install = {
          phase: "collision",
          source,
          target: result.path,
          installedVersion: result.installed_version,
          newVersion: result.new_version,
        };
        return;
      case "failed":
        install = {
          phase: "failed",
          source,
          code: result.code,
          detail: result.detail,
          blocklisted: result.blocklisted,
          cause: result.cause,
        };
        break;
    }
  } catch (err) {
    install = { phase: "idle" };
    report(err);
    return;
  }
  await refreshPlugins();
}

/** The collision prompt's Replace. */
export async function confirmReplace(): Promise<void> {
  if (install.phase === "collision") {
    await installFrom(install.source, true);
  }
}

/** Closes the install dialog (Cancel on the collision prompt, Done/Close on a result). */
export function closeInstall(): void {
  if (install.phase !== "installing") {
    install = { phase: "idle" };
  }
}

/** The result's "Show in plugin manager": opens the manager on the first new effect (or, after
 * a failure, on the blocklisted file). */
export function showInstallInManager(): void {
  let focus: string | null = null;
  if (install.phase === "installed") {
    focus = install.effects[0]?.id ?? null;
  } else if (install.phase === "failed" && install.blocklisted) {
    focus = `path:${install.source}`;
  }
  install = { phase: "idle" };
  openPluginManager({ focus });
}

/** Starts listening for scan progress and loads the list once (crash flags for the rack). */
export async function initPlugins(): Promise<() => void> {
  const unlisten = await listen<PluginScanProgressDto>("plugin_scan_progress" satisfies EventName, (event) =>
    onScanProgress(event.payload),
  );
  void refreshPlugins();
  return () => {
    if (typeof unlisten === "function") {
      unlisten();
    }
  };
}

/** Test/teardown helper. */
export function resetPluginsForTest(): void {
  open = false;
  tab = "plugins";
  focusKey = null;
  list = null;
  loading = false;
  error = false;
  folders = null;
  scan = null;
  lastScan = null;
  rescanning = false;
  query = "";
  sortKey = "name";
  sortDir = "asc";
  pending = null;
  install = { phase: "idle" };
  uninstallPrompt = null;
  crashCounts = {};
}
