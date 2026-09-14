/**
 * Live output analyzer store (T-208, SPEC-007 §2.9): subscribes to `VXSA` frames while the panel
 * (or, later, the EQ graph) is mounted, and holds the latest decoded frame for consumers. The
 * `AnalyzerPanel` component owns the peak-hold ballistics (`peakHold.ts`) itself, since each
 * consumer of this stream may want its own.
 *
 * **Persistence (H-16, SPEC-007 §2.9: "visibility and response settings are persisted").**
 * Visibility, response and the peak-hold toggle live here (not as `AnalyzerPanel` local state) so
 * they can be seeded from `Settings` before the panel ever mounts (`applyAnalyzerPrefs`, called by
 * `App.svelte` once settings load, mirroring `spectral.svelte.ts`'s `applySpectralDefaults`) and
 * so hiding the panel (`setAnalyzerVisible(false)`) doesn't lose the user's response/peak-hold
 * choice. Every setter here also debounces-free-writes straight to `settings_set` — cheap,
 * infrequent user actions, unlike the waveform viewport's drag-driven writes.
 */
import { Channel } from "@tauri-apps/api/core";
import type { AnalyzerResponseDto } from "../ipc/bindings";
import { analyzerSetResponse, analyzerSubscribe, analyzerUnsubscribe } from "../ipc/commands";
import { type AnalyzerFrame, decodeVxsa } from "../ipc/analyzer";
import { toArrayBuffer } from "../ipc/telemetry";
import { saveSettings } from "../state/settings.svelte";

let frame = $state.raw<AnalyzerFrame | undefined>(undefined);
let response = $state<AnalyzerResponseDto>("medium");
let peakHold = $state(true);
let visible = $state(true);
let subscriberId: number | undefined;

/** The latest decoded frame, and the persisted visibility/response/peak-hold prefs. */
export function analyzerState(): {
  readonly frame: AnalyzerFrame | undefined;
  readonly response: AnalyzerResponseDto;
  readonly visible: boolean;
  peakHold: boolean;
} {
  return {
    get frame() {
      return frame;
    },
    get response() {
      return response;
    },
    get visible() {
      return visible;
    },
    get peakHold() {
      return peakHold;
    },
    set peakHold(next: boolean) {
      setAnalyzerPeakHold(next);
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
 * Subscribes to the analyzer at the current response (seeded by {@link applyAnalyzerPrefs}
 * before the panel mounts, SPEC-007 §2.9's default is Medium). Returns a teardown function that
 * unsubscribes (the tap turns off once the last subscriber leaves).
 */
export async function initAnalyzer(): Promise<() => void> {
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

/** Changes the averaging response (Fast/Medium/Slow), persisted to `Settings`. */
export async function setAnalyzerResponse(next: AnalyzerResponseDto): Promise<void> {
  response = next;
  void saveSettings({ analyzer_response: next });
  if (subscriberId !== undefined) {
    try {
      await analyzerSetResponse(subscriberId, next);
    } catch {
      // The next subscribe (or a reconnect) will pick the setting up again.
    }
  }
}

/** Toggles peak hold, persisted to `Settings`. */
export function setAnalyzerPeakHold(next: boolean): void {
  peakHold = next;
  void saveSettings({ analyzer_peak_hold: next });
}

/** Shows/hides the panel (View → Analyzer), persisted to `Settings`. Hiding it unmounts the
 * panel, which tears down the subscription — SPEC-007 §2.9's "panel hidden and no subscriber →
 * completely off". */
export function setAnalyzerVisible(next: boolean): void {
  visible = next;
  void saveSettings({ analyzer_visible: next });
}

/** Seeds visibility/response/peak-hold from `Settings.analyzer_*` (App.svelte, once loaded). */
export function applyAnalyzerPrefs(prefs: {
  visible: boolean;
  response: AnalyzerResponseDto;
  peakHold: boolean;
}): void {
  visible = prefs.visible;
  response = prefs.response;
  peakHold = prefs.peakHold;
}

/** Test/teardown helper. */
export function resetAnalyzerForTest(): void {
  frame = undefined;
  response = "medium";
  peakHold = true;
  visible = true;
  subscriberId = undefined;
}
