<script lang="ts">
  import { t, type MessageKey } from "../i18n";
  import { isPlatformMac, type ShortcutScope } from "../shortcuts/registry";
  import { shortcutRows, type ShortcutRow } from "../shortcuts/shortcutLabel";
  import { Dialog, Kbd } from "../ui";
  import { closeShortcutsDialog, shortcutsDialogState } from "./shortcutsDialog.svelte";

  /**
   * T-701: Help ▸ Keyboard Shortcuts — every registry entry, grouped by scope, with its
   * platform-formatted binding (`shortcutRows`, so this dialog can never disagree with what the
   * app actually does — same rule as the menu `Kbd` chips). Only scopes that currently have
   * entries get a heading ("dialog"/"text-input" are reserved, T-701's report explains why).
   */
  const SCOPE_ORDER: readonly ShortcutScope[] = ["global", "waveform", "dialog", "text-input"];
  const SCOPE_LABEL_KEYS: Record<ShortcutScope, MessageKey> = {
    global: "shortcuts.scope.global",
    waveform: "shortcuts.scope.waveform",
    dialog: "shortcuts.scope.dialog",
    "text-input": "shortcuts.scope.text-input",
  };

  const groups = $derived.by(() => {
    const rows = shortcutRows(isPlatformMac());
    const byScope = new Map<ShortcutScope, ShortcutRow[]>();
    for (const row of rows) {
      const list = byScope.get(row.entry.scope) ?? [];
      list.push(row);
      byScope.set(row.entry.scope, list);
    }
    return SCOPE_ORDER.map((scope) => ({ scope, rows: byScope.get(scope) ?? [] })).filter(
      (group) => group.rows.length > 0,
    );
  });

  function onKeydown(event: KeyboardEvent): void {
    event.stopPropagation();
    if (event.key === "Escape") {
      closeShortcutsDialog();
    }
  }
</script>

{#if shortcutsDialogState().open}
  <Dialog
    actions={[{ label: t("shortcuts.close"), role: "primary", testid: "shortcuts-close", onclick: closeShortcutsDialog }]}
    size="md"
    title={t("shortcuts.title")}
    titleId="shortcuts-dialog-title"
    testid="shortcuts-dialog"
    onkeydown={onKeydown}
  >
    <p>{t("shortcuts.not_remappable")}</p>
    {#each groups as group (group.scope)}
      <h3>{t(SCOPE_LABEL_KEYS[group.scope])}</h3>
      <ul class="rows">
        {#each group.rows as row (row.entry.action)}
          <li class="row" data-testid="shortcuts-row-{row.entry.action}">
            <span class="label">{t(row.entry.labelKey)}</span>
            <Kbd keys={row.display} />
          </li>
        {/each}
      </ul>
    {/each}
  </Dialog>
{/if}

<style>
  .rows {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--pv-space-4);
  }

  .label {
    color: var(--pv-text-primary);
    font-size: var(--pv-text-sm);
  }
</style>
