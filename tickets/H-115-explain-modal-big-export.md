# H-115 — Explain My Voice: much bigger, annotations you can hide, an honest EQ toggle, and an export (owner request)

- **Tier:** Sonnet, UI/UX quality bar
- **Owner's words:** "The voice spectrum analysis opened after a long time. The window is small. I did not find something to toggle the comments in the picture — I need a toggle so I can see the whole smoothed graph. The EQ advice I toggle on and off but I don't find where it is. The screen is very small — it's OK to make it very big — and a button to export that screen, to download the analysis and this whole window, so I can send it to my audio engineer."
- **Owner's screenshot:** `/tmp/claude-1000/-home-lucas-Documents-dev-audition/e6923653-879f-4025-bf4c-c0fb63498c26/images/6.png` (a 3756×2121 display — the modal occupies a small fraction of it).

## Findings behind each point
1. **Too small.** The dialog is sized `min(840px, 90vh)` tall on the shared `xl` width (H-102). On a large, high-resolution display that is a small box in the middle of the screen. Pixel caps suit a laptop and fail a 4K monitor.
2. **The annotation cards cover the curve**, and there is no toggle for them — the five toggles are Raw FFT, Smoothed, Harmonics, Voice Bands, EQ Advice. The owner wants to see the whole spectrum unobstructed.
3. **The EQ Advice toggle appears to do nothing** — and on this take that is *correct*: the only finding (low-mid body) crossed its threshold by 0.2 dB, and H-94 deliberately suggests no EQ within 1 dB of a threshold, so there is no dashed curve to draw. The bug is that the UI does not **say** so. A toggle that silently changes nothing reads as broken.
4. **"Opened after a long time"** is H-108 (the analysis wait); make sure this ticket does not regress it, and show H-108's progress here too if it lands first.

- **Read first:** CLAUDE.md, MEMORY.md (H-92, H-93, H-94, H-101, H-102, H-104, and the renderer lesson: **the real app renders with WebGL2, the preview harness defaults to Canvas2D**), `ui/src/lib/analyzer/explain/`, the shared `Dialog` kit.

## Scope (in)
1. **Big by default, and bigger on demand.** Size the modal relative to the screen (a large share of the viewport, not a pixel cap), with a **maximise / full-screen** control. Keep it usable on a laptop and on phone width.
2. **An "Annotations" toggle** that hides the cards and leader lines, leaving the curves, markers and bands. Remember the choice.
3. **EQ Advice must never be a silent no-op.** When there is nothing to draw, say so beside the toggle ("No EQ change suggested for this take"), with the reason available.
4. **Export**, so the owner can send the analysis to their engineer: at least an **image of the full analysis** (graph, annotations and summary) and a **self-contained report** (PDF or HTML) with the graph, every finding's measured value and interpretation, the take's duration and the date. Offer a native save dialog. The export must render the same content the user sees, at a resolution that stays legible when printed.
5. Keep H-102's proportions — the graph dominant — at every size.

## Verification
Screenshots at laptop and large-display sizes, normal and maximised, annotations on and off, **on the WebGL2 renderer** (`&renderer=webgl2`), plus the exported image and report opened and looked at, into the scratchpad `h115/`.

`just check` must pass.
