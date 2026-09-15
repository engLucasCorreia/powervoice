/**
 * The guided tours (T-709): the Welcome tour (the user guide's first-recording flow) and the short
 * contextual tours started from a panel's "?". Content is i18n keys only; every `target` names a
 * `data-tour="…"` anchor in the app shell (`tourAnchors.test.ts` fails if one disappears).
 *
 * Bump a tour's `version` when its steps change meaningfully: someone who completed or skipped the
 * older version is offered it again (`progress.ts`).
 */
import type { MessageKey, MessageParams } from "../i18n";
import { shortcutLabelForAction } from "../shortcuts/shortcutLabel";
import { setDockTab } from "../layout/layoutSettings.svelte";
import { openPluginManager, pluginsState, setManagerTab } from "../plugins/plugins.svelte";
import { recordState } from "../state/record.svelte";
import type { ActionId } from "../shortcuts/actions";
import type { IconName } from "../ui/icons";
import type { TourPlacement } from "./tourPlacement";

export const TOUR_IDS = ["welcome", "rack", "noise", "loudness", "punch", "plugins"] as const;
export type TourId = (typeof TOUR_IDS)[number];

export interface TourStep {
  /** Stable within its tour (tests, preview). */
  id: string;
  /**
   * `data-tour` anchors to point at, in order of preference — the first one on screen wins. The
   * LAST one is the anchor the shell always renders (checked by `tourAnchors.test.ts`); earlier
   * ones may be conditional (the Noise Reduction slot's capture row). No target: a centred card.
   */
  target?: readonly string[];
  /** Preferred side for the card (default below); it flips to stay in the window. */
  placement?: TourPlacement;
  titleKey: MessageKey;
  bodyKey: MessageKey;
  /** Placeholder values for the title/body/hint (shortcut labels for this platform). */
  params?: () => MessageParams;
  /** An icon drawn large at the top of the card. */
  illustration?: IconName;
  /** Runs when the step becomes current: bring its target on screen (a dock tab, a dialog). */
  enter?: () => void;
  /**
   * Action-gated step: it advances by itself once `done()` turns true while the step is showing
   * (reactive — read app state through a store). Next stays available to skip the action.
   */
  waitFor?: { done: () => boolean; hintKey: MessageKey };
  /** The pointer may reach the highlighted element (to click Record, to open Audio Devices). */
  interactive?: boolean;
}

export interface TourDef {
  id: string;
  version: number;
  /** The tour's name (Help → Tours, the "?" tooltip, the card's eyebrow). */
  nameKey: MessageKey;
  steps: readonly TourStep[];
}

const key = (action: ActionId): string => shortcutLabelForAction(action) ?? "";

const showLoudnessTab = (): void => setDockTab("loudness");

function showPluginManager(): void {
  if (!pluginsState().open) {
    openPluginManager({ tab: "plugins" });
  } else if (pluginsState().tab !== "plugins") {
    setManagerTab("plugins");
  }
}

const WELCOME: TourDef = {
  id: "welcome",
  version: 1,
  nameKey: "tour.name.welcome",
  steps: [
    {
      id: "intro",
      titleKey: "tour.welcome.intro.title",
      bodyKey: "tour.welcome.intro.body",
      illustration: "waveform",
    },
    {
      id: "devices",
      target: ["devices"],
      titleKey: "tour.welcome.devices.title",
      bodyKey: "tour.welcome.devices.body",
      interactive: true,
    },
    {
      id: "record",
      target: ["record"],
      titleKey: "tour.welcome.record.title",
      bodyKey: "tour.welcome.record.body",
      params: () => ({ shortcut: key("record.toggle") }),
      interactive: true,
      waitFor: { done: () => recordState().state.recording, hintKey: "tour.welcome.record.wait" },
    },
    {
      id: "time",
      target: ["time"],
      titleKey: "tour.welcome.time.title",
      bodyKey: "tour.welcome.time.body",
      params: () => ({ shortcut: key("record.toggle") }),
    },
    {
      id: "editor",
      target: ["editor"],
      titleKey: "tour.welcome.editor.title",
      bodyKey: "tour.welcome.editor.body",
      params: () => ({
        zoom_in: key("waveform.zoom_in"),
        zoom_out: key("waveform.zoom_out"),
        spectral: key("spectral.toggle"),
      }),
    },
    {
      id: "markers",
      target: ["markers"],
      placement: "right",
      titleKey: "tour.welcome.markers.title",
      bodyKey: "tour.welcome.markers.body",
      params: () => ({ shortcut: key("marker.add") }),
    },
    {
      id: "rack",
      target: ["rack"],
      placement: "left",
      titleKey: "tour.welcome.rack.title",
      bodyKey: "tour.welcome.rack.body",
    },
    {
      id: "noise",
      target: ["menu-effects"],
      titleKey: "tour.welcome.noise.title",
      bodyKey: "tour.welcome.noise.body",
      params: () => ({ shortcut: key("nr.capture_noise_print") }),
    },
    {
      id: "dock",
      target: ["dock"],
      placement: "top",
      titleKey: "tour.welcome.dock.title",
      bodyKey: "tour.welcome.dock.body",
    },
    {
      id: "finish",
      target: ["menu-help"],
      titleKey: "tour.welcome.finish.title",
      bodyKey: "tour.welcome.finish.body",
    },
  ],
};

const RACK: TourDef = {
  id: "rack",
  version: 1,
  nameKey: "tour.name.rack",
  steps: [
    { id: "panel", target: ["rack"], placement: "left", titleKey: "tour.rack.panel.title", bodyKey: "tour.rack.panel.body" },
    { id: "add", target: ["rack-add"], placement: "left", titleKey: "tour.rack.add.title", bodyKey: "tour.rack.add.body" },
    { id: "slots", target: ["rack-slots"], placement: "left", titleKey: "tour.rack.slots.title", bodyKey: "tour.rack.slots.body" },
    { id: "presets", target: ["rack-ab"], placement: "left", titleKey: "tour.rack.presets.title", bodyKey: "tour.rack.presets.body" },
  ],
};

const NOISE: TourDef = {
  id: "noise",
  version: 1,
  nameKey: "tour.name.noise",
  steps: [
    { id: "select", target: ["editor"], titleKey: "tour.noise.select.title", bodyKey: "tour.noise.select.body" },
    {
      id: "capture",
      target: ["nr-capture", "menu-effects"],
      placement: "left",
      titleKey: "tour.noise.capture.title",
      bodyKey: "tour.noise.capture.body",
      params: () => ({ shortcut: key("nr.capture_noise_print") }),
    },
    { id: "reduce", target: ["rack-add"], placement: "left", titleKey: "tour.noise.reduce.title", bodyKey: "tour.noise.reduce.body" },
  ],
};

const LOUDNESS: TourDef = {
  id: "loudness",
  version: 1,
  nameKey: "tour.name.loudness",
  steps: [
    {
      id: "analyze",
      target: ["loudness-controls"],
      placement: "top",
      titleKey: "tour.loudness.analyze.title",
      bodyKey: "tour.loudness.analyze.body",
      enter: showLoudnessTab,
    },
    {
      id: "readouts",
      target: ["loudness-readouts"],
      placement: "top",
      titleKey: "tour.loudness.readouts.title",
      bodyKey: "tour.loudness.readouts.body",
      enter: showLoudnessTab,
    },
    {
      id: "acx",
      target: ["acx"],
      placement: "top",
      titleKey: "tour.loudness.acx.title",
      bodyKey: "tour.loudness.acx.body",
      enter: showLoudnessTab,
    },
    { id: "normalize", target: ["normalize"], titleKey: "tour.loudness.normalize.title", bodyKey: "tour.loudness.normalize.body" },
  ],
};

const PUNCH: TourDef = {
  id: "punch",
  version: 1,
  nameKey: "tour.name.punch",
  steps: [
    { id: "select", target: ["editor"], titleKey: "tour.punch.select.title", bodyKey: "tour.punch.select.body" },
    { id: "settings", target: ["punch"], titleKey: "tour.punch.settings.title", bodyKey: "tour.punch.settings.body" },
    {
      id: "record",
      target: ["record"],
      titleKey: "tour.punch.record.title",
      bodyKey: "tour.punch.record.body",
      params: () => ({ shortcut: key("record.toggle") }),
    },
  ],
};

const PLUGINS: TourDef = {
  id: "plugins",
  version: 1,
  nameKey: "tour.name.plugins",
  steps: [
    { id: "tabs", target: ["plugins-tabs"], titleKey: "tour.plugins.tabs.title", bodyKey: "tour.plugins.tabs.body", enter: showPluginManager },
    {
      id: "toolbar",
      target: ["plugins-toolbar"],
      titleKey: "tour.plugins.toolbar.title",
      bodyKey: "tour.plugins.toolbar.body",
      enter: showPluginManager,
    },
    { id: "list", target: ["plugins-list"], titleKey: "tour.plugins.list.title", bodyKey: "tour.plugins.list.body", enter: showPluginManager },
    {
      id: "install",
      target: ["plugins-install"],
      titleKey: "tour.plugins.install.title",
      bodyKey: "tour.plugins.install.body",
      enter: showPluginManager,
    },
  ],
};

export const TOURS: Readonly<Record<TourId, TourDef>> = {
  welcome: WELCOME,
  rack: RACK,
  noise: NOISE,
  loudness: LOUDNESS,
  punch: PUNCH,
  plugins: PLUGINS,
};

export function isTourId(value: string): value is TourId {
  return (TOUR_IDS as readonly string[]).includes(value);
}
