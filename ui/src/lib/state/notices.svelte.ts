import type { IpcError, Notice } from "../ipc/bindings";

/** A `Notice` plus a local id the UI uses as a Svelte `{#each}` key / dismiss handle. */
export interface ActiveNotice extends Notice {
  localId: string;
}

const TOAST_AUTO_DISMISS_MS = 4000;

let toasts = $state<ActiveNotice[]>([]);
let banners = $state<ActiveNotice[]>([]);
let counter = 0;

function nextLocalId(): string {
  counter += 1;
  return `notice-${counter}`;
}

/** Read-only accessor for components: `noticesState().toasts` / `.banners`. */
export function noticesState(): { readonly toasts: ActiveNotice[]; readonly banners: ActiveNotice[] } {
  return {
    get toasts() {
      return toasts;
    },
    get banners() {
      return banners;
    },
  };
}

/**
 * Adds a notice (ADR-003 `notice` event, or a locally-built one from an `IpcError` — see
 * `fromIpcError.ts`). A persistent notice is a banner; a banner sharing an existing banner's `id`
 * replaces it in place (e.g. a device-lost banner turning into "reconnected" — SPEC-001 §2.3). A
 * non-persistent notice is a toast that auto-dismisses after ~4s.
 */
export function pushNotice(notice: Notice): string {
  if (notice.persistent) {
    const localId = notice.id ?? nextLocalId();
    banners = [...banners.filter((b) => b.localId !== localId), { ...notice, localId }];
    return localId;
  }
  const localId = nextLocalId();
  toasts = [...toasts, { ...notice, localId }];
  setTimeout(() => dismissToast(localId), TOAST_AUTO_DISMISS_MS);
  return localId;
}

export function dismissToast(localId: string): void {
  toasts = toasts.filter((toast) => toast.localId !== localId);
}

export function dismissBanner(localId: string): void {
  banners = banners.filter((banner) => banner.localId !== localId);
}

/** Test/teardown helper. */
export function clearNotices(): void {
  toasts = [];
  banners = [];
}
