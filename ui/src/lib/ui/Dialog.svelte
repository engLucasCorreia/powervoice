<script lang="ts" module>
  let uid = 0;
</script>

<script lang="ts">
  import { onDestroy, onMount, type Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  /**
   * Modal dialog shell (H-25 phase 2, design-system §12): backdrop, a `role="dialog"` /
   * `"alertdialog"` box labelled by its 15 px title, a scrolling body and a right-aligned footer.
   * It takes focus on open (the box itself, so Escape works at once and Tab reaches the first
   * control), traps Tab inside, and returns focus to the opener on close. Key semantics (Escape
   * cancels, Enter applies…) stay with each dialog through `onkeydown`. Common form elements in
   * the body (fieldsets, radios, checkboxes, text inputs, selects, hints) get the system look
   * here, so feature dialogs carry no per-dialog chrome CSS.
   */
  type Size = "sm" | "md" | "lg";

  let {
    title,
    titleId,
    role = "dialog",
    size = "md",
    testid,
    onkeydown,
    children,
    footer,
    ...rest
  }: {
    title: string;
    titleId?: string;
    role?: "dialog" | "alertdialog";
    size?: Size;
    testid?: string;
    onkeydown?: (event: KeyboardEvent) => void;
    children: Snippet;
    footer?: Snippet;
  } & Omit<HTMLAttributes<HTMLDivElement>, "title" | "role" | "onkeydown" | "children"> = $props();

  uid += 1;
  const fallbackId = `pv-dialog-${uid}-title`;
  const headingId = $derived(titleId ?? fallbackId);

  let box: HTMLDivElement | undefined = $state();
  let opener: HTMLElement | null = null;

  const FOCUSABLE =
    'button:not([disabled]), [href], input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function focusables(): HTMLElement[] {
    return box ? [...box.querySelectorAll<HTMLElement>(FOCUSABLE)] : [];
  }

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === "Tab") {
      const items = focusables();
      const first = items[0];
      const last = items[items.length - 1];
      if (first && last) {
        const active = document.activeElement;
        if (event.shiftKey && (active === first || active === box)) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && active === last) {
          event.preventDefault();
          first.focus();
        }
      }
    }
    onkeydown?.(event);
  }

  onMount(() => {
    opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    queueMicrotask(() => {
      if (box && !box.contains(document.activeElement)) {
        box.focus();
      }
    });
  });

  onDestroy(() => {
    if (opener && opener.isConnected) {
      opener.focus();
    }
  });
</script>

<div class="pv-backdrop">
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div
    {...rest}
    bind:this={box}
    class="pv-dialog"
    {role}
    aria-modal="true"
    aria-labelledby={headingId}
    data-size={size}
    data-testid={testid}
    tabindex="-1"
    onkeydown={handleKeydown}
  >
    <h2 id={headingId} class="title">{title}</h2>
    <div class="body">
      {@render children()}
    </div>
    {#if footer}
      <div class="footer">{@render footer()}</div>
    {/if}
  </div>
</div>

<style>
  .pv-backdrop {
    position: fixed;
    inset: 0;
    z-index: var(--pv-z-dialog);
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--pv-space-6);
    background: var(--pv-bg-backdrop);
    animation: pv-fade var(--pv-duration-slow) var(--pv-ease-standard);
  }

  .pv-dialog {
    position: relative;
    display: flex;
    flex-direction: column;
    width: 480px;
    max-width: 90vw;
    max-height: 85vh;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-lg);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-3);
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    outline: none;
    animation: pv-rise var(--pv-duration-slow) var(--pv-ease-standard);
  }

  .pv-dialog[data-size="sm"] {
    width: 400px;
  }

  .pv-dialog[data-size="lg"] {
    width: 640px;
  }

  .title {
    flex: none;
    margin: 0;
    padding: var(--pv-space-5) var(--pv-space-5) var(--pv-space-3);
    font-size: var(--pv-text-lg);
    line-height: var(--pv-leading-lg);
    font-weight: var(--pv-weight-semibold);
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-3);
    min-height: 0;
    padding: 0 var(--pv-space-5) var(--pv-space-2);
    overflow-y: auto;
  }

  .footer {
    display: flex;
    flex: none;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--pv-space-2);
    padding: var(--pv-space-4) var(--pv-space-5) var(--pv-space-5);
  }

  /* ── The system look for form content inside any dialog body ── */
  .body :global(p) {
    margin: 0;
    color: var(--pv-text-secondary);
  }

  .body :global(h3) {
    margin: var(--pv-space-2) 0 0;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-weight: var(--pv-weight-semibold);
    color: var(--pv-text-secondary);
  }

  .body :global(fieldset) {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    margin: 0;
    padding: 0;
    border: none;
  }

  .body :global(legend) {
    margin-bottom: var(--pv-space-2);
    padding: 0;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    font-weight: var(--pv-weight-semibold);
    color: var(--pv-text-secondary);
  }

  .body :global(input[type="radio"]),
  .body :global(input[type="checkbox"]) {
    width: 14px;
    height: 14px;
    margin: 0;
    accent-color: var(--pv-accent);
  }

  .body :global(input[type="text"]),
  .body :global(input[type="number"]),
  .body :global(input:not([type])) {
    height: var(--pv-control-h-md);
    padding: 0 var(--pv-control-px-sm);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-md);
    background: var(--pv-field-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
    font-variant-numeric: tabular-nums;
  }

  .body :global(input[type="range"]) {
    accent-color: var(--pv-accent);
  }

  .body :global(select) {
    height: var(--pv-control-h-md);
    padding: 0 var(--pv-control-px-sm);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-md);
    background: var(--pv-control-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-md);
  }

  .body :global(input:focus-visible),
  .body :global(select:focus-visible) {
    border-color: var(--pv-accent);
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .body :global(input[aria-invalid="true"]),
  .body :global(input.invalid) {
    border-color: var(--pv-danger-text);
  }

  .body :global(.hint) {
    margin: 0;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    color: var(--pv-text-tertiary);
  }

  .body :global(.error) {
    margin: 0;
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
    color: var(--pv-danger-text);
  }

  .body :global(.warning) {
    color: var(--pv-warning-text);
  }

  /* Determinate job progress: a slim accent bar on a track. */
  .body :global(progress) {
    width: 100%;
    height: 6px;
    appearance: none;
    border: none;
    border-radius: var(--pv-radius-full);
    background: var(--pv-control-track);
    overflow: hidden;
  }

  .body :global(progress::-webkit-progress-bar) {
    background: var(--pv-control-track);
  }

  .body :global(progress::-webkit-progress-value) {
    background: var(--pv-accent);
    border-radius: var(--pv-radius-full);
  }

  .body :global(progress::-moz-progress-bar) {
    background: var(--pv-accent);
    border-radius: var(--pv-radius-full);
  }

  /* Layout helpers for dialog forms: `.options` (a row of radio/checkbox choices), `.option` (one
     choice: control + text), `.field-row` (label, control, unit/extra on one line). */
  .body :global(.options) {
    display: flex;
    flex-wrap: wrap;
    gap: var(--pv-space-2) var(--pv-space-4);
  }

  .body :global(.option) {
    display: inline-flex;
    align-items: center;
    gap: var(--pv-space-2);
    min-height: var(--pv-hit-min);
    color: var(--pv-text-primary);
  }

  .body :global(.field-row) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--pv-space-2);
  }

  .body :global(.field-row > label) {
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }

  .body :global(.unit) {
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
  }

  /* The footer's flexible gap: `<span class="spacer"></span>` pushes following buttons right. */
  .footer :global(.spacer) {
    flex: 1;
  }

  @keyframes pv-fade {
    from {
      opacity: 0;
    }
  }

  @keyframes pv-rise {
    from {
      opacity: 0;
      transform: translateY(4px) scale(0.99);
    }
  }
</style>
