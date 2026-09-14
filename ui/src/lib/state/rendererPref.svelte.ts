import type { RendererPreference } from "../render/rendererMode";

/**
 * H-13: the "or a setting" clause of ADR-009 §4's renderer choice. This is a local, in-memory
 * preference — not a persisted app `Settings` field — since this ticket makes no Rust changes
 * (CLAUDE.md: "No new dependencies unless the ticket or an ADR names them" and the ticket's own
 * "No Rust changes expected"). A future ticket that wires a real settings-panel control should
 * back this with `Settings.renderer_preference` (ts-rs) instead of reinventing storage here; see
 * the H-13 ticket report's open question.
 */
let preference = $state<RendererPreference>("auto");

export function rendererPref(): { readonly value: RendererPreference } {
  return {
    get value() {
      return preference;
    },
  };
}

export function setRendererPreference(next: RendererPreference): void {
  preference = next;
}
