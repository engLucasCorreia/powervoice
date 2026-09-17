/**
 * Coalesces `rack_response_curve` requests to at most one in flight per animation frame
 * (SPEC-015 §2.6.6 "at most one request is in flight. Responses with an older seq are
 * dropped."). Framework-free and injectable (`scheduleFrame`/`cancelFrame`) so it's directly
 * testable without a real animation-frame clock.
 */

/** What one request carries: the EQ graph sends a frequency list, the transfer graph a point
 * count. `seq` is this request's sequence number — a command that echoes it in its response (the
 * transfer graph's `VXTC` frame, SPEC-016 §4.12) passes it on; the EQ graph ignores it. */
export type CurveSender<T, A = number[]> = (args: A, seq: number) => Promise<T>;

function defaultSchedule(cb: () => void): number {
  return typeof requestAnimationFrame === "function"
    ? requestAnimationFrame(cb)
    : (setTimeout(cb, 16) as unknown as number);
}

function defaultCancel(id: number): void {
  if (typeof cancelAnimationFrame === "function") {
    cancelAnimationFrame(id);
  } else {
    clearTimeout(id);
  }
}

export class CoalescedCurveRequest<T, A = number[]> {
  readonly #send: CurveSender<T, A>;
  readonly #onResult: (result: T) => void;
  readonly #scheduleFrame: (cb: () => void) => number;
  readonly #cancelFrame: (id: number) => void;
  #frame: number | null = null;
  #pending: { args: A } | null = null;
  #seq = 0;

  constructor(
    send: CurveSender<T, A>,
    onResult: (result: T) => void,
    scheduleFrame: (cb: () => void) => number = defaultSchedule,
    cancelFrame: (id: number) => void = defaultCancel,
  ) {
    this.#send = send;
    this.#onResult = onResult;
    this.#scheduleFrame = scheduleFrame;
    this.#cancelFrame = cancelFrame;
  }

  /** Queues `args` for the next animation frame; a call before that frame replaces the queued
   * request (latest wins) rather than issuing a second one. */
  request(args: A): void {
    this.#pending = { args };
    if (this.#frame === null) {
      this.#frame = this.#scheduleFrame(() => this.#flush());
    }
  }

  /** Test helper: sends a queued request immediately instead of waiting for the frame. */
  flushNow(): void {
    if (this.#frame !== null) {
      this.#cancelFrame(this.#frame);
      this.#frame = null;
      this.#flush();
    }
  }

  /** Cancels a queued (not yet sent) request, e.g. on component teardown. Does not cancel a
   * request already in flight — its response is simply dropped by the sequence check. */
  cancel(): void {
    if (this.#frame !== null) {
      this.#cancelFrame(this.#frame);
      this.#frame = null;
    }
    this.#pending = null;
  }

  #flush(): void {
    this.#frame = null;
    const pending = this.#pending;
    this.#pending = null;
    if (!pending) {
      return;
    }
    const seq = ++this.#seq;
    this.#send(pending.args, seq).then(
      (result) => {
        if (seq === this.#seq) {
          this.#onResult(result);
        }
      },
      () => {
        // A failed request (no curve extension, rack unavailable) leaves the last result on
        // screen rather than clearing the graph.
      },
    );
  }
}
