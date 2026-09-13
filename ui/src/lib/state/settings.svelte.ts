import { getSettings, setSettings } from "../ipc/commands";
import type { IpcError, Settings } from "../ipc/bindings";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "./notices.svelte";

let current = $state<Settings | null>(null);
let loading = $state(false);
let error = $state<string | null>(null);

/** Read-only accessor for components: `settingsState().current` / `.loading` / `.error`. */
export function settingsState(): {
  readonly current: Settings | null;
  readonly loading: boolean;
  readonly error: string | null;
} {
  return {
    get current() {
      return current;
    },
    get loading() {
      return loading;
    },
    get error() {
      return error;
    },
  };
}

function isIpcError(value: unknown): value is IpcError {
  return (
    typeof value === "object" &&
    value !== null &&
    "code" in value &&
    "key" in value &&
    "params" in value
  );
}

function reportError(err: unknown): void {
  if (isIpcError(err)) {
    error = err.key;
    pushNotice(noticeFromIpcError(err));
  } else {
    error = "error.unknown";
  }
}

/** Loads settings from the Rust store (T-104) into the shared reactive state. */
export async function loadSettings(): Promise<void> {
  loading = true;
  error = null;
  try {
    current = await getSettings();
  } catch (err) {
    reportError(err);
  } finally {
    loading = false;
  }
}

/**
 * Merges `patch` into the current settings and saves it (atomic write on the Rust side). Does
 * nothing if settings haven't been loaded yet — call {@link loadSettings} first.
 */
export async function saveSettings(patch: Partial<Settings>): Promise<void> {
  if (!current) {
    return;
  }
  const next: Settings = { ...current, ...patch };
  loading = true;
  error = null;
  try {
    current = await setSettings(next);
  } catch (err) {
    reportError(err);
  } finally {
    loading = false;
  }
}

/** Test/teardown helper. */
export function resetSettingsStateForTest(): void {
  current = null;
  loading = false;
  error = null;
}
