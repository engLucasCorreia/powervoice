import { describe, expect, it } from "vitest";
import {
  canToggle,
  countPlugins,
  fileName,
  filterPlugins,
  formatLabel,
  isInInstallFolder,
  portsKey,
  rowKey,
  sortPlugins,
  statusInfo,
} from "./pluginList";
import { FLAGGED_ID, pluginEntry, pluginFixtures } from "../test/fixtures";

const names = (list: { name: string }[]) => list.map((e) => e.name);

describe("plugin list logic (T-809)", () => {
  it("words every status with a tone, an icon and its reason", () => {
    expect(statusInfo({ kind: "ok" })).toMatchObject({ tone: "success", labelKey: "plugins.status.ok", detailKey: null });
    expect(statusInfo({ kind: "disabled" })).toMatchObject({ tone: "neutral", labelKey: "plugins.status.disabled" });
    const causes = { crashed: "plugins.cause.crashed", timed_out: "plugins.cause.timed_out", manual: "plugins.cause.manual" } as const;
    for (const [cause, key] of Object.entries(causes)) {
      expect(
        statusInfo({ kind: "blocklisted", reason: "x", cause: cause as keyof typeof causes }),
      ).toMatchObject({ tone: "danger", icon: "blocked", labelKey: "plugins.status.blocklisted", detailKey: key });
    }
    expect(statusInfo({ kind: "flagged", crash_count: 1 })).toMatchObject({
      tone: "warning",
      labelKey: "plugins.status.flagged",
      detailKey: "plugins.crashed_once",
    });
    expect(statusInfo({ kind: "flagged", crash_count: 3 })).toMatchObject({
      detailKey: "plugins.crashed_times",
      detailParams: { count: 3 },
    });
    expect(
      statusInfo({ kind: "shadowed", by: "/home/u/.clap/acme-deesser.clap" }),
    ).toMatchObject({
      tone: "neutral",
      labelKey: "plugins.status.shadowed",
      detailKey: "plugins.shadowed_by",
      detailParams: { path: "acme-deesser.clap" },
    });
  });

  it("only treats a direct child of the install folder as installed (H-29)", () => {
    expect(isInInstallFolder("/home/u/.clap/acme.clap", "/home/u/.clap")).toBe(true);
    expect(isInInstallFolder("/home/u/.clap/acme.clap", "/home/u/.clap/")).toBe(true);
    expect(isInInstallFolder("C:\\Plugins\\CLAP\\acme.clap", "C:\\Plugins\\CLAP")).toBe(true);
    expect(isInInstallFolder("/usr/lib/clap/acme.clap", "/home/u/.clap")).toBe(false);
    // A look-alike sibling folder is not a prefix match.
    expect(isInInstallFolder("/home/u/.clap-extra/acme.clap", "/home/u/.clap")).toBe(false);
    // A nested subfolder doesn't count either — installs are never nested.
    expect(isInInstallFolder("/home/u/.clap/nested/acme.clap", "/home/u/.clap")).toBe(false);
    expect(isInInstallFolder("/home/u/.clap/acme.clap", null)).toBe(false);
  });

  it("spells every backend's format badge and upper-cases unknown ones", () => {
    expect(["clap", "vst3", "lv2", "jsfx", "CLAP", "au"].map(formatLabel)).toEqual(["CLAP", "VST3", "LV2", "JSFX", "CLAP", "AU"]);
  });

  it("summarises ports: mono, stereo, in/out, or unknown", () => {
    expect(portsKey(null)).toBeNull();
    expect(portsKey({ input_channels: 1, output_channels: 1 })?.key).toBe("plugins.ports.mono");
    expect(portsKey({ input_channels: 2, output_channels: 2 })?.key).toBe("plugins.ports.stereo");
    expect(portsKey({ input_channels: 1, output_channels: 2 })).toEqual({
      key: "plugins.ports.in_out",
      params: { input: 1, output: 2 },
    });
  });

  it("searches every field, all terms must match, case-insensitively", () => {
    const list = pluginFixtures();
    expect(names(filterPlugins(list, "acme"))).toEqual(["De-esser", "Hum Remover"]);
    expect(names(filterPlugins(list, "VST3"))).toEqual(["Tape Saturator"]);
    expect(names(filterPlugins(list, "downloads"))).toEqual(["glitchy-comp"]);
    expect(names(filterPlugins(list, "acme hum"))).toEqual(["Hum Remover"]);
    expect(filterPlugins(list, "   ")).toHaveLength(list.length);
    expect(filterPlugins(list, "nothing-like-this")).toEqual([]);
    // Status words come from the caller (localized).
    const words = (e: (typeof list)[number]) => e.status.kind;
    expect(names(filterPlugins(list, "flagged", words))).toEqual(["Breath Control"]);
  });

  it("sorts by name, vendor, format or status, both ways, ties by name", () => {
    const list = pluginFixtures();
    expect(names(sortPlugins(list, "name", "asc"))).toEqual([
      "Breath Control",
      "De-esser",
      "glitchy-comp",
      "Hum Remover",
      "Loudness Rider",
      "slow-limiter",
      "Small Room",
      "Tape Saturator",
    ]);
    expect(names(sortPlugins(list, "name", "desc"))[0]).toBe("Tape Saturator");
    // Empty vendors first, then alphabetical; ties broken by name.
    expect(names(sortPlugins(list, "vendor", "asc")).slice(0, 4)).toEqual([
      "glitchy-comp",
      "slow-limiter",
      "De-esser",
      "Hum Remover",
    ]);
    expect(sortPlugins(list, "format", "asc").map((e) => e.format)).toEqual([
      "clap",
      "clap",
      "clap",
      "clap",
      "clap",
      "jsfx",
      "lv2",
      "vst3",
    ]);
    // Problems first: blocklisted, flagged, disabled, then OK.
    expect(sortPlugins(list, "status", "asc").map((e) => e.status.kind)).toEqual([
      "blocklisted",
      "blocklisted",
      "blocklisted",
      "flagged",
      "disabled",
      "ok",
      "ok",
      "ok",
    ]);
    expect(sortPlugins(list, "status", "desc")[0]!.status.kind).toBe("ok");
    expect(list[0]!.name).toBe("De-esser"); // the input isn't mutated
  });

  it("counts statuses and knows which rows can be switched", () => {
    const list = pluginFixtures();
    expect(countPlugins(list)).toEqual({
      total: 8,
      disabled: 1,
      blocklisted: 3,
      flagged: 1,
      shadowed: 0,
    });
    const byName = (n: string) => list.find((e) => e.name === n)!;
    expect(canToggle(byName("De-esser"))).toBe(true);
    expect(canToggle(byName("Tape Saturator"))).toBe(true);
    expect(canToggle(byName("Breath Control"))).toBe(true);
    expect(canToggle(byName("Hum Remover"))).toBe(false);
    expect(canToggle(byName("glitchy-comp"))).toBe(false);
  });

  it("keys rows by module id, or by path when there is none", () => {
    expect(rowKey(pluginEntry({ id: FLAGGED_ID }))).toBe(FLAGGED_ID);
    expect(rowKey(pluginEntry({ id: "", path: "/a/b.clap" }))).toBe("path:/a/b.clap");
    expect(fileName("/a/b/c.clap")).toBe("c.clap");
    expect(fileName("C:\\Plugins\\x.clap")).toBe("x.clap");
  });
});
