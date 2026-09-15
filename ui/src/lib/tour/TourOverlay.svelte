<script lang="ts">
  import { tick, untrack } from "svelte";
  import { t } from "../i18n";
  import { isEditableTarget } from "../keymap";
  import { Button, Icon } from "../ui";
  import type { AnchorRect, Size } from "../ui/placement";
  import { blockingModal, findTourTarget, visibleRect } from "./targets";
  import { nextStep, prevStep, skipTour, tourState } from "./tour.svelte";
  import { CARD_FALLBACK_SIZE, placeTourCard, spotlightRect, type PlacedCard } from "./tourPlacement";

  /**
   * The guided tour (T-709): a dimmed backdrop with a rounded, accent-ringed cut-out around the
   * step's target and an anchored step card (title, body, optional illustration, "Step n of N",
   * Back/Next/Skip/Done). The spotlight and card follow the target through window resizes,
   * scrolling and panel splitters (re-measured every frame while a tour runs, plus on resize and
   * scroll). Pointer input outside the cut-out is blocked; an `interactive` step lets the pointer
   * reach its target (click Record). While a modal dialog the step doesn't point into is open, the
   * tour steps aside and comes back when it closes.
   *
   * Keyboard: → / Enter next, ← back, Esc skips. Focus moves to the card on every step and each
   * step is announced (`aria-live`). Motion follows the tokens (0 ms under reduced motion), and a
   * target is scrolled into view without smooth scrolling then.
   */
  const ts = tourState();

  let card: HTMLDivElement | undefined = $state();
  let spot = $state<AnchorRect | null>(null);
  let placed = $state<PlacedCard | null>(null);
  let paused = $state(false);
  let reducedMotion = $state(false);
  let targetEl: HTMLElement | null = null;

  const step = $derived(ts.step);
  const params = $derived(step?.params?.() ?? {});
  const title = $derived(step ? t(step.titleKey, params) : "");
  const body = $derived(step ? t(step.bodyKey, params) : "");
  const tourName = $derived(ts.tour ? t(ts.tour.nameKey) : "");
  const stepLabel = $derived(t("tour.step_of", { n: ts.index + 1, count: ts.count }));
  const announcement = $derived(
    step ? t("tour.announce", { tour: tourName, n: ts.index + 1, count: ts.count, title }) : "",
  );
  /** An action-gated step whose action hasn't happened yet: Next turns secondary (skip it). */
  const waiting = $derived(step?.waitFor ? !step.waitFor.done() : false);

  function viewportSize(): Size {
    const root = document.documentElement;
    return { width: root.clientWidth || window.innerWidth, height: root.clientHeight || window.innerHeight };
  }

  function cardSize(): Size {
    if (!card || card.offsetWidth === 0) {
      return CARD_FALLBACK_SIZE;
    }
    // The natural height even while a previous `maxHeight` limits the box.
    const scroller = card.querySelector<HTMLElement>(".content");
    const overflow = scroller ? scroller.scrollHeight - scroller.clientHeight : 0;
    return { width: card.offsetWidth, height: card.offsetHeight + Math.max(0, overflow) };
  }

  function sameRect(a: AnchorRect | null, b: AnchorRect | null): boolean {
    return a === b || (!!a && !!b && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height);
  }

  function samePlaced(a: PlacedCard, b: PlacedCard | null): boolean {
    return (
      !!b &&
      a.left === b.left &&
      a.top === b.top &&
      a.placement === b.placement &&
      a.maxHeight === b.maxHeight &&
      a.arrowPx === b.arrowPx
    );
  }

  /** Finds the target and places the spotlight and card. Always called untracked. */
  function measure(): void {
    const current = step;
    if (!current) {
      return;
    }
    const viewport = viewportSize();
    const el = findTourTarget(current.target);
    targetEl = el;
    const nextPaused = blockingModal(el, card ?? null);
    const visible = el ? visibleRect(el) : null;
    const nextSpot = visible ? spotlightRect(visible, viewport) : null;
    const nextPlaced = placeTourCard(nextSpot, cardSize(), viewport, current.placement ?? "bottom");
    if (nextPaused !== paused) {
      paused = nextPaused;
    }
    if (!sameRect(nextSpot, spot)) {
      spot = nextSpot;
    }
    if (!samePlaced(nextPlaced, placed)) {
      placed = nextPlaced;
    }
  }

  // prefers-reduced-motion (the tokens already zero the durations; this also stops smooth
  // scrolling and the card's entrance).
  $effect(() => {
    if (typeof window.matchMedia !== "function") {
      return;
    }
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotion = query.matches;
    const onChange = (): void => {
      reducedMotion = query.matches;
    };
    query.addEventListener?.("change", onChange);
    return () => query.removeEventListener?.("change", onChange);
  });

  // Every step: find the target, bring it on screen, place the card (again once the new content
  // has laid out).
  $effect(() => {
    const current = step;
    void ts.index;
    if (!current) {
      return;
    }
    untrack(() => {
      measure();
      targetEl?.scrollIntoView?.({ block: "nearest", inline: "nearest", behavior: reducedMotion ? "auto" : "smooth" });
      void tick().then(() => untrack(measure));
    });
  });

  // Focus moves to the card on every step (and when it comes back after a dialog).
  $effect(() => {
    const el = card;
    void ts.index;
    void ts.tour;
    if (el) {
      untrack(() => el.focus({ preventScroll: true }));
    }
  });

  // Action-gated steps advance once their action happens while the step shows. The value at entry
  // is the baseline, so Back onto an already-satisfied step doesn't bounce forward again.
  let gateKey = "";
  let gateBaseline = false;
  $effect(() => {
    const current = step;
    const tour = ts.tour;
    const index = ts.index;
    const done = current?.waitFor ? current.waitFor.done() : null;
    untrack(() => {
      if (done === null || !tour) {
        gateKey = "";
        return;
      }
      const key = `${tour.id}:${index}`;
      if (key !== gateKey) {
        gateKey = key;
        gateBaseline = done;
      } else if (done && !gateBaseline) {
        gateKey = "";
        nextStep();
      } else if (!done) {
        gateBaseline = false;
      }
    });
  });

  // Tracking: resize and scroll at once, and every frame for splitters/layout changes.
  $effect(() => {
    if (!ts.active) {
      return;
    }
    const onChange = (): void => untrack(measure);
    window.addEventListener("resize", onChange);
    window.addEventListener("scroll", onChange, true);
    const hasFrames = typeof requestAnimationFrame === "function";
    let frame = 0;
    const loop = (): void => {
      untrack(measure);
      frame = requestAnimationFrame(loop);
    };
    if (hasFrames) {
      frame = requestAnimationFrame(loop);
    }
    return () => {
      window.removeEventListener("resize", onChange);
      window.removeEventListener("scroll", onChange, true);
      if (hasFrames) {
        cancelAnimationFrame(frame);
      }
    };
  });

  // Keyboard, in the capture phase so the app's own shortcuts (Space = play) never see the keys
  // the tour uses.
  $effect(() => {
    if (!ts.active) {
      return;
    }
    const swallow = (event: KeyboardEvent): void => {
      event.preventDefault();
      event.stopPropagation();
    };
    const onKeydown = (event: KeyboardEvent): void => {
      if (paused) {
        return;
      }
      const target = event.target;
      const inCard = card !== undefined && target instanceof Node && card.contains(target);
      const onPage = target === null || target === document.body || target === document.documentElement;
      if (event.key === "Escape") {
        swallow(event);
        void skipTour();
        return;
      }
      if (event.altKey || event.ctrlKey || event.metaKey || !(inCard || onPage) || isEditableTarget(target)) {
        return;
      }
      switch (event.key) {
        case "ArrowRight":
          swallow(event);
          nextStep();
          break;
        case "ArrowLeft":
          swallow(event);
          prevStep();
          break;
        case "Enter":
          if (target === card || onPage) {
            swallow(event);
            nextStep();
          } else {
            // A card button activates itself.
            event.stopPropagation();
          }
          break;
        case " ":
          // A card button's own Space — never the transport's play/pause.
          event.stopPropagation();
          break;
        default:
          break;
      }
    };
    window.addEventListener("keydown", onKeydown, true);
    return () => window.removeEventListener("keydown", onKeydown, true);
  });

  /** Pointer shields around the cut-out (and over it unless the step is interactive). */
  const shields = $derived.by((): string[] => {
    if (!spot) {
      return [];
    }
    const right = spot.left + spot.width;
    const bottom = spot.top + spot.height;
    const list = [
      `left: 0; top: 0; right: 0; height: ${spot.top}px`,
      `left: 0; top: ${bottom}px; right: 0; bottom: 0`,
      `left: 0; top: ${spot.top}px; width: ${spot.left}px; height: ${spot.height}px`,
      `left: ${right}px; top: ${spot.top}px; right: 0; height: ${spot.height}px`,
    ];
    if (!step?.interactive) {
      list.push(`left: ${spot.left}px; top: ${spot.top}px; width: ${spot.width}px; height: ${spot.height}px`);
    }
    return list;
  });
</script>

{#if ts.active && step && !paused}
  <div
    class="pv-tour"
    data-testid="tour-overlay"
    data-tour-id={ts.tour?.id}
    data-step={step.id}
    data-reduced-motion={reducedMotion ? "true" : undefined}
  >
    {#if spot}
      <div
        class="spot"
        data-testid="tour-spotlight"
        style:left="{spot.left}px"
        style:top="{spot.top}px"
        style:width="{spot.width}px"
        style:height="{spot.height}px"
      ></div>
      {#each shields as style, i (i)}
        <div class="shield" data-testid="tour-shield" {style}></div>
      {/each}
    {:else}
      <div class="scrim" data-testid="tour-scrim"></div>
    {/if}

    <div
      bind:this={card}
      class="card"
      role="dialog"
      aria-modal="false"
      aria-labelledby="pv-tour-title"
      aria-describedby="pv-tour-body"
      tabindex="-1"
      data-testid="tour-card"
      data-placement={placed?.placement ?? "center"}
      style:left="{placed?.left ?? 0}px"
      style:top="{placed?.top ?? 0}px"
      style:max-height={placed?.maxHeight == null ? undefined : `${placed.maxHeight}px`}
      style:--pv-tour-arrow={placed?.arrowPx == null ? undefined : `${placed.arrowPx}px`}
    >
      {#if placed && placed.arrowPx !== null}
        <span class="arrow" aria-hidden="true"></span>
      {/if}
      <div class="content">
        <div class="meta">
          <span class="tour-name">{tourName}</span>
          <span class="count" data-testid="tour-step-count">{stepLabel}</span>
        </div>
        <div class="progress" aria-hidden="true">
          {#each ts.tour?.steps ?? [] as s, i (s.id)}
            <span class:done={i <= ts.index}></span>
          {/each}
        </div>
        {#if step.illustration}
          <span class="art" aria-hidden="true" data-testid="tour-illustration">
            <Icon name={step.illustration} size="lg" />
          </span>
        {/if}
        <h2 id="pv-tour-title" class="title" data-testid="tour-title">{title}</h2>
        <p id="pv-tour-body" class="body" data-testid="tour-body">{body}</p>
        {#if step.waitFor}
          <p class="wait" data-testid="tour-wait" data-met={waiting ? undefined : "true"}>
            <Icon name={waiting ? "info" : "success"} size="sm" />
            <span>{t(step.waitFor.hintKey, params)}</span>
          </p>
        {/if}
      </div>
      <div class="footer">
        {#if !ts.isLast}
          <Button variant="ghost" testid="tour-skip" onclick={() => void skipTour()}>{t("tour.skip")}</Button>
        {/if}
        <span class="spacer"></span>
        {#if ts.index > 0}
          <Button icon="chevronLeft" testid="tour-back" onclick={prevStep}>{t("tour.back")}</Button>
        {/if}
        <Button
          variant={waiting ? "secondary" : "primary"}
          iconEnd={ts.isLast ? undefined : "chevronRight"}
          testid="tour-next"
          onclick={nextStep}
        >
          {ts.isLast ? t("tour.done") : t("tour.next")}
        </Button>
      </div>
    </div>
  </div>
{/if}
<div class="sr-only" aria-live="polite" data-testid="tour-live">{ts.active ? announcement : ""}</div>

<style>
  .pv-tour {
    position: fixed;
    inset: 0;
    z-index: var(--pv-z-tour);
    pointer-events: none;
    font-family: var(--pv-font-sans);
  }

  .scrim,
  .shield {
    position: fixed;
    pointer-events: auto;
  }

  .scrim {
    inset: 0;
    background: var(--pv-bg-backdrop);
    animation: pv-tour-fade var(--pv-duration-slow) var(--pv-ease-standard);
  }

  /* The cut-out: a rounded box whose giant spread shadow is the backdrop, ringed in the accent. */
  .spot {
    position: fixed;
    box-sizing: border-box;
    border-radius: var(--pv-radius-lg);
    box-shadow:
      0 0 0 var(--pv-focus-width) var(--pv-accent),
      0 0 0 200vmax var(--pv-bg-backdrop);
    pointer-events: none;
    transition:
      left var(--pv-duration-slow) var(--pv-ease-standard),
      top var(--pv-duration-slow) var(--pv-ease-standard),
      width var(--pv-duration-slow) var(--pv-ease-standard),
      height var(--pv-duration-slow) var(--pv-ease-standard);
  }

  /* Level-3 surface (design-system §5), like a dialog, but anchored. */
  .card {
    position: fixed;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    width: min(22rem, calc(100vw - 2 * var(--pv-space-3)));
    border: var(--pv-border-width) solid var(--pv-border);
    border-radius: var(--pv-radius-lg);
    background: var(--pv-bg-overlay);
    box-shadow: var(--pv-shadow-3);
    color: var(--pv-text-primary);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    text-align: left;
    pointer-events: auto;
    outline: none;
    transition:
      left var(--pv-duration-slow) var(--pv-ease-standard),
      top var(--pv-duration-slow) var(--pv-ease-standard);
    animation: pv-tour-rise var(--pv-duration-slow) var(--pv-ease-standard);
  }

  .card:focus-visible {
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: var(--pv-focus-offset);
  }

  /* The pointer: a rotated square half-tucked under the edge that faces the target. */
  .arrow {
    position: absolute;
    width: var(--pv-space-3);
    height: var(--pv-space-3);
    box-sizing: border-box;
    border: var(--pv-border-width) solid var(--pv-border);
    background: var(--pv-bg-overlay);
    transform: rotate(45deg);
  }

  .card[data-placement="bottom"] .arrow {
    top: calc(var(--pv-space-3) / -2);
    left: calc(var(--pv-tour-arrow) - var(--pv-space-3) / 2);
    border-right: none;
    border-bottom: none;
  }

  .card[data-placement="top"] .arrow {
    bottom: calc(var(--pv-space-3) / -2);
    left: calc(var(--pv-tour-arrow) - var(--pv-space-3) / 2);
    border-left: none;
    border-top: none;
  }

  .card[data-placement="right"] .arrow {
    left: calc(var(--pv-space-3) / -2);
    top: calc(var(--pv-tour-arrow) - var(--pv-space-3) / 2);
    border-right: none;
    border-top: none;
  }

  .card[data-placement="left"] .arrow {
    right: calc(var(--pv-space-3) / -2);
    top: calc(var(--pv-tour-arrow) - var(--pv-space-3) / 2);
    border-left: none;
    border-bottom: none;
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: var(--pv-space-2);
    min-height: 0;
    padding: var(--pv-space-4) var(--pv-space-4) var(--pv-space-2);
    overflow-y: auto;
  }

  .meta {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--pv-space-3);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    color: var(--pv-text-tertiary);
  }

  .tour-name {
    min-width: 0;
    overflow: hidden;
    font-weight: var(--pv-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .count {
    flex: none;
    font-variant-numeric: tabular-nums;
  }

  .progress {
    display: flex;
    gap: var(--pv-space-half);
    margin-bottom: var(--pv-space-1);
  }

  .progress span {
    flex: 1;
    height: 3px;
    border-radius: var(--pv-radius-full);
    background: var(--pv-control-track);
    transition: background-color var(--pv-duration-base) var(--pv-ease-standard);
  }

  .progress span.done {
    background: var(--pv-accent);
  }

  .art {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    align-self: flex-start;
    width: calc(var(--pv-space-8) + var(--pv-space-2));
    height: calc(var(--pv-space-8) + var(--pv-space-2));
    border-radius: var(--pv-radius-md);
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
  }

  .title {
    margin: 0;
    font-size: var(--pv-text-lg);
    line-height: var(--pv-leading-lg);
    font-weight: var(--pv-weight-semibold);
  }

  .body {
    margin: 0;
    color: var(--pv-text-secondary);
  }

  .wait {
    display: flex;
    align-items: flex-start;
    gap: var(--pv-space-2);
    margin: var(--pv-space-1) 0 0;
    padding: var(--pv-space-2) var(--pv-space-3);
    border-radius: var(--pv-radius-md);
    background: var(--pv-accent-soft);
    color: var(--pv-accent-text);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .wait :global(svg) {
    flex: none;
    margin-top: 1px;
  }

  .footer {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-2);
    padding: var(--pv-space-2) var(--pv-space-4) var(--pv-space-4);
  }

  .footer .spacer {
    flex: 1;
  }

  /* The ghost Skip lines its text up with the card's content edge. */
  .footer :global([data-testid="tour-skip"]) {
    margin-left: calc(-1 * var(--pv-space-2));
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }

  .pv-tour[data-reduced-motion] .spot,
  .pv-tour[data-reduced-motion] .card,
  .pv-tour[data-reduced-motion] .scrim,
  .pv-tour[data-reduced-motion] .progress span {
    transition: none;
    animation: none;
  }

  @keyframes pv-tour-fade {
    from {
      opacity: 0;
    }
  }

  @keyframes pv-tour-rise {
    from {
      opacity: 0;
      transform: translateY(4px);
    }
  }
</style>
