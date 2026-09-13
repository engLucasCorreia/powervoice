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
  .param {
    display: grid;
    grid-template-columns: 8rem 1fr;
    align-items: center;
    gap: 0.5rem;
    padding: 0.15rem 0;
  }

  .label {
    color: var(--text-secondary);
    font-size: 0.8rem;
  }

  .readout {
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  .toggle {
    justify-self: start;
    background: var(--surface-inset);
    color: var(--text-secondary);
    border: 1px solid var(--surface-border);
    border-radius: 3px;
    padding: 0.1rem 0.6rem;
  }

  .toggle.on {
    background: var(--accent);
    color: var(--text-on-accent);
    border-color: var(--accent);
  }

  select {
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 3px;
    padding: 0.1rem 0.3rem;
  }

  .control {
    display: grid;
    grid-template-columns: 1fr 4rem;
    align-items: center;
    gap: 0.4rem;
  }

  .slider {
    position: relative;
    height: 0.9rem;
    background: var(--surface-inset);
    border: 1px solid var(--surface-border);
    border-radius: 3px;
    cursor: ew-resize;
    touch-action: none;
  }

  .slider.stepped {
    background-image: repeating-linear-gradient(
      to right,
      var(--surface-border) 0,
      var(--surface-border) 1px,
      transparent 1px,
      transparent 12.5%
    );
  }

  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--accent);
    border-radius: 2px;
    pointer-events: none;
  }

  .value,
  .value-input {
    font-variant-numeric: tabular-nums;
    text-align: right;
    background: var(--surface-panel-raised);
    color: var(--text-primary);
    border: 1px solid var(--surface-border);
    border-radius: 3px;
    padding: 0.1rem 0.3rem;
  }

  .value-input.invalid {
    border-color: var(--meter-red);
  }

  .error {
    grid-column: 2;
    color: var(--meter-red);
    font-size: 0.75rem;
  }
</style>
