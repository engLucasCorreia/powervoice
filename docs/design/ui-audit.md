# PowerVoice UI audit (H-25, phase 1)

- **Date:** 2026-09-14 · **Code:** `main` at `d7d0e7d` · **Screenshot:** the owner's running app,
  3401×1360 px (the 2126×850 window at 1.6× scale).
- **Method:** the screenshot, plus every `.svelte` file under `ui/src/lib` (styles and markup),
  measured with greps over the style blocks and a WCAG contrast calculation on `tokens.css`.
- **Priorities:** **P0** broken, blocks use · **P1** hurts every session (hierarchy, legibility,
  consistency, accessibility) · **P2** polish.
- **Owner feedback to keep:** "The spectrum analyzer is very good, and the meters that jump when I
  play are also very good." Their *rendering* (analyzer fill + amber peak hold, hover readout; meter
  ballistics and colours) stays exactly as is; only the chrome around them changes.
- **Who fixes what:** layout/splitters/scales are **H-24** (in progress); everything else is H-25
  phase 2 (plan in `design-system.md` §17).

## 1. Top findings

| # | Finding | Evidence | P |
|---|---|---|---|
| 1 | The workspace collapses to a ~20 px strip and the bottom dock fills the window; the Markers header overlaps the Loudness panel's Processed/Source buttons. | Screenshot: waveform strip at y≈135, analyzer grid from y≈220 to the bottom; "MARKERS" drawn over "Processed". `App.svelte` `.shell` has 4 row tracks for 5 children. | P0 (H-24) |
| 2 | No hierarchy in the toolbar: 22 controls in one row, all the same grey 13 px text button — Play looks exactly like "Audio devices…" and "−16.0 LUFS". The six normalize favourites take a third of the toolbar. | Screenshot top row; `Toolbar.svelte`, `RecordControls.svelte`, `NormalizeToolbarButtons.svelte`. | P1 |
| 3 | Recording isn't unmistakable. Idle Record is a grey text button identical to "Input"; while recording it turns red with `#e6e7ea` text at **3.17:1** (fails AA). The CLIP lamp is 11 px grey text that reads as disabled. | `RecordControls.svelte` `.rec.recording`, `.clip`. | P1 |
| 4 | There is no component layer. The same `button {}` rule is copied into **24** files; the dialog shell (backdrop + box) into **17**; **13** different font sizes (0.6rem … 1rem, mixing `rem` and `em`); **~25** padding combinations; **5** radii; **7** z-index values (1 … 1000). | Style-block greps (§4). | P1 |
| 5 | Two density systems in one window: toolbar controls are 13 px text on ~30 px buttons, while analyzer and spectral controls are 11 px (0.7rem) text on ~16 px tall buttons with 2–3 px radii — below the 24 px minimum hit target. | Screenshot (analyzer header: Fast/Medium/Slow, Floor, Ceiling); `AnalyzerPanel.svelte`, `SpectralView.svelte`. | P1 |
| 6 | Hints and "not analyzed" readouts use `--text-disabled` (`#5c5f66`): **2.49:1** on panels, 2.28:1 on raised surfaces — unreadable, and it signals "disabled" when the text is live. | `ExportDialog .hint`, `LoudnessPanel .readout.muted`. | P1 |
| 7 | Units are missing or inconsistent. The analyzer's Floor/Ceiling show "-120"/"0" with no unit and an ASCII hyphen; the remaining-disk readout "280:17:07 left" has no unit (hours:minutes:seconds?); number fields use `type="number"` with locale-dependent minus signs. | Screenshot; `AnalyzerPanel.svelte`, `RecordControls.svelte`. | P1 |
| 8 | Colours come from tokens that don't exist, so the hard-coded fallbacks are what actually renders: `var(--error, #c0392b)` (both Normalize dialogs), `var(--danger, #d05050)` (Recovery), `var(--accent, #4a90d9)` / `var(--text-on-accent, #fff)` (Loudness). `#ff5c5c` is the record colour via a fallback in five places in `WaveformView`. | Greps of `.svelte` files. | P1 |
| 9 | Empty states are bare grey sentences ("No file open", "No modules yet — Add module to build a chain") with no action, no shortcut and no visual anchor. The first thing a new user sees is an empty grey area. | `EditorView`/`WaveformView` `.empty`, `RackPanel .empty`. | P1 |
| 10 | Icons are text glyphs: `▸`/`▾` for disclosure, `&times;` for dismiss, a text power button on rack slots; no icon set, so actions that normally get icons (transport, bypass, delete) are words or ad-hoc characters. | `RackSlot.svelte`, `Toast.svelte`, `Banner.svelte`, `MenuItemRow.svelte`. | P2 |

## 2. Screen by screen

### 2.1 Window and shell
| Issue | P |
|---|---|
| Layout rows mismatch (5 children, 4 tracks); fixed column widths; no splitters except waveform/spectral. | P0 (H-24) |
| Everything sits on the same two greys (`#1a1c20` app, `#202226` panel) with 1 px `#34373d` borders at 1.34:1 — panels don't separate, so the eye finds no edges. There's no inset "well" around the audio, so the waveform doesn't read as the hero. | P1 |
| Typography stack starts with `"Inter"`, which is not bundled (A-017: system font stack) — the app renders in whatever `system-ui` resolves to anyway, but metrics differ per machine. | P2 |
| No light theme and no theme switch; `tokens.css` is dark-only. | P2 |

### 2.2 Menu bar
| Issue | P |
|---|---|
| The document name ("Untitled \*") is a menu-bar item squeezed between File and Edit. It's document state, not a menu; it belongs in the window title and at the start of the transport bar. | P1 |
| 0.85em text with 0.2 rem padding makes the bar ~24 px; menu rows are 0.35 rem padded; menu rows have no min height. Hover = full accent fill with dark text — heavy next to the calmer popovers elsewhere. | P2 |
| Hand-rolled popovers elsewhere (record context menu, rack slot menu, Add module, rack presets) use a *different* hover (`--surface-panel-raised`), padding (0.3 rem 0.6 rem) and radius than the real menus. | P1 |

### 2.3 Toolbar / transport
| Issue | P |
|---|---|
| "PowerVoice 0.1.0" (app name + version) occupies the most valuable spot, top-left. It belongs in About. | P1 |
| Transport actions are text buttons ("Return to start", "Play", "Stop", "Play from start") — wide, slow to scan, no icons; Stop is disabled grey text that looks broken. | P1 |
| Two time displays in different formats: playhead `00:00:15.207` and record elapsed `0:00:00.0`, both in bordered boxes of the same weight as buttons. The playhead time should be the single largest element in the chrome. | P1 |
| Normalize favourites (−1.0 dB, −0.1 dB, −3.0 dB, −16.0 LUFS, −19.0 LUFS, −23.0 LUFS) are six full buttons. They're occasional actions and belong in one "Normalize" split button/menu (they're already in Effects → Favorites). | P1 |
| Active state (Spectral) is a blue border + blue text only — the same as hover, so hover looks like selection. | P1 |
| Nothing is grouped: no separators between transport / record / monitoring / view / devices. | P1 |

### 2.4 Record controls and the Punch & pre-roll popover
| Issue | P |
|---|---|
| "Input" (arm) and "Record" look identical; armed = blue border. No lamp, no red. | P1 |
| Recording: red fill with light text **3.17:1**; the phase pill ("Pre-roll 2.0 s") and the red monitor-latency pill use the same failing pair. | P1 |
| "Monitoring" label + native select ("Off") + latency text are three differently styled things; the amber/red latency pills use `--surface-panel` as text colour. | P2 |
| The popover is a 22 rem stack of native checkboxes, `type="number"` inputs (spinner arrows, locale minus) and 0.8 rem labels; units ("s", "ms") are loose text after the inputs; no sections. | P1 |
| Offset entry placeholder is the untranslated literal `0.00 / 150 smp`. | P2 |

### 2.5 Markers / Properties panel (left)
| Issue | P |
|---|---|
| Title is 0.8 rem uppercase tracked text; the Add/Delete buttons are 0.75 rem text buttons with 0.1 rem padding (~18 px tall). | P1 |
| Selected row = full accent fill with `--surface-panel` text — heavier than the waveform selection it mirrors. | P2 |
| "Properties" is a heading with nothing under it. | P2 |
| Empty state is one grey sentence. | P2 |

### 2.6 Editor (waveform, ruler, spectral pane)
| Issue | P |
|---|---|
| Collapsed height; no amplitude scale; long fixed `00:00:01.000` ruler labels. | P0/P1 (H-24) |
| The scrollbar is a native `<input type="range">` — it looks like a volume slider, not a scrollbar. | P1 |
| The waveform/spectral divider is a flat 6 px bar that turns fully accent on hover; no grip. | P2 |
| The spectral toolbar uses 0.7 rem controls with 3 px radii (a third density system). | P1 |
| "No file open" empty state: no action, no hint that Record also works without a file. | P1 |

### 2.7 Rack panel and slots
| Issue | P |
|---|---|
| Uppercase tracked title; the A/B toggle is a 0.75 rem pill; the latency line floats below the header with no label. | P2 |
| Slot header: power toggle is a 1.4 rem text circle, disclosure is `▸`/`▾` text, the slot menu is `⋯`-style text. Bypassed = `opacity: 0.6` on the whole card (fades the controls you'd use to un-bypass). | P1 |
| Param rows: 8 rem fixed label column, 0.9 rem-tall custom sliders, value boxes with 3 px radius; the invalid state's red border is the only error signal besides a small message. | P2 |
| Slot menus and preset submenus are hand-rolled with their own hover/padding (see 2.2). | P1 |
| EQ graph chrome: the 0.75 rem range toggle and 0.7 rem band labels are a separate micro-style; the curve rendering itself is fine. | P2 |

### 2.8 Loudness / ACX panel
| Issue | P |
|---|---|
| One wrapping flex row: title, Processed/Source toggle, Analyze, six readouts and the ACX section all at 0.75 rem, so the result (Integrated LUFS) has no more weight than a button label. | P1 |
| Processed/Source toggle colours come from fallbacks (`#4a90d9`, `#fff`). | P1 |
| "Analyzing 42%" lives inside the button label; there's no progress bar. | P2 |
| The ACX table marks pass/fail by colour plus a glyph; the result line is colour + word. OK for colour-blind users, but the table has no units column alignment. | P2 |

### 2.9 Meter bridge
| Issue | P |
|---|---|
| Keep the meters' look. Chrome only: the "Meters" label, input meter and output meter sit in one row with no In/Out labels; readouts "Peak −∞ dBFS" / "RMS −∞ dBFS" are fine (units present) but 0.75 rem secondary text. | P2 |
| In the screenshot the bridge is vertically centred in a ~600 px tall region (H-24 layout). | P0 (H-24) |

### 2.10 Analyzer panel
| Issue | P |
|---|---|
| Keep the rendering. The header's Fast/Medium/Slow buttons are 0.7 rem with 2 px radii (~16 px tall); Floor/Ceiling selects have no unit; the Peak hold checkbox is a native blue checkbox, the only native-styled checkbox in view. | P1 |
| No frequency/dB labels on the grid. | P1 (H-24) |

### 2.11 Dialogs (17)
Normalize, Normalize (LUFS), normalize progress, Export, New Recording, Low Disk, Calibration, Preferences, Recovery, Unsaved Changes, Confirm, Recent Missing, Save As, Channel Choice, Clip Prompt, About, Audio Devices.

| Issue | P |
|---|---|
| 17 copies of the same backdrop/box CSS with drifting values: backdrop z-index 900 vs 1000, `min-width` 20/24/28 rem, gaps 0.6/0.75 rem. | P1 |
| The primary button is "accent border + accent text" — weaker than a filled button, so the dialog's main action doesn't stand out. Button order is right (Cancel, then primary) but not platform-aware. | P1 |
| Titles are 1 rem (16 px) with no header/footer structure; no close button; fieldsets with legends (a dated look) in Export/New Recording. | P2 |
| Focus handling varies: some dialogs `stopPropagation` on keydown, several don't trap focus or restore it on close. | P1 |

### 2.12 Notices (toasts, banners)
| Issue | P |
|---|---|
| Dismiss is a `&times;` text glyph; level is a coloured border only (the error/warning distinction relies on colour). | P2 |
| Toasts use a one-off shadow (`rgba(0,0,0,.35)`) and z-index 1000 (the same as dialogs). | P2 |

## 3. Global states

| State | Today | P |
|---|---|---|
| Hover | Border turns accent — the same treatment as "active/selected". | P1 |
| Active/pressed | No pressed style on buttons. | P2 |
| Selected/on | Accent border + accent text (toolbar) · accent fill (analyzer responses, A/B, markers) — three different treatments. | P1 |
| Focus | A global `:focus-visible` outline exists (good), with a 1 px offset that disappears against the accent-filled controls. | P2 |
| Disabled | Only the text colour changes; `cursor: not-allowed` in some places, nothing in others. | P2 |
| Loading | Only a text change ("Analyzing…", "Checking…"). | P2 |
| Motion | None, and no `prefers-reduced-motion` handling. | P2 |

## 4. Measurements

| Measure | Value |
|---|---|
| `button {}` rule copies | 24 files |
| Dialog shells (`.backdrop`) | 17 |
| Distinct `font-size` values | 13 (0.6rem, 0.65rem, 0.7rem, 0.7em, 0.75rem, 0.78rem, 0.8rem, 0.85rem, 0.85em, 0.9rem, 0.9em, 1rem, inherit) |
| Distinct `padding` shorthands | ~25 |
| Border radii | 2, 3, 4, 6 px and 50 % |
| z-index values | 1, 10, 20, 50, 100, 900, 1000 |
| Uppercase tracked headings | 3 files (Markers, Rack, Add module) |
| `title=` tooltips (native, 1 s delay, unstyled) | 26 |

### Contrast of today's tokens (WCAG AA: 4.5:1 text, 3:1 non-text)
| Pair | Ratio | |
|---|---|---|
| `--text-primary` on `--surface-panel` | 12.88 | pass |
| `--text-secondary` on `--surface-panel` | 5.87 | pass |
| `--text-disabled` on `--surface-panel` (used for hints) | **2.49** | fail |
| `--text-disabled` on `--surface-panel-raised` | **2.28** | fail |
| `--text-primary` on `--meter-red` (recording button, phase pill) | **3.17** | fail |
| `--accent` on `--surface-panel` | 6.07 | pass |
| `--surface-border` on `--surface-panel` (panel edges) | 1.34 | weak edge |

The new tokens (`design-tokens.css`) pass every text/background pair in both themes; `ui/src/lib/theme/contrast.test.ts` enforces it.

## 5. Status after phase 2 (H-25, 2026-09-14)

| # | Finding | Status |
|---|---|---|
| 1 | Workspace collapse / Rack off-screen | Fixed (H-24 layout + H-25 `minmax(0, 1fr)` shell column and `fitSideColumns`). |
| 2 | Toolbar without hierarchy | Fixed: grouped transport bar, icon keys with tooltips, one Normalize menu. |
| 3 | Recording not unmistakable | Fixed: Record key lamp → solid red Stop, tally line, red take time; AA-passing red fill. |
| 4 | No component layer | Fixed: kit in `ui/src/lib/ui/`, one Dialog shell for 17 dialogs, tokens for every restyled screen. |
| 5 | Two density systems | Fixed: sm/md/lg control sizes everywhere (analyzer and spectral toolbars on sm). |
| 6 | Hints in disabled grey | Fixed on restyled screens (`--pv-text-tertiary`, AA). |
| 7 | Missing units | Fixed: analyzer floor/ceiling in dB, "Hz"/"dBFS" once on the axes, normalize favourites with dBFS/LUFS. "280:17:07 left" format is the record store's (unchanged). |
| 8 | Undefined tokens / hard-coded fallbacks | Fixed (dialogs, loudness, `--wave-record`). |
| 9 | Bare empty states | Fixed: editor, markers, rack. |
| 10 | Text-glyph icons | Fixed: Lucide via the icon registry. |

