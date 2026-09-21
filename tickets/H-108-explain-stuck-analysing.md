# H-108 — "Analyzing…" never finishes, and burns a core while it waits (owner-reported, live repro)

- **Tier:** Sonnet. **Reproduced on the owner's machine on the released v0.4.0 build.**
- **Owner's words:** "I hit Explain My Voice, the Analysing started and it's turning — maybe a percentage or a progress info would be good."

## What was observed (orchestrator, on the live instance, before anyone touched it)
- Document: an **unsaved recording** ("Untitled*", a 14.5 s take), analyzer in **Live** mode.
- The Explain button showed **"Analyzing…"** with a spinner, indefinitely. The modal never opened.
- **`WebKitWebProcess`'s main thread was in state `R` at ~91 % of a core**, with ~185 s of accumulated CPU; every other WebView thread slept. The backend main thread ran at ~39 %. So the *UI's JavaScript* was spinning, not the analysis working.
- The log has **no line at all** for the spectrum-analyze job: H-96 added start/finish logging to export/normalize/bake but not to spectrum-analyze, so there is no way to tell from the log whether the job started, failed or finished.
- This is the **same signature as the owner's earlier export freeze** (a job UI stuck while the WebView spins) — H-98/H-103 could never reproduce that one. This may be the reproduction.

## Where to look
`AnalyzerPanel.svelte`'s completion effect for `pendingExplain` (~line 238) can wait forever in two ways without clearing `pendingExplain`:
1. the job reports `done`, but `diag.averageReport?.job_id !== j.jobId` never becomes true — the report never arrives, or arrives with a different id (the preview mock always uses id 11, so it would never catch a real mismatch);
2. the report arrives, but `diag.averages` is empty for `diag.averageSource`, so `result` is undefined and nothing happens.
Neither alone explains a hot loop — find what spins. An effect re-triggering itself (the H-83 class), the H-96 `job_status` poll interacting with this store, or the spinner's own rendering are all candidates; profile, don't guess.
Also check: does the average job work at all on an **unsaved recorded document** with **no rack**, and what is `averageSource` then?

- **Read first:** CLAUDE.md, MEMORY.md (H-83, H-92, H-96, H-98, H-103), `ui/src/lib/analyzer/AnalyzerPanel.svelte`, `ui/src/lib/analyzer/diagnostics.svelte.ts`, `src-tauri/src/spectrum.rs`, `scripts/repro/gui_harness.py`.

## Scope (in)
1. **Reproduce and find the cause** — ideally on the real app with a JS profile (a `just dev` build has the web inspector). State the root cause.
2. **Fix it** so the analysis always ends in one of three states the user can see: done (modal opens), failed (a clear message), or cancelled.
3. **The owner's request — progress:** show real progress while analysing ("Analyzing… 42 %", the job already emits `fraction`), and a Cancel. A spinner with no number gives the user no way to tell slow from stuck — which is exactly how this bug hid.
4. **Log** spectrum-analyze start/finish like the other job kinds.
5. A timeout backstop: if no progress arrives for a documented interval, stop pretending and say so.

## Tests
The specific failure found, as a regression test; progress display; cancel; the no-progress timeout; the unsaved-recording case.

`just check` must pass.
