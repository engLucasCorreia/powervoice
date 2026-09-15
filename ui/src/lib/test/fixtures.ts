/**
 * Shared test factories for generated DTOs (see `ui/src/lib/ipc/bindings.ts`).
 *
 * Every factory returns a complete, valid object that `satisfies` its generated type; callers
 * pass only the fields that matter for a given test as `overrides`. This is the ONE place
 * hand-written DTO literals live for tests (and the dev preview) — before H-18, adding a field to
 * a shared DTO meant editing 10+ test files by hand, which caused repeated cross-branch merge
 * conflicts (see MEMORY.md H-10/T-301).
 *
 * new DTO field → update this file only
 *
 * (e.g. T-709's `Settings.tours` was added to `settingsFixture`'s returned object here and
 * nowhere else.)
 */
import type {
  DevicePrefsDto,
  DocumentDto,
  DocumentProbeDto,
  HistoryStateDto,
  ParamInfoDto,
  PluginEntryDto,
  PluginFoldersDto,
  RackSlotDto,
  RackStateDto,
  RecordStateDto,
  Settings,
  TransportStateDto,
} from "../ipc/bindings";

/** `Settings.device` default: PipeWire host, no devices selected (SPEC-001 §3 factory default). */
function devicePrefsDto(overrides: Partial<DevicePrefsDto> = {}): DevicePrefsDto {
  return {
    host: "pipewire",
    input_device: null,
    input_channel: 1,
    output_device: null,
    sample_rate_hz: null,
    buffer_size_frames: null,
    ...overrides,
  };
}

/** Full `Settings` (`settings_get`/`settings_set`), factory defaults throughout. */
export function settingsFixture(overrides: Partial<Settings> = {}): Settings {
  return {
    version: 1,
    device: devicePrefsDto(overrides.device),
    default_format: { sample_rate_hz: 48_000, bit_depth: "24" },
    monitor_mode: "off",
    monitor_hint_shown: false,
    telemetry_rate_hz: 60,
    memory_budget_mib: 2048,
    normalize_dialog: { value: -1, unit: "db" },
    recent_files: [],
    spectral_defaults: {
      freq_scale: "log",
      colormap: "inferno",
      display_floor_db: -120,
      display_ceil_db: 0,
      fft_size: null,
    },
    analyzer_visible: true,
    analyzer_response: "medium",
    analyzer_peak_hold: true,
    multichannel_policy: "ask",
    renderer_preference: "auto",
    layout: {
      markers_width_px: 240,
      rack_width_px: 280,
      dock_height_px: 240,
      markers_collapsed: false,
      rack_collapsed: false,
      dock_tab: "meters",
    },
    record: {
      mode: "insert",
      punch_on_selection: true,
      preroll_s: 5,
      postroll_s: 1,
      preroll_at_cursor: false,
      hear_original: false,
      punch_xfade_ms: 10,
    },
    record_offsets: [],
    save_dither: "tpdf",
    theme: "dark",
    playhead_follow: true,
    plugins: { custom_folders: [], disabled: [] },
    tours: { progress: [] },
    snap_to_zero_crossing: false,
    analyzer_diagnostics: {
      peak_labels: true,
      panel_visible: false,
      inspector_fft_size: 16_384,
      inspector_window: "hann",
      inspector_smoothing: "none",
      inspector_scale: "log",
      inspector_response: "medium",
    },
    ...overrides,
  };
}

/** A stereo `document_probe` result (WAV/PCM, identical L/R not assumed). */
export function documentProbeDto(overrides: Partial<DocumentProbeDto> = {}): DocumentProbeDto {
  return {
    container: "wav",
    codec: "pcm",
    sample_rate_hz: 48_000,
    channels: [
      { label: "Left", is_lfe: false },
      { label: "Right", is_lfe: false },
    ],
    len_samples: 48_000,
    channel_peaks_dbfs: [-6, -6],
    identical_channels: false,
    suggested_channel: null,
    ...overrides,
  };
}

/** A clean, saved `DocumentDto` ("take.wav", 10 s @ 48 kHz mono). */
export function docDto(overrides: Partial<DocumentDto> = {}): DocumentDto {
  return {
    name: "take.wav",
    path: "/home/user/take.wav",
    sample_rate_hz: 48_000,
    len_samples: 480_000,
    dirty: false,
    audio_rev: 1,
    sidecar_dirty: false,
    spectral_view: null,
    waveform_view: null,
    recovered: false,
    ...overrides,
  };
}

/** Idle record panel state: input configured, nothing armed/recording. */
export function recordStateDto(overrides: Partial<RecordStateDto> = {}): RecordStateDto {
  return {
    input_device: "Mic",
    input_channel: 1,
    input_status: "healthy",
    armed: false,
    input_open: false,
    input_rate_hz: 48_000,
    recording: false,
    finishing: false,
    monitor: "off",
    monitoring: false,
    monitor_latency_us: null,
    monitor_dropouts: 0,
    dropout_count: 0,
    disk_remaining_s: null,
    ...overrides,
  };
}

/** Stopped transport, nothing loaded. */
export function transportStateDto(overrides: Partial<TransportStateDto> = {}): TransportStateDto {
  return {
    playing: false,
    playhead_samples: 0,
    play_start_samples: 0,
    doc_len_samples: 0,
    doc_rate_hz: 48_000,
    can_play: false,
    loop_enabled: false,
    loop_range: null,
    ...overrides,
  };
}

/** Nothing to undo/redo, no pending labels. */
export function historyStateDto(overrides: Partial<HistoryStateDto> = {}): HistoryStateDto {
  return {
    can_undo: false,
    can_redo: false,
    undo_label: null,
    redo_label: null,
    undo_label_params: {},
    redo_label_params: {},
    ...overrides,
  };
}

/** A single automatable Gain (dB) param — the default `params`/`values` a `rackSlotDto` needs. */
export function paramInfoDto(overrides: Partial<ParamInfoDto> = {}): ParamInfoDto {
  return {
    id: 0,
    key: "gain_db",
    name: { text: "Gain", key: null },
    group: null,
    unit: { kind: "db" },
    min: -60,
    max: 12,
    default: 0,
    taper: { kind: "linear" },
    step: null,
    enum_labels: [],
    decimals: 1,
    smoothing_ms: 20,
    flags: {
      automatable: true,
      stepped: false,
      boolean: false,
      read_only: false,
      hidden: false,
      bypass: false,
    },
    ...overrides,
  };
}

/**
 * A single active, non-sandboxed `RackSlotDto` (org.powervoice.gain). `values` defaults from
 * `params` (index-aligned, SPEC-012 §2.2) unless overridden explicitly.
 */
export function rackSlotDto(overrides: Partial<RackSlotDto> = {}): RackSlotDto {
  const params = overrides.params ?? [paramInfoDto()];
  return {
    uid: 1,
    module: "org.powervoice.gain@1.0.0",
    module_id: "org.powervoice.gain",
    name: "Gain",
    bypass: false,
    latency_samples: 0,
    status: { kind: "active" },
    params,
    groups: [],
    values: params.map((p) => ({ id: p.id, value: p.default, normalized: 0.5, text: String(p.default) })),
    noise_profile: null,
    curve_handles: null,
    telemetry: [],
    sandboxed: false,
    has_editor: false,
    editor_open: false,
    ...overrides,
  };
}

/** The rack panel's whole state: `slots`, no A/B, no added latency. */
export function rackStateDto(slots: RackSlotDto[] = [], ab = false, latency_samples = 0): RackStateDto {
  return { slots, ab, latency_samples };
}

/** A registered, healthy CLAP effect (T-809/H-29's plugin manager row). */
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

/** `pluginFixtures()`'s flagged row's id (the rack's flagged-slot affordance points at it). */
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

/** The plugin manager's Folders tab / Preferences → Plugins (T-809). */
export function folderFixture(overrides: Partial<PluginFoldersDto> = {}): PluginFoldersDto {
  return {
    install: "/home/u/.clap",
    install_folders: ["/home/u/.clap", "/home/u/.vst3"],
    standard: ["/home/u/.clap", "/usr/lib/clap", "/home/u/.vst3", "/usr/lib/vst3"],
    custom: ["/media/plugins"],
    modules: "/home/u/.local/share/app.powervoice.editor/modules",
    ...overrides,
  };
}
