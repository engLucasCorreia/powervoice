import { invoke } from "@tauri-apps/api/core";
import type { SpikeEnv } from "./types";

/**
 * Returns the spike environment when `powervoice-app` was built with `--features spike` (T-007 /
 * ADR-009), or `null` for a normal build. The `spike_env` command only exists in a spike build,
 * so a normal production build simply fails this `invoke()` and `main.ts` falls back to the real
 * `App` — this is how the spike UI stays out of the production path without a second, easy to
 * forget, UI-side build flag that has to be kept in sync with the Cargo feature.
 */
export async function detectSpike(): Promise<SpikeEnv | null> {
  try {
    return await invoke<SpikeEnv>("spike_env");
  } catch {
    return null;
  }
}
