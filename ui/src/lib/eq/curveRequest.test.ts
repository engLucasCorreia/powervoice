import { describe, expect, it, vi } from "vitest";
import { CoalescedCurveRequest } from "./curveRequest";

/**
 * `rack_response_curve` request coalescing tests (S3-07, SPEC-015 §2.6.6 "at most one request is
 * in flight. Responses with an older seq are dropped."). The scheduler is injected so a "frame"
 * is a synchronous callback under test control — no fake timers needed.
 */

function syncScheduler(): { schedule: (cb: () => void) => number; cancel: (id: number) => void } {
  let next = 1;
  return {
    schedule: (cb) => {
      const id = next++;
      // Not called synchronously here — the test calls `run()` itself so it controls timing.
      pending.set(id, cb);
      return id;
    },
    cancel: (id) => {
      pending.delete(id);
    },
  };
}

const pending = new Map<number, () => void>();

function runFrame(): void {
  const cbs = [...pending.values()];
  pending.clear();
  for (const cb of cbs) {
    cb();
  }
}

function deferred<T>(): { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void } {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("CoalescedCurveRequest", () => {
  it("sends the queued points on the next frame", () => {
    pending.clear();
    const { schedule, cancel } = syncScheduler();
    const send = vi.fn().mockResolvedValue({ ok: true });
    const onResult = vi.fn();
    const req = new CoalescedCurveRequest(send, onResult, schedule, cancel);

    req.request([20, 1_000]);
    expect(send).not.toHaveBeenCalled();
    runFrame();
    expect(send).toHaveBeenCalledWith([20, 1_000]);
  });

  it("coalesces multiple requests before the frame into one call with the latest points", () => {
    pending.clear();
    const { schedule, cancel } = syncScheduler();
    const send = vi.fn().mockResolvedValue({ ok: true });
    const req = new CoalescedCurveRequest(send, vi.fn(), schedule, cancel);

    req.request([1]);
    req.request([2]);
    req.request([3]);
    runFrame();
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith([3]);
  });

  it("drops a response whose request is no longer the latest (stale-drop, AC-20)", async () => {
    pending.clear();
    const { schedule, cancel } = syncScheduler();
    const first = deferred<string>();
    const second = deferred<string>();
    const send = vi.fn().mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const onResult = vi.fn();
    const req = new CoalescedCurveRequest(send, onResult, schedule, cancel);

    req.request([1]);
    runFrame(); // first request in flight
    req.request([2]);
    runFrame(); // second request in flight (first has not resolved yet)

    second.resolve("second");
    await Promise.resolve();
    await Promise.resolve();
    expect(onResult).toHaveBeenCalledWith("second");

    first.resolve("first"); // arrives late: must be dropped
    await Promise.resolve();
    await Promise.resolve();
    expect(onResult).toHaveBeenCalledTimes(1);
    expect(onResult).not.toHaveBeenCalledWith("first");
  });

  it("a rejected request does not throw and leaves the last result untouched", async () => {
    pending.clear();
    const { schedule, cancel } = syncScheduler();
    const send = vi.fn().mockRejectedValue(new Error("no ResponseCurve support"));
    const onResult = vi.fn();
    const req = new CoalescedCurveRequest(send, onResult, schedule, cancel);

    req.request([1]);
    runFrame();
    await Promise.resolve();
    await Promise.resolve();
    expect(onResult).not.toHaveBeenCalled();
  });

  it("flushNow sends immediately without waiting for the injected frame", () => {
    pending.clear();
    const { schedule, cancel } = syncScheduler();
    const send = vi.fn().mockResolvedValue({});
    const req = new CoalescedCurveRequest(send, vi.fn(), schedule, cancel);

    req.request([1]);
    req.flushNow();
    expect(send).toHaveBeenCalledWith([1]);
  });

  it("cancel drops a queued (not yet sent) request", () => {
    pending.clear();
    const { schedule, cancel } = syncScheduler();
    const send = vi.fn().mockResolvedValue({});
    const req = new CoalescedCurveRequest(send, vi.fn(), schedule, cancel);

    req.request([1]);
    req.cancel();
    runFrame();
    expect(send).not.toHaveBeenCalled();
  });
});
