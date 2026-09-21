# H-102 — The Explain modal works, but it doesn't look like the reference yet

- **Tier:** Sonnet, UI/UX quality bar. **Verify with screenshots of the window's contents — and read them.**
- **From:** the orchestrator's own visual check of H-92 (the agent's environment couldn't screenshot). The feature is correct end to end: the average job runs, the summary reads well, the findings and markers are right. The *layout* is not there yet. Compare `annotated_voice_spectrum.png` (the owner's target) with `scratchpad/h92/dark-explain-modal.png` (what we ship today).

## What is wrong, concretely
1. **The graph is the smallest thing in the modal.** It gets roughly a fifth of the height while the
   summary block takes the top half. In the reference the annotated spectrum *is* the page. The
   graph should dominate; the summary supports it.
2. **Annotation text is truncated mid-sentence** — "The loudest peak in the spectrum is th…",
   "Median 130 Hz (C3 −16¢) over the voi…". H-94 wrote careful sentences and the UI is cutting them
   off, which is worse than not showing them. Either give the cards room, or show a short form on
   the graph with the full text in the list beneath — but never an ellipsis in the middle of a
   measurement.
3. **Cards sit on top of the curve and run past the plot's edges**, including one clipped by the
   modal's bottom. H-93's solver takes a `rect` and `avoid` regions — it can only respect bounds it
   is actually given, so check what is being passed.
4. **The modal doesn't use its own space**: wide empty margins beside the summary, a cramped plot
   below. Rebalance so the graph gets the room.
5. Harmonic markers (H1, H2) are drawn over the curve's own peaks and collide with the F0 label.

## Scope (in)
Make it look like the reference, at desktop width first, then check tablet and phone. The content
and the words are already right — this is layout, proportion and legibility only. Do not change
what is measured or how it is worded.

## Verification (the whole point)
Screenshots of the modal's **contents** in Dark, Light and High Contrast, desktop and phone, into
the session scratchpad `h102/`, placed side by side with the reference in your report, with your own
honest read of each. `grim` works from the orchestrator's session: `hyprctl clients -j` for the
geometry, then `grim -g "<x>,<y> <w>x<h>"`. If it hangs for you, check `pgrep -x grim` for a stale
process and kill only that; if it still hangs, say so rather than skipping the check.

`just check` must pass.
