<script lang="ts">
  import { analyzerState, setAnalyzerVisible } from "../analyzer/analyzer.svelte";
  import { t, tDynamic } from "../i18n";
  import { dispatchAction } from "../shortcuts";
  import { shortcutLabelForAction } from "../shortcuts/shortcutLabel";
  import MenuBarMenu from "../menu/MenuBarMenu.svelte";
  import { MENU_MNEMONICS } from "../menu/menubar.svelte";
  import type { RendererPreference } from "../render/rendererMode";
  import { rendererPref, setRendererPreference } from "../state/rendererPref.svelte";
  import { saveSettings, settingsState } from "../state/settings.svelte";
  import { spectralState } from "../state/spectral.svelte";
  import { setTimeRulerFormat, timeRulerFormatState } from "../state/waveformView.svelte";
  import { chooseTheme } from "../theme/chooseTheme";
  import { themeState, THEMES } from "../theme/theme.svelte";
  import type { MenuEntry } from "../ui/menuModel";
  import type { TimeRulerFormatDto } from "../ipc/bindings";

  /**
   * View menu (H-19): Spectral/Analyzer toggles, waveform zoom, and the H-13 renderer override
   * (View → Renderer, backed by `Settings.renderer_preference`). H-26: on the shared menu.
   * H-28 item 3: Follow Playhead (SPEC-003 §3/SPEC-006 §2.8, `Settings.playhead_follow`), same
   * checkbox pattern as Analyzer — no shortcut is named in either spec, so none is bound.
   * T-708: Theme ▸ (Dark / Light / Match System / High Contrast), radio rows from the same theme
   * list as Preferences → Appearance; applies at once and persists. SPEC-003 names no theme
   * shortcut, so none is bound.
   * T-206: Time Format ▸ (Timecode / Samples / Seconds, SPEC-006 §2.5, per-document —
   * `waveformView.svelte.ts`'s `timeRulerFormatState`, no shortcut named by the spec) and Snap to
   * Zero Crossing (SPEC-006 §2.10, `Settings.snap_to_zero_crossing`, same checkbox pattern as
   * Follow Playhead).
   */
  const analyzer = analyzerState();
  const spectral = spectralState();
  const renderer = rendererPref();
  const theme = themeState();
  const timeFormat = timeRulerFormatState();
  const playheadFollow = $derived(settingsState().current?.playhead_follow ?? true);
  const snapToZeroCrossing = $derived(settingsState().current?.snap_to_zero_crossing ?? false);

  function togglePlayheadFollow(): void {
    void saveSettings({ playhead_follow: !playheadFollow });
  }

  function toggleSnapToZeroCrossing(): void {
    void saveSettings({ snap_to_zero_crossing: !snapToZeroCrossing });
  }

  const TIME_FORMAT_OPTIONS: readonly { value: TimeRulerFormatDto; labelKey: string }[] = [
    { value: "timecode", labelKey: "menu.view.time_format_timecode" },
    { value: "samples", labelKey: "menu.view.time_format_samples" },
    { value: "seconds", labelKey: "menu.view.time_format_seconds" },
  ];

  const RENDERER_OPTIONS: readonly { value: RendererPreference; labelKey: string }[] = [
    { value: "auto", labelKey: "menu.view.renderer_auto" },
    { value: "webgl2", labelKey: "menu.view.renderer_webgl2" },
    { value: "canvas2d", labelKey: "menu.view.renderer_canvas2d" },
  ];

  /** Applies the choice to the live renderers (H-13's store, no reload needed) and persists it
   * (H-19: `Settings.renderer_preference`, `just gen-types`) so it survives a restart. */
  function pickRenderer(value: RendererPreference): void {
    setRendererPreference(value);
    void saveSettings({ renderer_preference: value });
  }

  const items = $derived<MenuEntry[]>([
    {
      kind: "checkbox",
      id: "spectral",
      label: t("spectral.toggle"),
      checked: spectral.visible,
      shortcut: shortcutLabelForAction("spectral.toggle"),
      testid: "menu-view-spectral",
      onselect: () => dispatchAction("spectral.toggle"),
    },
    {
      kind: "checkbox",
      id: "analyzer",
      label: t("menu.view.analyzer"),
      checked: analyzer.visible,
      testid: "menu-view-analyzer",
      onselect: () => setAnalyzerVisible(!analyzer.visible),
    },
    {
      kind: "checkbox",
      id: "playhead-follow",
      label: t("menu.view.playhead_follow"),
      checked: playheadFollow,
      testid: "menu-view-playhead-follow",
      onselect: togglePlayheadFollow,
    },
    {
      kind: "checkbox",
      id: "snap-to-zero-crossing",
      label: t("menu.view.snap_to_zero_crossing"),
      checked: snapToZeroCrossing,
      testid: "menu-view-snap-to-zero-crossing",
      onselect: toggleSnapToZeroCrossing,
    },
    { kind: "separator", id: "sep-zoom" },
    {
      kind: "item",
      id: "zoom-in",
      label: t("menu.view.zoom_in"),
      shortcut: shortcutLabelForAction("waveform.zoom_in"),
      testid: "menu-zoom-in",
      onselect: () => dispatchAction("waveform.zoom_in"),
    },
    {
      kind: "item",
      id: "zoom-out",
      label: t("menu.view.zoom_out"),
      shortcut: shortcutLabelForAction("waveform.zoom_out"),
      testid: "menu-zoom-out",
      onselect: () => dispatchAction("waveform.zoom_out"),
    },
    { kind: "separator", id: "sep-renderer" },
    {
      kind: "submenu",
      id: "theme",
      label: t("menu.view.theme"),
      testid: "menu-theme",
      menuTestid: "menu-theme-list",
      minWidth: 180,
      items: THEMES.map((choice) => ({
        kind: "radio" as const,
        id: choice.pref,
        label: tDynamic(choice.menuLabelKey),
        checked: theme.pref === choice.pref,
        testid: `menu-theme-${choice.pref}`,
        onselect: () => chooseTheme(choice.pref),
      })),
    },
    {
      kind: "submenu",
      id: "renderer",
      label: t("menu.view.renderer"),
      testid: "menu-renderer",
      minWidth: 160,
      items: RENDERER_OPTIONS.map((option) => ({
        kind: "radio" as const,
        id: option.value,
        label: tDynamic(option.labelKey),
        checked: renderer.value === option.value,
        testid: `menu-renderer-${option.value}`,
        onselect: () => pickRenderer(option.value),
      })),
    },
    {
      kind: "submenu",
      id: "time-format",
      label: t("menu.view.time_format"),
      testid: "menu-time-format",
      menuTestid: "menu-time-format-list",
      minWidth: 160,
      items: TIME_FORMAT_OPTIONS.map((option) => ({
        kind: "radio" as const,
        id: option.value,
        label: tDynamic(option.labelKey),
        checked: timeFormat.current === option.value,
        testid: `menu-time-format-${option.value}`,
        onselect: () => setTimeRulerFormat(option.value),
      })),
    },
  ]);
</script>

<MenuBarMenu
  id="view"
  label={t("menu.view")}
  mnemonic={MENU_MNEMONICS.view}
  {items}
  triggerTestid="menu-trigger-view"
  menuTestid="view-menu"
  minWidth={224}
/>
