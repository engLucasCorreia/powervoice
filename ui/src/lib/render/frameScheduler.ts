/**
 * H-43: the shared, on-demand frame scheduler every canvas renderer (waveform, spectral, EQ graph,
 * analyzer plot) and the transport's playhead/meter animation draw from. It replaces H-32's rule
 * of one perpetual `requestAnimationFrame` loop per renderer, which redrew every idle view at the
 * display rate and kept the web view's main thread busy while nothing changed.
 *
 * - A client **requests a frame when its inputs change** (`invalidate()`: new data, viewport,
 *   theme, size, playhead, telemetry…). Any number of requests before the frame coalesce into one
 *   `requestAnimationFrame` for all clients together.
 * - A client's callback returns `true` while it is **still animating** (playback, recording, a
 *   meter decaying, a zoom animation); it then runs again next frame. When every client has
 *   settled, no frame is scheduled at all — an idle app costs nothing.
 * - **H-32's robustness is kept**: each client runs in its own `try/catch`, so one throwing draw
 *   never starves the others; an animating client keeps getting frames even if one throws
 *   (`try/finally` reschedules while active); a thrown draw is retried on the next frames (at most
 *   {@link MAX_RETRIES_AFTER_THROW} in a row, so a draw that always throws can't become a new
 *   perpetual loop); and any later `invalidate()` always draws again.
 * - A safety net ({@link installInputInvalidation}) redraws every client once on discrete user
 *   input (pointer down/up, key, resize, focus, visibility) — a renderer that missed an input
 *   heals at the user's next gesture, much like H-32's perpetual loop healed on the next frame.
 *
 * Clients run in `priority` order within a frame (lower first): the transport updates the playhead
 * before the renderers that draw it read it.
 */

/** The frame clock (injectable for tests); defaults to `requestAnimationFrame`. */
export interface FrameSource {
  request(callback: (now: number) => void): number;
  cancel(id: number): void;
}

/** One frame of a client: return `true` while still animating (wants the next frame too). */
export type FrameCallback = (now: number) => boolean | void;

export interface FrameClientOptions {
  /** Lower runs first within a frame (default 0). */
  priority?: number;
  /** For diagnostics only. */
  name?: string;
}

export interface FrameClient {
  /** Requests one frame (coalesced with every other request until it runs). */
  invalidate(): void;
  /** Stops scheduling this client; a pending frame for it is dropped. */
  dispose(): void;
}

export interface FrameStats {
  /** Animation frames the scheduler asked the browser for and ran. */
  frames: number;
  /** Client callbacks run (a frame runs every due client once). */
  runs: number;
  /** Client callbacks that threw. */
  errors: number;
}

/** A draw that throws is retried on at most this many following frames before the scheduler
 * waits for the next `invalidate()`. */
export const MAX_RETRIES_AFTER_THROW = 3;

interface Entry {
  callback: FrameCallback;
  priority: number;
  order: number;
  name: string;
  failures: number;
  disposed: boolean;
}

/** The browser's rAF, looked up at call time (tests install fake timers after import); a ~60 Hz
 * timer where there is none. */
const defaultSource: FrameSource = {
  request(callback) {
    if (typeof requestAnimationFrame === "function") {
      return requestAnimationFrame(callback);
    }
    return setTimeout(() => callback(typeof performance !== "undefined" ? performance.now() : Date.now()), 16) as unknown as number;
  },
  cancel(id) {
    if (typeof cancelAnimationFrame === "function") {
      cancelAnimationFrame(id);
    } else {
      clearTimeout(id);
    }
  },
};

function defaultOnError(err: unknown, name: string): void {
  if (import.meta.env.DEV) {
    console.error(`[frameScheduler] ${name || "client"} frame failed; retrying`, err);
  }
}

export class FrameScheduler {
  readonly stats: FrameStats = { frames: 0, runs: 0, errors: 0 };
  private readonly source: FrameSource;
  private readonly onError: (err: unknown, name: string) => void;
  private readonly entries = new Set<Entry>();
  private dirty = new Set<Entry>();
  private frameId = 0;
  private scheduled = false;
  private nextOrder = 0;

  constructor(options: { source?: FrameSource; onError?: (err: unknown, name: string) => void } = {}) {
    this.source = options.source ?? defaultSource;
    this.onError = options.onError ?? defaultOnError;
  }

  /** Registers a client; nothing runs until it (or {@link invalidateAll}) requests a frame. */
  client(callback: FrameCallback, options: FrameClientOptions = {}): FrameClient {
    const entry: Entry = {
      callback,
      priority: options.priority ?? 0,
      order: this.nextOrder++,
      name: options.name ?? "",
      failures: 0,
      disposed: false,
    };
    this.entries.add(entry);
    return {
      invalidate: () => {
        if (entry.disposed) {
          return;
        }
        this.dirty.add(entry);
        this.schedule();
      },
      dispose: () => {
        entry.disposed = true;
        this.entries.delete(entry);
        this.dirty.delete(entry);
        if (this.dirty.size === 0 && this.scheduled) {
          this.source.cancel(this.frameId);
          this.scheduled = false;
        }
      },
    };
  }

  /** Requests one frame for every live client. */
  invalidateAll(): void {
    for (const entry of this.entries) {
      this.dirty.add(entry);
    }
    if (this.dirty.size > 0) {
      this.schedule();
    }
  }

  /** Whether a frame is currently scheduled (diagnostics, tests). */
  get pending(): boolean {
    return this.scheduled;
  }

  /** Test helper: drops a pending frame (e.g. one requested from a fake timer that will never
   * fire after `vi.useRealTimers()`), every pending request and the stats. Clients stay
   * registered. */
  resetForTest(): void {
    if (this.scheduled) {
      try {
        this.source.cancel(this.frameId);
      } catch {
        // the timer implementation that issued the id may be gone
      }
    }
    this.scheduled = false;
    this.dirty.clear();
    this.stats.frames = 0;
    this.stats.runs = 0;
    this.stats.errors = 0;
  }

  private schedule(): void {
    if (this.scheduled) {
      return;
    }
    this.scheduled = true;
    this.frameId = this.source.request(this.onFrame);
  }

  private readonly onFrame = (now: number): void => {
    this.scheduled = false;
    this.stats.frames += 1;
    const due = [...this.dirty].sort((a, b) => a.priority - b.priority || a.order - b.order);
    this.dirty = new Set();
    try {
      for (const entry of due) {
        if (entry.disposed) {
          continue;
        }
        let again = false;
        let threw = false;
        try {
          this.stats.runs += 1;
          again = entry.callback(now) === true;
          entry.failures = 0;
        } catch (err) {
          threw = true;
          entry.failures += 1;
          this.stats.errors += 1;
          try {
            this.onError(err, entry.name);
          } catch {
            // reporting must never break the frame
          }
        } finally {
          // An animating client always gets its next frame, even after a throw; a client that
          // threw is retried a bounded number of times.
          if (!entry.disposed && (again || (threw && entry.failures <= MAX_RETRIES_AFTER_THROW))) {
            this.dirty.add(entry);
          }
        }
      }
    } finally {
      if (this.dirty.size > 0) {
        this.schedule();
      }
    }
  };
}

/** The app-wide scheduler every renderer shares (one rAF per frame for all of them). */
export const frameScheduler = new FrameScheduler();

/** Registers a client on the shared {@link frameScheduler}. */
export function createFrameClient(callback: FrameCallback, options?: FrameClientOptions): FrameClient {
  return frameScheduler.client(callback, options);
}

const INPUT_EVENTS = ["pointerdown", "pointerup", "keydown", "keyup", "resize", "focus"] as const;

/**
 * The safety net (see the module comment): redraw every client of `scheduler` once on discrete
 * user input. Returns the removal. `App.svelte` installs it once for the shared scheduler.
 */
export function installInputInvalidation(
  scheduler: FrameScheduler = frameScheduler,
  target: Window | undefined = typeof window !== "undefined" ? window : undefined,
): () => void {
  if (!target) {
    return () => {};
  }
  const onInput = (): void => scheduler.invalidateAll();
  for (const type of INPUT_EVENTS) {
    target.addEventListener(type, onInput, { capture: true, passive: true });
  }
  const onVisibility = (): void => {
    if (target.document?.visibilityState !== "hidden") {
      scheduler.invalidateAll();
    }
  };
  target.document?.addEventListener("visibilitychange", onVisibility);
  return () => {
    for (const type of INPUT_EVENTS) {
      target.removeEventListener(type, onInput, { capture: true });
    }
    target.document?.removeEventListener("visibilitychange", onVisibility);
  };
}
