# H-110 — Dragging any control inside a rack slot drags the whole slot instead (owner-reported)

- **Tier:** Sonnet. **Affects every module, not just the EQ.**
- **Owner's words:** "When I select an effect in the Rack, for example Parametric EQ, and try to drag a point or a value, the widget comes with my mouse first, and only after I release does the item I selected move — then I need to click again. The widget keeps moving with my mouse when I want to set something or drag a setting. Not only EQ — check this problem."

## Cause (confirmed by reading the code)
`ui/src/lib/rack/RackSlot.svelte` (~line 456) puts **`draggable="true"` on the whole slot
`<section>`**, not on the grip. HTML5 drag-and-drop therefore starts a native drag of the entire
card from a press-and-move **anywhere inside it** — on an EQ node, a parameter slider, the
Dynamics/Noise Gate transfer-graph handles, the noise profile graph. The browser shows the card's
drag ghost following the pointer and the inner control never receives its own drag until release,
which is exactly the reported behaviour. The grip icon (`.grip`, ~line 467) exists but is purely
decorative (`aria-hidden`).

- **Read first:** CLAUDE.md, MEMORY.md (the rack reorder work, H-63/H-77's transfer graph handles, H-84/H-86's EQ graph gestures, H-85's noise profile graph), `ui/src/lib/rack/RackSlot.svelte`, `ui/src/lib/rack/RackPanel.svelte` (the reorder handlers), and every draggable control rendered inside a slot.

## Scope (in)
1. **Only the grip starts a reorder drag.** Pressing and dragging on any control inside a slot must
   drive that control, immediately, on the first press — no ghost, no second click.
2. Audit **every** interactive element rendered inside a slot: EQ nodes (mouse and wheel), sliders,
   number fields (text selection by drag must work too), the transfer-graph threshold handles, the
   noise profile graph's hover, the preset menu. List them in your report with before/after.
3. Keep reordering fully usable: drag by the grip, and a **keyboard** way to move a slot up or down
   if one does not exist — dragging must not be the only way (it is the least accessible one).
4. Make the grip look like the handle it now is — cursor, hover state — so users can find it.

## Tests
A drag on an EQ node, a slider and a transfer-graph handle changes the value and does **not** start
a slot drag; a drag on the grip still reorders; the keyboard reorder path.

## Verification
The failure is visual and gestural, so screenshots alone won't prove it. Say in your report exactly
how you verified the fix drives the control on the first press.

`just check` must pass.
