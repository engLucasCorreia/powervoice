import type { AcxCheckReportDto, IpcError } from "../ipc/bindings";
import { acxCheck } from "../ipc/commands";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "../state/notices.svelte";
import { canAnalyzeLoudness, loudnessState } from "./loudness.svelte";

/**
 * ACX Check store (S4-03): the Loudness panel's "ACX Check" button. Synchronous — no job id, no
 * progress/cancel, unlike the analysis job above — and always the whole document (ACX submissions
 * are whole chapters, not selections). Reuses the Loudness panel's own "processed"/"source"
 * toggle ([`loudnessState`]) rather than adding a second one.
 */

let running = $state(false);
let report = $state<AcxCheckReportDto | null>(null);

/** Read-only accessor for components. */
export function acxState(): {
  readonly running: boolean;
  readonly report: AcxCheckReportDto | null;
} {
  return {
    get running() {
      return running;
    },
    get report() {
      return report;
    },
  };
}

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

/** The Loudness panel's "ACX Check" button — same document-open gating as `canAnalyzeLoudness`
 * (the panel's own `disabled` prop already checks this too; guarded again here so a direct call
 * from a test or a stray double-click can't fire the command with nothing open). */
export async function runAcxCheck(): Promise<void> {
  if (!canAnalyzeLoudness()) {
    return;
  }
  running = true;
  try {
    report = await acxCheck({ source: loudnessState().source });
  } catch (err) {
    if (isIpcError(err)) {
      pushNotice(noticeFromIpcError(err));
    }
  } finally {
    running = false;
  }
}

/** Test/teardown helper. */
export function resetAcxForTest(): void {
  running = false;
  report = null;
}
