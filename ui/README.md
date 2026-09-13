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

## Linux WebKitGTK DMA-BUF default

WebKitGTK's GPU (DMA-BUF) compositing path measured ~1.7x worse frame times on this project's test
hardware (ADR-009 §3: AMD Phoenix/Mesa, not just NVIDIA as first assumed). So, **on Linux,
`WEBKIT_DISABLE_DMABUF_RENDERER=1` is the default**, applied two ways (ADR-009 Amendment 1 / MEMORY
D-016):

- `powervoice-app` itself sets it at startup, before the WebView is created (`src-tauri/src/webkit.rs`),
  unless it's already set.
- `just dev` and `just spike` set it too, so the Tauri CLI's own dev-server process sees it from the
  start.

**Opt out** (keep the default WebKit DMA-BUF renderer) with:

```sh
POWERVOICE_WEBKIT_DMABUF=1 just dev
```

Setting `WEBKIT_DISABLE_DMABUF_RENDERER` yourself (to `0` or otherwise) also takes precedence —
PowerVoice never overrides a value you already set.
