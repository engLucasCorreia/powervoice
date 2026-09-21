# H-106 — Make the tours actually teach the app (owner request)

- **Tier:** Sonnet, UI/UX quality bar
- **Owner's words:** "add the instructions to the tour, improve that of course, to teach what the things does, how to use the app".
- **The state today:** six tours exist (`ui/src/lib/tour/tours.ts`): welcome 10 steps, rack 4, noise 3, loudness 4, punch 3, plugins 4.

  **Correction (orchestrator, after the fact):** this ticket originally claimed noise and punch had
  one step each and that the tours "point rather than teach". That was wrong — my counting script
  only matched steps written across multiple lines and missed the single-line form, and the bodies
  already met the teaching bar from T-709/H-39. The agent checked instead of padding tours that were
  fine, which is the right instinct. **The real gap was that Explain My Voice had no tour at all.**
- **Read first:** CLAUDE.md, MEMORY.md (T-709's tour implementation), `ui/src/lib/tour/`, `ui/src/lib/i18n/en.json`'s `tour.*` keys, `docs/user-guide.md` (the prose already exists there — reuse its substance rather than inventing a second explanation), and H-94's `explain/prose.ts` for the register: plain, concrete, no exclamation marks, no selling.

## Scope (in)
1. **Fill out the thin tours.** Noise reduction and punch-in each need a real walkthrough: what the feature is for, the steps in order, and what the user should expect to see or hear at each one.
2. **Teach, don't point.** Each step says what the thing does and when you would use it — "capture a second of room tone so the reducer knows what silence sounds like", not "this is the noise panel".
3. **A tour for Explain My Voice** — the feature is new and unfamiliar, and it is the one most likely to be misread. Cover what the graph shows, that it is an average over a section, and what the annotations mean.
4. Check every tour still targets elements that exist, after a lot of UI change (the analyzer panel, the markers panel and the rack have all moved on).
5. Make the tours discoverable: Help → Tours exists, but a first-run user should be told the tours are there.

## Verification
Walk every tour end to end in the preview harness and screenshot each one's first and last step into the scratchpad `h106/` (recipe: `tickets/H-102-explain-modal-layout.md`; the app's preview supports `?preview&scene=tour&step=n&tour=<id>`). A tour step pointing at a missing element is the failure mode to hunt.

`just check` must pass.
