# PowerVoice design system (H-25)

Status: **phase 1 + phase 2 applied** (tokens, kit, gallery; every screen on the system — see
§17 for what changed per screen and what's deferred), **H-26 follow-ups applied** (§19: one menu
and popover, platform dialog button order, true minus everywhere, axis labels that never collide).
Decisions: A-017 (Lucide icons via `@lucide/svelte`; system font stack). Audit:
`docs/design/ui-audit.md`.

- Tokens: `ui/src/lib/theme/design-tokens.css` (`--pv-*`, imported once from `main.ts`).
- Kit: `ui/src/lib/ui/` (import from `ui/src/lib/ui/index.ts`).
- Gallery: `npm --prefix ui run dev`, then open <http://localhost:1420/?gallery> (dev builds only).
- Preview: <http://localhost:1420/?preview> (`&theme=light`) mounts the real App against mocked
  IPC (`ui/src/dev/previewIpc.ts`, dev builds only) — for layout/screenshot checks without Tauri.
- Contrast gate: `ui/src/lib/theme/contrast.test.ts`; light parity: `themeParity.test.ts`.
- Legacy bridge: `ui/src/lib/theme/theme-bridge.css` points `tokens.css` chrome names at the roles.

## 1. Direction

**Studio at night.** PowerVoice is where a narrator spends hours with one voice. The chrome is a
quiet graphite console with a slight cool bias; the take — waveform, spectrogram, meters,
analyzer — is the only thing that glows. There is one accent (signal blue) for selection, focus
and the one primary action per group. Red means one thing: **on air** — the record lamp, a
broadcast-style tally line across the top of the transport while recording, and the recording
time. Clip red, warning amber and pass green appear only when they carry meaning.

What makes it PowerVoice rather than a generic dark app:
- **The tally.** Recording gets a single, unmistakable moment: the Record key fills red, a 2 px red
  tally line runs along the transport bar, the elapsed time turns red, and a slow-breathing lamp says
  "Recording". Nothing else in the app is red at rest.
- **The time is the chrome's hero.** The playhead time is the one large element (20 px, tabular),
  everything else is 11–13 px.
- **Numbers are instruments.** Every value is tabular, uses a true minus sign (−23.0), and carries
  its unit in a quieter tone ("−23.0 LUFS", "−1.2 dBTP", "48 kHz", "12 ms").
- **Wells for audio.** Waveform, spectrogram, meters and analyzer sit in the darkest surface
  (`--pv-bg-inset`), recessed below the panels, so the content reads first.

Things we deliberately don't do: uppercase tracked labels, gradients, glows, cards inside cards,
decorative motion, colour without a word or icon next to it.

## 2. Colour roles

Components use **roles only**, never raw hex. Dark is the default on `:root`; `data-theme="light"`
or `data-theme="high-contrast"` on `<html>` (or any subtree) switches every role. High Contrast
(T-708) values are in `design-tokens.css`; see §15.

### Surfaces (darkest → lightest)
| Role | Dark | Light | Use |
|---|---|---|---|
| `--pv-bg-inset` | `#0c0e11` | `#e4e7eb` | Wells: waveform, spectrogram, meter tracks, text fields |
| `--pv-bg-app` | `#111317` | `#eceef1` | Window background, gaps between panels |
| `--pv-bg-panel` | `#17191d` | `#f7f8fa` | Docked panels, transport bar, menu bar |
| `--pv-bg-raised` | `#1d2025` | `#ffffff` | Cards in panels (rack slots), table headers |
| `--pv-bg-overlay` | `#22252b` | `#ffffff` | Menus, popovers, dialogs, tooltips |
| `--pv-bg-backdrop` | 60 % near-black | 35 % ink | Modal scrim |

### Controls and borders
| Role | Dark | Light | Use |
|---|---|---|---|
| `--pv-control-bg` / `-hover` / `-active` | `#23262c` / `#2b2f36` / `#33373f` | `#fff` / `#eef0f3` / `#e2e5ea` | Secondary buttons |
| `--pv-control-bg-selected` | `#373c46` | `#ffffff` | Selected segment pill |
| `--pv-control-track` | `#0c0e11` | `#e4e7eb` | Segmented/switch/slider tracks |
| `--pv-field-bg` | `#0c0e11` | `#ffffff` | Text/number entry fields (recessed in dark, white in light) |
| `--pv-border-subtle` | `#22252b` | `#e6e8ec` | Dividers inside one surface |
| `--pv-border` | `#2f333a` | `#d3d7de` | Panel edges, cards, menus |
| `--pv-border-strong` | `#434852` | `#aab1bb` | Hover emphasis |
| `--pv-border-control` | `#6c727d` | `#858c97` | Text fields/selects (≥ 3:1, WCAG 1.4.11) |

### Text
| Role | Dark | Light | Use |
|---|---|---|---|
| `--pv-text-primary` | `#e6e8ec` | `#1a1d22` | Content, labels on controls |
| `--pv-text-secondary` | `#a4aab4` | `#525964` | Field labels, panel titles, unselected segments |
| `--pv-text-tertiary` | `#8a909b` | `#5f6671` | Units, meta, axis labels, hints (still AA everywhere) |
| `--pv-text-disabled` | `#5d626b` | `#a3a9b2` | Disabled controls **only** — never hints |
| `--pv-text-on-accent` / `-on-record` / `-on-danger` | white | white | Text on filled buttons |
| `--pv-text-on-warning` / `-on-success` | near-black | near-black | Text on amber/green pills |

### Accent (one colour)
| Role | Dark | Light | Use |
|---|---|---|---|
| `--pv-accent` | `#4da3ff` | `#1f6fe0` | Indicators: selected tab underline, slider fill, switch on, selection border |
| `--pv-accent-text` | `#6cb2ff` | `#1a64cc` | Accent text/icons (pressed toggles, links) |
| `--pv-accent-fill` (`-hover`, `-active`) | `#2a68c8` | `#1a64cc` | Primary button background (white text ≥ 4.5:1) |
| `--pv-accent-soft` | `#1b2a3d` | `#e3eefc` | Pressed toggle / selected row background |
| `--pv-focus-ring` | `#6cb2ff` | `#1a64cc` | Focus outline |

### Semantic (meaning only)
| Role family | Meaning | Where |
|---|---|---|
| `--pv-record*` | **On air** | Record key (lamp at rest, filled while recording), tally line, recording time, "Recording" status. Not used for errors. |
| `--pv-danger-*` | Destructive / error | Delete confirmations, invalid fields, failed checks |
| `--pv-warning*` | Needs attention | Dropouts, fallback device, monitoring latency ≥ 20 ms, low disk |
| `--pv-success*` | Passed / healthy | ACX pass, device connected |
| `--pv-meter-safe/caution/over`, `--pv-clip`, `--pv-playhead`, `--pv-waveform`, `--pv-selection-*` | Audio content | Same values as today's `tokens.css` in dark (the meters the owner likes); light gets its own. |

Each semantic family has `-text` (text on surfaces), `-fill` (solid background), `-soft` (tinted
badge background). Rule: **colour never carries meaning alone** — pair it with a word, an icon or a
position (Badge text, StatusDot label, ACX "Pass"/"Fail").

## 3. Typography

System font stack (A-017): `system-ui, -apple-system, "Segoe UI", Cantarell, "Noto Sans", Ubuntu,
Roboto, "Helvetica Neue", Arial, sans-serif`. Mono (`--pv-font-mono`) only for code-like text (log
paths, token names in the gallery) — never for numbers; numbers use `tabular-nums`.

| Token | Size / line | Weight | Use |
|---|---|---|---|
| `--pv-text-xs` | 11 / 16 px | 400–500 | Units, axis labels, badges, kbd, meta |
| `--pv-text-sm` | 12 / 16 px | 400–600 | Field labels, panel titles (600), sm controls, tooltips |
| `--pv-text-md` | 13 / 18 px | 400–500 | Default UI text and md/lg controls (500) |
| `--pv-text-lg` | 15 / 20 px | 600 | Dialog titles |
| `--pv-text-xl` | 20 / 28 px | 400 | Transport time, empty-state title (600) |

Rules: sentence case everywhere (menus follow platform title case only where the OS does); no
letter-spacing except +0.01em on the xl time; one weight step per level of hierarchy; body copy
≤ 72 characters per line in dialogs.

## 4. Spacing and grid

4 px base, 8 px rhythm: `--pv-space-1` 4 · `-2` 8 · `-3` 12 · `-4` 16 · `-5` 20 · `-6` 24 · `-8` 32
(`--pv-space-half` 2 px for optical nudges only).

| Where | Value |
|---|---|
| Gap between controls in a group | 4 px |
| Gap between groups (with a Separator) | 8 px |
| Panel body padding | 12 px |
| Panel header padding | 12 px left, 4 px right (icon buttons sit flush) |
| Dialog padding | 20 px (header/body/footer), 12 px between fields |
| Gutters between docked panels | 1 px border (no gaps; panels tile) |

## 5. Radii, borders, elevation

| Token | Value | Use |
|---|---|---|
| `--pv-radius-sm` | 4 px | ≤ 24 px controls, badges, kbd, tooltips |
| `--pv-radius-md` | 6 px | 28–32 px controls, fields, cards, menus |
| `--pv-radius-lg` | 8 px | Dialogs, popovers, gallery cards |
| `--pv-radius-full` | pill | Switches, dots |

Elevation: **level 0** panels (1 px `--pv-border` edge, no shadow) · **level 1** raised cards
(`--pv-bg-raised`, `--pv-shadow-1` optional) · **level 2** menus/popovers/tooltips
(`--pv-bg-overlay`, border + `--pv-shadow-2`) · **level 3** dialogs (`--pv-bg-overlay`,
`--pv-shadow-3`, backdrop). Layers: dropdown 100 · sticky 200 · overlay 900 · dialog 1000 · toast
1100 · tooltip 1200 (`--pv-z-*`).

## 6. Motion

Motion answers an action; nothing moves on its own except the recording lamp.

| Token | Value | Use |
|---|---|---|
| `--pv-duration-fast` | 120 ms | Hover/press colour changes, tooltip fade |
| `--pv-duration-base` | 160 ms | Disclosure chevrons, switch knob, tally line |
| `--pv-duration-slow` | 200 ms | Dialog/popover enter |
| `--pv-ease-standard` / `-exit` | `cubic-bezier(0.2,0,0,1)` / `(0.4,0,1,1)` | Enter / exit |

`prefers-reduced-motion: reduce` sets all durations to 0 and stops the lamp's breathing. Meters
and the analyzer are data, not motion — they keep animating.

## 7. States

| State | Treatment |
|---|---|
| Hover | Background one step lighter (`-hover`); ghost controls gain a background. Never an accent border (that means "selected"). |
| Pressed (`:active`) | Background one more step (`-active`). |
| Focus (keyboard) | 2 px `--pv-focus-ring` outline, 2 px offset (0 inside tracks/segments). Always visible, never removed. |
| Selected / on | Segments: raised pill. Tabs: 2 px accent underline. Toggles/IconButtons: `--pv-accent-soft` + `--pv-accent-text`. Rows: `--pv-accent-soft`. |
| Disabled | `--pv-text-disabled`, borders drop to subtle, no hover. Disabled controls stay in place (no reflow). |
| Loading | Button keeps its label, adds a spinner, `aria-busy`, stays focusable; long jobs get a progress bar. |
| Invalid | `--pv-danger-text` border + a message naming what's accepted ("Enter a number from −60.0 dB to 0.0 dB"). |
| Recording | See §1: filled Record key, tally line, red time, "Recording" lamp. |

## 8. Iconography

- **Lucide** via `@lucide/svelte` (A-017; H-26 replaced the deprecated `lucide-svelte`), wrapped by `Icon.svelte`. Components ask for a *semantic*
  name from `ui/src/lib/ui/icons.ts` (`returnToStart`, `bypass`, `marker` …) — one concept, one
  icon, app-wide. Add new icons to the registry, never import Lucide directly in features.
- Sizes: 14 px (sm controls), 16 px (default), 20 px (lg transport keys). Stroke 1.75. `filled` only
  for the record dot.
- Icons are decorative (`aria-hidden`) unless they stand alone as status — then `label`.
- **Icon + tooltip** for frequent secondary actions whose icon is conventional (transport, add,
  delete, bypass, more, settings). **Text labels** for primary actions and anything without a
  universal icon (Analyze, Apply, Normalize…). Text + icon for important actions that benefit from
  both (Open file…, New recording…).

## 9. Controls and sizes

| Size | Height | Padding-x | Text | Icon | Use |
|---|---|---|---|---|---|
| sm | 24 px | 8 px | 12 px | 14 px | Panel headers, dense panel toolbars (analyzer, spectral), rack params |
| md | 28 px | 10 px | 13 px | 16 px | Default: dialogs, transport secondary, fields |
| lg | 32 px | 12 px | 13 px | 20 px | Primary transport keys (Play, Record), empty-state actions |

Minimum hit target 24 × 24 px. Toolbar height 44 px; panel header 32 px; menu bar 28 px; status
bar 24 px.

### Which component
| Need | Use |
|---|---|
| Main action of a dialog/panel (max one per group) | `Button variant="primary"` |
| Other actions | `Button` (secondary) · `Button variant="ghost"` in headers/toolbars |
| Destructive confirmation | `Button variant="danger"` (in the confirm dialog, not on the first click) |
| Frequent action with a conventional icon | `IconButton` (label = name + tooltip, `shortcut` from the keymap) |
| On/off view state in a toolbar (Spectral, Loop, A/B) | `ToggleButton` or `IconButton pressed` |
| Setting that applies immediately | `Toggle` (switch) |
| One of 2–4 short options | `SegmentedControl` |
| One of many / long labels | `Select` (labels carry units) |
| Numeric entry | `NumberField` (unit suffix, arrows, clamps) |
| Continuous value with a range | `Slider` (+ `NumberField` next to it when precision matters) |
| Views sharing one area | `Tabs` |
| Panel title row | `PanelHeader` |
| Status word | `Badge` (soft by default; solid only for recording/clip) |
| Status lamp | `StatusDot` (always with a label) |
| Measured value | `Readout` |
| Shortcut hint | `Kbd` (text from `shortcutLabelForAction`) |
| Group divider | `Separator` |
| Nothing here yet | `EmptyState` |

## 10. Panel anatomy

```
┌──────────────────────────────────────────┐
│ Title          meta          [+] [⋯]     │  PanelHeader, 32 px, border-bottom subtle
├──────────────────────────────────────────┤
│                                          │  body: 12 px padding, scrolls on its own
│  content / list / params                 │  sections separated by a subtle divider +
│                                          │  a 12 px semibold secondary sub-title
├──────────────────────────────────────────┤
│ footer (optional): status / count        │  24 px, xs tertiary text
└──────────────────────────────────────────┘
```
Docked panels tile edge to edge on `--pv-bg-panel`; the editor well and the analyzer/meter wells are
`--pv-bg-inset`. Collapsible panels use PanelHeader's disclosure; collapse state persists (H-24 layout
settings).

## 11. Transport bar (toolbar grouping)

```
┌─────────────────────────────────────────────────────────────────────────────────────────────┐▔▔ tally (recording only)
│ Untitled*  │ ⏮ ▶ ■ ⟲ │  0:01:23.456  │ ● 0:00:12.3 ◉ Recording │ 🎙 Input ▾  🎧 Dry ▾ 12 ms │ ≋ Spectral │ Normalize ▾ │  ⚙ │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
  document     transport    time (xl)       record group             input & monitoring        view        favourites   devices
```
- Groups separated by `Separator vertical`, 8 px apart; controls inside a group 4 px apart.
- Transport: IconButtons (Play/Pause and Record are `lg`), tooltips with shortcuts.
- Record group: `IconButton variant="record"` (lamp at rest, filled + Stop glyph while recording),
  elapsed `Readout` (muted → record tone), StatusDot while recording, disk-remaining Badge (amber
  when low, with units: "4 h 12 min left").
- Monitoring: `Select` (Off/Dry/Through rack) + latency Badge (warning ≥ 20 ms, danger ≥ 40 ms).
- Normalize favourites collapse into one split button (last-used favourite + menu).
- App name and version move to About; the document name moves here (and to the window title).
- At narrow widths groups wrap as whole groups, never mid-group; the time never wraps.

## 12. Dialog anatomy

- Width by content: 400 px (confirm), 480 px (forms), 640 px (Preferences/Export); max 90 vw/85 vh,
  body scrolls.
- Header: 15 px semibold title (+ optional close IconButton). Body: fields stacked, 12 px apart,
  labels above (`layout="stacked"`). Footer: buttons 8 px apart, described by **role** through
  `Dialog`'s `actions` prop — never hand-ordered.
- **Button order per platform** (H-26, `ui/dialogActions.ts`, platform from `ui/platform.ts`):
  - Linux and macOS: `[utility][destructive] ··· [alternate][Cancel][Primary]` — primary last,
    Cancel just left of it, a destructive alternative ("Don't save") apart on the far left.
  - Windows: `[utility] ··· [Primary][alternate][destructive][Cancel]` — primary first, Cancel last.
  - Roles: `primary` (the default answer, Enter), `cancel` (Esc), `alternate` (another answer:
    "Keep as float", "Retry"), `destructive` (loses work), `utility` (not an answer: "ACX preset").
    Variants follow the role (primary → primary, destructive/utility → ghost, others secondary).
    Destructive primary uses `variant: "danger"`. Focus goes to the first field (or the dialog), is
    trapped, and returns to the opener on close.
- Phase 2 adds one `Dialog.svelte` shell to the kit and moves all 17 dialogs onto it.

## 13. Numbers and units

- Tabular figures everywhere a number can change (`font-variant-numeric: tabular-nums`).
- True minus sign U+2212 for negatives — in every readout **and every axis label** (analyzer dB,
  amplitude ruler, EQ gain, meters, loudness, offsets); never "−0.0"; silence is "−∞". One
  formatter: `formatNumber` / `formatWithUnit`. Parsers (`parseNumber`, `NumberField`) accept an
  ASCII `-`, U+2212 and pasted dash variants. Editable text fields with their own parser (the
  Normalize dialogs' target) and module-provided parameter text (Rust) keep their own format.
- Value and unit joined by a no-break space ("−23.0 LUFS"); `%` attaches directly ("50%").
- Units: dB (gain/relative), dBFS (level), dBTP (true peak), LUFS/LU (loudness), Hz/kHz (≥ 1 kHz
  shows kHz), ms (< 1 s), s, samples. Show the unit next to every value, or once in a column header.
- Fixed decimals per quantity: levels 1 decimal, LUFS 1, frequency 0 (Hz) / 1–2 (kHz), times per
  zoom (H-24 ruler rules).
- Helpers: `formatNumber`, `formatWithUnit`, `parseNumber` in `ui/src/lib/ui/units.ts`.

## 14. Accessibility checklist

- Text ≥ 4.5:1, indicators and field boundaries ≥ 3:1 in both themes — enforced by
  `contrast.test.ts` (add a pair when you add a role).
- Every control is reachable by Tab, has a visible focus ring, and a name (visible label,
  `aria-label` for icon buttons, `aria-labelledby` for switches).
- Composite widgets follow WAI-ARIA APG: radio group (SegmentedControl), tabs (automatic
  activation), slider, spinbutton (NumberField), switch, tooltip, disclosure (PanelHeader).
- Hit targets ≥ 24 × 24 px.
- No information by colour alone; status always has a word or icon.
- Respect `prefers-reduced-motion`.

## 15. Theming (T-708)

**Themes:** Dark (default, A-018), Light ("clear"), Match system (live `prefers-color-scheme`),
High Contrast (dark-based: every text role ≥ 7:1 on every surface — WCAG AAA — indicators
≥ 4.5:1, a 3 px focus ring, 2 px content lines). `Settings.theme` = `dark | light | system |
high_contrast` (Rust `ThemePref`).

**One token block per theme.** `design-tokens.css` has a `[data-theme="dark|light|high-contrast"]`
block each, holding every colour role — chrome (`--pv-*`) and audio content (`--wave-*`,
`--spec-*`, `--analyzer-*`, `--eq-*`, `--pv-meter-*`) — plus `--pv-focus-width`,
`--pv-stroke-content` (playhead/marker/record-head/sample lines) and `--pv-stroke-emphasis` (EQ
curve). `tokens.css` keeps only base element styles; `theme-bridge.css` aliases the pre-H-25 chrome
names (`--surface-*`, `--text-*`, `--accent*`, `--meter-*`) onto the roles.

**Adding a theme:** a block in `design-tokens.css`, an entry in `RESOLVED_THEMES` and `THEMES`
(`theme/theme.svelte.ts`: labels, caption, menu label), a Rust `ThemePref` variant (`just
gen-types`). Tests fail if the block misses a token, a contrast pair fails, or the lists drift.

**Switching:** Preferences → Appearance (`theme/ThemePicker.svelte` — a radio group of cards, each
with a live miniature rendered under that theme's own `data-theme`; Match system is split
diagonally) and View → Theme ▸ (radio rows on the shared `Menu`). Both call
`theme/chooseTheme.ts` (apply at once + persist). No shortcut: SPEC-003 names none.

**No flash:** `applyThemePref` mirrors the preference into `localStorage["powervoice.theme"]`;
an inline script in `ui/index.html` stamps `data-theme` (and a provisional `color-scheme`) before
the stylesheet and app load; Settings re-applies the authoritative value once loaded
(`theme/boot.test.ts` runs that script against `resolveTheme`).

**Renderers:** canvas/WebGL code reads colours and line widths only through
`theme/themeColors.ts::themeColors()` — typed (`{ css, rgba }` per colour), read from
`getComputedStyle(<html>)` with the token file as fallback, cached and invalidated when the
resolved theme changes (`themeState().revision`). The rAF loops (waveform, spectral, analyzer) pick
the new colours up on their next frame; the EQ graph's `$effect` re-runs. No reload.

**Spectrogram:** its well stays dark in every theme (colormaps start at black), so Inferno stays
the default everywhere; the user's colormap choice (spectral defaults / sidecar) always wins —
themes never touch it. The "frozen" notice uses `--spec-scrim` / `--spec-scrim-text`.

**Meters and analyzer:** same ballistics and anatomy in every theme (the "jump" is motion, not
colour). Light deepens the meter greens/ambers until they hold 3:1 on a light track.

**Guards:** `contrast.test.ts` (every pair in every theme; AAA for High Contrast; content pairs —
waveform, playhead, markers, meters, EQ bands on their wells), `themeParity.test.ts` (content
shapes the renderers parse, stroke widths, UI list ↔ Rust enum ↔ token blocks),
`colorLiterals.test.ts` (no hex/`rgb()`/`hsl()`/named colour in any `.ts`/`.svelte` outside the
token files and the colormap tables).

**Preview:** `?preview&theme=dark|light|system|high_contrast` (with `&scene=`, `&dialog=`,
`&menu=theme` for View → Theme ▸ open).

## 16. Component kit reference

All in `ui/src/lib/ui/`, Svelte 5 runes, tokens only, each with a `*.test.ts` (ARIA, states,
keyboard).

| Component | Key props | Contract |
|---|---|---|
| `Button` | `variant` primary/secondary/ghost/danger · `size` · `icon`/`iconEnd` · `loading` · `disabled` | `<button type=button>`; loading = `aria-busy` + `aria-disabled`, stays focusable, swallows clicks |
| `IconButton` | `icon` · `label` (required) · `shortcut` · `variant` ghost/secondary/primary/record · `pressed` · `active` · `size` | `aria-label`, `aria-keyshortcuts`, optional `aria-pressed`; tooltip shows label + Kbd |
| `Tooltip` | `text` · `shortcut` · `placement` · `describe` · `delayMs` | `role=tooltip`; 500 ms hover, instant on keyboard focus and between neighbours; Esc closes; one open at a time; fixed + flip/clamp |
| `ToggleButton` | `pressed` (bindable) · `icon` · `size` | `aria-pressed` |
| `SegmentedControl` | `options` · `value` (bindable) · `label` · `size` | radiogroup; roving tabindex; arrows select; skips disabled |
| `Select` | `options` (typed values, labels with units) · `value` · `label` · `layout` · `hideLabel` | native `<select>` + `<label for>` |
| `Slider` | `value` · `min`/`max`/`step`/`bigStep` · `unit`/`format` · `defaultValue` · `oninput`/`onchange` | slider role, `aria-valuetext` with unit, arrows/Shift/Page/Home/End, drag, dbl-click reset, bipolar fill |
| `NumberField` | `value` · `min`/`max`/`step` · `unit` · `decimals` · `signed` · `label` | spinbutton on a text input; Enter/blur commit (clamped), Esc revert, arrows step; invalid message |
| `Toggle` | `checked` (bindable) · `label` · `description` | `role=switch`, named by its label, label click toggles |
| `PanelHeader` | `title` · `level` · `meta` · `actions` snippet · `collapsible` · `expanded` · `controls` | heading; disclosure button inside it with `aria-expanded`/`aria-controls` |
| `Tabs` | `tabs` · `selected` (bindable) · `label` · `idPrefix` · `panel` snippet | tablist/tab/tabpanel wiring, automatic activation |
| `Badge` | `tone` · `variant` soft/solid · `icon` | text always present |
| `StatusDot` | `tone` · `label` (required) · `showLabel` · `pulse` | `role=img` + label, or visible label with decorative dot |
| `Readout` | `value`+`decimals` or `text` · `unit` · `label` · `tone` · `size` sm/md/xl | tabular value, quieter unit |
| `Kbd` | `keys` | nested `<kbd>` per key |
| `Separator` | `orientation` · `decorative` | `role=separator` + `aria-orientation` |
| `EmptyState` | `icon` · `title` · `description` · `actions` · `shortcuts` · `size` | section labelled by its heading |
| `Icon` | `name` (registry) · `size` · `label` · `filled` | decorative unless labelled |
| `Menu` (H-26) | `open` · `anchor` (element or point) · `items: MenuEntry[]` · `label` · `placement` · `minWidth` (`"anchor"`) · `initialFocus` · `onclose(reason)` · `onnavigate` | `role=menu`; items `menuitem` / `menuitemcheckbox` / `menuitemradio`, submenus, separators, headings, notes, inline custom content, trailing icon action; ↑↓ Home End, typeahead, Enter/Space, → ← submenus and menu-bar neighbours, Esc returns focus to the trigger, Tab closes; Kbd shortcut chips; hover highlights and opens submenus |
| `Popover` (H-26) | `open` · `anchor` · `placement` · `variant` menu/panel · `role` · `label` · `minWidth` · `onclose(reason)` | fixed, flips/shifts inside the viewport (`placement.ts`), scrolls when it can't fit; outside press closes; Esc closes and refocuses the anchor |
| `Dialog` `actions` (H-26) | `DialogAction[]` (`label`, `role`, `onclick`, `testid`, `icon`, `variant`, `disabled`, `loading`) | footer ordered per platform (`orderDialogActions`) |

## 17. Phase 2 plan (after H-24 merges)

Order is chosen so every step ships a visible improvement and leaves `just check` green. Each step:
replace local CSS with kit components + `--pv-*` roles, keep every `data-testid` (update tests only
where the DOM contract deliberately changes), verify at 1280×720 and 2126×850 in both themes.

1. **Foundation.** Map `tokens.css` legacy names onto the roles (§15) so untouched screens pick up the
   new palette; system font stack in `html, body`; fix the undefined `--error`/`--danger`/fallback
   colours; `WaveformView`'s `#ff5c5c` fallbacks → `--pv-record`. Add `Dialog.svelte` and
   `Menu`/`Popover` primitives to the kit (menu-bar menus already have the right behaviour —
   extract their look).
2. **Transport bar** (Toolbar + RecordControls + NormalizeToolbarButtons): the grouped layout of §11
   — icon transport with tooltips + shortcuts, xl time `Readout`, on-air Record key + tally line +
   red elapsed time, disk remaining with units, Monitoring `Select` + latency Badge, Spectral
   `ToggleButton`, normalize favourites as one split button, app name/version → About, document name
   → start of the bar. Punch & pre-roll popover rebuilt with `SegmentedControl` (Insert/Overwrite),
   `Toggle`s, `NumberField`s with s/ms units, sections.
3. **Menu bar and menus:** 28 px bar, 24 px menu rows, `Kbd`-styled shortcuts, the document name
   leaves the bar; record context menu, rack slot menu, Add module and preset menus move onto the
   shared menu primitive (one hover, one padding, one radius).
4. **Editor chrome:** `EmptyState` (no document: "Open a file or start recording" with Open/New
   recording buttons and shortcuts); styled overlay scrollbar instead of the range input; divider
   with a grip; spectral toolbar on `sm` kit controls; rulers/amplitude scale in tertiary xs text on
   the H-24 tick generators.
5. **Markers panel:** `PanelHeader` (count meta, add/delete IconButtons with shortcuts), 28 px rows,
   selected row `--pv-accent-soft`, sm `EmptyState`; drop the empty "Properties" heading until it has
   content.
6. **Rack panel:** `PanelHeader` (A/B `ToggleButton`, latency meta), slot cards on `--pv-bg-raised`
   with a drag grip, bypass `IconButton` (`bypass` icon, pressed = active), disclosure chevron,
   `more` menu; bypassed = "Bypassed" Badge + dimmed body only (header stays full contrast); params on
   `Slider` + `NumberField` / `Select` / `Toggle`; EQ graph chrome (range `SegmentedControl`, band
   labels) — curve rendering untouched. Coordinate with T-802's slot status (crash/hang Badges use
   danger/warning soft).
7. **Bottom dock:** `Tabs` (Meters | Analyzer | Loudness) if H-24 made it a tab strip, else
   `PanelHeader`s. Analyzer header on `sm` kit controls (response `SegmentedControl`, Floor/Ceiling
   `Select`s with dB, Peak hold `Toggle`) — rendering untouched. Meter bridge: In/Out labels,
   `Readout`s. Loudness: `Readout` grid (Integrated as the lead value), Processed/Source
   `SegmentedControl`, Analyze `Button loading` + progress bar, ACX results with pass/fail `Badge`s
   (icon + word) and a units column.
8. **Dialogs (17):** move onto `Dialog.svelte` — stacked labels, `NumberField`/`Select`/`Toggle`,
   filled primary, platform button order, focus trap/restore. Order: Normalize, Normalize (LUFS),
   progress, Unsaved Changes, Confirm, Save As, Export, New Recording, Low Disk, Calibration, Audio
   Devices, Recovery, Recent Missing, Channel Choice, Clip Prompt, About, Preferences.
9. **Notices:** Toast/Banner with status icons + words, close `IconButton`, `--pv-z-toast`, overlay
   elevation.
10. **Preferences → Appearance → Theme** (Dark/Light/Match system; `Settings` field + `gen-types`),
    light values for the content tokens (`--wave-*`, `--spec-*`, `--analyzer-*`, `--eq-*`), and a
    theme-switch test.
11. **Screenshot pass:** 1280×720 and 2126×850, dark and light, recording and idle; nothing
    overlaps, clips or wraps mid-group.

Tests added in phase 2: per-screen structure tests stay green; new tests for the transport on-air
state, the dialog shell (focus trap, Esc/Enter, button order per platform), theme switching, and the
contrast gate extended to content tokens in light.

### 17.1 Phase 2 status (H-25, 2026-09-14)

| Step | Status |
|---|---|
| 1 Foundation | Done: `theme-bridge.css` (legacy names → roles, system font), undefined `--error`/`--danger` gone with the dialogs, `--wave-record(-head)` tokens (no more `#ff5c5c` fallback), `Dialog.svelte`. *Deferred:* extracting a shared Menu/Popover primitive — menus and popovers are restyled in place to one look. |
| 2 Transport bar | Done as §11, with two changes: the normalize favourites are one **Normalize** menu (peak dBFS / loudness LUFS) rather than a split button, and the document name is centred in the menu bar rather than at the start of the transport bar. The Record key is a text key ("● Record" → solid red "■ Stop") because it's the primary action. |
| 3 Menus | Done: 28 px bar, 24 px rows, icon marks, tokenized popups; record/rack/Add-module popovers match. |
| 4 Editor chrome | Done: actionable empty state, slim scrollbar, hairline divider, spectral toolbar on small controls; rulers keep H-24's tick generators. |
| 5 Markers | Done. |
| 6 Rack | Done, incl. T-802 status chips and Retry. |
| 7 Bottom dock | Done: H-24's tabs restyled; analyzer header + 20 Hz…20k axis with edge-aligned labels and the unit once; meters as In/Out rows; loudness readout grid + ACX badge. Analyzer rendering and meter look unchanged. |
| 8 Dialogs | Done: all 17 on `Dialog.svelte`. *Deferred:* Windows button order (primary first) — Linux/macOS order everywhere today. Native radios/checkboxes/number inputs stay inside dialogs (they're form-style and tests drive them with `change` events); they get the system look from the Dialog body styles. |
| 9 Notices | Done. |
| 10 Theme | Done: `Settings.theme` (Rust, `ThemePref` dark/light/system, default dark) + Preferences → Appearance; light values for every content token (`themeParity.test.ts`). Waveform/spectrogram/EQ canvases pick a new theme up on their next redraw; the analyzer immediately. |
| 11 Screenshot pass | Done with `?preview` at 1167×1326 and 2126×850, dark and light (no document state). |

Known gaps: the analyzer's dB labels use an ASCII hyphen (H-24's `dbAxisTicks`); param rows keep
their own slider (not the kit `Slider`) because their drag/wheel/typing behaviour is spec'd there.

## 18. Working with the kit

- New UI strings → `ui/src/lib/i18n/en.json`; shortcuts → `shortcutLabelForAction`; icons → the
  registry.
- Style with roles; if a value you need isn't a token, add a token (and a contrast pair if it's a
  colour) rather than a literal.
- Svelte 5: kit components expose `$bindable` values plus `onchange` callbacks; tests dispatch
  synthetic events with `{ bubbles: true }` and `flushSync()` before asserting (helpers in
  `ui/src/lib/ui/testing.ts`).
- Vitest stubs CSS imports to "" except files matched by `test.css.include` in `vitest.config.ts`
  (only `design-tokens.css`, for the contrast test).

## 19. H-26 follow-ups

### 19.1 One menu, one popover
Every dropdown renders through `Menu` (on `Popover`): the five menu-bar menus (`menu/MenuBarMenu.svelte`
wraps a trigger + `Menu`; the feature menus only build `MenuEntry[]`), Normalize ▾, Add module
(as wide as its button), the rack slot menu with its Presets submenu, Effects → Favorites / Rack
Presets, and the Record context menu (opens at the pointer, or under the key from the keyboard).
The Punch & pre-roll panel is a `Popover` (`role=dialog`). One row look (24 px, neutral highlight,
check/dot column only when needed, right-aligned `Kbd` chips, chevron for submenus), one
elevation, one dismissal model. Menu-bar menus now also close on an outside click and switch on
hover while one is open. Inline forms inside a menu (a preset name) are `custom` entries; keys
typed there never drive the menu except Escape.

### 19.2 Axis labels never collide
All scale labels go through `ui/axisLabels.ts` (tested): `fitAxisLabels` keeps labels inside the
axis, edge-aligned at its ends (never cut off), clear of each other and of reserved slots, in
priority order; `fitGutterLabels` does the same for vertical rulers with a unit in the corner;
`rectsOverlap` for canvas-drawn graphs. Units always have their own slot:

| Axis | Unit slot |
|---|---|
| Analyzer dB | Axis-title band above the gutter ("dBFS"); ticks in the cell below |
| Analyzer frequency | Corner cell under the dB gutter ("Hz") |
| EQ gain | Axis title in the graph's toolbar row, above the gain labels ("dB") |
| EQ frequency | Reserved slot at the right end of the frequency row ("Hz"); the bottom-left corner belongs to the gain scale (`eq/axisLayout.ts`) |
| Spectral frequency ruler | Top-left corner of the gutter; a tick that would touch it is dropped |
| Time ruler | No unit (the labels are timecode); a label that would run past the right end is dropped |
| Waveform amplitude ruler | Top-left corner; labels via `fitGutterLabels` (0 dBFS edge labels align inward); `amplitudeTicksDbfs` drops mirrored pairs that would crowd the centerline |
| Meters | No scale today — only readouts, which use the true minus |

### 19.3 Preview scenes
`?preview` takes `&scene=` (see `ui/src/dev/previewIpc.ts`) to open the real App on fixtures —
a document in waveform or spectral view, recording, a 4-module rack with an EQ graph,
loudness/ACX results, and each dialog — for screenshots at 1280×720 and 2126×850.

## 20. Plugin manager (T-809)

- **Where:** Effects → Manage Plugins… / Install Module…, Preferences → Plugins (summary, the
  user's own folders, Manage plugins…), and a flagged rack slot's warning key (it opens the manager
  on that plugin). Code: `ui/src/lib/plugins/`; the list logic (search, sort, status wording,
  format badges) is pure and tested in `pluginList.ts`.
- **Dialog `size="xl"`** (new in the kit): 880 px wide with a fixed height (`min(680px, 85vh)`), and
  the body fills it. It's for workspace dialogs whose content filters and scrolls inside, so the
  box doesn't jump while the user types in a search field.
- **Layout:** full-bleed `Tabs` (Plugins | Folders) under the title. On the Plugins tab: a
  toolbar (search field, Rescan ▾ with *new and changed* / *everything*, primary Install
  module…), a slim progress row while scanning ("Scanning 7 of 19 · file.clap"), then the list
  as a recessed well (`--pv-bg-panel`) with a sticky header and a count/last-scan status line.
- **Rows** (two lines): the name, with the path under it in xs mono tertiary; the vendor, with the
  version under it; a format chip (outlined; CLAP / VST3 / LV2 / JSFX); channels (Mono / Stereo /
  "1 in · 2 out" / —); a params count (tabular, right-aligned); a status `Badge` with an icon and a
  word, plus its reason under it; an Enabled switch (`Toggle hideLabel`, named "Show ‹name› in Add
  module"), or **Unblock** for a blocklisted file; and ⋯ (clear crash warning, block this file,
  show in the file manager, unblock). Status tones: OK success, Disabled neutral, Blocklisted
  danger (the cause: crashed / timed out / blocked by you), Flagged warning ("Crashed 3 times
  while running"). The row the manager was opened on gets `--pv-accent-soft` and a 2 px accent
  inset.
- **States:** loading (a spinner and a word), error (an `EmptyState` with Try again), empty (an
  `EmptyState` with Install module… and Add folder…), and no search match (Clear search).
- **Install module… dialogs** (`sm`/`md`): *Installing* (an indeterminate bar and the trust note);
  *Replace ‹file›?* (`alertdialog`, Cancel, and Replace as a danger primary); *Module installed*
  (the effects added, where to find them, Show in plugin manager); *Couldn't install ‹file›* (the
  reason, a mono detail line, and the blocklist note).
- **`Toggle hideLabel`** (new in the kit): the label is kept for assistive tech only, for a switch
  in a table row whose column header already says what it does.

