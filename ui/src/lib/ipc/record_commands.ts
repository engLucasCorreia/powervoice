import { invoke } from "@tauri-apps/api/core";
import type { CommandName, DefaultFormatDto, MonitorMode, RecordStateDto } from "./bindings";

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

/**
 * Starts a new recording; `replace` = the user confirmed replacing a document with audio.
 * `format` (H-06, the New Recording dialog): the chosen sample rate / bit depth — omitted, Record
 * with no document uses the current default format without showing the dialog (SPEC-002 §2.2).
 */
export async function recordStart(replace: boolean, format?: DefaultFormatDto): Promise<RecordStateDto> {
  return invoke<RecordStateDto>(
    "record_start" satisfies CommandName,
    format ? { replace, format } : { replace },
  );
}

/** Stops the recording (the take becomes the document once committed). */
export async function recordStop(): Promise<RecordStateDto> {
  return invoke<RecordStateDto>("record_stop" satisfies CommandName);
}

/** Sets (and saves) the monitoring mode. */
export async function recordSetMonitor(mode: MonitorMode): Promise<RecordStateDto> {
  return invoke<RecordStateDto>("record_set_monitor" satisfies CommandName, { mode });
}

/**
 * H-07: `count` `(min, max)` buckets of the take being captured, from bucket `startBucket`, as
 * raw `VXPK` bytes (decode with `decodeVxpk`; zero buckets while no take is being captured).
 */
export async function recordPeaksGet(startBucket: number, count: number): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("record_peaks_get" satisfies CommandName, { startBucket, count });
}
