# H-96 — Job UIs hang at "running" when the job finishes too fast (owner-reported)

- **Tier:** Sonnet. **Real bug, reproduced on the owner's machine.**
- **Owner report:** "the app is hanging at exporting". Their export had in fact **completed successfully** — the file was on disk, complete, minutes earlier — while the UI still showed the export in progress. Both the WebView (~72 % of a core) and the backend main thread kept burning CPU in that state.

## The race
`ui/src/lib/export/export.svelte.ts::startExport` does:

```
const started = await exportStart(request);   // the backend starts the job immediately
job = { jobId: started.job_id, fraction: 0, state: "running" };
await ensureListening();                      // ...and only now is `job_progress` subscribed
```

The backend job can run to completion — emitting its progress events **and the terminal `Done`** —
before that `listen()` call is attached. Every event emitted in the gap is lost, so the store never
leaves `running` and the panel says "exporting" forever. A short file makes it near-certain.

**The same pattern is in at least three more places**, so this is a class, not a one-off:
`ui/src/lib/state/normalize.svelte.ts` (~line 137), `ui/src/lib/state/normalizeLufs.svelte.ts`
(~line 111), `ui/src/lib/state/bake.svelte.ts` (~line 122). `ui/src/lib/state/edit.svelte.ts:210`
subscribes differently — check whether it is already correct and say so.

- **Read first:** CLAUDE.md, MEMORY.md (H-50 — job services emit their result **before** the terminal `Done`, which is the backend half of this same ordering concern; H-30's job events; H-70's save job), specs/ADR-003 (job events), the four modules above.

## Scope (in)
1. **Subscribe before starting.** Attach the `job_progress` listener before issuing the start
   command, in every job path, so no event can be missed. Keep the unsubscribe lifecycle correct
   (no leaked listeners when a job fails to start or a second job begins).
2. **Belt and braces:** even with correct ordering, a UI that has missed a terminal event must be
   able to recover — decide and implement how (e.g. reconcile against the backend's job state when
   the panel is shown, or a terminal-state query on a timeout). Say what you chose and why.
3. **The CPU burn** while stuck: find out what spins when a job is wedged in `running` and stop
   that too, or explain why it is unavoidable. A hung panel must not cost 70 % of a core.
4. Check whether the backend's own logging can tell an operator that an export happened — the
   owner's log had **no export line at all**, which made a completed export look like a lost one.
   Add a concise start/finish log line if that's the case.

## Tests
- A job whose terminal event fires **before** the listener would have attached, for each of the
  four paths: the UI still reaches its terminal state. This is the regression test; it must fail
  against today's code.
- The recovery mechanism from item 2.
- No leaked listeners across repeated jobs.

`just check` must pass.
