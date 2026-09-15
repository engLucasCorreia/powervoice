import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { flushSync, mount, unmount } from "svelte";
import { afterEach, describe, expect, it } from "vitest";
import type { DocumentProbeDto } from "../ipc/bindings";
import { clearNotices } from "../state/notices.svelte";
import { docDto, documentProbeDto as stereoProbe } from "../test/fixtures";
import ChannelChoiceDialog from "./ChannelChoiceDialog.svelte";
import { documentState, openDocument, resetDocumentStateForTest } from "./document.svelte";

afterEach(() => {
  clearMocks();
  clearNotices();
  resetDocumentStateForTest();
});

/** Drives `document.svelte.ts`'s `openDocument`, which is what actually reacts to
 * `dialog.channel_choice` and opens the prompt this dialog renders — a raw `documentOpen` IPC
 * call wouldn't touch the store's prompt state at all. Returns once the prompt has appeared. */
async function openPromptFor(probe: DocumentProbeDto): Promise<{ wait: Promise<boolean> }> {
  mockIPC((cmd, args) => {
    if (cmd === "document_open") {
      const channelChoice = (args as { channelChoice: unknown }).channelChoice;
      if (!channelChoice) {
        throw {
          code: "needs_confirmation",
          key: "dialog.channel_choice",
          params: { probe: JSON.stringify(probe) },
        };
      }
      return docDto({ name: "stereo.wav", path: "/home/user/stereo.wav", len_samples: 48_000 });
    }
    throw new Error(`unmocked command: ${cmd}`);
  });
  const wait = openDocument("/home/user/stereo.wav");
  await new Promise((r) => setTimeout(r, 0));
  return { wait };
}

describe("ChannelChoiceDialog (SPEC-005 §2.4)", () => {
  it("is hidden with no pending prompt", () => {
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ChannelChoiceDialog, { target });
    flushSync();
    expect(target.querySelector('[data-testid="channel-choice-dialog"]')).toBeNull();
    unmount(app);
    target.remove();
  });

  it("preselects the suggested channel and shows the silent-channel hint", async () => {
    await openPromptFor(
      stereoProbe({ suggested_channel: 0, channel_peaks_dbfs: [-20, -80] }),
    );
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ChannelChoiceDialog, { target });
    flushSync();

    expect(target.querySelector('[data-testid="channel-choice-dialog"]')).not.toBeNull();
    const channelRadio = target.querySelector<HTMLInputElement>(
      'input[name="channel-choice-mode"][value="channel"]',
    )!;
    expect(channelRadio.checked).toBe(true);
    const select = target.querySelector<HTMLSelectElement>('[data-testid="channel-choice-select"]')!;
    expect(select.value).toBe("0");
    const hint = target.querySelector('[data-testid="channel-choice-silent-hint"]');
    expect(hint?.textContent).toContain("Right");
    expect(hint?.textContent).toContain("Left");

    unmount(app);
    target.remove();
  });

  it("defaults to Mix to mono with no silent-channel hint when nothing is suggested", async () => {
    await openPromptFor(stereoProbe());
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ChannelChoiceDialog, { target });
    flushSync();

    const averageRadio = target.querySelector<HTMLInputElement>(
      'input[name="channel-choice-mode"][value="average"]',
    )!;
    expect(averageRadio.checked).toBe(true);
    expect(target.querySelector('[data-testid="channel-choice-silent-hint"]')).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Open sends the picked channel; Cancel resolves null and closes", async () => {
    const { wait } = await openPromptFor(stereoProbe({ suggested_channel: 0 }));
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ChannelChoiceDialog, { target });
    flushSync();

    const select = target.querySelector<HTMLSelectElement>('[data-testid="channel-choice-select"]')!;
    select.value = "1";
    select.dispatchEvent(new Event("change", { bubbles: true }));
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="channel-choice-open"]')!.click();
    await wait;
    flushSync();
    expect(documentState().channelChoicePrompt).toBeNull();

    unmount(app);
    target.remove();
  });

  it("Cancel closes the dialog without sending a second document_open", async () => {
    const { wait } = await openPromptFor(stereoProbe());
    const target = document.createElement("div");
    document.body.appendChild(target);
    const app = mount(ChannelChoiceDialog, { target });
    flushSync();

    target.querySelector<HTMLButtonElement>('[data-testid="channel-choice-cancel"]')!.click();
    await wait;
    flushSync();
    expect(target.querySelector('[data-testid="channel-choice-dialog"]')).toBeNull();

    unmount(app);
    target.remove();
  });
});
