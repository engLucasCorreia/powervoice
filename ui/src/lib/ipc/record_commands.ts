import { invoke } from "@tauri-apps/api/core";
import type {
  CommandName,
  DefaultFormatDto,
  MonitorMode,
  RecordOffsetDto,
  RecordOffsetSource,
  RecordStartedDto,
  RecordStateDto,
} from "./bindings";

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

/**
 * T-304 (SPEC-022 §2.2): Record with the current selection — a new recording into an empty
 * document, a punch-in over a non-empty selection, else Insert/Overwrite at the selection start or
 * the cursor.
 */
export async function recordStartAt(selection: [number, number] | null): Promise<RecordStartedDto> {
  return invoke<RecordStartedDto>("record_start_at" satisfies CommandName, { selection });
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

/** T-304 (SPEC-022 §2.13): the current device setup's recording offset. */
export async function recordOffsetGet(): Promise<RecordOffsetDto> {
  return invoke<RecordOffsetDto>("record_offset_get" satisfies CommandName);
}

/** T-304: stores the offset for the current device setup (clamped to ±500 ms). */
export async function recordOffsetSet(
  offsetMs: number,
  source: RecordOffsetSource,
  confidence: number | null,
): Promise<RecordOffsetDto> {
  return invoke<RecordOffsetDto>("record_offset_set" satisfies CommandName, {
    offsetMs,
    source,
    confidence,
  });
}

/** T-304 (SPEC-022 §2.14): starts a calibration job (`verify`: with the stored offset applied). */
export async function calibrationRun(verify: boolean): Promise<number> {
  return invoke<number>("calibration_run" satisfies CommandName, { verify });
}

/** T-304: aborts the running calibration. */
export async function calibrationCancel(): Promise<void> {
  return invoke<void>("calibration_cancel" satisfies CommandName);
}
