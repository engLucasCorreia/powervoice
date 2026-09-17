import type { NoticeActionId } from "../ipc/bindings";
import { undo } from "../state/edit.svelte";
import { goToFirstDropout } from "../markers/markers.svelte";

/**
 * H-67: dispatches a `Notice.action.id` to the one thing the UI knows how to do for it. This is
 * the *only* place a `NoticeActionId` variant is wired to behaviour — the event itself never
 * carries a callback, so a new variant is safe to add on the backend without ever executing
 * arbitrary frontend code, and TypeScript flags a missing case here as soon as one is added.
 */
export function dispatchNoticeAction(id: NoticeActionId): void {
  switch (id) {
    case "go_to_first_dropout":
      goToFirstDropout();
      return;
    case "undo_marker_delete":
      // H-64 (SPEC-009 §2.6): the "Deleted N markers" toast's Undo button — the same
      // `history_undo` command as Ctrl+Z, since the delete is always the top undo entry right
      // after this toast appears.
      void undo();
      return;
    default: {
      // Exhaustiveness guard: a new `NoticeActionId` variant fails to compile here until handled.
      const _never: never = id;
      void _never;
    }
  }
}
