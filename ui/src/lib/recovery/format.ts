/**
 * Formatting helpers for the recovery dialog (T-301, SPEC-004 §2.7): sizes, the recovered
 * recording length ("0:02.5") and the last edit time. Numbers and units only — every sentence
 * around them is an i18n key.
 */

/** "3.2 GB", "150 MB", "12 KB" (decimal units). */
export function formatBytes(bytes: number): string {
  if (bytes >= 1e9) {
    return `${(bytes / 1e9).toFixed(1)} GB`;
  }
  if (bytes >= 1e6) {
    return `${Math.round(bytes / 1e6)} MB`;
  }
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

/** `m:ss.s` of `samples` at `rateHz` (SPEC-004 AC-10: "recording 0:02.5"). */
export function formatDuration(samples: number, rateHz: number): string {
  const tenths = rateHz > 0 ? Math.round((samples / rateHz) * 10) : 0;
  const minutes = Math.floor(tenths / 600);
  const seconds = (tenths - minutes * 600) / 10;
  return `${minutes}:${seconds.toFixed(1).padStart(4, "0")}`;
}

/** The last edit time in the user's locale. */
export function formatTimestamp(unixMs: number): string {
  return new Date(unixMs).toLocaleString();
}
