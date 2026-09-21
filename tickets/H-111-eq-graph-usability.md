# H-111 — The EQ works well; its graph needs to be bigger, readable and directly editable (owner request)

- **Tier:** Sonnet, UI/UX quality bar
- **Owner's words:** "The effect rack is very cool. The EQ works very well, just the UI is not so good. I can't drag the window more to the left to make it bigger and easier to see, so it stays always small. The points of different colours are cool, but I need a hover function to know which frequency my mouse is at, and each point should display, when I hover over it, frequency, Q and gain — right now they are blind points, and I only see the values in the collapsible options below. I want to delete and add points when I right-click somewhere on the EQ line."

## What the code says today
- **The rack column is capped** at `min(480px, 35 % of the main area)` — `SIDE_COLUMN_MAX_PX` and `sideColumnMaxPx` in `ui/src/App.svelte` (~lines 144–151). On the owner's screen that is why the splitter stops moving.
- An **expanded EQ view already exists** (H-84: the graph's "Expand" button opens a large floating window). The owner did not find it, which is itself a finding: it is not discoverable enough.
- `EqGraph.svelte`'s own header (~line 80) lists **"the right-click context menu, and hover tooltips"** as still out of scope since S3-07 — both deferred twice (H-84, H-86). This ticket is where they get built.

- **Read first:** CLAUDE.md, MEMORY.md (H-84, H-86, H-26's shared `Menu`, H-66's right-click menu pattern on the waveform), specs/SPEC-015 §2.6 (§2.6.4 gestures and whatever it says about right-click and hover), `ui/src/lib/eq/EqGraph.svelte`, `ui/src/lib/eq/EqExpandedView.svelte`, `ui/src/lib/eq/nodes.ts`, `ui/src/lib/eq/keyboardNav.ts`, `ui/src/App.svelte`'s column limits, and the Spectrum Inspector's / analyzer's hover readout (the house style to match).

## Scope (in)
1. **Let the rack grow.** Raise the column cap so the rack can take a real share of the window — decide the new limit so the waveform keeps a usable minimum rather than disappearing, and say what you chose. Keep it persisted as today.
2. **Make the expanded view discoverable** — it already solves "too small"; a user who cannot find it has not been helped. Consider a double-click on the graph background, or a more obvious control.
3. **Cursor readout:** hovering anywhere on the graph shows the frequency under the pointer and the curve's gain there, in the style the analyzer already uses.
4. **Node tooltips:** hovering a node shows its band name, **frequency, gain and Q** (or slope for the HP/LP bands), updating live while dragging. The values already exist as `aria-valuetext` — make them visible.
5. **Right-click to add and delete bands:** right-click on the curve or empty graph → "Add band here" enables the nearest free band at that frequency; right-click on a node → "Delete band" disables it (plus its other actions: reset, bypass). The EQ has a fixed set of bands, so "add" means enabling a free one — when none is free, say so in the menu rather than failing silently. Keyboard-reachable, like H-66's menu.
6. Everything above works in both the compact graph and the expanded view.

## Coordination
H-110 is fixing the rack slot's drag hijack (`draggable` on the whole card), which currently breaks
dragging these nodes at all. Build on its merge; don't duplicate it.

## Verification
Screenshots of the node tooltip, the cursor readout and the context menu, compact and expanded, Dark
and Light, into the scratchpad `h111/`. Look at them.

`just check` must pass.
