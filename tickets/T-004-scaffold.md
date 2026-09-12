# T-004 — Tauri 2 + Svelte 5 scaffold, theme, i18n, shared types, mocks

- **Milestone / wave:** M0 / W2
- **Tier:** Sonnet
- **Depends on:** T-001, T-002 (ADR-003 decides the shared-types tool)
- **PROMPT refs:** §2, §3.6, §4 · **ADR refs:** ADR-001, ADR-003

## Goal
`just dev` opens an empty Audition-like dark window; UI tooling (svelte-check, vitest) runs inside `just check`; one typed command round-trips Rust↔TS.

## Scope
**In:**
- `src-tauri/` as workspace member, package name `powervoice-app`, Tauri **2.11.x**. Commands: `app_info() -> AppInfo { name, version }`.
- `ui/`: Svelte 5 + TypeScript (strict) + Vite 8, Vitest 5, svelte-check 4.7. Tauri CLI as an npm devDependency (`@tauri-apps/cli`), no global install. Configure `tauri.conf.json` (`frontendDist: ../ui/dist`, `devUrl`, identifier `app.powervoice.editor`, window title "PowerVoice", min size 1024×640).
- Layout shell with empty placeholder panels: top toolbar + transport, center editor, right rack panel, left/bottom markers/properties, bottom meter bridge. Resizable splitters not required yet.
- Dark theme as CSS custom properties (tokens: surfaces, text, accent, waveform, selection, meter green/yellow/red, focus ring) in `ui/src/lib/theme/`.
- i18n: a small typed helper (no heavy library): `ui/src/lib/i18n/en.json` + `t(key, params?)` with compile-time key checking; every visible string uses it.
- Shared types per ADR-003 (ts-rs or tauri-specta): generated to `ui/src/lib/ipc/bindings.ts`; a `just gen-types` recipe; `just check` fails if generated types are stale.
- Vitest configured with `@tauri-apps/api/mocks` (`mockIPC`); one component test that mocks `app_info` and asserts the version renders.
- Update `justfile`: `dev` → `npm --prefix ui run tauri dev`; `build` includes the Tauri bundle build (Linux only for now); `check` UI steps now active; `check-cross` excludes `powervoice-app`.
- Document in `ui/README.md` the Linux env workaround hook: if `POWERVOICE_WEBKIT_SAFE=1`, set `WEBKIT_DISABLE_DMABUF_RENDERER=1` before launching (implement in `justfile` `dev`).

**Out:** real features, audio, the spike page (T-007).

## Acceptance tests
- [ ] vitest: layout renders all five regions; `app_info` mock displays version.
- [ ] `just check` green (Rust + UI).
- [ ] `just dev` launches (agent: verify the process starts and the Vite server responds; a screenshot is not required).

## Definition of Done
- [ ] Above green; report includes the exact `npm` dependency versions installed.
