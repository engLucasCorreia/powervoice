# PowerVoice UI

Svelte 5 + TypeScript (strict) frontend, built with Vite. Talks to the Rust core only through the
typed wrappers in `src/lib/ipc/commands.ts`, over generated types in `src/lib/ipc/bindings.ts`
(ADR-003; regenerate with `just gen-types` from the repo root, never hand-edit that file).

## Running

From the repo root:

- `just dev` — starts the Vite dev server and the Tauri app pointed at it.
- `just check` — `svelte-check` + `vitest run`, plus the Rust checks (also fails if
  `bindings.ts` is stale — run `just gen-types`).
- `just build` — production build (`vite build`) + the Tauri bundle.

Directly in `ui/`, the usual `npm run dev` / `npm run build` / `npm run check` / `npm run test`
scripts work the same way `just` invokes them.

## Linux WebKitGTK workaround

WebKitGTK's GPU (DMA-BUF) compositing path can be slow or broken on some Linux setups (notably
some NVIDIA configurations). If `just dev` renders a blank/garbled window or a laggy UI, set:

```sh
POWERVOICE_WEBKIT_SAFE=1 just dev
```

`just dev` then sets `WEBKIT_DISABLE_DMABUF_RENDERER=1` before launching, which forces a software
compositing path. This is off by default because it costs some GPU performance.
