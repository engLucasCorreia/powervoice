import { Channel, invoke } from "@tauri-apps/api/core";
import { describePayloadType } from "./binary";
import { withTimeout } from "./timeout";
import type { IpcThroughputResult } from "./types";

const MB = 1024 * 1024;
/** 10 MB over a local IPC transport should complete in well under a second; this is a generous
 * upper bound so a genuinely broken transport times out instead of hanging the suite. */
const TIMEOUT_MS = 15_000;

function toResult(mechanism: "channel" | "response", buf: ArrayBuffer, elapsedMs: number): IpcThroughputResult {
  return {
    mechanism,
    bytes: buf.byteLength,
    elapsedMs,
    mbPerSecond: elapsedMs > 0 ? buf.byteLength / MB / (elapsedMs / 1000) : 0,
    payloadType: describePayloadType(buf),
  };
}

/** 10 MB via `ipc::Response`, timed end-to-end from `invoke()` call to promise resolution. */
async function measureResponseThroughput(): Promise<IpcThroughputResult> {
  const t0 = performance.now();
  const buf = await withTimeout(invoke<ArrayBuffer>("spike_ipc_response_10mb"), TIMEOUT_MS, () => {
    throw new Error(`spike_ipc_response_10mb timed out after ${TIMEOUT_MS}ms`);
  });
  const elapsedMs = performance.now() - t0;
  return toResult("response", buf, elapsedMs);
}

/** 10 MB via a `Channel`, timed end-to-end from `invoke()` call to the channel message arriving
 * (not to `invoke()`'s own — separate — resolution, since the bytes travel as a distinct
 * message). */
async function measureChannelThroughput(): Promise<IpcThroughputResult> {
  const t0 = performance.now();
  let resolveFrame: (buf: ArrayBuffer) => void = () => {};
  const framePromise = new Promise<ArrayBuffer>((resolve) => {
    resolveFrame = resolve;
  });
  const channel = new Channel<ArrayBuffer>((message) => resolveFrame(message));
  await invoke("spike_ipc_channel_10mb", { channel });
  const buf = await withTimeout(framePromise, TIMEOUT_MS, () => {
    throw new Error(`spike_ipc_channel_10mb timed out after ${TIMEOUT_MS}ms waiting for the Channel frame`);
  });
  const elapsedMs = performance.now() - t0;
  return toResult("channel", buf, elapsedMs);
}

/** ADR-003 scope item 3: 10 MB via `Channel` and via `Response`, measured end-to-end in ms. */
export async function runIpcThroughputBenches(): Promise<IpcThroughputResult[]> {
  return [await measureResponseThroughput(), await measureChannelThroughput()];
}
