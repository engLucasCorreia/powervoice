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
 * Same interpolation as {@link t}, but for a key that only exists as a runtime string — an
 * `IpcError`/`Notice` key that arrived over IPC (T-104), which can't be checked against
 * `MessageKey` at compile time. Falls back to showing the raw key rather than throwing, so an
 * unrecognized (e.g. newer-than-this-build) key is at least visible instead of crashing the
 * notice/toast UI.
 */
export function tDynamic(key: string, params?: MessageParams): string {
  const template = messages[key];
  if (template === undefined) {
    return key;
  }
  return interpolate(template, params);
}
