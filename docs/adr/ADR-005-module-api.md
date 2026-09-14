# ADR-005 — Module API
- Status: accepted (owner, M0 checkpoint 2026-09-12)
- Date: 2026-09-12
- Deciders: owner, orchestrator

## Context

Every rack slot holds a processor behind one contract (PROMPT §2 "Module system", §3.4, §3.7). The
built-in modules (Gain, Noise Gate, Noise Reduction, Parametric EQ, Dynamics, True-Peak Limiter)
are compiled in for v1. In M8 the same contract has to carry external plugins (CLAP, VST3, LV2,
JSFX, VST2) through format adapters that run out of process, and separately installed modules
packaged as CLAP bundles (ADR-006). The rack, latency compensation, presets, sidecar and the
**generic parameter UI** only ever see this API.

So the API has to be:
- **Real-time safe**: `process()` never allocates, locks, blocks, logs or panics on valid input.
- **Sample-accurate and deterministic**: offline render (export, bake, ACX check, CLI) must match
  playback, and golden tests must be bit-reproducible.
- **Plugin-shaped**: it maps onto CLAP (the model we follow most closely), VST3, LV2 and JSFX
  without a rewrite. We checked CLAP 1.2.x `plugin.h`, `ext/params.h`, `ext/state.h`,
  `ext/latency.h`, `ext/tail.h` and `events.h`. Parameters use **plain** (non-normalized) double
  values. Events come as a time-sorted per-block list with sample offsets. `activate(sample_rate,
  min, max_frames)` runs on the main thread. Latency may only change inside `activate`, and an
  active plugin must `request_restart()`. Tail ≥ `INT32_MAX` means infinite.
- **Implementable by T-005 without re-deciding anything.**

Naming note: PROMPT §3.7 sketches `prepare(sample_rate, max_block)` and `tail_samples()`. This ADR
replaces them with `activate(&ActivateConfig)` and `tail() -> Tail` (same concepts, CLAP
vocabulary).

## Decision

### 1. Crate and dependencies
`crates/module-api` depends on nothing in the workspace. External dependencies: `serde` (+ derive),
for descriptor/schema/state serialization, and `thiserror`, for error types (the workspace standard
for libraries; no runtime cost). Feature `test-util` adds `ModuleTestHost` (T-005) and pulls
`assert_no_alloc` as an optional dependency. No `ts-rs` here: per ADR-003, `src-tauri` mirrors the
serializable types below (`ParamInfo`, `ParamGroup`, `TelemetryInfo`, …) as DTOs and generates the
TS types. Smoothers, filters and other DSP helpers live in `dsp`, not here.

All sketches below are normative for names, fields and semantics. T-005 may add private fields,
helper methods and trait impls (`Debug`, `Clone`, `PartialEq`, ...) and must report any other
deviation.

### 2. Identity: descriptor, reference, factory

```rust
pub const MODULE_API_VERSION: u32 = 1;

/// Semantic version. Display/FromStr: "MAJOR.MINOR.PATCH" (no pre-release/build tags in v1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version { pub major: u32, pub minor: u32, pub patch: u32 }   // serde: "1.2.0"

/// "id@version", e.g. "org.powervoice.gain@1.0.0". What presets and the sidecar store.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleRef { pub id: String, pub version: Version }          // serde: the string form

/// UI text: English fallback + optional i18n key (built-ins must set keys; adapters give text only).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalizedText { pub text: String, pub key: Option<String> }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    /// Built-ins and packaged modules (ADR-006): reverse-DNS, `[a-z0-9.-]+`, e.g. "org.powervoice.gain".
    /// External plugins (adapters): "<format>:<native id>" — "clap:com.u-he.diva",
    /// "vst3:<32 hex cid>", "lv2:<uri>", "jsfx:<path relative to the effects root>", "vst2:<id>".
    pub id: String,
    pub version: Version,          // adapters parse a leading "x.y.z", missing parts = 0, else 0.0.0
    pub name: LocalizedText,
    pub vendor: String,
    pub description: LocalizedText,
    pub url: Option<String>,
    /// CLAP feature strings (see `features`), e.g. ["audio-effect", "compressor", "mono"].
    pub features: Vec<String>,
    /// The `ModuleState::format_version` this build writes.
    pub state_format_version: u32,
    /// MODULE_API_VERSION the module was built against.
    pub api_version: u32,
}

pub mod features {
    pub const AUDIO_EFFECT: &str = "audio-effect";   pub const ANALYZER: &str = "analyzer";
    pub const EQUALIZER: &str = "equalizer";         pub const FILTER: &str = "filter";
    pub const COMPRESSOR: &str = "compressor";       pub const EXPANDER: &str = "expander";
    pub const GATE: &str = "gate";                   pub const LIMITER: &str = "limiter";
    pub const RESTORATION: &str = "restoration";     pub const UTILITY: &str = "utility";
    pub const MASTERING: &str = "mastering";         pub const MONO: &str = "mono";
}

pub trait ModuleFactory: Send + Sync {
    fn descriptor(&self) -> &ModuleDescriptor;
    /// [control thread] A new, inactive instance with default parameter values.
    fn create(&self) -> Result<Box<dyn Module>, ModuleError>;
    /// Read-only factory presets.
    fn presets(&self) -> Vec<ModulePreset> { Vec::new() }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModulePreset { pub key: String, pub name: LocalizedText, pub state: ModuleState }
```

Built-in ids: `org.powervoice.gain`, `org.powervoice.noise-gate`, `org.powervoice.noise-reduction`,
`org.powervoice.parametric-eq`, `org.powervoice.dynamics`, `org.powervoice.true-peak-limiter`. Ids are
permanent; renaming the product does not change them (see Open questions). The module registry
(`id → Arc<dyn ModuleFactory>`) lives in `rack`. Only one version per id is installed at a time.

**Resolving a `ModuleRef` from a sidecar or preset:** look up by `id` (the version is informational).
- No such id → **placeholder slot**: audio passes through dry, latency 0, the UI shows "Missing module
  `id@version`", and the stored ref and state are written back verbatim on save, so nothing is lost.
- `StateError::TooNew` while loading (§10) → the same placeholder, with the message "requires a newer
  version".
- Otherwise load. The next save writes the installed version.

### 3. Parameters

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParamId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub u32);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParamInfo {
    /// Stable forever. Never reused after a parameter is removed. Used on the RT path.
    pub id: ParamId,
    /// Stable string key `^[a-z][a-z0-9_]*$`, unique per module, e.g. "threshold_db".
    /// The sidecar and presets store keys. Renaming a key requires a state migration (§10).
    pub key: String,
    pub name: LocalizedText,
    pub group: Option<GroupId>,
    pub unit: Unit,
    /// Plain values (what the DSP means), finite, min < max, min <= default <= max.
    pub min: f64,
    pub max: f64,
    pub default: f64,
    pub taper: Taper,
    /// Some(s): legal values are min + k*s (implies STEPPED).
    pub step: Option<f64>,
    /// Non-empty => enum: min = 0, max = len-1, step = 1, STEPPED; unit None.
    pub enum_labels: Vec<LocalizedText>,
    /// Decimals shown by value_to_text in the displayed unit (e.g. 1 => "-6.0 dB").
    pub decimals: u8,
    /// Declared smoothing time the module applies to changes of this parameter
    /// (0 = takes effect exactly at the event offset). Informative for the UI and tests;
    /// the module implements it (the module owns smoothing).
    pub smoothing_ms: f32,
    pub flags: ParamFlags,
}

/// Bit set (hand-written, no `bitflags` dependency). Serialized as u32.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ParamFlags(u32);
impl ParamFlags {
    pub const NONE: Self        = Self(0);
    /// Host may change it during processing (all user-editable params of built-ins).
    pub const AUTOMATABLE: Self = Self(1 << 0);
    /// Only discrete values (step / enum / bool).
    pub const STEPPED: Self     = Self(1 << 1);
    /// On/off: min 0, max 1, step 1. Implies STEPPED.
    pub const BOOL: Self        = Self(1 << 2);
    /// Output: the module reports it via out_events; the host never sends events for it.
    /// Not AUTOMATABLE, not saved in state.
    pub const READ_ONLY: Self   = Self(1 << 3);
    /// Not shown by the generic UI (still saved in state).
    pub const HIDDEN: Self      = Self(1 << 4);
    /// The module's own bypass switch (BOOL, 1.0 = bypassed). See §9.
    pub const BYPASS: Self      = Self(1 << 5);
    pub const fn contains(self, other: Self) -> bool { self.0 & other.0 == other.0 }
    pub const fn union(self, other: Self) -> Self { Self(self.0 | other.0) }
    pub const fn bits(self) -> u32 { self.0 }
}
// impl BitOr / BitOrAssign for ParamFlags

/// Mapping plain value v <-> normalized control position t in [0, 1], plus text semantics.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[non_exhaustive]
pub enum Taper {
    /// t = (v - min) / (max - min)
    Linear,
    /// t = ln(v / min) / ln(max / min). Requires 0 < min < max. Frequencies, times, Q, ratio.
    Log,
    /// Plain value in dB (unit Db/Dbfs/Dbtp required), t linear in dB.
    /// neg_inf_at_min: the value `min` means -inf dB (linear gain 0): value_to_text shows
    /// "-inf dB", text_to_value accepts "-inf"/"−∞"; the module must treat it as silence.
    Db { neg_inf_at_min: bool },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Unit { None, Db, Dbfs, Dbtp, Lufs, Hz, Ms, Seconds, Percent, Ratio, Samples,
                /// Adapter-provided unit label, display only.
                Custom(String) }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParamGroup {
    pub id: GroupId,
    pub key: String,
    pub name: LocalizedText,
    pub parent: Option<GroupId>,
    /// BOOL param rendered as the group's header toggle (Dynamics sections, EQ bands).
    pub enable_param: Option<ParamId>,
    pub collapsed_by_default: bool,
}

impl ParamInfo {
    pub fn validate(&self) -> Result<(), SchemaError>;          // invariants below
    pub fn clamp_quantize(&self, v: f64) -> f64;                // clamp to range, snap to step
    pub fn to_normalized(&self, v: f64) -> f64;                 // per Taper, clamped
    pub fn from_normalized(&self, t: f64) -> f64;               // inverse, then clamp_quantize
    pub fn value_to_text(&self, v: f64) -> String;              // rules below
    pub fn text_to_value(&self, s: &str) -> Option<f64>;        // None on garbage; result clamp_quantized
}
/// Checks id/key uniqueness, group references, enable_param is BOOL, and each ParamInfo::validate.
pub fn validate_schema(params: &[ParamInfo], groups: &[ParamGroup]) -> Result<(), SchemaError>;
```

**Invariants** (checked by `validate`, enforced by `ModuleTestHost`):
- Values finite, `min < max`, default in range.
- `Log` ⇒ `min > 0`. `Db` ⇒ unit ∈ {Db, Dbfs, Dbtp}, and `Linear` with a dB unit is rejected, so there
  is exactly one way to declare a dB parameter.
- `step` ⇒ STEPPED. `BOOL` ⇒ `0..1`, step 1. Enum labels ⇒ `0..len-1`, step 1.
- `BYPASS` ⇒ `BOOL`. `READ_ONLY` ⇒ not `AUTOMATABLE`.

**Text rules** (`value_to_text` / `text_to_value`; locale-neutral: `.` decimal separator, and `−`
(U+2212) is accepted as minus):

| Kind | Display | Parse accepts |
|---|---|---|
| Db / Dbfs / Dbtp / Lufs | `-6.0 dB`, `-1.0 dBTP`, `-inf dB` | `-6`, `-6 dB`, `−6db`, `-inf` (if `neg_inf_at_min`) |
| Hz | `< 1000`: `250 Hz`; `≥ 1000`: `1.25 kHz` (2 decimals) | `1250`, `1250 Hz`, `1.25k`, `1.25 kHz` |
| Ms | `< 1000`: `12.0 ms`; `≥ 1000`: `1.20 s` | `12`, `12 ms`, `1.2 s` |
| Seconds / Samples / Percent | `1.50 s`, `480 smp`, `50 %` | number with or without the unit |
| Ratio | `4.0:1` | `4`, `4:1` |
| Bool | `On` / `Off` (i18n keys `param.on`/`param.off`) | on/off/true/false/1/0 |
| Enum | the label | label (case-insensitive, trimmed) or integer index |
| None / Custom | number (`decimals`) | number |

Round-trip contract (T-005 test): for any in-range v, `text_to_value(value_to_text(v))` equals
`clamp_quantize(v)` within half a displayed unit of the last shown decimal. For stepped, enum and
bool parameters it is exact.

Built-in modules never need custom formatting. External plugins supply their own text through the
reserved `ParamText` extension (§11).

### 4. Parameter events (per block, sample-accurate)

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamEvent {
    /// Offset within the block: 0 <= offset < frames; all offsets are 0 when frames == 0.
    pub offset: u32,
    pub id: ParamId,
    /// New plain value, already clamp_quantize'd by the host.
    pub value: f64,
}

pub const DEFAULT_EVENT_CAPACITY: usize = 512;

/// Host-side builder. Fixed capacity (allocated off the audio thread); never reallocates.
pub struct EventList { /* Vec<ParamEvent> with fixed capacity */ }
impl EventList {
    pub fn with_capacity(capacity: usize) -> Self;
    pub fn clear(&mut self);
    /// Err(Full) at capacity; Err(OutOfOrder) if offset < last pushed offset.
    pub fn push(&mut self, e: ParamEvent) -> Result<(), EventListError>;
    pub fn as_slice(&self) -> &[ParamEvent];
    /// Copies events with offset in [start, start+len) into `out`, rebased to 0 (block splitting).
    pub fn split_into(&self, start: u32, len: u32, out: &mut EventList) -> Result<(), EventListError>;
}

/// Module -> host parameter reports (READ_ONLY params; adapter-originated changes). Fixed capacity.
pub struct OutputEvents { /* same storage */ }
impl OutputEvents { pub fn try_push(&mut self, e: ParamEvent) -> Result<(), ParamEvent>; }

/// Splits 0..frames at event offsets. Each segment first applies `events` (all events whose
/// offset == start), then renders `len` samples. For frames == 0: one segment (0, 0, all events).
pub struct Segment<'a> { pub start: u32, pub len: u32, pub events: &'a [ParamEvent] }
pub fn segments(frames: u32, events: &[ParamEvent]) -> impl Iterator<Item = Segment<'_>>;
```

**Host guarantees:**
- Events are sorted by `offset` (non-decreasing). For the same `(offset, id)`, the last one wins.
- Values are clamped and quantized. No events are sent for READ_ONLY params.
- Count ≤ capacity. If a list is full, the rack carries the rest over to offset 0 of the next block.
  Values are delayed, never lost.

UI edits have no sub-block timing and arrive at offset 0. Real offsets come from block splitting,
offline renders and future automation.

**Module duty:** a change at offset k starts affecting output at sample k (then ramps over
`smoothing_ms`). A block with `frames == 0` is a legal "parameter flush" and must apply its events.
Smoothing is the module's job, because only the module knows the right domain (dB vs linear gain,
coefficient interpolation vs parameter ramp).

### 5. Lifecycle, modes, context

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessMode { Realtime, Offline }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelLayout { pub inputs: u16, pub outputs: u16 }
impl ChannelLayout {
    pub const MONO: Self = Self { inputs: 1, outputs: 1 };
    pub const STEREO: Self = Self { inputs: 2, outputs: 2 };
    pub const MONO_TO_STEREO: Self = Self { inputs: 1, outputs: 2 };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActivateConfig {
    pub sample_rate: f64,
    /// >= 1. The host never passes more frames per process() call.
    pub max_block: u32,
    pub mode: ProcessMode,
    /// One of supported_layouts(). Always MONO in v1, possibly through the host shim (§8).
    pub layout: ChannelLayout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tail {
    /// Output decays to exactly zero or below -120 dBFS within n samples after the input
    /// becomes silent. Samples(0) = no tail.
    Samples(u64),
    /// Host caps the render (policy in the bake/export spec, T-602).
    Infinite,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Transport {
    /// True while playing or recording; false for monitoring-only processing.
    pub playing: bool,
    /// Document position of the block's first input sample, when the input is document audio.
    pub position_samples: Option<u64>,
}

#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostRequest {
    /// Latency or tail must change, or an activate-time resource must be rebuilt
    /// (e.g. a new FFT size). The host performs a replacement (§12).
    Restart,
}

pub struct ProcessContext<'a> {
    /// 0 <= frames <= max_block. Every input/output slice has exactly this length.
    pub frames: u32,
    /// Samples processed since activate (CLAP steady_time). Not reset by reset().
    pub steady_time: u64,
    pub transport: Transport,
    pub events: &'a [ParamEvent],
    pub out_events: &'a mut OutputEvents,
    requests: u32, // private bit set; read by the host after process()
}
impl<'a> ProcessContext<'a> {
    pub fn new(frames: u32, steady_time: u64, transport: Transport,
               events: &'a [ParamEvent], out_events: &'a mut OutputEvents) -> Self;
    /// RT-safe (sets a bit). The host forwards it to the control thread via the RT event ring.
    pub fn request(&mut self, r: HostRequest);
    pub fn requested(&self, r: HostRequest) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessStatus {
    Continue,
    /// Output could not be produced correctly (adapters only: sandbox died, IPC deadline).
    /// The module must still have written finite samples (silence or latency-matched dry).
    /// The host bypasses the slot (crossfade), marks it failed and notifies the user.
    /// Built-in modules never return Error.
    Error,
}
```

### 6. The `Module` trait

```rust
pub trait Module: Send {
    // --- Metadata: any non-audio thread, any state. Constant for the instance's lifetime. ---
    fn descriptor(&self) -> &ModuleDescriptor;
    /// Display order = slice order. Constant for the instance's lifetime; an external plugin
    /// that changes its parameter list is handled by its adapter through a replacement (§12).
    fn params(&self) -> &[ParamInfo];
    fn groups(&self) -> &[ParamGroup] { &[] }
    fn supported_layouts(&self) -> &[ChannelLayout] { &[ChannelLayout::MONO] }

    // --- Lifecycle: control thread, never concurrent with process(). ---
    /// [inactive -> active] May allocate and block (buffers, FFT plans).
    /// On Ok the module is in reset state.
    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError>;
    /// [active -> inactive] May deallocate.
    fn deactivate(&mut self);
    /// [active] Valid after activate, constant until deactivate.
    /// Output sample n corresponds to input sample n - latency.
    fn latency_samples(&self) -> u32;
    /// [active] Valid after activate, constant until deactivate.
    fn tail(&self) -> Tail;

    // --- Audio: the audio thread (or an offline render thread), active only, RT-safe. ---
    /// inputs.len() == layout.inputs, outputs.len() == layout.outputs, all slices ctx.frames long.
    /// No allocation, locks, I/O, logging, syscalls, unbounded loops; no panic on valid input;
    /// output finite for finite input; denormal-safe (the host also sets FTZ/DAZ).
    fn process(&mut self, ctx: &mut ProcessContext<'_>,
               inputs: &[&[f32]], outputs: &mut [&mut [f32]]) -> ProcessStatus;
    /// [active] Seek / loop wrap / transport start: clear delay lines, envelopes, overlap
    /// buffers; smoothers snap to their targets. Parameter values unchanged. RT-safe.
    fn reset(&mut self);

    // --- Values & state: control thread, instance not live (§7). ---
    /// Current target plain value (last event or load_state). None for unknown ids.
    fn param_value(&self, id: ParamId) -> Option<f64>;
    /// [active or inactive, not processing]
    fn save_state(&self) -> Result<ModuleState, StateError>;
    /// [inactive] `state` is already at descriptor().state_format_version (host calls
    /// migrate_state first). Unknown keys ignored, missing keys -> default, values clamp_quantized.
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError>;
    /// Upgrade an older state. Default: formats are additive, so the result is `state` with its
    /// format_version bumped.
    fn migrate_state(&self, state: ModuleState) -> Result<ModuleState, StateError> {
        Ok(ModuleState { format_version: self.descriptor().state_format_version, ..state })
    }

    // --- Extensions (§11): control thread, not processing. ---
    fn extension(&self, id: ExtensionId) -> Option<Extension> { None }
}

#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    #[error("unsupported configuration: {0}")] Unsupported(String),
    #[error("resource error: {0}")] Resource(String),
    #[error("external module failure: {0}")] External(String),
}
```

Reference shape of a built-in `process` (the pattern TestGain in T-005 uses):

```rust
fn process(&mut self, ctx: &mut ProcessContext<'_>, inputs: &[&[f32]], outputs: &mut [&mut [f32]])
    -> ProcessStatus {
    let (input, output) = (inputs[0], &mut *outputs[0]);
    for seg in segments(ctx.frames, ctx.events) {
        for ev in seg.events { self.apply(ev); }            // set smoother targets
        let r = seg.start as usize..(seg.start + seg.len) as usize;
        for (o, i) in output[r.clone()].iter_mut().zip(&input[r]) { *o = *i * self.gain.next(); }
    }
    ProcessStatus::Continue
}
```

In-place processing (input and output aliasing the same buffer, as CLAP allows) cannot be expressed
safely in Rust. The rack ping-pongs between two preallocated buffers, which costs no copies.

### 7. Threading

A module is `Send`, not required to be `Sync`, and has exactly one owner at a time. It is **live**
while it belongs to a chain installed on the audio thread. **A live instance only receives
`process` and `reset`.** Every other call happens before it is inserted or after it is removed. Both
transfers use the rack-edit swap (T-103): the chain is built off-thread, handed over through a
lock-free queue, and old chains come back through a return queue and are dropped off-thread
(ADR-002).

| Call | Thread | Instance state | May allocate/block |
|---|---|---|---|
| `descriptor`, `params`, `groups`, `supported_layouts` | any non-audio | any | no |
| `activate` / `deactivate` | control | inactive / active, not live | yes |
| `latency_samples`, `tail` | control | active, not live | no |
| `process`, `reset` | audio (or offline render) | active | **no (RT)** |
| `param_value`, `save_state`, `extension` | control | not processing | yes |
| `load_state` | control | inactive | yes |
| `migrate_state` | control | any, not processing | yes |
| Extension handle methods | any non-audio | concurrent with `process` | per extension |

While a module is live, the UI and control thread use:
1. Metadata copied at insertion: `Arc<[ParamInfo]>`, groups, descriptor, latency, tail.
2. The host's **parameter mirror**: the host sends every value, so it always knows the target values.
   It also learns READ_ONLY values and adapter changes from `out_events`.
3. Extension handles (`Send + Sync`), queried once after `activate` and before insertion.

**State rule S1:** a built-in module's persistent state is exactly its parameter values plus the blob
last given to `load_state`. `process` never mutates persistent state. The host can therefore
snapshot a live built-in (autosave, presets) from its mirror plus the committed blob, without
touching the instance. External plugins can change internal state on their own (for example through
their editor in M9). Their adapters implement the reserved `LiveState` extension (§11).

### 8. Latency, tail, channel layouts
- Latency and tail are fixed while active. The only way to change them is `ctx.request(HostRequest::Restart)`.
  Example: NR receives an `fft_size` event. It keeps running at the old size and requests a restart.
  The replacement instance is activated with the new value (§12). The rack then recomputes latency
  compensation (T-401).
- Total rack latency is the sum of slot latencies. Bypassed slots count too, because their dry path
  is latency-matched.
- **Layouts:** v1 only negotiates 1-in/1-out. If `supported_layouts()` contains `MONO`, it is used.
  Otherwise, with `STEREO`, the rack wraps the module in the single **dual-mono shim** (in `rack`,
  adapter-agnostic): the input is fed to both channels and the left output is taken, while latency,
  tail, params and state pass through. With `MONO_TO_STEREO`, the left output is taken. Anything else
  is rejected at insertion ("unsupported channel layout"). Adapters do not implement their own shim.

### 9. Bypass is host-owned
- **Per-slot bypass:** slot output = crossfade between the module output and the slot input delayed
  by the module's latency. The rack owns the delay line, allocated at activate. The crossfade is
  **linear (equal-gain), 15 ms** (T-103 may tune it within 10–20 ms). The module **keeps being
  processed while bypassed**, with its output discarded, so its state stays warm, latency stays
  constant and un-bypassing is seamless. The CPU cost is accepted and can be revisited in T-704.
- If a module declares a `BYPASS` param, the host's bypass toggle sends events to that param
  (1.0 = bypassed) instead of crossfading itself. The module then owns click-free, latency-constant
  bypass (CLAP/VST3 semantics). Built-in modules do not declare `BYPASS`.
- **Whole-rack A/B:** the dry signal is the rack input delayed by the total rack latency, with the
  same 15 ms crossfade. Modules keep processing.
- The bypass flag belongs to the slot (sidecar/rack preset), not to `ModuleState`.

### 10. State, presets, migration

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleState {
    pub format_version: u32,
    /// key -> plain value, for every non-READ_ONLY param (HIDDEN included). BTreeMap for
    /// deterministic output (stable sidecar diffs). Values finite (-inf dB = `min`, see Taper::Db).
    pub params: BTreeMap<String, f64>,
    /// Opaque module data (noise profile, external plugin chunk). JSON: standard base64 string,
    /// via a small hand-written serde helper (no base64 dependency).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "blob_base64")]
    pub blob: Option<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("state format {found} is newer than supported {supported}")]
    TooNew { found: u32, supported: u32 },
    #[error("invalid state blob: {0}")] InvalidBlob(String),
    #[error("state migration failed: {0}")] Migration(String),
}

/// Host-side load pipeline (module-api helper, used by rack/project).
pub fn prepare_state(m: &dyn Module, s: ModuleState) -> Result<ModuleState, StateError>;
// 1. s.format_version > current -> TooNew   2. < current -> m.migrate_state(s)
// 3. drop unknown keys, add missing keys at default, clamp_quantize every value.
```

- **External plugins are blob-only:** `params` is empty and `blob` is the plugin's own chunk. The
  adapter frames multi-part states, e.g. VST3 component + controller state. `format_version` is the
  adapter's framing version.
- In the sidecar (owned by `project`), a slot is
  `{ "module": "org.powervoice.gain@1.0.0", "bypass": false, "state": { "format_version": 1, "params": { "gain_db": -6.0 } } }`.
- A module preset is `{ module, name, state }`. A rack preset is an ordered list of slots. The storage
  format is T-406's.
- Module `version` is identity and display. `format_version` alone drives migration.
- **Determinism rule:** given the same state, config, input and event list, two instances produce
  bit-identical output (no unseeded randomness, no dependence on timing). v1 built-ins produce the same
  output in `Realtime` and `Offline` mode, so preview equals export. `ProcessMode` exists for adapters
  (CLAP `render`, VST3 `kOffline`) and for future quality modes.

### 11. Extensions (runtime query)

```rust
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExtensionId { Telemetry, ResponseCurve, NoiseProfile }
impl ExtensionId {
    /// Wire id; also the CLAP custom-extension id (ADR-006).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Telemetry => "org.powervoice.telemetry/1",
            Self::ResponseCurve => "org.powervoice.response-curve/1",
            Self::NoiseProfile => "org.powervoice.noise-profile/1",
        }
    }
}

#[non_exhaustive]
#[derive(Clone)]
pub enum Extension {
    Telemetry(Arc<dyn Telemetry>),
    ResponseCurve(Arc<dyn ResponseCurve>),
    NoiseProfile(Arc<dyn NoiseProfile>),
}

/// Typed helpers (return None if absent or if the module answered with the wrong variant).
pub fn telemetry(m: &dyn Module) -> Option<Arc<dyn Telemetry>>;
pub fn response_curve(m: &dyn Module) -> Option<Arc<dyn ResponseCurve>>;
pub fn noise_profile(m: &dyn Module) -> Option<Arc<dyn NoiseProfile>>;
```

Contract: the module returns the matching variant or `None`. The host queries every known id once
after `activate`, before insertion, and keeps the handles for the instance's lifetime. A handle stays
safe to call after the instance is dropped (it's an `Arc`) and then returns its last values. New
extensions are new enum variants, which is additive thanks to `#[non_exhaustive]`. We chose a typed
enum over `dyn Any` downcasting: there are no casts, it is checked at compile time, and the host
only understands the extensions it knows about anyway.

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TelemetryInfo {
    pub id: u32, pub key: String, pub name: LocalizedText, pub unit: Unit,
    pub min: f64, pub max: f64, pub kind: TelemetryKind, pub group: Option<GroupId>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TelemetryKind {
    /// Applied gain change in dB, <= 0 for reduction (e.g. -6.0).
    GainReduction,
    Level,
    /// 0/1 lamp, e.g. "gate open".
    Indicator,
    Value,
}

/// Lock-free readouts (e.g. gain reduction). Single reader: the host's meter publisher (30–60 Hz).
pub trait Telemetry: Send + Sync {
    fn channels(&self) -> &[TelemetryInfo];
    /// Wait-free. Value in the channel's unit.
    fn read(&self, index: usize) -> f32;
}

/// Provided implementation; the module keeps an Arc and writes from process().
pub struct AtomicF32(/* AtomicU32 bits, Relaxed */);
pub struct TelemetryCells { /* infos + Box<[cell]> + read epoch */ }
impl TelemetryCells {
    pub fn new(channels: Vec<TelemetryInfo>, hold: Vec<Hold>) -> Arc<Self>;
    /// RT-safe, wait-free. With Hold::Min/Max the value is held until the next read. The reader
    /// bumps an epoch counter and the writer restarts its accumulator when it sees a new epoch,
    /// so short peaks between UI reads are not lost and there are no CAS loops.
    pub fn write(&self, index: usize, value: f32);
}
pub enum Hold { Latest, Min, Max }
impl Telemetry for TelemetryCells { /* ... */ }

/// EQ graph from the same coefficient code as process() (`dsp`), for the *target* parameter
/// values, so the graph is exact (PROMPT §5: ±0.1 dB vs analytic).
pub trait ResponseCurve: Send + Sync {
    /// values: plain values in params() order. Pure, deterministic; no allocation beyond `out_db`.
    fn magnitude_db(&self, values: &[f64], sample_rate: f64, freqs_hz: &[f64], out_db: &mut [f64]);
    /// Individually drawable components (EQ bands). 0 = total only.
    fn component_count(&self, values: &[f64]) -> usize { 0 }
    fn component_magnitude_db(&self, _component: usize, _values: &[f64], _sample_rate: f64,
                              _freqs_hz: &[f64], _out_db: &mut [f64]) {}
    /// Draggable graph handles.
    fn handles(&self) -> &[CurveHandle] { &[] }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CurveHandle { pub component: usize, pub freq: ParamId, pub gain: Option<ParamId>,
                         pub q: Option<ParamId>, pub enable: Option<ParamId> }

/// Capture and store a noise profile (NR module).
pub trait NoiseProfile: Send + Sync {
    /// Analyse a noise-only excerpt (mono, as seen at the slot's input) with the given plain
    /// values (e.g. FFT size). Returns the complete new state blob. Worker thread; may allocate
    /// and take long; must poll `cancel`.
    fn capture(&self, noise: &[f32], sample_rate: f64, values: &[f64], cancel: &AtomicBool)
        -> Result<Vec<u8>, ModuleError>;
    fn min_capture_samples(&self, sample_rate: f64, values: &[f64]) -> usize;
    /// Decode a blob for the profile graph: (frequency Hz, level dBFS) points.
    fn describe(&self, blob: &[u8], out: &mut Vec<(f32, f32)>) -> Result<(), ModuleError>;
}
```

The NR module never receives a captured profile on the live instance. The host stores it as the
slot's committed blob and performs a replacement (§12). The capture input is the audio at the slot's
input: the host offline-renders the preceding slots over the selection (details in T-502).

**Reserved for M8** (added as `ExtensionId` variants when needed; ids fixed now):
- `ParamText` (`org.powervoice.param-text/1`): adapter-provided `value_to_text`/`text_to_value`,
  callable concurrently with processing, which overrides `ParamInfo`'s rules.
- `LiveState` (`org.powervoice.live-state/1`): snapshot the state of a live instance.
- M9 adds a GUI extension.

### 12. Changes to a live module

| Change | Mechanism |
|---|---|
| Parameter value (UI, param-only preset) | Mirror updated, then `ParamEvent` through the UI→audio queue |
| Bypass | Host crossfade (§9), or events to the `BYPASS` param |
| Blob state (noise print, preset with blob, plugin state load) | Replacement |
| `HostRequest::Restart` (latency/tail/FFT size) | Replacement |
| Sample rate, `max_block`, mode | New chain. The rack runs at the output-device rate, which changes only when the device is (re)opened, e.g. on a document-rate change (ADR-002 §5), so the stream is stopped anyway. Offline renders always build their own chain from committed states |
| Insert / remove / reorder | Rack edit (T-103) |

**Replacement procedure** (control thread):
1. `new = factory.create()`, then `new.load_state(prepare_state(committed))`. `committed` =
   `{ current format_version, params from the mirror by key, committed blob }`, or
   `LiveState::snapshot()` for adapters.
2. `new.activate(cfg)`, then read latency and tail and query extensions.
3. Rack-edit swap: the audio thread crossfades old→new over 15 ms and latency compensation is
   recomputed. Both instances process during the crossfade.
4. The old instance returns through the return queue, then `deactivate()` and drop off-thread.

If the audio stream is not running, the host may instead deactivate and re-activate the same
instance in place, because it owns it exclusively.

### 13. Generic parameter UI (derived from the schema)
The UI receives the metadata above (generated TS types, ADR-003). It never runs taper or format code
itself.
- **Layout:** ungrouped params come first as the main section, then groups in `groups()` order (one
  nesting level rendered; deeper levels are flattened into "Parent / Child" titles). Within a section
  the order is `params()` order. A group with `enable_param` shows a header toggle and its body is
  dimmed but still editable when disabled. `collapsed_by_default` is honoured, and the user's
  collapse state is kept in the sidecar view state.
- **Widget from flags:**
  - `HIDDEN` and `BYPASS` are not shown (the slot header's bypass button maps to `BYPASS`).
  - `READ_ONLY` shows as a readout.
  - `BOOL` is a toggle; enum labels become a dropdown; `STEPPED` numeric is a slider with detents.
  - Continuous params are an Audition-style horizontal slider plus a value field.
  - Modules with more than 32 visible params get a filter box (external plugins).
- **Normalized positions:** the UI sends `set_param_normalized(slot, id, t)` while dragging (Shift =
  fine ×0.1, wheel = one step or 1 % of travel, double-click = default). Text entry sends
  `set_param_text(slot, id, text)`. The control thread maps through `ParamInfo` or `ParamText`,
  updates the mirror, sends the event, and echoes `param_changed { slot, id, value, normalized, text }`.
  This is the same event used for host-originated changes such as presets and plugin GUIs, so Rust is
  the single source of truth for mapping and formatting.
- **Extensions:**
  - `ResponseCurve` adds a graph above the params. The EQ gets its custom panel (T-409) built on the
    same handles.
  - `Telemetry` channels become meters in their group header, or in the module header when there is no
    group (T-410).
  - `NoiseProfile` adds "Capture noise print" and a profile graph (T-504).
  - Custom panels for built-ins may replace the generic layout, but they only use this schema and
    these extensions. The generic UI remains the fallback for every module and the only UI for
    external plugins until M9.

### 14. Mapping to plugin formats (for M8 adapters and ADR-006)

| Module API | CLAP 1.2 | VST3 (SDK ≥ 3.8) | LV2 / JSFX |
|---|---|---|---|
| descriptor id/version/name/vendor/features | `clap_plugin_descriptor` (features are CLAP strings) | `PClassInfo2` (cid → `vst3:<hex>`), subCategories | plugin URI / `desc:` + file path |
| `ParamId` u32, plain values, min/max/default | `clap_param_info` (`clap_id`, plain doubles) | `ParamID`, **normalized** values → adapter uses `normalizedParamToPlain`/`plainParamToNormalized` | control ports `lv2:minimum/maximum/default` / `sliderN` ranges (plain) |
| key | none → adapter key `p<id>` (stable because ids are stable) | `p<id>` | port symbol / `sliderN` |
| flags | STEPPED, HIDDEN, READONLY, BYPASS, AUTOMATABLE, ENUM | `kCanAutomate`, `kIsReadOnly`, `kIsHidden`, `kIsBypass`, `kIsList`, `stepCount` | `lv2:integer`, `lv2:toggled`, `lv2:enumeration`, output ports / hidden sliders `-` |
| group | `clap_param_info.module` path | `unitId` + `IUnitInfo` | `pg:group` / — |
| text | `params.value_to_text/text_to_value` via `ParamText` | `getParamStringByValue`/`getParamValueByString` | unit + scale points / slider enum |
| per-block events with offsets | `clap_input_events` `PARAM_VALUE`, `header.time` = offset | `IParameterChanges` queues, `sampleOffset` | atom/port values split at offsets / sliders at block start |
| `out_events` | `clap_output_events` | `outputParameterChanges` | output control ports |
| `activate(sr, max_block, mode)` | `activate(sr, 1, max_block)` + `render.set` + `start_processing` on first process | `setupProcessing` (`kRealtime`/`kOffline`, maxSamplesPerBlock) + `setActive(true)` + `setProcessing(true)` | `instantiate(sr)` + `activate` / `@init` |
| `reset()` | `reset` | `setProcessing(false)` then `setProcessing(true)` (documented to reset buffers) | `deactivate`/`activate` off-thread → adapter keeps a spare / `@init` |
| `latency_samples` / `Restart` | `latency.get` / `host.request_restart` (`latency.changed` during activate) | `getLatencySamples` / `restartComponent(kLatencyChanged)` | `lv2:reportsLatency` port / `pdc_delay` |
| `tail()` | `tail.get` (≥ `INT32_MAX` → `Infinite`) | `getTailSamples` (`kInfiniteTail`) | — → `Samples(0)` |
| `save_state`/`load_state` (blob-only) | `state.save/load` streams | `IComponent::getState/setState` + controller state, framed | LV2 state ext / serialized slider values + `@serialize` |
| layouts | `audio-ports` (+ `audio-ports-config`) | `setBusArrangements` | port counts / `in_pin`/`out_pin` |
| `BYPASS` | `CLAP_PARAM_IS_BYPASS` | `kIsBypass` | `lv2:designation lv2:enabled` (inverted) / — |
| Telemetry, ResponseCurve, NoiseProfile | custom extensions (ADR-006) | — | — |

CLAP's main-thread calls may run concurrently with its audio thread. Our non-`Sync` rule is stricter,
and the sandbox adapter (ADR-008) bridges the two.

### 15. Test obligations (`ModuleTestHost`, feature `test-util`)
For any `Module`:
- `validate_schema` passes.
- `activate` at 44.1/48/96 kHz.
- `process` with random block sizes in `0..=max_block`, including 0, and random valid events at random
  offsets, under `assert_no_alloc`. Outputs are finite.
- `reset` under `assert_no_alloc`.
- `latency_samples`/`tail` are callable after activate.
- State round-trip: save, then load into a fresh instance, then activate: bit-identical output on the
  same input and events.
- Two instances are bit-identical (determinism).
- `text_to_value(value_to_text(v))` round-trips per §3.
- An event at offset k affects output from sample k (within `smoothing_ms`).
- `extension()` returns the variant matching its id.

## Consequences
**Positive**
- One contract for built-ins, packaged modules and adapters. It maps almost 1:1 onto CLAP and cleanly
  onto VST3/LV2/JSFX.
- Sample-accurate, deterministic processing gives golden tests and preview-equals-export.
- The RT rules can be checked mechanically by `ModuleTestHost`.
- The live-instance rule (only `process`/`reset` while live) removes concurrency bugs inside modules.
- The generic UI needs no per-module code, and formatting and mapping live in Rust only.

**Negative**
- The host has to keep a parameter mirror and committed blobs per slot.
- Blob changes and restarts go through replacement instances, which briefly doubles the slot's CPU
  during the crossfade.
- Bypassed modules keep consuming CPU.
- Plain `f64` values and a JSON-friendly schema are slightly heavier than raw normalized floats.
- Slider drags round-trip through the control thread for text (≤ 1 frame of lag).

**Follow-ups**
- T-005: this crate plus `ModuleTestHost` plus TestGain.
- T-103: rack swap, bypass, mirror, dual-mono shim, placeholder slot.
- T-401: latency compensation and Restart handling.
- T-405: generic UI.
- T-406: presets.
- T-402/T-409: ResponseCurve.
- T-403/T-410: Telemetry.
- T-502/T-503: NoiseProfile and restart on FFT size.
- M8 adapters: `ParamText`, `LiveState`.

## Alternatives considered
- **Normalized 0..1 values (VST3 style):** rejected. Plain values keep the sidecar readable and
  migratable and avoid precision loss. CLAP, LV2 and JSFX are plain; only the VST3 adapter converts.
- **Shared atomic parameters with framework-side smoothing (nih-plug style):** rejected. There are no
  sample offsets, event ordering and offline determinism get harder, and it does not match CLAP events
  or out-of-process adapters.
- **Separate processor/controller objects (VST3 split):** rejected as over-engineering. The host mirror
  plus `Send + Sync` extension handles cover everything the UI needs while a module is live.
- **Using CLAP (via `clack`) as the in-process API:** rejected. It would mean C-ABI ergonomics and
  `unsafe` for every built-in. CLAP remains the packaging ABI (ADR-006).
- **Host-owned smoothing:** rejected. The right smoothing domain is module-specific.
- **`dyn Any` extension lookup by string:** rejected in favour of the typed enum. Strings are kept only
  as wire ids.

## Open questions
1. **Owner:** the id namespace `org.powervoice.*` follows the placeholder product name. Ids are permanent
   once the first sidecar is written (M3, T-306). Confirm, or give the final namespace, before M3.
   Decide it together with the Tauri app identifier (ADR-004 uses `app.powervoice.editor`). Both depend on
   the final name or domain.
2. Processing bypassed modules costs CPU. Keep it, or pause and reset with a crossfade from dry?
   Revisit in T-704 against the "< 20 % of one core" target.
3. Are rack edits (parameter changes, noise-print capture) part of the undo history? ADR-004 and the M4
   specs decide. This API supports either.

## Amendment 1 — slices + hardening (S3-06, S3-07, H-03), 2026-09-14
- **§13 command `param_set_plain(slot, id, value)`** (S3-07, SPEC-015/016 §4.12): graph handles send
  plain values; the control thread clamps/quantizes, updates the mirror, sends the event and echoes
  `param_changed`, like `param_set_normalized` / `param_set_text`. Engine side:
  `RackCommand::SetParamPlain`.
- **Typed extensions in the host:** `RackHost` captures a module's `NoiseProfile` and `ResponseCurve`
  handles when the slot is loaded (the instance itself moves to the audio thread) and exposes them per
  slot (`SlotInfo::noise_profile`, `SlotInfo::curve_handles`). `Telemetry` handles belong to one
  instance and are refreshed whenever the slot's instance is replaced (`replace_with`).
- **Meters:** the rack UI renders every `GainReduction`-kind telemetry channel in the slot header
  (generic, not module-specific); other kinds are T-410. Values reach the UI through ADR-003's `VXMT`
  frame (ADR-003 Amendment 3).

## Amendment 2 — open question 1 closed (2026-09-14)
- Open question 1 (id namespace vs. Tauri identifier) was settled at the M0 checkpoint with the product
  name PowerVoice (T-009): module ids use `org.powervoice.*` and the app identifier is
  `app.powervoice.editor` (ADR-004), both now shipped in sidecars (T-306), presets (T-406) and bundles
  (T-705). They are permanent.
