<script lang="ts">
  import { tick, untrack } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";
  import Icon from "./Icon.svelte";
  import Kbd from "./Kbd.svelte";
  import { toAriaKeyShortcuts } from "./kbd";
  import Menu from "./Menu.svelte";
  import {
    isNavigable,
    isTypeaheadKey,
    moveMenuFocus,
    typeaheadIndex,
    TYPEAHEAD_RESET_MS,
    type MenuActionEntry,
    type MenuCloseReason,
    type MenuEntry,
    type MenuSubmenuEntry,
  } from "./menuModel";
  import type { Placement } from "./placement";
  import Popover from "./Popover.svelte";
  import type { PopoverAnchor } from "./types";

  /**
   * The one menu (H-26, WAI-ARIA APG menu): every dropdown in the app renders through this — the
   * menu bar's five menus, Normalize ▾, Add module, the rack slot and preset menus, the Record
   * context menu. Data-driven (`MenuEntry[]`, `menuModel.ts`): items, checkbox and radio items,
   * submenus, separators, group headings, notes and inline custom content (a name field).
   *
   * Keyboard: ↑/↓ (wrapping, skipping disabled items), Home/End, typeahead, Enter/Space, → opens a
   * submenu (or moves to the next menu-bar menu via `onnavigate`), ← closes a submenu (or moves
   * to the previous menu-bar menu), Escape closes one level and returns focus to its trigger, Tab
   * closes everything. Delete runs an item's trailing action. Hover highlights (and focuses) a
   * row and opens its submenu. Positioning, outside clicks and elevation come from `Popover`.
   */
  let {
    open,
    anchor,
    items,
    label,
    testid,
    placement = "bottom-start",
    minWidth,
    initialFocus = "first",
    submenu = false,
    onclose,
    onnavigate,
    ...rest
  }: {
    open: boolean;
    anchor: PopoverAnchor | null | undefined;
    items: MenuEntry[];
    /** Accessible name of the menu. */
    label: string;
    testid?: string;
    placement?: Placement;
    minWidth?: number | "anchor";
    /** Where focus lands when it opens: an item, the menu itself (pointer opens) or nowhere. */
    initialFocus?: "first" | "last" | "menu" | "none";
    /** Internal: this menu is a submenu of another. */
    submenu?: boolean;
    onclose: (reason: MenuCloseReason) => void;
    /** Menu-bar menus: ←/→ with no submenu to open or close move to the adjacent menu. */
    onnavigate?: (direction: 1 | -1) => void;
  } & Omit<HTMLAttributes<HTMLDivElement>, "role" | "onkeydown" | "onclose" | "children" | "class" | "style"> = $props();

  let popupEl: HTMLDivElement | undefined = $state();
  let itemEls: (HTMLButtonElement | undefined)[] = $state([]);
  let openSubId = $state<string | null>(null);
  let subFocus = $state<"first" | "none">("first");
  let typed = "";
  let typedAt = 0;
  let restoreTo: HTMLElement | null = null;

  const nav = $derived(items.filter(isNavigable));
  const navIndex = $derived(new Map(nav.map((entry, index) => [entry.id, index])));
  const disabledFlags = $derived(nav.map((entry) => entry.disabled ?? false));
  const hasMarks = $derived(
    items.some(
      (e) => e.kind === "checkbox" || e.kind === "radio" || (e.kind === "item" && e.icon !== undefined),
    ),
  );

  function currentIndex(): number {
    const active = document.activeElement;
    for (let i = 0; i < nav.length; i++) {
      if (itemEls[i] !== undefined && itemEls[i] === active) {
        return i;
      }
    }
    return -1;
  }

  function focusIndex(index: number | null): void {
    if (index !== null) {
      itemEls[index]?.focus({ preventScroll: false });
    }
  }

  // Focus on open: the first/last enabled item (keyboard opens) or the menu itself (pointer).
  $effect(() => {
    const el = popupEl;
    const mode = initialFocus;
    if (!open || !el) {
      return;
    }
    untrack(() => {
      if (!submenu) {
        restoreTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      }
      if (mode === "menu") {
        el.focus();
      } else if (mode === "first" || mode === "last") {
        focusIndex(moveMenuFocus(-1, mode === "first" ? "ArrowDown" : "ArrowUp", disabledFlags));
      }
    });
  });

  // A closed menu forgets its open submenu.
  $effect(() => {
    if (!open) {
      untrack(() => (openSubId = null));
    }
  });

  function closeWith(reason: MenuCloseReason): void {
    openSubId = null;
    if (!submenu && reason !== "outside") {
      const target = anchor instanceof HTMLElement ? anchor : restoreTo;
      if (target?.isConnected) {
        target.focus();
      }
    }
    onclose(reason);
  }

  function focusIntoOpenSubmenu(entry: MenuSubmenuEntry): void {
    const sub = popupEl?.querySelector<HTMLElement>(`[data-submenu-for="${CSS.escape(entry.id)}"]`);
    sub?.querySelector<HTMLElement>('[role^="menuitem"]:not(:disabled)')?.focus();
  }

  function openSub(entry: MenuSubmenuEntry, focus: "first" | "none"): void {
    if (entry.disabled) {
      return;
    }
    if (openSubId === entry.id) {
      if (focus === "first") {
        focusIntoOpenSubmenu(entry);
      }
      return;
    }
    entry.onopen?.();
    subFocus = focus;
    openSubId = entry.id;
  }

  function onSubClose(entry: MenuSubmenuEntry, reason: MenuCloseReason): void {
    if (reason === "escape") {
      openSubId = null;
      const index = navIndex.get(entry.id);
      void tick().then(() => focusIndex(index ?? null));
      return;
    }
    if (reason === "outside") {
      openSubId = null;
      return;
    }
    closeWith(reason);
  }

  function activate(entry: MenuActionEntry): void {
    if (entry.disabled) {
      return;
    }
    if (entry.kind === "submenu") {
      openSub(entry, "first");
      return;
    }
    if (entry.keepOpen) {
      entry.onselect();
      return;
    }
    closeWith("select");
    entry.onselect();
  }

  function onItemPointerEnter(entry: MenuActionEntry, index: number, event: PointerEvent): void {
    if (event.pointerType === "touch" || entry.disabled) {
      return;
    }
    itemEls[index]?.focus({ preventScroll: true });
    if (entry.kind === "submenu") {
      openSub(entry, "none");
    } else if (openSubId !== null) {
      openSubId = null;
    }
  }

  function onKeydown(event: KeyboardEvent): void {
    const target = event.target instanceof HTMLElement ? event.target : null;
    // Typing in inline content (a preset name) never drives the menu — except Escape.
    if (target && target !== popupEl && target.closest("[data-menu-custom]") && event.key !== "Escape") {
      return;
    }
    // A key a nested submenu didn't consume: only Tab concerns this level (it closes everything).
    if (target && target.closest('[role="menu"]') !== popupEl && event.key !== "Tab") {
      return;
    }
    const index = currentIndex();
    const entry = index >= 0 ? nav[index] : undefined;
    const consume = (): void => {
      event.preventDefault();
      event.stopPropagation();
    };
    switch (event.key) {
      case "ArrowDown":
      case "ArrowUp":
      case "Home":
      case "End":
        consume();
        focusIndex(moveMenuFocus(index, event.key, disabledFlags));
        return;
      case "ArrowRight":
        if (entry?.kind === "submenu" && !entry.disabled) {
          consume();
          openSub(entry, "first");
        } else if (!submenu && onnavigate) {
          consume();
          onnavigate(1);
        }
        return;
      case "ArrowLeft":
        if (submenu) {
          consume();
          closeWith("escape");
        } else if (onnavigate) {
          consume();
          onnavigate(-1);
        }
        return;
      case "Enter":
      case " ":
        if (entry) {
          consume();
          activate(entry);
        }
        return;
      case "Escape":
        consume();
        closeWith("escape");
        return;
      case "Tab":
        if (!submenu) {
          closeWith("tab");
        }
        return;
      case "Delete":
        if (entry?.kind === "item" && entry.trailing) {
          consume();
          entry.trailing.onselect();
        }
        return;
      default:
        break;
    }
    if (isTypeaheadKey(event)) {
      consume();
      const now = Date.now();
      typed = now - typedAt > TYPEAHEAD_RESET_MS ? event.key : typed + event.key;
      typedAt = now;
      focusIndex(
        typeaheadIndex(
          nav.map((e) => e.label),
          disabledFlags,
          typed,
          index,
        ),
      );
    }
  }

  function roleOf(entry: MenuActionEntry): "menuitem" | "menuitemcheckbox" | "menuitemradio" {
    return entry.kind === "checkbox" ? "menuitemcheckbox" : entry.kind === "radio" ? "menuitemradio" : "menuitem";
  }

  function shortcutOf(entry: MenuActionEntry): string | undefined {
    return entry.kind === "submenu" ? undefined : (entry.shortcut ?? undefined);
  }
</script>

{#snippet actionButton(entry: MenuActionEntry, index: number)}
  {@const shortcut = shortcutOf(entry)}
  <button
    {...entry.kind === "item" ? entry.attrs : undefined}
    bind:this={itemEls[index]}
    type="button"
    class="item"
    class:muted={entry.kind === "item" && entry.muted}
    role={roleOf(entry)}
    tabindex="-1"
    aria-checked={entry.kind === "checkbox" || entry.kind === "radio" ? entry.checked : undefined}
    aria-haspopup={entry.kind === "submenu" ? "menu" : undefined}
    aria-expanded={entry.kind === "submenu" ? openSubId === entry.id : undefined}
    aria-keyshortcuts={shortcut ? toAriaKeyShortcuts(shortcut) : undefined}
    data-testid={entry.testid}
    title={entry.kind === "submenu" ? undefined : entry.title}
    disabled={entry.disabled ?? false}
    onclick={() => activate(entry)}
    onpointerenter={(event) => onItemPointerEnter(entry, index, event)}
  >
    {#if hasMarks}
      <span class="mark" aria-hidden="true">
        {#if entry.kind === "checkbox" && entry.checked}
          <Icon name="check" size="sm" />
        {:else if entry.kind === "radio" && entry.checked}
          <span class="dot"></span>
        {:else if entry.kind === "item" && entry.icon}
          <Icon name={entry.icon} size="sm" />
        {/if}
      </span>
    {/if}
    <span class="label">{entry.label}</span>
    {#if shortcut}
      <span class="shortcut"><Kbd keys={shortcut} size="xs" /></span>
    {/if}
    {#if entry.kind === "submenu"}
      <span class="chevron" aria-hidden="true"><Icon name="chevronRight" size="sm" /></span>
    {/if}
  </button>
{/snippet}

<Popover
  {...rest}
  {open}
  {anchor}
  {placement}
  variant="menu"
  role="menu"
  {label}
  {testid}
  {minWidth}
  gapPx={submenu ? 2 : 4}
  alignOffsetPx={submenu ? -5 : 0}
  closeOnEscape={false}
  bind:element={popupEl}
  onclose={(reason) => closeWith(reason)}
  onkeydown={onKeydown}
>
  {#each items as entry (entry.id)}
    {#if entry.kind === "separator"}
      <div class="separator" role="separator"></div>
    {:else if entry.kind === "heading"}
      <div class="heading" role="presentation">{entry.label}</div>
    {:else if entry.kind === "note"}
      <div class="note" role="presentation">{entry.label}</div>
    {:else if entry.kind === "custom"}
      <div class="custom" role="none" data-menu-custom>{@render entry.content()}</div>
    {:else}
      {@const index = navIndex.get(entry.id) ?? -1}
      {#if entry.kind === "item" && entry.trailing}
        {@const trailing = entry.trailing}
        <div class="row" role="none">
          {@render actionButton(entry, index)}
          <button
            type="button"
            class="trailing"
            tabindex="-1"
            aria-label={trailing.label}
            title={trailing.label}
            data-testid={trailing.testid}
            onclick={(event) => {
              event.stopPropagation();
              trailing.onselect();
            }}
          >
            <Icon name={trailing.icon} size="sm" />
          </button>
        </div>
      {:else}
        {@render actionButton(entry, index)}
      {/if}
      {#if entry.kind === "submenu" && openSubId === entry.id}
        <Menu
          submenu
          open
          anchor={itemEls[index]}
          items={entry.items}
          label={entry.label}
          testid={entry.menuTestid}
          placement="right-start"
          minWidth={entry.minWidth}
          initialFocus={subFocus}
          onclose={(reason) => onSubClose(entry, reason)}
          data-submenu-for={entry.id}
        />
      {/if}
    {/if}
  {/each}
</Popover>

<style>
  /* Menu rows (H-25 look, now in one place): 24 px, neutral highlight on hover/focus (never a full
     accent fill), right-aligned Kbd chips, a check/dot column only when the menu has one. */
  .item {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-2);
    width: 100%;
    min-height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: none;
    border-radius: var(--pv-radius-sm);
    background: none;
    color: var(--pv-text-primary);
    font-family: var(--pv-font-sans);
    font-size: var(--pv-text-md);
    line-height: var(--pv-leading-md);
    text-align: left;
    cursor: default;
  }

  .item:focus,
  .item[aria-expanded="true"] {
    background: var(--pv-control-bg-active);
    outline: none;
  }

  .item:focus-visible {
    outline: none;
  }

  .item:disabled {
    color: var(--pv-text-disabled);
  }

  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .item.muted .label {
    color: var(--pv-text-tertiary);
  }

  .shortcut {
    display: inline-flex;
    flex: none;
    margin-left: var(--pv-space-6);
  }

  .item:disabled .shortcut {
    opacity: 0.5;
  }

  .mark {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--pv-icon-sm);
    color: var(--pv-accent-text);
  }

  .item:disabled .mark {
    color: var(--pv-text-disabled);
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--pv-radius-full);
    background: currentColor;
  }

  .chevron {
    display: inline-flex;
    flex: none;
    color: var(--pv-text-tertiary);
  }

  .separator {
    flex: none;
    height: var(--pv-border-width);
    margin: var(--pv-space-1);
    background: var(--pv-border-subtle);
  }

  .heading {
    flex: none;
    padding: var(--pv-space-2) var(--pv-space-2) var(--pv-space-1);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-xs);
    line-height: var(--pv-leading-xs);
    font-weight: var(--pv-weight-semibold);
  }

  .heading:first-child {
    padding-top: var(--pv-space-1);
  }

  .note {
    flex: none;
    padding: var(--pv-space-1) var(--pv-space-2);
    color: var(--pv-text-tertiary);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .custom {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--pv-space-2);
    padding: var(--pv-space-1) var(--pv-space-2) var(--pv-space-2);
    max-width: 18rem;
  }

  /* Inline content (preset name field, replace confirmation) gets the system form look here, so
     feature menus carry no chrome CSS: `<p>` text, a text field, `.actions` buttons, a checkbox
     `<label>`. */
  .custom :global(p) {
    margin: 0;
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
    line-height: var(--pv-leading-sm);
  }

  .custom :global(input[type="text"]) {
    height: var(--pv-control-h-sm);
    padding: 0 var(--pv-space-2);
    border: var(--pv-border-width) solid var(--pv-border-control);
    border-radius: var(--pv-radius-sm);
    background: var(--pv-field-bg);
    color: var(--pv-text-primary);
    font-family: inherit;
    font-size: var(--pv-text-sm);
  }

  .custom :global(input[type="text"]:focus-visible) {
    border-color: var(--pv-accent);
    outline: var(--pv-focus-width) solid var(--pv-focus-ring);
    outline-offset: 0;
  }

  .custom :global(label) {
    display: flex;
    align-items: center;
    gap: var(--pv-space-2);
    color: var(--pv-text-secondary);
    font-size: var(--pv-text-sm);
  }

  .custom :global(input[type="checkbox"]) {
    margin: 0;
    accent-color: var(--pv-accent);
  }

  .custom :global(.actions) {
    display: flex;
    justify-content: flex-end;
    gap: var(--pv-space-1);
  }

  .row {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--pv-space-half);
  }

  .row .item {
    flex: 1;
    min-width: 0;
  }

  .trailing {
    display: inline-flex;
    flex: none;
    align-items: center;
    justify-content: center;
    width: var(--pv-control-h-sm);
    height: var(--pv-control-h-sm);
    padding: 0;
    border: none;
    border-radius: var(--pv-radius-sm);
    background: none;
    color: var(--pv-text-tertiary);
    cursor: default;
  }

  .trailing:hover {
    background: var(--pv-control-bg-hover);
    color: var(--pv-danger-text);
  }
</style>
