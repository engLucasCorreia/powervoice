import { invoke } from "@tauri-apps/api/core";
import type { AppInfo, CommandName } from "./bindings";

/**
 * Hand-written typed wrappers around `invoke` (ADR-003). Every command gets one wrapper here;
 * they use generated types (`./bindings`) only, never hand-copied shapes. The `satisfies
 * CommandName` on each command literal is what would catch a typo or a name Rust doesn't export.
 */

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info" satisfies CommandName);
}
