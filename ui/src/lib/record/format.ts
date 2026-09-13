/** `h:mm:ss.t` for a recorded length (SPEC-002 §2.2 record panel elapsed time). */
export function formatElapsed(samples: number, rateHz: number): string {
  const tenths = rateHz > 0 ? Math.floor((samples * 10) / rateHz) : 0;
  const t = tenths % 10;
  const s = Math.floor(tenths / 10) % 60;
  const m = Math.floor(tenths / 600) % 60;
  const h = Math.floor(tenths / 36_000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${h}:${pad(m)}:${pad(s)}.${t}`;
}

/** H-11 (SPEC-002 §2.5): `h:mm:ss`/`m:ss` for the record panel's remaining recording time. */
export function formatRemaining(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  const sec = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}

/** H-11 (SPEC-002 §3 `disk_warn_min`): remaining recording time below this many minutes turns
 * the display amber and asks for confirmation before Record. */
export const DISK_WARN_MINUTES = 10;
