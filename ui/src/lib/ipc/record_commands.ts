import { invoke } from "@tauri-apps/api/core";
import type { CommandName, MonitorMode, RecordStateDto } from "./bindings";

/**
 * S1-04 recording commands: typed `invoke` wrappers (ADR-003) using generated types only. Kept
 * apart from `commands.ts` so the recording slice stays self-contained.
 */

/** The record panel state. */
export async function recordGet(): Promise<RecordStateDto> {
  return invoke<RecordStateDto>("record_get" satisfies CommandName);
}

/** Arms (opens the input, starts the meter) or disarms. */
export async function recordArm(armed: boolean): Promise<RecordStateDto> {
  return invoke<RecordStateDto>("record_arm" satisfies CommandName, { armed });
}

/** Starts a new recording; `replace` = the user confirmed replacing a document with audio. */
export async function recordStart(replace: boolean): Promise<RecordStateDto> {
  return invoke<RecordStateDto>("record_start" satisfies CommandName, { replace });
}

/** Stops the recording (the take becomes the document once committed). */
export async function recordStop(): Promise<RecordStateDto> {
  return invoke<RecordStateDto>("record_stop" satisfies CommandName);
}

/** Sets (and saves) the monitoring mode. */
export async function recordSetMonitor(mode: MonitorMode): Promise<RecordStateDto> {
  return invoke<RecordStateDto>("record_set_monitor" satisfies CommandName, { mode });
}
