<script lang="ts">
  import { t } from "../i18n";
  import type { ParamInfoDto, ParamValueDto } from "../ipc/bindings";
  import { localized } from "./localized";
  import { setParamNormalized, setParamText } from "./rack.svelte";

  /**
   * One parameter's widget, derived only from its schema (SPEC-012 §2.6): a readout for
   * READ_ONLY, a toggle for BOOL, a dropdown for an enum, otherwise an Audition-style slider
   * (detented when STEPPED) plus a value field. Widgets never format or parse a value themselves
   * — `value.text` is always Rust's `value_to_text`, and committing typed text calls
   * `param_set_text`, which Rust parses.
   */
  let {
    slot,
    param,
    value,
  }: {
    slot: number;
    param: ParamInfoDto;
    value: ParamValueDto | undefined;
  } = $props();

  const normalized = $derived(value?.normalized ?? param.min);
  const text = $derived(value?.text ?? "");
  const plain = $derived(value?.value ?? param.default);
  const readOnly = $derived(param.flags.read_only);

  let editing = $state(false);
  let draft = $state("");
  let invalid = $state(false);

  function clamp01(n: number): number {
    return Math.min(1, Math.max(0, n));
  }

  function stepNormalized(): number {
    return param.step ? param.step / (param.max - param.min) : 0.01;
  }

  function startDrag(event: PointerEvent): void {
    if (readOnly) {
      return;
    }
    const el = event.currentTarget as HTMLElement;
    el.setPointerCapture(event.pointerId);
    const startX = event.clientX;
    const startNorm = normalized;
    const rect = el.getBoundingClientRect();
    const move = (e: PointerEvent) => {
      const scale = e.shiftKey ? 0.1 : 1; // Shift = fine (SPEC-012 §2.6)
      const delta = ((e.clientX - startX) / Math.max(1, rect.width)) * scale;
      setParamNormalized(slot, param.id, clamp01(startNorm + delta));
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  function onWheel(event: WheelEvent): void {
    if (readOnly) {
      return;
    }
    event.preventDefault();
    const dir = event.deltaY < 0 ? 1 : -1;
    setParamNormalized(slot, param.id, clamp01(normalized + dir * stepNormalized()));
  }

  function resetToDefault(): void {
    if (readOnly) {
      return;
    }
    void setParamText(slot, param.id, String(param.default));
  }

  function startEdit(): void {
    if (readOnly || param.flags.boolean) {
      return;
    }
    draft = text;
    invalid = false;
    editing = true;
  }

  async function commitEdit(): Promise<void> {
    if (!editing) {
      return;
    }
    const ok = await setParamText(slot, param.id, draft, { notifyOnError: false });
    if (ok) {
      editing = false;
      invalid = false;
    } else {
      invalid = true;
    }
  }

  function cancelEdit(): void {
    editing = false;
    invalid = false;
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      void commitEdit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      cancelEdit();
    }
  }

  function toggleBool(): void {
    void setParamText(slot, param.id, plain >= 0.5 ? "0" : "1");
  }

  function onEnumChange(event: Event & { currentTarget: HTMLSelectElement }): void {
    void setParamText(slot, param.id, event.currentTarget.value);
  }
</script>

<div class="param" data-testid="param-row" data-key={param.key}>
  <span class="label">{localized(param.name)}</span>
  {#if readOnly}
    <span class="readout" data-testid="param-readout">{text}</span>
  {:else if param.flags.boolean}
    <button
      type="button"
      class="toggle"
      class:on={plain >= 0.5}
      role="switch"
      aria-checked={plain >= 0.5}
      data-testid="param-toggle"
      onclick={toggleBool}
    >
      {text}
    </button>
  {:else if param.enum_labels.length > 0}
    <select data-testid="param-enum" value={String(Math.round(plain))} onchange={onEnumChange}>
      {#each param.enum_labels as label, i (i)}
        <option value={String(i)}>{localized(label)}</option>
      {/each}
    </select>
  {:else}
    <div class="control">
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <div
        class="slider"
        class:stepped={param.flags.stepped}
        role="slider"
        tabindex="0"
        aria-valuemin={param.min}
        aria-valuemax={param.max}
        aria-valuenow={plain}
        aria-label={localized(param.name)}
        data-testid="param-slider"
        onpointerdown={startDrag}
        onwheel={onWheel}
        ondblclick={resetToDefault}
      >
        <div class="fill" style={`width: ${normalized * 100}%`}></div>
      </div>
      {#if editing}
        <input
          class="value-input"
          class:invalid
          data-testid="param-value-input"
          value={draft}
          oninput={(e) => (draft = e.currentTarget.value)}
          onkeydown={onKeydown}
          onblur={commitEdit}
        />
      {:else}
        <button type="button" class="value" data-testid="param-value" onclick={startEdit}>
          {text}
        </button>
      {/if}
    </div>
  {/if}
  {#if invalid}
    <span class="error" data-testid="param-error">{t("rack.param.invalid")}</span>
  {/if}
</div>

<style>
  /* H-25: label column, then the control; slider track and value box follow the kit. */
  .param {
    display: grid;
    grid-template-columns: minmax(6.5rem, 38%) minmax(0, 1fr);
    align-items: center;
    gap: var(--pv-space-2);
    min-height: var(--pv-control-h-sm);
    font-family: var(--pv-font-sans);
  }

  .label {
    overflow: hidden;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .readout {
    color: var(--pv-text-primary);
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
  }

  .toggle {
    justify-self: start;
    height: 22px;
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-secondary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
    cursor: default;
  }

  .toggle.on {
    border-color: var(--pv-accent);
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
  }

  select {
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
  }

  .control {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 4.75rem;
    align-items: center;
    gap: var(--pv-space-2);
  }

  .slider {
    position: relative;
    height: 6px;
    border-radius: var(--pv-radius-full);
    background: var(--pv-control-track);
    box-shadow: inset 0 0 0 var(--pv-border-width) var(--pv-border);
    cursor: ew-resize;
    touch-action: none;
  }

  .slider::after {
    content: "";
    position: absolute;
    inset: -8px 0;
  }

  .slider.stepped {
    background-image: repeating-linear-gradient(
      to right,
      var(--pv-border-strong) 0,
      var(--pv-border-strong) 1px,
      transparent 1px,
      transparent 12.5%
    );
  }

  .slider:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 4px;
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: var(--pv-radius-full);
    background: var(--pv-accent);
    pointer-events: none;
  }

  .value,
  .value-input {
    height: 22px;
    padding: 0 var(--pv-space-1);
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-field-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
    font-variant-numeric: tabular-nums;
    text-align: right;
    white-space: nowrap;
    cursor: text;
  }

  .value:hover {
    border-color: var(--pv-border-control);
  }

  .value-input {
    width: 100%;
    border-color: var(--pv-accent);
  }

  .value-input.invalid {
    border-color: var(--pv-danger-text);
  }

  .error {
    grid-column: 2;
    color: var(--pv-danger-text);
    font-size: var(--pv-text-xs);
  }
</style>
