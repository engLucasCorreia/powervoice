import { invoke } from "@tauri-apps/api/core";
import type { AppInfo, CommandName, Settings } from "./bindings";

/**
 * Hand-written typed wrappers around `invoke` (ADR-003). Every command gets one wrapper here;
 * they use generated types (`./bindings`) only, never hand-copied shapes. The `satisfies
 * CommandName` on each command literal is what would catch a typo or a name Rust doesn't export.
 */

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info" satisfies CommandName);
}

/** T-104: current settings, from the in-memory cache (never touches disk on the Rust side). */
export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("settings_get" satisfies CommandName);
}

/** T-104: replaces the settings file (atomic write) and returns the canonical saved value. */
export async function setSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("settings_set" satisfies CommandName, { settings });
}
