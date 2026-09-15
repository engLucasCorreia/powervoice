/**
 * Applies an "Add EQ band here" move (H-42, SPEC-007 §8.10) through the rack store's normal
 * commands: `rack_add` for a Parametric EQ when the rack has none (appended at the end), then one
 * `param_set_plain` per parameter of the plan from `eqSuggest.ts`. The caller shows the notice.
 */
import type { RackStateDto } from "../ipc/bindings";
import { addModule, rackState, setParamPlain } from "../rack/rack.svelte";
import type { EqAction } from "./diagnosticsHints";
import { EQ_MODULE_ID, eqSlotIndex, planEqAction, type EqPlan } from "./eqSuggest";

export type EqApplyResult =
  | { outcome: "applied"; band: EqPlan["band"] }
  | { outcome: "no_free_band" }
  | { outcome: "failed" };

export async function applyEqAction(
  action: EqAction,
  state: RackStateDto = rackState().state,
): Promise<EqApplyResult> {
  let current = state;
  let index = eqSlotIndex(current);
  if (index === null) {
    await addModule(EQ_MODULE_ID, current.slots.length);
    current = rackState().state;
    index = eqSlotIndex(current);
  }
  const slot = index === null ? undefined : current.slots[index];
  if (index === null || !slot) {
    return { outcome: "failed" };
  }
  const plan = planEqAction(slot, action);
  if (!plan) {
    return { outcome: "no_free_band" };
  }
  for (const set of plan.sets) {
    await setParamPlain(index, set.id, set.value);
  }
  return { outcome: "applied", band: plan.band };
}
