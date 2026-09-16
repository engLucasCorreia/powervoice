# H-62 — Surface sandbox event-ring drops

- **Tier:** Sonnet (no review loop)
- **From:** H-55's open question. `HostEnd::push_event` and the proxy drop events when the ring is full; the count lands in `events_dropped` and nobody ever sees it. Under heavy automation that is a silent, timing-dependent divergence between a plugin's state and what the host asked for.
- **Read first:** CLAUDE.md, MEMORY.md (H-55's reset barrier and its "chunk boundaries are timing-dependent" finding; H-30's notice pattern; H-67's notice actions; T-902's slot status; T-702 i18n lint), specs/SPEC-014, specs/SPEC-022 (notice conventions), `crates/sandbox-ipc/src/plugin.rs`, `crates/plugin-host/`, the rack slot status path and `ui/src/lib/rack/`.

## Scope (in)
1. Propagate `events_dropped` out of the IPC layer to the host, sampled off the audio thread (no allocation or locking where events are pushed).
2. Surface it: a slot status that says the plugin missed automation, and a notice the first time it happens per slot per session, worded for a user ("some parameter changes were dropped…"), not for a developer. Do not spam it.
3. Clear the status when it stops (define and document the window you use).

## Tests
- A test that fills the ring and asserts the count reaches the host and the status/notice appears once.
- The RT-safety tests still pass (`no_alloc`).

## Out
Enlarging the ring or changing the drop policy — measure first; if you think the ring is simply too small, report the number, don't change it.

`just check` must pass.
