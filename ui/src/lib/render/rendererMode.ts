/**
 * Renderer selection (H-13, ADR-009 §2/§4): WebGL2 is the primary renderer for both the waveform
 * (SPEC-006 §2.13) and the spectral pane (SPEC-007 §2.13); Canvas2D is the automatic fallback
 * when WebGL2 is unavailable, a context is lost, or a setting forces it. This module holds the
 * pure decision (no DOM) so it's directly testable; {@link ../render/glContext.ts} does the real
 * `getContext`/`webglcontextlost` wiring around it.
 */

export type RendererKind = "webgl2" | "canvas2d";

/** "auto" (ADR-009's default: WebGL2 when available) or an explicit forced choice — the "or a
 * setting" clause of H-13's scope. Forcing "webgl2" without support still falls back (§4: a
 * `null`/throwing context creation always means Canvas2D — there is no third state). */
export type RendererPreference = "auto" | RendererKind;

/**
 * Picks the renderer kind for a fresh view (ADR-009 §4): an explicit `"canvas2d"` preference
 * always wins; otherwise WebGL2 is used only if it's actually available (a preference of
 * `"webgl2"` cannot force a broken/absent context to work).
 */
export function chooseRenderer(preference: RendererPreference, webgl2Available: boolean): RendererKind {
  if (preference === "canvas2d") {
    return "canvas2d";
  }
  return webgl2Available ? "webgl2" : "canvas2d";
}

/**
 * ADR-009 §4: once a WebGL2 context is lost mid-session, the view falls back to Canvas2D **for
 * the remainder of its lifetime** rather than attempting to recreate the context repeatedly. This
 * models that one-way latch: once `lost` is `true`, the kind is `"canvas2d"` regardless of
 * `preference`, until the view (and this object) is torn down and recreated.
 */
export function rendererKindAfterLoss(
  preference: RendererPreference,
  webgl2Available: boolean,
  lost: boolean,
): RendererKind {
  if (lost) {
    return "canvas2d";
  }
  return chooseRenderer(preference, webgl2Available);
}
