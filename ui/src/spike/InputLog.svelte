<script lang="ts">
  import { t } from "../lib/i18n";

  interface LogEntry {
    id: number;
    text: string;
  }

  let entries = $state<LogEntry[]>([]);
  let nextId = 0;
  let dragStart: { x: number; y: number } | null = $state(null);

  function log(text: string): void {
    const stamp = new Date().toISOString().slice(11, 23);
    entries = [{ id: nextId++, text: `${stamp}  ${text}` }, ...entries].slice(0, 40);
  }

  function describeKey(e: KeyboardEvent): string {
    const mods = [e.ctrlKey && "Ctrl", e.shiftKey && "Shift", e.altKey && "Alt", e.metaKey && "Meta"]
      .filter(Boolean)
      .join("+");
    return mods ? `${mods}+${e.code}` : e.code;
  }

  function onKeydown(e: KeyboardEvent): void {
    log(`keydown  ${describeKey(e)}`);
  }

  function onKeyup(e: KeyboardEvent): void {
    log(`keyup    ${describeKey(e)}`);
  }

  function onPointerDown(e: PointerEvent): void {
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const rect = target.getBoundingClientRect();
    dragStart = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    log(`pointerdown  page=(${e.clientX},${e.clientY})  local=(${dragStart.x.toFixed(0)},${dragStart.y.toFixed(0)})`);
  }

  function onPointerMove(e: PointerEvent): void {
    if (!dragStart) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const local = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    log(
      `pointermove  page=(${e.clientX},${e.clientY})  local=(${local.x.toFixed(0)},${local.y.toFixed(0)})  delta=(${(local.x - dragStart.x).toFixed(0)},${(local.y - dragStart.y).toFixed(0)})`,
    );
  }

  function onPointerUp(e: PointerEvent): void {
    dragStart = null;
    log(`pointerup    page=(${e.clientX},${e.clientY})`);
  }
</script>

<svelte:window onkeydown={onKeydown} onkeyup={onKeyup} />

<section class="input-log" data-testid="spike-input-log">
  <h2>{t("spike.inputLog.title")}</h2>
  <p class="hint">{t("spike.inputLog.hint")}</p>
  <div
    class="drag-surface"
    data-testid="spike-drag-surface"
    role="presentation"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
  >
    {t("spike.inputLog.dragSurface")}
  </div>
  <ul data-testid="spike-input-log-entries">
    {#each entries as entry (entry.id)}
      <li>{entry.text}</li>
    {/each}
  </ul>
</section>

<style>
  .input-log {
    border-top: 1px solid var(--surface-border);
    padding-top: 12px;
    margin-top: 16px;
  }

  .hint {
    color: var(--text-secondary);
    max-width: 60ch;
  }

  .drag-surface {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    max-width: 420px;
    height: 96px;
    border: 1px dashed var(--surface-border);
    border-radius: 4px;
    background: var(--surface-inset);
    color: var(--text-secondary);
    user-select: none;
    touch-action: none;
  }

  ul {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    max-height: 200px;
    overflow-y: auto;
    font-family: monospace;
    font-size: 11px;
  }

  li {
    padding: 1px 0;
    border-bottom: 1px solid var(--surface-border);
  }
</style>
