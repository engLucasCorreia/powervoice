/**
 * Live output analyzer store (T-208, SPEC-007 §2.9): subscribes to `VXSA` frames while the panel
 * (or, later, the EQ graph) is mounted, and holds the latest decoded frame for consumers. The
 * `AnalyzerPanel` component owns the peak-hold ballistics (`peakHold.ts`) itself, since each
 * consumer of this stream may want its own.
 */
import { Channel } from "@tauri-apps/api/core";
import type { AnalyzerResponseDto } from "../ipc/bindings";
import { analyzerSetResponse, analyzerSubscribe, analyzerUnsubscribe } from "../ipc/commands";
import { type AnalyzerFrame, decodeVxsa } from "../ipc/analyzer";
import { toArrayBuffer } from "../ipc/telemetry";

let frame = $state.raw<AnalyzerFrame | undefined>(undefined);
let response = $state<AnalyzerResponseDto>("medium");
let subscriberId: number | undefined;

/** The latest decoded frame (`undefined` before the first one arrives) and the current response. */
export function analyzerState(): {
  readonly frame: AnalyzerFrame | undefined;
  readonly response: AnalyzerResponseDto;
} {
  return {
    get frame() {
      return frame;
    },
    get response() {
      return response;
    },
  };
}

function onMessage(message: unknown): void {
  const buf = toArrayBuffer(message);
  const decoded = buf ? decodeVxsa(buf) : null;
  if (decoded) {
    frame = decoded;
  }
}

/**
 * Subscribes to the analyzer at `initialResponse` (SPEC-007 §2.9's default is Medium). Returns a
 * teardown function that unsubscribes (the tap turns off once the last subscriber leaves).
 */
export async function initAnalyzer(
  initialResponse: AnalyzerResponseDto = "medium",
): Promise<() => void> {
  response = initialResponse;
  try {
    subscriberId = await analyzerSubscribe(
      new Channel<ArrayBuffer>((message) => onMessage(message)),
      response,
    );
  } catch {
    // Without the analyzer subscription the panel just stays at rest.
  }
  return () => {
    const id = subscriberId;
    subscriberId = undefined;
    frame = undefined;
    if (id !== undefined) {
      analyzerUnsubscribe(id).catch(() => {
        // A failed unsubscribe during teardown is harmless.
      });
    }
  };
}

/** Changes the averaging response (Fast/Medium/Slow), persisted by the caller. */
export async function setAnalyzerResponse(next: AnalyzerResponseDto): Promise<void> {
  response = next;
  if (subscriberId !== undefined) {
    try {
      await analyzerSetResponse(subscriberId, next);
    } catch {
      // The next subscribe (or a reconnect) will pick the setting up again.
    }
  }
}

/** Test/teardown helper. */
export function resetAnalyzerForTest(): void {
  frame = undefined;
  response = "medium";
  subscriberId = undefined;
}
