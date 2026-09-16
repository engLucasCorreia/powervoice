import en from "./en.json";

/**
 * Every user-facing string goes through a key in this file (CLAUDE.md). This is a small typed
 * helper, not a full i18n library: one locale (`en.json`) for now, `{placeholder}` substitution,
 * and compile-time checking that a key actually exists.
 */
type Messages = typeof en;
export type MessageKey = keyof Messages;
export type MessageParams = Record<string, string | number>;

function interpolate(template: string, params?: MessageParams): string {
  if (!params) {
    return template;
  }
  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name];
    return value === undefined ? match : String(value);
  });
}

export function t<K extends MessageKey>(key: K, params?: MessageParams): string {
  return interpolate(en[key], params);
}

const messages: Record<string, string> = en;

/**
 * H-44 (ADR-006 §3): strings merged from installed `.voxmod` packages' `locales/<lang>.json`
 * files — namespaced `modules.<id>.*` only, never overlapping {@link messages} (the backend
 * enforces the namespace and rejects a file that tries to reach outside it; this filter is
 * defense in depth, not the only guard). Replaced wholesale by {@link setModuleMessages} whenever
 * the set of installed packages changes — app start, after "Install module…", after "Uninstall…"
 * — so an uninstalled package's strings disappear too, not just get amended over.
 */
let moduleMessages: Record<string, string> = {};

/** The one namespace a package's merged strings may ever occupy (ADR-006 §3). */
const MODULE_MESSAGE_PREFIX = "modules.";

/**
 * Replaces the whole runtime-merged package message table with `next` (typically the result of
 * `pluginsModuleLocales(lang)`, covering every currently installed package at once). Any entry
 * outside {@link MODULE_MESSAGE_PREFIX}, or that would shadow a real `en.json` key, is dropped —
 * belt and braces on top of the backend's own validation.
 */
export function setModuleMessages(next: Record<string, string>): void {
  const filtered: Record<string, string> = {};
  for (const [key, value] of Object.entries(next)) {
    if (key.startsWith(MODULE_MESSAGE_PREFIX) && !(key in messages)) {
      filtered[key] = value;
    }
  }
  moduleMessages = filtered;
}

/**
 * Same interpolation as {@link t}, but for a key that only exists as a runtime string — an
 * `IpcError`/`Notice` key that arrived over IPC (T-104), or a `modules.<id>.*` key an installed
 * package contributed (H-44) — which can't be checked against `MessageKey` at compile time. Falls
 * back to showing the raw key rather than throwing, so an unrecognized (e.g. newer-than-this-build,
 * or an uninstalled package's) key is at least visible instead of crashing the notice/toast UI.
 */
export function tDynamic(key: string, params?: MessageParams): string {
  const template = messages[key] ?? moduleMessages[key];
  if (template === undefined) {
    return key;
  }
  return interpolate(template, params);
}
