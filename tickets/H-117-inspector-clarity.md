# H-117 — The Spectrum Inspector: say what every curve is, and put Explain My Voice where the analysis is (owner-reported)

- **Tier:** Sonnet, UI/UX quality bar
- **Owner's words:** "The UI is a bit confusing. What is this grey dashed graph on the Spectrum Inspector? The Explain My Voice button is outside, but it just showed up when I opened the Spectrum analysis page?"
- **Owner's screenshot:** `/tmp/claude-1000/-home-lucas-Documents-dev-audition/e6923653-879f-4025-bf4c-c0fb63498c26/images/7.png`

## What the code says
1. **The dashed gray curve is the room tone** — the spectrum of the quiet stretches between phrases, drawn by the Average analysis (`SpectrumInspector.svelte` ~line 183: `{ key: "noise", tone: "noise", dashed: true }`). It is useful: the gap between it and the voice is the signal-to-noise across frequency. **`AnalyzerPanel.svelte` labels it with a legend chip ("Room tone"), but the Spectrum Inspector has no legend at all**, so in the larger, more detailed view the same curve is unexplained. Snapshots A and B are drawn there unlabelled too.
2. **Explain My Voice lives only in the analyzer dock** (H-92 put it in `AnalyzerPanel`), not in the Inspector — the page the owner reasonably reads as "the analysis". Worse, the Inspector in the screenshot already holds an 11.3 s Average result, which is exactly what Explain My Voice needs; clicking the dock button starts *another* average job, which is part of why the owner waited so long (see H-108).

- **Read first:** CLAUDE.md, MEMORY.md (H-42's Inspector, H-92's button and its average job, H-108's analysis wait), `ui/src/lib/analyzer/SpectrumInspector.svelte`, `ui/src/lib/analyzer/SpectrumPlot.svelte`, `AnalyzerPanel.svelte`'s `legend` snippet (the chip style to reuse), `ui/src/lib/analyzer/explain/explainModal.svelte.ts` (`openExplainVoice`).

## Scope (in)
1. **A legend in the Inspector** for every curve it draws — voice (Live or Average), room tone, Snapshot A, Snapshot B — reusing the dock's chip style so the two views read the same. Each entry says in a few words what it is; the room tone's should make its purpose clear (what is quiet between phrases, compare it with the voice to judge noise).
2. **Explain My Voice inside the Inspector**, and when an Average result for the current section already exists there, open the analysis **from that result immediately** instead of running a new job. Only start a job when there is nothing to reuse.
3. Keep the dock button too, but make the two behave identically.
4. Check the rest of the Inspector for other unlabelled marks: the vertical cursor line, the horizontal reference line, the peak labels.

## Coordination
H-108 is changing `AnalyzerPanel.svelte`'s Explain wait and adding progress/Cancel. Keep your changes in the Inspector and call the shared `openExplainVoice` path, so you do not collide.

## Verification
Screenshots of the Inspector with the legend, in Average and Live, Dark and Light, on **WebGL2** (`&renderer=webgl2`), into the scratchpad `h117/`. Time how long Explain takes to open from an existing Average and report it.

`just check` must pass.
