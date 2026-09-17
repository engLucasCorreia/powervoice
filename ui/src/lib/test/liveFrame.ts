/**
 * Delivers a live IPC-channel message into a component under test, through the SAME `Channel`
 * object the component itself created and handed to a `*_subscribe` command — not a stand-in for
 * it (H-88, from H-87's open question).
 *
 * The gap this closes: several small canvas graphs (`EqGraph`'s spectrum overlay,
 * `NoiseProfileGraph`'s live curve/hover readout) hold their own `analyzer_subscribe`
 * `Channel<ArrayBuffer>` and derive what they draw/announce from the decoded `VXSA` frames it
 * receives. Before this helper, a component test could only mock `analyzer_subscribe` to fail ("no
 * real Tauri window") or resolve with a bare id — nothing could push a frame back down that
 * channel, so a component's *live* behaviour could only be tested in its *absence* (e.g. H-87's
 * hover readout could assert "Live −∞ dB" but never a real live level).
 *
 * How this works, with **no production seam at all**: `@tauri-apps/api/mocks`'s `mockIPC` hands
 * your handler the exact `args` object the component's `invoke()` call was built with. A real
 * Tauri IPC round trip would structurally clone/serialize that object crossing the webview
 * boundary, but the mock never does — it calls your handler with the live JS objects in-process.
 * So the `channel: Channel<ArrayBuffer>` argument a component passes to e.g.
 * `analyzerSubscribe(new Channel<ArrayBuffer>(...), ...)` arrives at your `mockIPC` handler as the
 * very same `Channel` instance the component holds, and `Channel.id` names a callback `mockIPC`
 * registered on `window.__TAURI_INTERNALS__`. Invoking that callback (via `runCallback`) is
 * exactly what a real Tauri webview does when the Rust side posts a channel message — so
 * `deliverChannelMessage` drives the component's own `onmessage`/decode path, the same one
 * production traffic uses, rather than reaching into the component to set its state directly.
 *
 * Usage (capture the channel from your `mockIPC` handler, then deliver into it after mounting):
 * ```ts
 * let channel: Channel<ArrayBuffer> | undefined;
 * mockIPC((cmd, args) => {
 *   if (cmd === "analyzer_subscribe") {
 *     channel = (args as { channel: Channel<ArrayBuffer> }).channel;
 *     return 1;
 *   }
 *   ...
 * });
 * const { target } = render(...);
 * await settle();
 * deliverChannelMessage(channel!, encodeVxsa({ levelsDb: [...] }));
 * await settle();
 * ```
 */
import type { Channel } from "@tauri-apps/api/core";

interface TauriInternals {
  runCallback?: (id: number, data: unknown) => void;
}

function internals(): TauriInternals {
  return (window as unknown as { __TAURI_INTERNALS__?: TauriInternals }).__TAURI_INTERNALS__ ?? {};
}

// `Channel` (see `@tauri-apps/api/core`) requires a contiguous per-channel `index` starting at 0:
// a message with an index it isn't expecting yet is queued, not delivered, waiting forever for the
// "message 0" that (for a fresh channel) already arrived. Tracked per `Channel` instance so
// multiple channels captured in the same test (e.g. a toggle-off/on re-subscribe) don't interfere.
const nextIndex = new WeakMap<Channel<unknown>, number>();

/**
 * Delivers one message to `channel`, exactly as a real Tauri webview would when the backend posts
 * a channel message (see the module doc comment above). Requires an active `mockIPC()` — throws
 * instead of silently doing nothing if `window.__TAURI_INTERNALS__.runCallback` isn't there.
 */
export function deliverChannelMessage<T>(channel: Channel<T>, message: T): void {
  const { runCallback } = internals();
  if (!runCallback) {
    throw new Error(
      "deliverChannelMessage: no active mockIPC() (window.__TAURI_INTERNALS__.runCallback is missing)",
    );
  }
  const key = channel as unknown as Channel<unknown>;
  const index = nextIndex.get(key) ?? 0;
  nextIndex.set(key, index + 1);
  runCallback(channel.id, { message, index });
}
