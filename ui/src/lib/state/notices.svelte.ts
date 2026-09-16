import { listen } from "@tauri-apps/api/event";
import type { EventName, Notice } from "../ipc/bindings";

/** A `Notice` plus a local id the UI uses as a Svelte `{#each}` key / dismiss handle. */
export interface ActiveNotice extends Notice {
  localId: string;
}

const TOAST_AUTO_DISMISS_MS = 4000;

let toasts = $state<ActiveNotice[]>([]);
let banners = $state<ActiveNotice[]>([]);
let counter = 0;
/** H-59: which push a banner id currently shows, so an expired auto-dismiss timer only removes
 * the banner it was started for (see `pushNotice`). */
const bannerGenerations = new Map<string, number>();

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
 * non-persistent notice is a toast that auto-dismisses after ~4s. H-59: a banner carrying
 * `auto_dismiss_ms` also dismisses itself after that long (SPEC-001 §2.3's "reconnected" banner).
 *
 * H-17: `cleared` instead *removes* the banner sharing `id` (e.g. SPEC-004 §2.5's disk-almost-full
 * banner once space is reclaimed) — nothing is added, so the returned id is only meaningful when
 * something was actually removed.
 */
export function pushNotice(notice: Notice): string {
  if (notice.cleared) {
    const localId = notice.id ?? "";
    dismissBanner(localId);
    return localId;
  }
  if (notice.persistent) {
    const localId = notice.id ?? nextLocalId();
    banners = [...banners.filter((b) => b.localId !== localId), { ...notice, localId }];
    counter += 1;
    const generation = counter;
    bannerGenerations.set(localId, generation);
    if (notice.auto_dismiss_ms !== null && notice.auto_dismiss_ms > 0) {
      // H-59 (SPEC-001 §2.3): a banner that goes away on its own — the "device reconnected"
      // banner replaces the "disconnected" one and then dismisses itself after ~4s. Dismiss only
      // if *this* banner is still the one showing under that id: a newer banner reusing the id
      // (a second disconnect before the timer fires) must not be swept away by the older timer.
      // Identity is a generation counter, not the object — `$state` hands back a proxy, so the
      // pushed object is never `===` the one in the array.
      setTimeout(() => {
        if (bannerGenerations.get(localId) === generation) {
          dismissBanner(localId);
        }
      }, notice.auto_dismiss_ms);
    }
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
  bannerGenerations.delete(localId);
}

/** Test/teardown helper. */
export function clearNotices(): void {
  toasts = [];
  banners = [];
  bannerGenerations.clear();
}

/**
 * S2-02: subscribes to the backend's `notice` event (ADR-003) and pushes every one it sends —
 * device-lost/reconnected banners (S1-01/T-104), recording notices (S1-04) and normalize's
 * `notice.normalize_silent`/`notice.normalize_already` (SPEC-010 §2.7) all arrive this way.
 * Returns the teardown.
 */
export async function initNotices(): Promise<() => void> {
  try {
    return await listen<Notice>("notice" satisfies EventName, (e) => {
      pushNotice(e.payload);
    });
  } catch {
    // Without the event, notices only follow direct command failures (still functional).
    return () => {};
  }
}
