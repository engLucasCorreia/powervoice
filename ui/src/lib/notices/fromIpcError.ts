import type { IpcError, Notice } from "../ipc/bindings";

/**
 * Converts a failed command's `IpcError` (ADR-003) into a one-shot error toast. Every command
 * caller should route a rejected `invoke()` through this (then `pushNotice(...)`) instead of
 * inventing its own error UI, so every command failure looks the same to the user.
 */
export function noticeFromIpcError(error: IpcError): Notice {
  return {
    level: "error",
    key: error.key,
    params: error.params,
    persistent: false,
    id: null,
    cleared: false,
    auto_dismiss_ms: null,
  };
}
