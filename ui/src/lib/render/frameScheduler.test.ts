import { afterEach, describe, expect, it, vi } from "vitest";
import { FrameScheduler, MAX_RETRIES_AFTER_THROW, type FrameSource } from "./frameScheduler";

/**
 * H-43: the shared on-demand frame scheduler that replaced H-32's perpetual per-renderer rAF
 * loops. A deterministic frame source stands in for `requestAnimationFrame`: `flush()` runs one
 * "vsync" worth of callbacks.
 */
function manualSource(): FrameSource & { flush(now?: number): number; readonly queued: number } {
  let next = 1;
  const queue = new Map<number, (now: number) => void>();
  let clock = 0;
  return {
    request(cb) {
      const id = next++;
      queue.set(id, cb);
      return id;
    },
    cancel(id) {
      queue.delete(id);
    },
    flush(now) {
      clock = now ?? clock + 16.7;
      const due = [...queue.values()];
      queue.clear();
      for (const cb of due) {
        cb(clock);
      }
      return due.length;
    },
    get queued() {
      return queue.size;
    },
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("FrameScheduler (H-43)", () => {
  it("runs nothing until a client invalidates, then exactly one frame, then stops", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    const draw = vi.fn(() => false);
    scheduler.client(draw);
    expect(source.queued).toBe(0);
    expect(source.flush()).toBe(0);

    scheduler.client(draw).invalidate();
    expect(source.queued).toBe(1);
    source.flush();
    expect(draw).toHaveBeenCalledTimes(1);
    // Idle: nothing scheduled any more.
    expect(source.queued).toBe(0);
    for (let i = 0; i < 60; i++) {
      source.flush();
    }
    expect(draw).toHaveBeenCalledTimes(1);
    expect(scheduler.stats.frames).toBe(1);
  });

  it("coalesces many invalidations (from many clients) into one animation frame", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    const a = vi.fn();
    const b = vi.fn();
    const ca = scheduler.client(a);
    const cb = scheduler.client(b);
    for (let i = 0; i < 10; i++) {
      ca.invalidate();
      cb.invalidate();
    }
    expect(source.queued).toBe(1);
    source.flush();
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
  });

  it("keeps running every frame while a client animates, and stops the frame after it settles", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    let remaining = 5;
    const draw = vi.fn(() => --remaining > 0);
    scheduler.client(draw).invalidate();
    for (let i = 0; i < 20; i++) {
      source.flush();
    }
    expect(draw).toHaveBeenCalledTimes(5);
    expect(source.queued).toBe(0);
  });

  it("restarts on invalidate after it went idle (e.g. playback starts)", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    let playing = false;
    const draw = vi.fn(() => playing);
    const client = scheduler.client(draw);
    client.invalidate();
    source.flush();
    source.flush();
    expect(draw).toHaveBeenCalledTimes(1);

    playing = true;
    client.invalidate();
    for (let i = 0; i < 60; i++) {
      source.flush();
    }
    expect(draw).toHaveBeenCalledTimes(61); // full rate while playing
    playing = false;
    source.flush(); // the frame that sees "stopped" draws the final state
    source.flush();
    expect(draw).toHaveBeenCalledTimes(62);
    expect(source.queued).toBe(0);
  });

  it("runs clients in priority order within a frame (the playhead update before the renderers)", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    const order: string[] = [];
    scheduler.client(() => void order.push("waveform")).invalidate();
    scheduler.client(() => void order.push("transport"), { priority: -100 }).invalidate();
    scheduler.client(() => void order.push("spectral")).invalidate();
    source.flush();
    expect(order).toEqual(["transport", "waveform", "spectral"]);
  });

  it("a thrown draw never stops future redraws: it is retried next frame, bounded, and any later invalidate draws again", () => {
    const source = manualSource();
    const errors: unknown[] = [];
    const scheduler = new FrameScheduler({ source, onError: (err) => errors.push(err) });
    let failing = true;
    const draw = vi.fn(() => {
      if (failing) {
        throw new Error("transient");
      }
      return false;
    });
    const other = vi.fn();
    const client = scheduler.client(draw);
    const otherClient = scheduler.client(other);
    client.invalidate();
    otherClient.invalidate();
    source.flush();
    // One client's throw doesn't starve another client in the same frame.
    expect(other).toHaveBeenCalledTimes(1);
    expect(errors).toHaveLength(1);
    // Retried on the following frames…
    for (let i = 0; i < 20; i++) {
      source.flush();
    }
    // …but a draw that keeps throwing doesn't turn into a perpetual loop.
    expect(draw).toHaveBeenCalledTimes(1 + MAX_RETRIES_AFTER_THROW);
    expect(source.queued).toBe(0);

    // The next change still draws (the H-32 guarantee).
    failing = false;
    client.invalidate();
    source.flush();
    expect(draw).toHaveBeenCalledTimes(2 + MAX_RETRIES_AFTER_THROW);

    // A transient throw heals on the very next frame.
    failing = true;
    client.invalidate();
    source.flush();
    failing = false;
    source.flush();
    expect(draw).toHaveBeenCalledTimes(4 + MAX_RETRIES_AFTER_THROW);
    expect(source.queued).toBe(0);
  });

  it("an animating client that throws keeps its frames coming (try/finally reschedules while active)", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    let n = 0;
    const draw = vi.fn(() => {
      n += 1;
      if (n % 2 === 0) {
        throw new Error("every other frame");
      }
      return true;
    });
    scheduler.client(draw).invalidate();
    for (let i = 0; i < 30; i++) {
      source.flush();
    }
    expect(draw).toHaveBeenCalledTimes(30);
  });

  it("dispose drops a pending frame and ignores later invalidations", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    const draw = vi.fn(() => true);
    const client = scheduler.client(draw);
    client.invalidate();
    client.dispose();
    source.flush();
    client.invalidate();
    source.flush();
    expect(draw).not.toHaveBeenCalled();
    expect(source.queued).toBe(0);
  });

  it("invalidateAll redraws every live client once (the user-input safety net)", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    const a = vi.fn();
    const b = vi.fn();
    scheduler.client(a);
    scheduler.client(b).dispose();
    scheduler.invalidateAll();
    source.flush();
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).not.toHaveBeenCalled();
  });

  it("an invalidate from inside a frame lands on the next frame, not a nested one", () => {
    const source = manualSource();
    const scheduler = new FrameScheduler({ source, onError: () => {} });
    let other: { invalidate(): void } | null = null;
    const b = vi.fn();
    scheduler
      .client(() => {
        other?.invalidate();
        return false;
      })
      .invalidate();
    other = scheduler.client(b);
    source.flush();
    expect(b).toHaveBeenCalledTimes(0);
    source.flush();
    expect(b).toHaveBeenCalledTimes(1);
  });

  it("uses the global requestAnimationFrame by default (an idle second under fake timers schedules no frame)", () => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "performance"] });
    try {
      const scheduler = new FrameScheduler({ onError: () => {} });
      const draw = vi.fn(() => false);
      const client = scheduler.client(draw);
      vi.advanceTimersByTime(1000);
      expect(draw).not.toHaveBeenCalled();
      client.invalidate();
      vi.advanceTimersByTime(1000);
      expect(draw).toHaveBeenCalledTimes(1);
      expect(scheduler.stats.frames).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
