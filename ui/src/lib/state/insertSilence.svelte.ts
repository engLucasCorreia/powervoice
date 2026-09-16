import type { EditTargetDto, IpcError } from "../ipc/bindings";
import { editInsertSilence } from "../ipc/commands";
import { documentState } from "../document/document.svelte";
import { noticeFromIpcError } from "../notices/fromIpcError";
import { pushNotice } from "./notices.svelte";
import { hasSelection, selectionState, setSelectionFromResult } from "./selection.svelte";
import { transportState } from "./transport.svelte";
import { formatDocumentTime, formatSamplesValue } from "../waveform/timeFormat";
import { timeRulerFormatState } from "./waveformView.svelte";
import {
  insertSilenceInRange,
  insertSilenceMaxSamples,
  parseInsertSilenceDuration,
} from "../edit/insertSilenceDuration";

/**
 * Edit → Insert Silence… dialog state (H-56, SPEC-008 §2.5). Unlike Normalize's dialog, this
 * isn't a job — the insert itself is instant (pure Silence pieces, §2.10) — so there is no
 * progress dialog here; `editInsertSilence` resolves directly.
 *
 * "The last accepted value is remembered for the rest of the app session, not persisted" (§2.5):
 * module-level state, not `settings.svelte.ts`.
 */

interface DialogState {
  text: string;
  valid: boolean;
}

let rememberedText = "1.000";
let dialogOpen = $state(false);
let dialog = $state<DialogState>({ text: rememberedText, valid: true });
/** Captured when the dialog opens (SPEC-006 §2.9: the cursor/selection never changes while a
 * modal dialog is open — `listener.ts::isModalDialogOpen` blocks the keymap meanwhile). */
let dialogTarget: EditTargetDto | null = null;
let dialogAt = 0;
let dialogRateHz = 0;

function isIpcError(value: unknown): value is IpcError {
  return typeof value === "object" && value !== null && "code" in value && "key" in value;
}

function report(err: unknown): void {
  if (isIpcError(err)) {
    pushNotice(noticeFromIpcError(err));
  }
}

function khzLabel(hz: number): string {
  const khz = hz / 1000;
  const rounded = Math.round(khz * 10) / 10;
  return Number.isInteger(rounded) ? `${rounded} kHz` : `${rounded.toFixed(1)} kHz`;
}

/** Parses `text` against `dialogRateHz`, `null` for unparseable or out-of-range text. */
function parsedSamplesOf(text: string): number | null {
  if (!(dialogRateHz > 0)) {
    return null;
  }
  const samples = parseInsertSilenceDuration(text, dialogRateHz);
  return samples !== null && insertSilenceInRange(samples, dialogRateHz) ? samples : null;
}

/** Parses the dialog's current text. */
function parsedSamples(): number | null {
  return parsedSamplesOf(dialog.text);
}

/** Read-only accessor for `InsertSilenceDialog.svelte`. */
export function insertSilenceState(): {
  readonly dialogOpen: boolean;
  readonly dialogText: string;
  readonly dialogValid: boolean;
  /** "= 48 000 samples at 48 kHz" (§2.5), or the raw sample count in Samples ruler mode. */
  readonly lengthLabel: string | null;
  /** "at 0:12.345", in the current time-ruler format. */
  readonly positionLabel: string;
  readonly maxSamples: number;
} {
  return {
    get dialogOpen() {
      return dialogOpen;
    },
    get dialogText() {
      return dialog.text;
    },
    get dialogValid() {
      return dialog.valid;
    },
    get lengthLabel() {
      const samples = parsedSamples();
      if (samples === null) {
        return null;
      }
      const format = timeRulerFormatState().current;
      return format === "samples"
        ? formatSamplesValue(samples)
        : `${formatSamplesValue(samples)} samples at ${khzLabel(dialogRateHz)}`;
    },
    get positionLabel() {
      return formatDocumentTime(dialogAt, dialogRateHz, timeRulerFormatState().current);
    },
    get maxSamples() {
      return insertSilenceMaxSamples(dialogRateHz || 48_000);
    },
  };
}

/** `[cursor]` or `[selection start, selection end)` — same resolution `paste()` uses (SPEC-008
 * §2.1/§2.5: "`p = S` if a selection exists, otherwise `p = c`"). `null` with no document open. */
function currentTarget(): { editTarget: EditTargetDto; at: number } | null {
  const doc = documentState().current;
  if (!(doc.sample_rate_hz > 0)) {
    return null;
  }
  if (hasSelection()) {
    const sel = selectionState().current!;
    return {
      editTarget: { kind: "range", start_samples: sel.startSample, end_samples: sel.endSample },
      at: sel.startSample,
    };
  }
  const at = transportState().playheadSamples;
  return { editTarget: { kind: "cursor", at_samples: at }, at };
}

/** `true` whenever a document is open (SPEC-008 §2.2: "Insert Silence… is enabled with no
 * selection", and unlike the other six ops, with no document job/recording gate exposed to the
 * menu here — the command itself still refuses those, `error.not_while_recording`/
 * `error.document_busy`). */
export function canInsertSilence(): boolean {
  return documentState().current.sample_rate_hz > 0;
}

/** Edit → Insert Silence… (§2.5): opens with the last accepted value remembered this session. */
export function openInsertSilenceDialog(): void {
  const resolved = currentTarget();
  if (!resolved) {
    return;
  }
  dialogTarget = resolved.editTarget;
  dialogAt = resolved.at;
  dialogRateHz = documentState().current.sample_rate_hz;
  dialog = { text: rememberedText, valid: parsedSamplesOf(rememberedText) !== null };
  dialogOpen = true;
}

export function closeInsertSilenceDialog(): void {
  dialogOpen = false;
}

export function setInsertSilenceDialogText(text: string): void {
  dialog = { text, valid: parsedSamplesOf(text) !== null };
}

/** Enter / the dialog's OK button. A no-op while the field is invalid. */
export async function applyInsertSilenceDialog(): Promise<void> {
  const samples = parsedSamples();
  if (samples === null || !dialogTarget) {
    return;
  }
  rememberedText = dialog.text;
  dialogOpen = false;
  const target = dialogTarget;
  try {
    const result = await editInsertSilence(target, samples);
    setSelectionFromResult(result.selection);
  } catch (err) {
    report(err);
  }
}

/** Test/teardown helper. */
export function resetInsertSilenceForTest(): void {
  dialogOpen = false;
  rememberedText = "1.000";
  dialog = { text: rememberedText, valid: true };
  dialogTarget = null;
  dialogAt = 0;
  dialogRateHz = 0;
}
