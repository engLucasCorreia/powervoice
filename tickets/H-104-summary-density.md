# H-104 — The engineering summary hides most of itself

- **Tier:** Haiku (small, contained)
- **From:** the orchestrator's screenshot review of H-102 (`scratchpad/h102/dark-desktop.png`).

H-102 rightly gave the graph the room it needed, and capped the summary into a scrolling strip so it
could. The side effect: on a desktop modal the voice profile shows **2 of its 6 rows** — Pitch and a
half-clipped Body — with Presence, Sibilance, Rumble and Hum below the fold. The mask-fade makes the
cut honest, but a user should not have to scroll a six-line summary inside a 840px-tall dialog.

## Scope (in)
1. Make the whole profile fit without scrolling at desktop height — a two-column grid of three rows
   is the obvious shape, and the modal has the width for it (there is empty space beside "Suggested
   focus" today).
2. Keep H-102's proportions: the graph stays dominant. This is about using the summary strip's own
   space better, not taking height back from the plot.
3. Keep the fade for the genuinely long cases (many focus items), so a cut still reads as "more
   below".

## Also decide (H-102's open question)
At phone width the solver correctly places **zero** cards on the chart, because H-94's full
sentences cannot fit a 130px card — everything goes to the list beneath. That is defensible, but it
leaves the phone chart decorative. Either accept it explicitly (and say so in the report) or give
phone-width cards a shorter form: the finding's title plus its measured value only, with the full
text still in the list.

## Verification
Screenshots at desktop and phone, Dark and Light, into the session scratchpad `h104/`, using the
recipe in the H-102 ticket. Look at them before reporting.

`just check` must pass.
