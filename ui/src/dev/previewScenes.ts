/**
 * Dev-only preview scenes (H-26): after the App mounts on mocked IPC (`previewIpc.ts`), put it in
 * a state worth looking at — a document open, spectral view, recording, a full rack, loudness
 * results, a dialog or a menu open — so every screen can be screenshotted at 1280×720 and
 * 2126×850 without a backend. Only store functions and real UI clicks are used, so what shows is
 * exactly what the App would render.
 */
import { emit } from "@tauri-apps/api/event";
import { openDocument, openSaveAsPrompt, requestClose, requestSave } from "../lib/document/document.svelte";
import { pickRecentFile } from "../lib/document/recentFiles.svelte";
import { openExportDialog } from "../lib/export/export.svelte";
import { openAbout } from "../lib/help/about.svelte";
import { dispatchAction } from "../lib/keymap";
import { runAcxCheck } from "../lib/loudness/acx.svelte";
import { startLoudnessAnalyze } from "../lib/loudness/loudness.svelte";
import { openMenu, type MenuId } from "../lib/menu/menubar.svelte";
import { installFrom, openPluginManager } from "../lib/plugins/plugins.svelte";
import { openPreferences } from "../lib/preferences/preferences.svelte";
import { openRecoveryStorage } from "../lib/recovery/recovery.svelte";
import { continueBakeConfirm, startBake } from "../lib/state/bake.svelte";
import { openNormalizeDialog } from "../lib/state/normalize.svelte";
import { openNormalizeLufsDialog } from "../lib/state/normalizeLufs.svelte";
import { openCalibration, openNewRecordingPrompt, toggleRecord } from "../lib/state/record.svelte";
import { PREVIEW_INSTALL_SOURCE, PREVIEW_PATH, PREVIEW_RATE_HZ, type PreviewOptions } from "./previewIpc";

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

function click(testid: string): void {
  document.querySelector<HTMLElement>(`[data-testid="${testid}"]`)?.click();
}

const MENU_BAR: readonly string[] = ["file", "edit", "view", "effects", "help"];

export async function runPreviewScene(options: PreviewOptions, menu: string | null): Promise<void> {
  const { scenes, dialog } = options;
  // Let every store's onMount listener register and the first IPC round-trips settle.
  await sleep(400);

  const wantsDocument =
    scenes.some((s) =>
      ["document", "spectral", "loudness", "rack", "plugins", "plugins-scanning", "plugins-folders"].includes(s),
    ) ||
    (dialog !== null && !["about", "preferences", "recovery", "new-recording", "audio-devices", "channel-choice"].includes(dialog));

  if (scenes.includes("recording")) {
    await emit("document_changed", {
      name: null,
      path: null,
      sample_rate_hz: PREVIEW_RATE_HZ,
      len_samples: 0,
      dirty: true,
      audio_rev: 1,
      sidecar_dirty: false,
      spectral_view: null,
      waveform_view: null,
      recovered: false,
    });
  } else if (wantsDocument || dialog === "channel-choice") {
    await openDocument(PREVIEW_PATH);
  }
  await sleep(150);

  if (scenes.includes("spectral")) {
    dispatchAction("spectral.toggle");
  }
  if (scenes.includes("loudness")) {
    await startLoudnessAnalyze();
    await emit("job_progress", { job_id: 7, kind: "loudness_analyze", state: "done", fraction: 1 });
    await emit("loudness_report", {
      job_id: 7,
      integrated_lufs: -19.4,
      max_momentary_lufs: -12.8,
      max_short_term_lufs: -15.9,
      lra_lu: 6.2,
      sample_peak_dbfs: -3.4,
      true_peak_dbtp: -3.1,
    });
    await runAcxCheck();
  }

  switch (dialog) {
    case "about":
      openAbout();
      break;
    case "preferences":
      openPreferences();
      break;
    case "export":
      openExportDialog("chapter-03");
      break;
    case "new-recording":
      openNewRecordingPrompt({ sample_rate_hz: PREVIEW_RATE_HZ, bit_depth: "24" });
      break;
    case "calibration":
      openCalibration();
      break;
    case "normalize":
      openNormalizeDialog();
      break;
    case "normalize-lufs":
      openNormalizeLufsDialog();
      break;
    case "recovery-storage":
      await openRecoveryStorage();
      break;
    case "save-as":
      openSaveAsPrompt();
      break;
    case "unsaved":
      void requestClose();
      break;
    case "recent-missing":
      void pickRecentFile("/media/usb/old-take.wav", false);
      break;
    case "audio-devices":
      click("open-audio-devices");
      break;
    case "confirm":
      void openDocument(PREVIEW_PATH);
      break;
    case "clip":
      void requestSave();
      break;
    case "low-disk":
      void toggleRecord();
      break;
    case "plugin-install":
    case "plugin-collision":
    case "plugin-failed":
      void installFrom(PREVIEW_INSTALL_SOURCE, false);
      break;
    case "bake":
    case "bake-progress":
      // T-602 dialogs (needs `scene=rack`): the noise-only confirm, or — `bake-progress` — past it
      // to a running job (the progress dialog appears after 250 ms).
      await startBake();
      if (dialog === "bake-progress") {
        await continueBakeConfirm();
      }
      await emit("job_progress", { job_id: 9, kind: "bake", state: "running", fraction: 0.42 });
      break;
    default:
      break;
  }

  // T-809: the plugin manager — every status; `plugins-scanning` mid-rescan.
  if (scenes.includes("plugins") || scenes.includes("plugins-scanning") || scenes.includes("plugins-folders")) {
    openPluginManager({ tab: scenes.includes("plugins-folders") ? "folders" : "plugins" });
    await sleep(100);
    if (scenes.includes("plugins-scanning")) {
      await emit("plugin_scan_progress", {
        done: 7,
        total: 19,
        current_path: "/usr/lib/clap/studio-tools.clap",
        summary: null,
      });
    }
  }

  if (scenes.includes("plugin-flag")) {
    await sleep(200);
    const slots = document.querySelectorAll<HTMLElement>('[data-testid="rack-slot"]');
    slots[slots.length - 1]?.scrollIntoView({ block: "center" });
  }

  if (menu) {
    await sleep(100);
    if (MENU_BAR.includes(menu)) {
      openMenu(menu as MenuId);
    } else if (menu === "normalize") {
      click("toolbar-normalize-menu");
    } else if (menu === "add-module") {
      click("rack-add");
    } else if (menu === "rack-slot") {
      click("rack-slot-menu");
    } else if (menu === "theme") {
      // T-708: View → Theme ▸ open.
      openMenu("view");
      await sleep(100);
      click("menu-theme");
    }
  }
}
