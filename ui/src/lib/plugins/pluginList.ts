/**
 * The plugin manager's list logic (T-809), kept pure so it's tested without the DOM: search,
 * sort, status wording and tones, format badges, port summaries and counts.
 */
import type { MessageKey } from "../i18n";
import type { PluginEntryDto, PluginPortsDto, PluginStatusDto } from "../ipc/bindings";
import type { IconName } from "../ui/icons";

export type SortKey = "name" | "vendor" | "format" | "status";
export type SortDir = "asc" | "desc";
export type StatusKind = PluginStatusDto["kind"];
export type StatusTone = "success" | "neutral" | "danger" | "warning";

/** A stable key per row: the module id, or the path of a blocklisted file with no id. */
export function rowKey(entry: PluginEntryDto): string {
  return entry.id || `path:${entry.path}`;
}

/** The file name of a path (either separator). */
export function fileName(path: string): string {
  return path.split(/[/\\]/).pop() || path;
}

/**
 * The format badge text. Every backend PROMPT §2 names (CLAP now; VST3, LV2, JSFX with
 * T-806–T-808) has its usual spelling; anything else is shown upper-cased.
 */
export function formatLabel(format: string): string {
  switch (format.toLowerCase()) {
    case "clap":
      return "CLAP";
    case "vst3":
      return "VST3";
    case "lv2":
      return "LV2";
    case "jsfx":
      return "JSFX";
    default:
      return format.toUpperCase();
  }
}

/** Status severity for sorting: problems first when sorting by status ascending. */
const STATUS_RANK: Record<StatusKind, number> = {
  blocklisted: 0,
  shadowed: 1,
  flagged: 2,
  disabled: 3,
  ok: 4,
};

export interface StatusInfo {
  kind: StatusKind;
  tone: StatusTone;
  icon: IconName;
  /** The badge word. */
  labelKey: MessageKey;
  /** The detail line under it (blocklist cause, crash count), if any. */
  detailKey: MessageKey | null;
  detailParams: Record<string, string | number>;
}

export function statusInfo(status: PluginStatusDto): StatusInfo {
  switch (status.kind) {
    case "ok":
      return { kind: "ok", tone: "success", icon: "success", labelKey: "plugins.status.ok", detailKey: null, detailParams: {} };
    case "disabled":
      return {
        kind: "disabled",
        tone: "neutral",
        icon: "hidden",
        labelKey: "plugins.status.disabled",
        detailKey: null,
        detailParams: {},
      };
    case "blocklisted":
      return {
        kind: "blocklisted",
        tone: "danger",
        icon: "blocked",
        labelKey: "plugins.status.blocklisted",
        detailKey:
          status.cause === "crashed"
            ? "plugins.cause.crashed"
            : status.cause === "timed_out"
              ? "plugins.cause.timed_out"
              : "plugins.cause.manual",
        detailParams: {},
      };
    case "flagged":
      return {
        kind: "flagged",
        tone: "warning",
        icon: "warning",
        labelKey: "plugins.status.flagged",
        detailKey: status.crash_count === 1 ? "plugins.crashed_once" : "plugins.crashed_times",
        detailParams: { count: status.crash_count },
      };
    case "shadowed":
      return {
        kind: "shadowed",
        tone: "neutral",
        icon: "copy",
        labelKey: "plugins.status.shadowed",
        detailKey: "plugins.shadowed_by",
        detailParams: { path: fileName(status.by) },
      };
  }
}

/** "Mono", "Stereo", or "1 in · 2 out"; `null` when the scan couldn't tell. */
export function portsKey(ports: PluginPortsDto | null): { key: MessageKey; params: Record<string, number> } | null {
  if (!ports) {
    return null;
  }
  const { input_channels: i, output_channels: o } = ports;
  if (i === o && i === 1) {
    return { key: "plugins.ports.mono", params: {} };
  }
  if (i === o && i === 2) {
    return { key: "plugins.ports.stereo", params: {} };
  }
  return { key: "plugins.ports.in_out", params: { input: i, output: o } };
}

/** The words a search matches: name, vendor, version, format, path, id and status. */
function haystack(entry: PluginEntryDto, statusWords: (entry: PluginEntryDto) => string): string {
  return [entry.name, entry.vendor, entry.version, formatLabel(entry.format), entry.path, entry.id, statusWords(entry)]
    .join("\n")
    .toLowerCase();
}

/**
 * Entries matching every whitespace-separated term of `query` (case-insensitive, any field).
 * `statusWords` gives the localized status text so "blocklisted" or "crashed" finds rows too.
 */
export function filterPlugins(
  list: readonly PluginEntryDto[],
  query: string,
  statusWords: (entry: PluginEntryDto) => string = () => "",
): PluginEntryDto[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) {
    return [...list];
  }
  return list.filter((entry) => {
    const text = haystack(entry, statusWords);
    return terms.every((term) => text.includes(term));
  });
}

const collator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });

function compare(a: PluginEntryDto, b: PluginEntryDto, key: SortKey): number {
  switch (key) {
    case "name":
      return collator.compare(a.name, b.name);
    case "vendor":
      return collator.compare(a.vendor, b.vendor);
    case "format":
      return collator.compare(formatLabel(a.format), formatLabel(b.format));
    case "status":
      return STATUS_RANK[a.status.kind] - STATUS_RANK[b.status.kind];
  }
}

/** Sorted by `key`, ties broken by name then path (stable and deterministic). */
export function sortPlugins(list: readonly PluginEntryDto[], key: SortKey, dir: SortDir): PluginEntryDto[] {
  const sign = dir === "asc" ? 1 : -1;
  return [...list].sort(
    (a, b) =>
      sign * compare(a, b, key) || collator.compare(a.name, b.name) || collator.compare(a.path, b.path),
  );
}

export interface PluginCounts {
  total: number;
  disabled: number;
  blocklisted: number;
  flagged: number;
  shadowed: number;
}

export function countPlugins(list: readonly PluginEntryDto[]): PluginCounts {
  const counts: PluginCounts = {
    total: list.length,
    disabled: 0,
    blocklisted: 0,
    flagged: 0,
    shadowed: 0,
  };
  for (const entry of list) {
    if (entry.status.kind !== "ok") {
      counts[entry.status.kind] += 1;
    }
  }
  return counts;
}

/** Whether a row can be switched on/off (a blocklisted or shadowed file isn't registered). */
export function canToggle(entry: PluginEntryDto): boolean {
  return entry.id !== "" && entry.status.kind !== "blocklisted";
}

/** Whether `path` is a direct child of `installDir` (H-29): only files there offer
 * "Uninstall…" in the row menu — anything else offers "Block" instead. Path-separator
 * agnostic (either OS); a look-alike sibling folder (`.clap-extra`) never matches. */
export function isInInstallFolder(path: string, installDir: string | null): boolean {
  if (!installDir) {
    return false;
  }
  const slashes = (p: string) => p.replace(/\\/g, "/");
  const dir = slashes(installDir).replace(/\/+$/, "");
  const file = slashes(path);
  return file.startsWith(`${dir}/`) && !file.slice(dir.length + 1).includes("/");
}
