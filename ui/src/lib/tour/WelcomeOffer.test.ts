import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { Settings } from "../ipc/bindings";
import { decideLater, initRecovery, resetRecoveryForTest } from "../recovery/recovery.svelte";
import { applyRecordStateForTest, resetRecordForTest } from "../state/record.svelte";
import { loadSettings, resetSettingsStateForTest, settingsState } from "../state/settings.svelte";
import { settingsFixture } from "../test/fixtures";
import { armWelcomeOffer, resetTourForTest, tourState } from "./tour.svelte";
import { TOURS } from "./tours";
import WelcomeOffer from "./WelcomeOffer.svelte";

let app: ReturnType<typeof mount> | null = null;
let host: HTMLElement | null = null;
let saved: Settings[] = [];

async function setup(options: { recoverable?: boolean; progress?: Settings["tours"]["progress"] } = {}): Promise<void> {
  saved = [];
  mockIPC((cmd, args) => {
    if (cmd === "settings_get") {
      return settingsFixture({ tours: { progress: options.progress ?? [] } });
    }
    if (cmd === "settings_set") {
      const settings = (args as { settings: Settings }).settings;
      saved.push(settings);
      return settings;
    }
    if (cmd === "recovery_list") {
      return options.recoverable ? [{ id: "s1", name: "take.wav" }] : [];
    }
    return null;
  });
  await loadSettings();
  armWelcomeOffer(settingsState().current?.tours);
  host = document.createElement("div");
  document.body.appendChild(host);
  app = mount(WelcomeOffer, { target: host, props: { delayMs: 0 } });
  flushSync();
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 5));
  flushSync();
}

const offer = (): HTMLElement | null => document.querySelector('[data-testid="tour-offer"]');
const button = (id: string): HTMLElement => document.querySelector<HTMLElement>(`[data-testid="${id}"]`)!;

afterEach(() => {
  if (app) {
    unmount(app);
    app = null;
  }
  host?.remove();
  host = null;
  resetTourForTest();
  resetRecoveryForTest();
  resetRecordForTest();
  resetSettingsStateForTest();
  clearMocks();
});

describe("Welcome tour offer (T-709)", () => {
  it("appears on first run once the crash-recovery check has finished", async () => {
    await setup();
    await settle();
    expect(offer()).toBeNull();

    await initRecovery();
    await settle();
    expect(offer()).not.toBeNull();
    expect(offer()?.textContent).toContain("Welcome to PowerVoice");
  });

  it("never appears over the crash-recovery dialog, and comes after it", async () => {
    await setup({ recoverable: true });
    await initRecovery();
    await settle();
    expect(offer()).toBeNull();

    decideLater();
    await settle();
    expect(offer()).not.toBeNull();
  });

  it("never appears while recording", async () => {
    await setup();
    applyRecordStateForTest({ recording: true });
    await initRecovery();
    await settle();
    expect(offer()).toBeNull();

    applyRecordStateForTest({ recording: false, finishing: true });
    await settle();
    expect(offer()).toBeNull();

    applyRecordStateForTest({ recording: false, finishing: false });
    await settle();
    expect(offer()).not.toBeNull();
  });

  it("isn't offered to someone who completed this version of the tour", async () => {
    await setup({ progress: [{ id: "welcome", version: TOURS.welcome.version, outcome: "completed" }] });
    await initRecovery();
    await settle();
    expect(offer()).toBeNull();
  });

  it("Don't show again saves the answer, and the next start doesn't offer it", async () => {
    await setup();
    await initRecovery();
    await settle();
    button("tour-offer-never").click();
    flushSync();
    expect(offer()).toBeNull();
    await settle();
    expect(saved).toHaveLength(1);
    expect(saved[0]!.tours.progress).toEqual([
      { id: "welcome", version: TOURS.welcome.version, outcome: "dismissed" },
    ]);

    armWelcomeOffer(saved[0]!.tours);
    await settle();
    expect(offer()).toBeNull();
    expect(tourState().offerArmed).toBe(false);
  });

  it("Later closes it without saving anything", async () => {
    await setup();
    await initRecovery();
    await settle();
    button("tour-offer-later").click();
    flushSync();
    await settle();
    expect(offer()).toBeNull();
    expect(saved).toHaveLength(0);
    expect(tourState().active).toBe(false);
  });

  it("Start tour runs the Welcome tour", async () => {
    await setup();
    await initRecovery();
    await settle();
    button("tour-offer-start").click();
    flushSync();
    expect(offer()).toBeNull();
    expect(tourState().tour?.id).toBe("welcome");
    expect(tourState().index).toBe(0);
  });
});
