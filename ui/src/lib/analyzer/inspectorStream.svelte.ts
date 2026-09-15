/**
 * The Spectrum Inspector's live stream (H-42, SPEC-007 §8.3): an `analyzer_inspector_subscribe`
 * subscription at the Inspector's FFT size / window / response, held only while the Inspector
 * is open on its Live source. The engine goes quiet on silence (one floor frame, then nothing),
 * so a stopped transport costs the UI nothing.
 */
import { Channel } from "@tauri-apps/api/core";
import type { InspectorConfigDto } from "../ipc/bindings";
import { analyzerInspectorConfigure, analyzerInspectorSubscribe, analyzerUnsubscribe } from "../ipc/commands";
import { decodeVxis, type InspectorFrame } from "../ipc/inspector";
import { toArrayBuffer } from "../ipc/telemetry";

let frame = $state.raw<InspectorFrame | null>(null);
let id: number | undefined;
let generation = 0;
let open = false;

export function inspectorStream(): { readonly frame: InspectorFrame | null } {
  return {
    get frame() {
      return frame;
    },
  };
}

/** Test hook / preview: a frame as if the engine had sent it. */
export function applyInspectorFrame(next: InspectorFrame | null): void {
  frame = next;
}

/** Opens (or reconfigures) the stream. */
export function openInspectorStream(config: InspectorConfigDto): void {
  if (open && id !== undefined) {
    void analyzerInspectorConfigure(id, config).catch(() => {});
    return;
  }
  if (open) {
    return;
  }
  open = true;
  const mine = ++generation;
  void analyzerInspectorSubscribe(
    new Channel<ArrayBuffer>((message) => {
      if (mine !== generation) {
        return;
      }
      const buf = toArrayBuffer(message);
      const decoded = buf ? decodeVxis(buf) : null;
      if (decoded) {
        frame = decoded;
      }
    }),
    config,
  )
    .then((subscribed) => {
      if (mine === generation) {
        id = subscribed;
      } else {
        void analyzerUnsubscribe(subscribed).catch(() => {});
      }
    })
    .catch(() => {
      // No engine (preview/tests).
    });
}

/** Closes the stream (the Inspector closed, or left its Live source). */
export function closeInspectorStream(): void {
  open = false;
  generation += 1;
  const current = id;
  id = undefined;
  frame = null;
  if (current !== undefined) {
    void analyzerUnsubscribe(current).catch(() => {});
  }
}

export function resetInspectorStreamForTest(): void {
  closeInspectorStream();
}
