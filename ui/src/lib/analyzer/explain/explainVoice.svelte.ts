/**
 * The frozen *Explain My Voice* snapshot, held for as long as the report is open (H-91 §1).
 *
 * The analyzer keeps streaming underneath — that is the point of freezing — so this store's one
 * job is to make sure the analysis is built **once per click** and then left alone:
 * {@link freezeExplainSnapshot} memoizes on the identity of what it was given (the curve
 * arrays, the report object, the smoothing width and the origin), so a component that
 * re-evaluates on every live frame gets the same object back instead of a new analysis.
 *
 * The snapshot itself is `$state.raw`: it is a deep, immutable result, never mutated in place.
 */
import { buildVoiceSnapshot, type VoiceSnapshot, type VoiceSnapshotInput } from "./snapshot";

let snapshot = $state.raw<VoiceSnapshot | null>(null);
let memo: { input: VoiceSnapshotInput; result: VoiceSnapshot } | null = null;

/** Read-only accessor for components. */
export function explainVoiceState(): { readonly snapshot: VoiceSnapshot | null } {
  return {
    get snapshot() {
      return snapshot;
    },
  };
}

function sameInput(a: VoiceSnapshotInput, b: VoiceSnapshotInput): boolean {
  return (
    a.freqsHz === b.freqsHz &&
    a.levelsDb === b.levelsDb &&
    a.report === b.report &&
    a.origin === b.origin &&
    a.resolution === b.resolution &&
    a.sampleRateHz === b.sampleRateHz &&
    a.smoothingOct === b.smoothingOct
  );
}

/**
 * Freezes the current analysis and returns it. Calling this again with the same inputs returns
 * the same object without recomputing anything.
 */
export function freezeExplainSnapshot(input: VoiceSnapshotInput): VoiceSnapshot {
  if (memo && sameInput(memo.input, input)) {
    snapshot = memo.result;
    return memo.result;
  }
  const result = buildVoiceSnapshot(input);
  memo = { input, result };
  snapshot = result;
  return result;
}

/** Drops the frozen analysis (the report was closed). */
export function clearExplainSnapshot(): void {
  snapshot = null;
  memo = null;
}

/** Test/teardown helper. */
export function resetExplainVoiceForTest(): void {
  clearExplainSnapshot();
}
