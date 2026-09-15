/**
 * Which desktop platform the UI runs on (H-26), for the few places where the platforms' own
 * conventions differ — today the dialog button order (Windows puts the primary action first;
 * macOS and Linux put it last). Tests inject a platform with `setPlatformForTest`.
 *
 * `navigator.platform` is deprecated but it is still what the Tauri webviews report reliably
 * ("Win32", "MacIntel", "Linux x86_64"); the user agent is the fallback.
 */
export type Platform = "windows" | "mac" | "linux";

let override: Platform | null = null;

export function detectPlatform(
  platform: string = navigatorField("platform"),
  userAgent: string = navigatorField("userAgent"),
): Platform {
  if (/mac|iphone|ipad/i.test(platform)) {
    return "mac";
  }
  if (/^win/i.test(platform)) {
    return "windows";
  }
  if (platform === "") {
    if (/windows/i.test(userAgent)) {
      return "windows";
    }
    if (/macintosh|mac os x/i.test(userAgent)) {
      return "mac";
    }
  }
  return "linux";
}

function navigatorField(field: "platform" | "userAgent"): string {
  if (typeof navigator === "undefined") {
    return "";
  }
  return navigator[field] ?? "";
}

/** The platform in effect: the test override if set, otherwise the detected one. */
export function currentPlatform(): Platform {
  return override ?? detectPlatform();
}

/** Test hook: force a platform (`null` restores detection). */
export function setPlatformForTest(platform: Platform | null): void {
  override = platform;
}
