/**
 * Coalesces `rack_response_curve` requests to at most one in flight per animation frame
 * (SPEC-015 §2.6.6 "at most one request is in flight. Responses with an older seq are
 * dropped."). Framework-free and injectable (`scheduleFrame`/`cancelFrame`) so it's directly
 * testable without a real animation-frame clock.
 */

export type CurveSender<T> = (points: number[]) => Promise<T>;

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

export class CoalescedCurveRequest<T> {
  readonly #send: CurveSender<T>;
  readonly #onResult: (result: T) => void;
  readonly #scheduleFrame: (cb: () => void) => number;
  readonly #cancelFrame: (id: number) => void;
  #frame: number | null = null;
  #pending: number[] | null = null;
  #seq = 0;

  constructor(
    send: CurveSender<T>,
    onResult: (result: T) => void,
    scheduleFrame: (cb: () => void) => number = defaultSchedule,
    cancelFrame: (id: number) => void = defaultCancel,
  ) {
    this.#send = send;
    this.#onResult = onResult;
    this.#scheduleFrame = scheduleFrame;
    this.#cancelFrame = cancelFrame;
  }

  /** Queues `points` for the next animation frame; a call before that frame replaces the queued
   * points (latest wins) rather than issuing a second request. */
  request(points: number[]): void {
    this.#pending = points;
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
    const points = this.#pending;
    this.#pending = null;
    if (!points) {
      return;
    }
    const seq = ++this.#seq;
    this.#send(points).then(
      (result) => {
        if (seq === this.#seq) {
          this.#onResult(result);
        }
      },
      () => {
        // A failed request (no ResponseCurve support, rack unavailable) leaves the last result
        // on screen rather than clearing the graph.
      },
    );
  }
}
