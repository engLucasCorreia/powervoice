/**
 * The one structural label left for H-92's own UI once H-94's `prose.ts`/`summary.ts` supplied
 * the finding/profile/focus wording: the name of a shaded voice-region band, which belongs to the
 * graph (H-92 scope item 3), not to any finding.
 */
import { t } from "../../i18n";
import type { VoiceBandId } from "./voiceBands";

/** "Rumble", "Low-mids", … — the shaded region's name. */
export function voiceBandLabel(id: VoiceBandId): string {
  return t(`explain.band.${id}` as `explain.band.${VoiceBandId}`);
}
