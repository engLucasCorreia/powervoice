/**
 * Plugin manager test fixtures (T-809). Not imported by production code. One entry per status
 * (and per format badge), shared by the list, store, dialog and preference tests.
 */
import type { PluginEntryDto, PluginFoldersDto } from "../ipc/bindings";

export function pluginEntry(overrides: Partial<PluginEntryDto> = {}): PluginEntryDto {
  return {
    id: "clap:com.acme.deesser",
    name: "De-esser",
    vendor: "Acme Audio",
    version: "2.1.0",
    format: "clap",
    path: "/home/u/.clap/acme-deesser.clap",
    status: { kind: "ok" },
    ports: { input_channels: 1, output_channels: 1 },
    param_count: 12,
    ...overrides,
  };
}

export const FLAGGED_ID = "clap:com.vocalift.breath-control";

/** Every status: ok ×3, flagged, disabled, blocklisted (manual with a name; crashed and timed
 * out without an id). Formats: CLAP, VST3, LV2, JSFX. */
export function pluginFixtures(): PluginEntryDto[] {
  return [
    pluginEntry(),
    pluginEntry({
      id: FLAGGED_ID,
      name: "Breath Control",
      vendor: "Vocalift",
      version: "1.4.2",
      path: "/home/u/.clap/breath-control.clap",
      status: { kind: "flagged", crash_count: 3 },
      ports: { input_channels: 2, output_channels: 2 },
      param_count: 8,
    }),
    pluginEntry({
      id: "vst3:northwind.tape",
      name: "Tape Saturator",
      vendor: "Northwind DSP",
      version: "3.0.1",
      format: "vst3",
      path: "/usr/lib/vst3/Tape Saturator.vst3",
      status: { kind: "disabled" },
      ports: { input_channels: 2, output_channels: 2 },
      param_count: 24,
    }),
    pluginEntry({
      id: "lv2:urn:studio:room",
      name: "Small Room",
      vendor: "Studio Tools",
      version: "1.0.0",
      format: "lv2",
      path: "/usr/lib/lv2/small-room.lv2",
      ports: { input_channels: 1, output_channels: 2 },
      param_count: 9,
    }),
    pluginEntry({
      id: "jsfx:loudness-rider",
      name: "Loudness Rider",
      vendor: "JSFX Community",
      version: "1.2.0",
      format: "jsfx",
      path: "/home/u/jsfx/loudness-rider.jsfx",
      ports: null,
      param_count: 0,
    }),
    pluginEntry({
      id: "clap:com.acme.hum",
      name: "Hum Remover",
      vendor: "Acme Audio",
      version: "1.0.3",
      path: "/media/plugins/hum-remover.clap",
      status: { kind: "blocklisted", reason: "blocked by the user", cause: "manual" },
    }),
    pluginEntry({
      id: "",
      name: "glitchy-comp",
      vendor: "",
      version: "",
      path: "/home/u/Downloads/glitchy-comp.clap",
      status: { kind: "blocklisted", reason: "crashed while being scanned", cause: "crashed" },
      ports: null,
      param_count: 0,
    }),
    pluginEntry({
      id: "",
      name: "slow-limiter",
      vendor: "",
      version: "",
      path: "/media/plugins/slow-limiter.clap",
      status: { kind: "blocklisted", reason: "timed out while being scanned", cause: "timed_out" },
      ports: null,
      param_count: 0,
    }),
  ];
}

export function folderFixture(): PluginFoldersDto {
  return {
    install: "/home/u/.clap",
    standard: ["/home/u/.clap", "/usr/lib/clap"],
    custom: ["/media/plugins"],
  };
}

/** Lets pending IPC promises and effects settle. */
export async function settle(rounds = 6): Promise<void> {
  const { flushSync } = await import("svelte");
  for (let i = 0; i < rounds; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  flushSync();
}
