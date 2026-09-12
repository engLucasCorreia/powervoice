/**
 * Hard wall-clock deadlines for every step of the automated suite. Hyprland can throttle (or, it
 * turns out in practice, entirely withhold) `requestAnimationFrame` for a window that is mapped
 * but not focused/occluded — see ADR-009. An automated run must still finish and write results
 * rather than hang forever, so every rAF- or IPC-driven step races against a deadline here and
 * reports a `timedOut`/incomplete result instead of blocking `POWERVOICE_SPIKE_EXIT=1` forever.
 */
export async function withTimeout<T>(promise: Promise<T>, ms: number, onTimeout: () => T): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeoutPromise = new Promise<T>((resolve, reject) => {
    // `onTimeout` may throw (to make the timeout show up as an error/rejection instead of a
    // fallback value) — do that inside the executor so it becomes a rejection, not an uncaught
    // exception in the timer callback that would leave this promise pending forever.
    timer = setTimeout(() => {
      try {
        resolve(onTimeout());
      } catch (e) {
        reject(e);
      }
    }, ms);
  });
  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}
