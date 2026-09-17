/**
 * The transfer graph's live operating point (H-77, SPEC-016 §2.6 item 2): where the module is
 * working on its own curve right now. Its own module because it is panel-local data — the
 * effective makeup is a parameter, not a telemetry channel, so the generic graph can't derive it.
 */

export interface OperatingPoint {
  /** The `input_level_dbfs` telemetry channel. */
  inputDbfs: number;
  /** The `gr_total_db` telemetry channel (≤ 0, makeup excluded). */
  grTotalDb: number;
  /** The effective makeup in dB (0 while its section is off). */
  makeupDb: number;
}
