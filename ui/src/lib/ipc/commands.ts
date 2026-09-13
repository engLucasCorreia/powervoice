import { Channel, invoke } from "@tauri-apps/api/core";
import type {
  AppInfo,
  CommandName,
  DevicePrefsDto,
  DevicesDto,
  Settings,
  TransportStateDto,
} from "./bindings";

/**
 * Hand-written typed wrappers around `invoke` (ADR-003). Every command gets one wrapper here;
 * they use generated types (`./bindings`) only, never hand-copied shapes. The `satisfies
 * CommandName` on each command literal is what would catch a typo or a name Rust doesn't export.
 */

export async function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info" satisfies CommandName);
}

/** T-104: current settings, from the in-memory cache (never touches disk on the Rust side). */
export async function getSettings(): Promise<Settings> {
  return invoke<Settings>("settings_get" satisfies CommandName);
}

/** T-104: replaces the settings file (atomic write) and returns the canonical saved value. */
export async function setSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("settings_set" satisfies CommandName, { settings });
}

/** S1-01: the Audio Devices dialog's data (the engine's current device list). */
export async function devicesList(): Promise<DevicesDto> {
  return invoke<DevicesDto>("devices_list" satisfies CommandName);
}

/** S1-01: applies (and, when it worked, saves) a device selection. */
export async function devicesSelect(prefs: DevicePrefsDto): Promise<DevicesDto> {
  return invoke<DevicesDto>("devices_select" satisfies CommandName, { prefs });
}

/** S1-01: current transport state. */
export async function transportGet(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_get" satisfies CommandName);
}

export async function transportPlay(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_play" satisfies CommandName);
}

export async function transportPause(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_pause" satisfies CommandName);
}

export async function transportStop(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_stop" satisfies CommandName);
}

export async function transportPlayFromStart(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_play_from_start" satisfies CommandName);
}

export async function transportReturnToStart(): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_return_to_start" satisfies CommandName);
}

export async function transportSeek(positionSamples: number): Promise<TransportStateDto> {
  return invoke<TransportStateDto>("transport_seek" satisfies CommandName, { positionSamples });
}

/** S1-01: binary `VXTM` telemetry frames at 60 Hz (ADR-003 §2) on `channel`. */
export async function telemetrySubscribe(channel: Channel<ArrayBuffer>): Promise<void> {
  return invoke<void>("telemetry_subscribe" satisfies CommandName, { channel });
}

/** S1-01: the engine's app clock in ns (clock sync, ADR-003 §3). */
export async function clockNowNs(): Promise<number> {
  return invoke<number>("clock_now_ns" satisfies CommandName);
}
