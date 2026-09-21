<script lang="ts">
  import { t } from "../../i18n";
  import { Button } from "../../ui";
  import { formatFreqShort, type FindingAction } from "../diagnosticsHints";
  import { applyEqAction } from "../eqApply";
  import type { IpcError } from "../../ipc/bindings";
  import { noticeFromIpcError } from "../../notices/fromIpcError";
  import { pushNotice } from "../../state/notices.svelte";

  /**
   * The one button behind H-94's `FindingProse.action` / `FocusItem.action` (H-92): "Add EQ band
   * here" through the existing `eqApply.ts` rack path, or "Copy {freq}" to the clipboard where
   * there is no de-esser module yet (H-91's `FindingAction` shape — the same one
   * `DiagnosticsPanel.svelte` already applies, reused rather than given a second mechanism).
   */
  let { action, testid }: { action: FindingAction | null; testid: string } = $props();

  let busy = $state(false);

  function isIpcError(value: unknown): value is IpcError {
    return typeof value === "object" && value !== null && "code" in value && "key" in value;
  }

  function notify(level: "info" | "warning", key: string, params: Record<string, string> = {}): void {
    pushNotice({ level, key, params, persistent: false, id: null, cleared: false, auto_dismiss_ms: null, action: null });
  }

  async function addEq(eq: NonNullable<Extract<FindingAction, { type: "eq" }>>["eq"]): Promise<void> {
    if (busy) {
      return;
    }
    busy = true;
    try {
      const result = await applyEqAction(eq);
      if (result.outcome === "applied") {
        const freq = formatFreqShort(eq.freqHz);
        notify(
          "info",
          result.band === "hp" ? "notice.analyzer.eq_hp_added" : "notice.analyzer.eq_added",
          result.band === "hp"
            ? { freq }
            : { band: String(result.band), what: t(`analyzer.eq_kind.${eq.kind}` as `analyzer.eq_kind.${typeof eq.kind}`), freq },
        );
      } else if (result.outcome === "no_free_band") {
        notify("warning", "notice.analyzer.eq_no_free_band");
      }
    } catch (err) {
      if (isIpcError(err)) {
        pushNotice(noticeFromIpcError(err));
      }
    } finally {
      busy = false;
    }
  }

  async function copyFreq(freqHz: number): Promise<void> {
    try {
      await navigator.clipboard.writeText(String(Math.round(freqHz)));
    } catch {
      // No clipboard permission: the notice still names the frequency.
    }
    notify("info", "notice.analyzer.freq_copied", { freq: formatFreqShort(freqHz) });
  }
</script>

{#if action?.type === "eq"}
  <Button size="sm" variant="ghost" icon="add" disabled={busy} testid={`${testid}-add-eq`} onclick={() => void addEq(action.eq)}>
    {t("analyzer.diag.add_eq")}
  </Button>
{:else if action?.type === "copy"}
  <Button size="sm" variant="ghost" icon="copy" testid={`${testid}-copy`} onclick={() => void copyFreq(action.freqHz)}>
    {t("analyzer.diag.copy_freq", { freq: formatFreqShort(action.freqHz) })}
  </Button>
{/if}
