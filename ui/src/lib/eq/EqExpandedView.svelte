<script lang="ts">
  import { t } from "../i18n";
  import { rackState } from "../rack/rack.svelte";
  import { transportState } from "../state/transport.svelte";
  import { IconButton } from "../ui";
  import EqGraph from "./EqGraph.svelte";
  import {
    EQ_EXPANDED_MIN_H,
    EQ_EXPANDED_MIN_W,
    closeEqExpanded,
    eqExpandedState,
    placeEqExpandedIfNeeded,
    setEqExpandedRect,
  } from "./eqExpanded.svelte";

  /**
   * The EQ graph's expanded view (H-84, SPEC-015 §2.6.1): an in-app floating panel — default
   * 900 × 400 CSS px, resizable down to 480 × 240, moved by its header, closed by its close
   * button or Esc — showing the same content as the compact panel with a larger graph, which
   * stays live underneath (mounted once at `App.svelte` level, like `SpectrumInspector`, and
   * opened from anywhere via `openEqExpanded(uid)`). Not modal: the app's shortcuts keep working.
   *
   * Non-OS window **(decided, autonomous, T-400, SPEC-015 §2.6.1)**: avoids multi-window Tauri
   * work and Wayland focus problems (ADR-009) — the same reasoning `SpectrumInspector` follows.
   */

  /** Non-graph chrome inside the window: title bar + toolbar/band-labels rows + padding. */
  const CHROME_PX = 96;

  const view = eqExpandedState();
  let windowEl: HTMLDivElement | undefined = $state();

  const slotIndex = $derived(
    view.openUid === null ? -1 : rackState().state.slots.findIndex((s) => s.uid === view.openUid),
  );
  const slot = $derived(slotIndex >= 0 ? rackState().state.slots[slotIndex] : undefined);
  const rateHz = $derived(transportState().state.doc_rate_hz || 48_000);
  const open = $derived(view.openUid !== null && slot !== undefined && slot.curve_handles !== null);

  // The slot can disappear (removed, or its module swapped) while expanded — close rather than
  // show a stale/empty window.
  $effect(() => {
    if (view.openUid !== null && !open) {
      closeEqExpanded();
    }
  });

  $effect(() => {
    if (open && typeof window !== "undefined") {
      placeEqExpandedIfNeeded(window.innerWidth, window.innerHeight);
    }
  });

  const rect = $derived(view.rect ?? { x: 40, y: 40, w: 900, h: 400 });
  const graphHeightPx = $derived(Math.max(80, rect.h - CHROME_PX));

  function close(): void {
    closeEqExpanded();
  }

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }

  function drag(e: PointerEvent, kind: "move" | "resize"): void {
    if (e.button !== 0 || (kind === "move" && (e.target as HTMLElement).closest("button"))) {
      return;
    }
    e.preventDefault();
    const start = { x: e.clientX, y: e.clientY, rect: { ...rect } };
    const move = (ev: PointerEvent): void => {
      const dx = ev.clientX - start.x;
      const dy = ev.clientY - start.y;
      if (kind === "move") {
        setEqExpandedRect({
          ...rect,
          x: Math.max(0, Math.min(window.innerWidth - 160, start.rect.x + dx)),
          y: Math.max(0, Math.min(window.innerHeight - 48, start.rect.y + dy)),
        });
      } else {
        setEqExpandedRect({
          ...rect,
          w: Math.max(EQ_EXPANDED_MIN_W, Math.min(window.innerWidth - start.rect.x, start.rect.w + dx)),
          h: Math.max(EQ_EXPANDED_MIN_H, Math.min(window.innerHeight - start.rect.y, start.rect.h + dy)),
        });
      }
    };
    const up = (): void => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }
</script>

{#if open && slot}
  <div
    bind:this={windowEl}
    class="eq-expanded"
    role="dialog"
    aria-modal="false"
    aria-labelledby="eq-expanded-title"
    tabindex="-1"
    data-testid="eq-expanded-view"
    style:left="{rect.x}px"
    style:top="{rect.y}px"
    style:width="{rect.w}px"
    style:height="{rect.h}px"
    onkeydown={handleKeydown}
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <header class="titlebar" title={t("eq.expanded.move")} onpointerdown={(e) => drag(e, "move")}>
      <h2 id="eq-expanded-title">{t("eq.expanded.title", { name: slot.name })}</h2>
      <span class="spacer"></span>
      <IconButton icon="close" size="sm" label={t("eq.expanded.close")} testid="eq-expanded-close" onclick={close} />
    </header>
    <div class="content">
      <EqGraph {slotIndex} rackSlot={slot} {rateHz} mode="expanded" {graphHeightPx} />
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="resize" title={t("eq.expanded.resize")} onpointerdown={(e) => drag(e, "resize")}></div>
  </div>
{/if}

<style>
  .eq-expanded {
    position: fixed;
    z-index: 900;
    display: flex;
    flex-direction: column;
    min-width: 0;
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-lg);
    background: var(--pv-bg-panel);
    box-shadow: var(--pv-shadow-3);
    color: var(--pv-text-secondary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-sm);
    overflow: hidden;
  }

  .eq-expanded:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
  }

  .titlebar {
    display: flex;
    align-items: center;
    gap: var(--pv-space-3);
    flex: none;
    height: var(--pv-panel-header-h);
    padding: 0 var(--pv-space-2) 0 var(--pv-space-3);
    border-bottom: var(--pv-border-width) solid var(--pv-border-subtle);
    background: var(--pv-bg-elevated, var(--pv-bg-panel));
    cursor: move;
    user-select: none;
  }

  h2 {
    margin: 0;
    color: var(--pv-text-primary);
    font-size: var(--pv-text-md);
    font-weight: var(--pv-weight-semibold);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .spacer {
    flex: 1;
  }

  .content {
    flex: 1;
    min-height: 0;
    padding: var(--pv-space-3);
    overflow: auto;
  }

  .resize {
    position: absolute;
    right: 0;
    bottom: 0;
    width: 14px;
    height: 14px;
    cursor: nwse-resize;
    background: linear-gradient(135deg, transparent 50%, var(--pv-border) 50%);
  }
</style>
