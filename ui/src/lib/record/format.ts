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
