import { tDynamic } from "../i18n";
import type { LocalizedTextDto } from "../ipc/bindings";

/**
 * Displays a module/parameter/group's localized text (ADR-005 §2): the i18n key when the module
 * declared one (every built-in), else the fallback text an adapter/plugin provides directly.
 */
export function localized(l: LocalizedTextDto): string {
  return l.key ? tDynamic(l.key) : l.text;
}
