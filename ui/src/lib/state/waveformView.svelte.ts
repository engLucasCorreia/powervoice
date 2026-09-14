import { sidecarViewSetWaveform } from "../ipc/commands";
import type { SelectionRange } from "../waveform/selection";

/**
 * Waveform viewport store (H-12, T-207/T-306 follow-up, SPEC-018 §2.6.5's `view.waveform`):
 * `startSample`/`samplesPerPixel` used to live as `EditorView`'s own local `$state`, bound
 * (`bind:`) into both `WaveformView` and `SpectralView` so a zoom/scroll gesture in either pane
 * updated both (SPEC-007 §2.3 "one viewport"). This module lifts that pair into a shared store so
 * `EditorView` can also persist it — debounced, per document, like `spectral.svelte.ts` — and
 * restore it on open (SPEC-018 §2.6.5: "restored exactly, then clamped ... An out-of-range
 * `samples_per_pixel` → zoom full").
 *
 * **Binding.** {@link waveformViewApi} returns an object whose `startSample`/`samplesPerPixel`
 * are get/set accessor properties over the shared module state, so `EditorView` can still do
 * `bind:startSample={wv.startSample}` exactly as it did with a local `let ... = $state(...)`.
 *
 * **Restore vs. zoom-to-fit.** The exact pixel viewport width is only known inside `WaveformView`
 * (its canvas container's measured size), so this store doesn't decide the final viewport itself.
 * Opening a document with a saved `waveform_view` records a *pending* restore keyed by
 * `audioKeyFor(rateHz, lenSamples)` ({@link setPendingRestore}); `WaveformView`'s existing
 * "zoom to fit a newly opened document" effect consumes it ({@link consumePendingRestore}) once
 * the viewport width is known, applying the clamp/zoom-full rule there (it already has
 * `clampSamplesPerPixel`/`clampStartSample` in scope). Selection and cursor need no viewport to
 * validate (SPEC-018 §2.6.5: "restored when within `[0, L]`"), so `document.svelte.ts` applies
 * those immediately on open.
 */

/** Matches `WaveformView.svelte`'s own zoom-to-fit effect's key (kept in one place). */
export function audioKeyFor(sampleRateHz: number, lenSamples: number): string {
  return `${sampleRateHz}:${lenSamples}`;
}

interface ViewportSnapshot {
  startSample: number;
  samplesPerPixel: number;
}

let state = $state<ViewportSnapshot>({ startSample: 0, samplesPerPixel: 1 });

interface PendingRestore {
  audioKey: string;
  startSample: number;
  samplesPerPixel: number;
}

let pendingRestore: PendingRestore | null = $state(null);

/** The get/set surface `EditorView` binds `WaveformView`/`SpectralView`'s props to. */
export interface WaveformViewApi {
  startSample: number;
  samplesPerPixel: number;
}

export function waveformViewApi(): WaveformViewApi {
  return {
    get startSample() {
      return state.startSample;
    },
    set startSample(value: number) {
      state = { ...state, startSample: value };
    },
    get samplesPerPixel() {
      return state.samplesPerPixel;
    },
    set samplesPerPixel(value: number) {
      state = { ...state, samplesPerPixel: value };
    },
  };
}

/** T-306-style debounce (matches `spectral.svelte.ts`'s `PERSIST_DEBOUNCE_MS`). */
const PERSIST_DEBOUNCE_MS = 250;
let persistTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * Debounced `sidecar_view_set_waveform` (fire-and-forget: a view-only change never marks the
 * document modified, SPEC-018 §2.4). `EditorView` calls this from an `$effect` that reads the
 * viewport, the selection and the transport's last-known (non-extrapolated) cursor position, so
 * it re-fires on any of those changing.
 */
export function schedulePersistWaveformView(
  startSample: number,
  samplesPerPixel: number,
  selection: SelectionRange | null,
  cursorSamples: number,
): void {
  if (persistTimer !== null) {
    clearTimeout(persistTimer);
  }
  persistTimer = setTimeout(() => {
    persistTimer = null;
    void sidecarViewSetWaveform({
      start_sample: startSample,
      samples_per_pixel: samplesPerPixel,
      selection: selection
        ? { start_sample: selection.startSample, end_sample: selection.endSample }
        : null,
      cursor_samples: cursorSamples,
    }).catch(() => {
      // Fire-and-forget, same as `spectral.svelte.ts`: no document open, or the IPC call itself
      // failed — the next Save just won't carry this particular viewport tweak.
    });
  }, PERSIST_DEBOUNCE_MS);
}

/**
 * T-306/H-12: records a restored `waveform_view` (`document.svelte.ts`, right after a successful
 * open) as pending for `audioKey` — `WaveformView`'s zoom-to-fit effect consumes it once it knows
 * the viewport's pixel width. `null` (no sidecar view) clears any stale pending restore instead.
 */
export function setPendingRestore(
  audioKey: string,
  startSample: number,
  samplesPerPixel: number,
): void {
  pendingRestore = { audioKey, startSample, samplesPerPixel };
}

/** Clears a pending restore without applying it (e.g. a document with no saved view opens). */
export function clearPendingRestore(): void {
  pendingRestore = null;
}

/**
 * Consumes the pending restore for `audioKey`, if any (one-shot: a later resize of the same
 * document must not re-apply it and fight the user's own zoom/scroll).
 */
export function consumePendingRestore(audioKey: string): PendingRestore | null {
  if (pendingRestore && pendingRestore.audioKey === audioKey) {
    const restore = pendingRestore;
    pendingRestore = null;
    return restore;
  }
  return null;
}

/** Test/teardown helper. */
export function resetWaveformViewForTest(): void {
  if (persistTimer !== null) {
    clearTimeout(persistTimer);
    persistTimer = null;
  }
  state = { startSample: 0, samplesPerPixel: 1 };
  pendingRestore = null;
}
