/**
 * Where the Dynamics module is working on its own curve right now (H-77, SPEC-016 §2.6 item 2):
 * x = the `input_level_dbfs` telemetry, y = x + `gr_total_db` + the **effective** makeup. Pure,
 * and built only on the schema plus the telemetry channel descriptions (ADR-005 §13): the makeup
 * is found by its schema key, and it counts only while its own section is enabled — `gr_total_db`
 * excludes makeup by definition (SPEC-016 §4.10), so without it the dot would sit below the
 * curve everywhere.
 */

import type { RackSlotDto } from "../ipc/bindings";
import type { OperatingPoint } from "../transfer/operatingPoint";

/** The graph's own floor: at or below it the dot is hidden (SPEC-016 §2.6). */
const HIDE_AT_OR_BELOW_DBFS = -80;

/** The Dynamics makeup parameter, and the enable of the group it belongs to. */
const MAKEUP_KEY = "compressor_makeup_db";

/** The effective makeup in dB: the parameter's value while its section is on, else 0. */
export function effectiveMakeupDb(slot: RackSlotDto): number {
  const makeup = slot.params.find((p) => p.key === MAKEUP_KEY);
  if (!makeup) {
    return 0;
  }
  const group = slot.groups.find((g) => g.id === makeup.group);
  const enable = group?.enable_param;
  if (enable !== null && enable !== undefined) {
    const on = slot.values.find((v) => v.id === enable);
    if (on && on.value < 0.5) {
      return 0;
    }
  }
  return slot.values.find((v) => v.id === makeup.id)?.value ?? 0;
}

/**
 * The operating point for the latest `VXMT` values of `slot`, or `null` while it is hidden: no
 * frame (the caller's `slotTelemetry` already returns `undefined` once one is 250 ms old), a
 * module without the two channels, or an input level at or below the graph's floor.
 */
export function operatingPointOf(
  slot: RackSlotDto,
  values: readonly number[] | undefined,
): OperatingPoint | null {
  if (!values) {
    return null;
  }
  const channels = slot.telemetry ?? [];
  const levelIndex = channels.findIndex((c) => c.kind === "level" && c.group === null);
  const grIndex = channels.findIndex((c) => c.kind === "gain_reduction" && c.group === null);
  if (levelIndex < 0 || grIndex < 0) {
    return null;
  }
  const inputDbfs = values[levelIndex];
  const grTotalDb = values[grIndex];
  if (
    inputDbfs === undefined ||
    grTotalDb === undefined ||
    !Number.isFinite(inputDbfs) ||
    !Number.isFinite(grTotalDb) ||
    inputDbfs <= HIDE_AT_OR_BELOW_DBFS
  ) {
    return null;
  }
  return { inputDbfs, grTotalDb, makeupDb: effectiveMakeupDb(slot) };
}
