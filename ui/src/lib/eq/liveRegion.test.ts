import { describe, expect, it } from "vitest";
import { EQ_LIVE_REGION_INTERVAL_MS, EqLiveAnnouncer, type AnnouncerClock } from "./liveRegion";

/** A manually-advanced clock: `setTimeout`/`clearTimeout` just record pending callbacks, run by
 * the test calling `advance(ms)` — deterministic, no real or vitest fake timers needed. */
function fakeClock(): AnnouncerClock & { advance(ms: number): void } {
  let now = 0;
  let nextId = 1;
  const timers = new Map<number, { at: number; fn: () => void }>();
  return {
    now: () => now,
    setTimeout: (fn, ms) => {
      const id = nextId++;
      timers.set(id, { at: now + ms, fn });
      return id;
    },
    clearTimeout: (handle) => {
      timers.delete(handle as number);
    },
    advance(ms: number): void {
      now += ms;
      for (const [id, timer] of [...timers]) {
        if (timer.at <= now) {
          timers.delete(id);
          timer.fn();
        }
      }
    },
  };
}

describe("EqLiveAnnouncer (SPEC-015 §2.6.5 aria-live throttle)", () => {
  it("announces the first change immediately", () => {
    const clock = fakeClock();
    const a = new EqLiveAnnouncer(clock);
    a.announce("Band 3 · 1.20 kHz · +3.0 dB · Q 1.00");
    expect(a.text).toBe("Band 3 · 1.20 kHz · +3.0 dB · Q 1.00");
  });

  it("drops an update within the 250 ms window but eventually announces the latest value", () => {
    const clock = fakeClock();
    const a = new EqLiveAnnouncer(clock);
    a.announce("v1");
    expect(a.text).toBe("v1");

    clock.advance(50);
    a.announce("v2"); // inside the window: deferred, not shown yet
    expect(a.text).toBe("v1");

    clock.advance(100);
    a.announce("v3"); // still inside the window: replaces the pending value
    expect(a.text).toBe("v1");

    clock.advance(EQ_LIVE_REGION_INTERVAL_MS - 150 + 1); // window elapses
    expect(a.text).toBe("v3"); // the trailing timer fired with the latest value, not v2
  });

  it("announces immediately again once a full window has elapsed with no pending update", () => {
    const clock = fakeClock();
    const a = new EqLiveAnnouncer(clock);
    a.announce("v1");
    clock.advance(EQ_LIVE_REGION_INTERVAL_MS + 1);
    a.announce("v2");
    expect(a.text).toBe("v2");
  });

  it("never fires more than one message per 250 ms even under a rapid burst", () => {
    const clock = fakeClock();
    const a = new EqLiveAnnouncer(clock);
    const seenAfterEachCall: string[] = [];
    for (let i = 0; i < 20; i++) {
      a.announce(`v${i}`);
      seenAfterEachCall.push(a.text);
      clock.advance(20); // 20 ms between key repeats, well under the 250 ms window
    }
    clock.advance(EQ_LIVE_REGION_INTERVAL_MS); // flush the final trailing timer

    // 20 updates over 400 ms can change the shown text at most ~2-3 times (250 ms apart), never 20.
    const distinctValues = new Set(seenAfterEachCall);
    expect(distinctValues.size).toBeLessThan(5);
    expect(a.text).toBe("v19"); // the final value is never lost, once its window elapses
  });

  it("ignores a repeat of the currently-shown or already-pending text", () => {
    const clock = fakeClock();
    const a = new EqLiveAnnouncer(clock);
    a.announce("v1");
    a.announce("v1"); // no-op, still shown
    expect(a.text).toBe("v1");
  });

  it("calls onChange immediately, and again for a deferred trailing announcement", () => {
    const clock = fakeClock();
    const seen: string[] = [];
    const a = new EqLiveAnnouncer(clock, (text) => seen.push(text));
    a.announce("v1");
    expect(seen).toEqual(["v1"]); // immediate

    clock.advance(50);
    a.announce("v2"); // deferred: no onChange yet
    expect(seen).toEqual(["v1"]);

    clock.advance(EQ_LIVE_REGION_INTERVAL_MS);
    expect(seen).toEqual(["v1", "v2"]); // the trailing timer notifies too
  });

  it("dispose() cancels a pending trailing announcement", () => {
    const clock = fakeClock();
    const a = new EqLiveAnnouncer(clock);
    a.announce("v1");
    clock.advance(50);
    a.announce("v2");
    a.dispose();
    clock.advance(1_000);
    expect(a.text).toBe("v1"); // the trailing timer never fired
  });
});
