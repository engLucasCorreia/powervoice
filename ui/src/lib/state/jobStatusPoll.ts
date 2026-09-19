import type { JobProgressDto } from "../ipc/bindings";
import { jobStatusGet } from "../ipc/commands";

/**
 * H-96 "belt and braces": every job store (export, normalize peak/LUFS, bake) subscribes to
 * `job_progress` *before* starting its job (the ordering fix for the actual bug this ticket
 * reports), so the terminal event should never be missed again. This poll is the fallback for if
 * it ever is anyway — a dropped IPC message, a webview reload mid-job, anything the ordering fix
 * doesn't cover — so a job never shows "running" forever with no way out.
 *
 * Polls `job_status(jobId)` every `intervalMs` while `stillRunning()` says the caller's own store
 * still thinks the job is running, applying whatever the backend's last known status was via
 * `apply` (normally the same `applyXJobProgress` function the real `job_progress` listener uses,
 * so a recovered terminal status drives the store exactly like a real event would). Stops itself
 * once `stillRunning()` turns false (the store already reached a terminal state, on its own or
 * via a previous poll) — the caller only needs to call the returned teardown early, for a new job
 * starting or the store resetting.
 */
export function startJobStatusPoll(
  jobId: number,
  stillRunning: () => boolean,
  apply: (status: JobProgressDto) => void,
  intervalMs = 3000,
): () => void {
  const id = setInterval(() => {
    if (!stillRunning()) {
      clearInterval(id);
      return;
    }
    void jobStatusGet(jobId)
      .then((status) => {
        if (status) {
          apply(status);
        }
      })
      .catch(() => {
        // Best-effort recovery only — a failed query just waits for the next tick, and the real
        // `job_progress` listener (if it was ever going to arrive) is unaffected.
      });
  }, intervalMs);
  return () => clearInterval(id);
}
