/**
 * Manage Presets… dialog state (H-22, SPEC-012 §2.7 follow-up): opened from the rack slot's
 * Presets ▸ submenu (scoped to that module) or Effects ▸ Rack Presets ▸ (scoped to the rack), it
 * lists user presets per module and rack presets with rename/delete/export, plus import — a
 * plain module-level store like every other dialog's (`plugins.svelte.ts`, `bake.svelte.ts`).
 */

export type ManagePresetsTab = "rack" | "module";

interface ManagePresetsState {
  open: boolean;
  tab: ManagePresetsTab;
  /** The module tab's selected module id, or `null` before one's ever been chosen. */
  moduleId: string | null;
}

let state = $state<ManagePresetsState>({ open: false, tab: "rack", moduleId: null });

/** Read-only accessor for components. */
export function managePresetsState(): {
  readonly open: boolean;
  readonly tab: ManagePresetsTab;
  readonly moduleId: string | null;
} {
  return {
    get open() {
      return state.open;
    },
    get tab() {
      return state.tab;
    },
    get moduleId() {
      return state.moduleId;
    },
  };
}

/** Opens the dialog on `tab` (`module` also selects `moduleId`, defaulting to whatever was last
 * selected — the dialog itself falls back to the first available module if that's `null`). */
export function openManagePresets(opts: { tab: ManagePresetsTab; moduleId?: string } = { tab: "rack" }): void {
  state = { open: true, tab: opts.tab, moduleId: opts.moduleId ?? state.moduleId };
}

export function closeManagePresets(): void {
  state = { ...state, open: false };
}

export function setManagePresetsTab(tab: ManagePresetsTab): void {
  state = { ...state, tab };
}

export function setManagePresetsModule(moduleId: string): void {
  state = { ...state, moduleId };
}

/** Test/teardown helper. */
export function resetManagePresetsForTest(): void {
  state = { open: false, tab: "rack", moduleId: null };
}
