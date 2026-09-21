/**
 * Whether the "Explain My Voice" modal is open (H-92). This is H-92's own concern; the frozen
 * analysis itself stays entirely H-91's (`explainVoice.svelte.ts`'s
 * `freezeExplainSnapshot`/`clearExplainSnapshot`) — this store only sequences the two calls a
 * button press and a close need: freeze-then-open, and close-then-forget.
 */
import { clearExplainSnapshot, freezeExplainSnapshot, resetExplainVoiceForTest } from "./explainVoice.svelte";
import type { VoiceSnapshotInput } from "./snapshot";

let open = $state(false);

/** Read-only accessor for components. */
export function explainModalState(): { readonly open: boolean } {
  return {
    get open() {
      return open;
    },
  };
}

/** Freezes `input` (H-91) and opens the modal on it. The live analyzer keeps running
 * underneath, untouched — freezing only ever reads the analyzer state, never pauses it. */
export function openExplainVoice(input: VoiceSnapshotInput): void {
  freezeExplainSnapshot(input);
  open = true;
}

/** Closes the modal and drops the frozen analysis, returning to plain live analysis. */
export function closeExplainVoice(): void {
  open = false;
  clearExplainSnapshot();
}

/** Test/teardown helper. */
export function resetExplainModalForTest(): void {
  open = false;
  resetExplainVoiceForTest();
}
