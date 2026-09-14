//! Typed extensions queried at runtime (ADR-005 §11).

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::descriptor::LocalizedText;
use crate::module::{Module, ModuleError};
use crate::param::{GroupId, ParamId, Unit};

/// Known extension ids. New extensions are new variants (additive thanks to `non_exhaustive`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ExtensionId {
    /// [`Telemetry`].
    Telemetry,
    /// [`ResponseCurve`].
    ResponseCurve,
    /// [`NoiseProfile`].
    NoiseProfile,
    /// [`AdapterHealth`] (T-802, ADR-005 Amendment 3): host-internal, answered only by
    /// out-of-process adapters (the plugin sandbox proxy); never a CLAP extension.
    AdapterHealth,
}

impl ExtensionId {
    /// Every known id, in declaration order (what the host queries after `activate`).
    pub const ALL: [ExtensionId; 4] = [
        ExtensionId::Telemetry,
        ExtensionId::ResponseCurve,
        ExtensionId::NoiseProfile,
        ExtensionId::AdapterHealth,
    ];

    /// Wire id; also the CLAP custom-extension id (ADR-006) — except
    /// [`AdapterHealth`](Self::AdapterHealth), which never leaves the host.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Telemetry => "org.powervoice.telemetry/1",
            Self::ResponseCurve => "org.powervoice.response-curve/1",
            Self::NoiseProfile => "org.powervoice.noise-profile/1",
            Self::AdapterHealth => "org.powervoice.adapter-health/1",
        }
    }
}

/// An extension handle returned by [`Module::extension`]. Handles are `Send + Sync`, stay valid
/// after the instance is dropped (they are `Arc`s) and then return their last values.
#[non_exhaustive]
#[derive(Clone)]
pub enum Extension {
    /// Lock-free readouts.
    Telemetry(Arc<dyn Telemetry>),
    /// Frequency-response graph.
    ResponseCurve(Arc<dyn ResponseCurve>),
    /// Noise-print capture.
    NoiseProfile(Arc<dyn NoiseProfile>),
    /// Out-of-process adapter health.
    AdapterHealth(Arc<dyn AdapterHealth>),
}

impl Extension {
    /// The id this variant answers.
    pub fn id(&self) -> ExtensionId {
        match self {
            Extension::Telemetry(_) => ExtensionId::Telemetry,
            Extension::ResponseCurve(_) => ExtensionId::ResponseCurve,
            Extension::NoiseProfile(_) => ExtensionId::NoiseProfile,
            Extension::AdapterHealth(_) => ExtensionId::AdapterHealth,
        }
    }
}

impl fmt::Debug for Extension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Extension::{:?}", self.id())
    }
}

/// The module's [`Telemetry`] handle; `None` if absent or answered with the wrong variant.
pub fn telemetry(m: &dyn Module) -> Option<Arc<dyn Telemetry>> {
    match m.extension(ExtensionId::Telemetry) {
        Some(Extension::Telemetry(t)) => Some(t),
        _ => None,
    }
}

/// The module's [`ResponseCurve`] handle; `None` if absent or answered with the wrong variant.
pub fn response_curve(m: &dyn Module) -> Option<Arc<dyn ResponseCurve>> {
    match m.extension(ExtensionId::ResponseCurve) {
        Some(Extension::ResponseCurve(r)) => Some(r),
        _ => None,
    }
}

/// The module's [`NoiseProfile`] handle; `None` if absent or answered with the wrong variant.
pub fn noise_profile(m: &dyn Module) -> Option<Arc<dyn NoiseProfile>> {
    match m.extension(ExtensionId::NoiseProfile) {
        Some(Extension::NoiseProfile(n)) => Some(n),
        _ => None,
    }
}

/// The module's [`AdapterHealth`] handle; `None` if absent or answered with the wrong variant.
pub fn adapter_health(m: &dyn Module) -> Option<Arc<dyn AdapterHealth>> {
    match m.extension(ExtensionId::AdapterHealth) {
        Some(Extension::AdapterHealth(h)) => Some(h),
        _ => None,
    }
}

/// The live fault state of an **out-of-process adapter** (T-802: the plugin sandbox proxy,
/// ADR-008), readable by the host's control thread at any time — also while the instance is live
/// on the audio thread (`Send + Sync`, lock-free or briefly locked on the adapter's side, never
/// touched by `process`' caller).
///
/// Its presence tells the host the module runs out of process: the rack then applies the
/// sandbox restart policy to its `ProcessStatus::Error` failures and shows its slot status
/// (ADR-005 Amendment 3).
pub trait AdapterHealth: Send + Sync {
    /// Why the adapter stopped producing its own output (`None` while healthy): a short English
    /// verb phrase that completes "‹module› … and was bypassed", e.g. `"crashed"` or
    /// `"stopped responding"`. Set before `process` returns `ProcessStatus::Error` for it.
    fn fault(&self) -> Option<String>;
}

/// Description of one telemetry channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TelemetryInfo {
    /// Channel id, stable per module.
    pub id: u32,
    /// Stable key.
    pub key: String,
    /// Display name.
    pub name: LocalizedText,
    /// Unit of the values.
    pub unit: Unit,
    /// Display minimum.
    pub min: f64,
    /// Display maximum.
    pub max: f64,
    /// What the value means.
    pub kind: TelemetryKind,
    /// Group whose header shows the meter (`None` = module header).
    pub group: Option<GroupId>,
}

/// Meaning of a telemetry channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TelemetryKind {
    /// Applied gain change in dB, `<= 0` for reduction (e.g. −6.0).
    GainReduction,
    /// A level.
    Level,
    /// 0/1 lamp, e.g. "gate open".
    Indicator,
    /// Any other value.
    Value,
}

/// Lock-free readouts (e.g. gain reduction). Single reader: the host's meter publisher
/// (30–60 Hz).
pub trait Telemetry: Send + Sync {
    /// Channel descriptions.
    fn channels(&self) -> &[TelemetryInfo];
    /// Wait-free. Value in the channel's unit.
    fn read(&self, index: usize) -> f32;
}

/// An `f32` stored as `AtomicU32` bits; all accesses `Relaxed`. RT-safe.
#[derive(Debug, Default)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    /// Creates the cell.
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }

    /// Reads the value.
    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    /// Writes the value.
    pub fn store(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// How a [`TelemetryCells`] channel combines writes between two reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hold {
    /// The last written value.
    Latest,
    /// The minimum since the previous read (e.g. gain reduction).
    Min,
    /// The maximum since the previous read (e.g. peak level).
    Max,
}

#[derive(Debug)]
struct Cell {
    hold: Hold,
    /// Written only by the writer: `(write sequence << 32) | f32 bits` of the accumulated value.
    packed: AtomicU64,
    /// Written only by the reader: the write sequence it consumed last (its read epoch).
    read_seq: AtomicU32,
}

fn pack(seq: u32, value: f32) -> u64 {
    (u64::from(seq) << 32) | u64::from(value.to_bits())
}

fn unpack(p: u64) -> (u32, f32) {
    ((p >> 32) as u32, f32::from_bits(p as u32))
}

/// Provided [`Telemetry`] implementation. The module keeps the `Arc`, writes from `process()`
/// (single writer), and returns a clone as [`Extension::Telemetry`].
///
/// Min/Max channels hold their extreme until the next read. Each write is tagged with a sequence
/// number; each read publishes the sequence it consumed (the read epoch). The writer restarts its
/// accumulator only when the reader has consumed its latest write; otherwise it keeps
/// accumulating. So short peaks between reads are never lost, there are no CAS loops, and a
/// value is reported twice only when a write races a read.
#[derive(Debug)]
pub struct TelemetryCells {
    infos: Vec<TelemetryInfo>,
    cells: Box<[Cell]>,
}

impl TelemetryCells {
    /// Creates the cells. Values start at 0.0 clamped to each channel's `[min, max]`.
    ///
    /// # Panics
    /// If `channels` and `hold` have different lengths.
    pub fn new(channels: Vec<TelemetryInfo>, hold: Vec<Hold>) -> Arc<Self> {
        assert_eq!(channels.len(), hold.len(), "one Hold per telemetry channel");
        let cells = channels
            .iter()
            .zip(hold)
            .map(|(info, hold)| {
                // 0.0 clamped to [min, max], without panicking on NaN or reversed bounds.
                let init = if info.min > 0.0 {
                    info.min as f32
                } else if info.max < 0.0 {
                    info.max as f32
                } else {
                    0.0
                };
                // read_seq == write seq: the initial value counts as consumed, so the first
                // write starts a fresh period.
                Cell {
                    hold,
                    packed: AtomicU64::new(pack(0, init)),
                    read_seq: AtomicU32::new(0),
                }
            })
            .collect();
        Arc::new(Self {
            infos: channels,
            cells,
        })
    }

    /// RT-safe, wait-free. **Single writer, at most one write per channel per block.** The write
    /// sequence is a `u32` that wraps after 2³² writes: ≈ 25 h at one write per sample at
    /// 48 kHz, years at one write per block. A wrap could make an old read look like a read of
    /// the latest write and restart a Min/Max period early. Out-of-range indices are ignored.
    /// With [`Hold::Min`]/[`Hold::Max`] the value is held until the next read.
    pub fn write(&self, index: usize, value: f32) {
        let Some(c) = self.cells.get(index) else {
            return;
        };
        // Correctness rests on two facts, not on Acquire/Release: (1) `packed` is one atomic
        // location with a single writer, so by per-location coherence this load returns our own
        // last store; (2) every store carries a unique sequence number, so `read_seq == seq`
        // holds exactly when the reader loaded our latest store. Value and sequence travel in
        // the same word, so no cross-location ordering is needed.
        let (seq, acc) = unpack(c.packed.load(Ordering::Relaxed));
        let consumed = c.read_seq.load(Ordering::Acquire) == seq;
        let next = match c.hold {
            Hold::Latest => value,
            _ if consumed => value,
            Hold::Min => acc.min(value),
            Hold::Max => acc.max(value),
        };
        c.packed
            .store(pack(seq.wrapping_add(1), next), Ordering::Release);
    }
}

impl Telemetry for TelemetryCells {
    fn channels(&self) -> &[TelemetryInfo] {
        &self.infos
    }

    /// Wait-free. Out-of-range indices read 0.0.
    fn read(&self, index: usize) -> f32 {
        let Some(c) = self.cells.get(index) else {
            return 0.0;
        };
        let (seq, value) = unpack(c.packed.load(Ordering::Acquire));
        c.read_seq.store(seq, Ordering::Release);
        value
    }
}

/// EQ graph from the same coefficient code as `process()` (`dsp`), for the *target* parameter
/// values, so the graph is exact.
pub trait ResponseCurve: Send + Sync {
    /// `values`: plain values in `params()` order. Pure, deterministic; no allocation beyond
    /// `out_db`.
    fn magnitude_db(&self, values: &[f64], sample_rate: f64, freqs_hz: &[f64], out_db: &mut [f64]);
    /// Individually drawable components (EQ bands). 0 = total only.
    fn component_count(&self, _values: &[f64]) -> usize {
        0
    }
    /// Magnitude of one component.
    fn component_magnitude_db(
        &self,
        _component: usize,
        _values: &[f64],
        _sample_rate: f64,
        _freqs_hz: &[f64],
        _out_db: &mut [f64],
    ) {
    }
    /// Draggable graph handles.
    fn handles(&self) -> &[CurveHandle] {
        &[]
    }
}

/// A draggable handle on a response graph, bound to parameters.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CurveHandle {
    /// Component the handle belongs to.
    pub component: usize,
    /// Frequency parameter (x axis).
    pub freq: ParamId,
    /// Gain parameter (y axis), if any.
    pub gain: Option<ParamId>,
    /// Q parameter (wheel), if any.
    pub q: Option<ParamId>,
    /// Enable parameter, if any.
    pub enable: Option<ParamId>,
}

/// Capture and store a noise profile (NR module).
pub trait NoiseProfile: Send + Sync {
    /// Analyses a noise-only excerpt (mono, as seen at the slot's input) with the given plain
    /// values (e.g. FFT size). Returns the complete new state blob. Worker thread; may allocate
    /// and take long; must poll `cancel`.
    fn capture(
        &self,
        noise: &[f32],
        sample_rate: f64,
        values: &[f64],
        cancel: &AtomicBool,
    ) -> Result<Vec<u8>, ModuleError>;
    /// Minimum excerpt length for [`capture`](Self::capture).
    fn min_capture_samples(&self, sample_rate: f64, values: &[f64]) -> usize;
    /// Decodes a blob for the profile graph: (frequency Hz, level dBFS) points.
    fn describe(&self, blob: &[u8], out: &mut Vec<(f32, f32)>) -> Result<(), ModuleError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(key: &str) -> TelemetryInfo {
        TelemetryInfo {
            id: 0,
            key: key.into(),
            name: LocalizedText::plain(key),
            unit: Unit::Db,
            min: -60.0,
            max: 0.0,
            kind: TelemetryKind::GainReduction,
            group: None,
        }
    }

    #[test]
    fn ids_are_stable_wire_strings() {
        assert_eq!(
            ExtensionId::Telemetry.as_str(),
            "org.powervoice.telemetry/1"
        );
        assert_eq!(
            ExtensionId::ResponseCurve.as_str(),
            "org.powervoice.response-curve/1"
        );
        assert_eq!(
            ExtensionId::NoiseProfile.as_str(),
            "org.powervoice.noise-profile/1"
        );
    }

    #[test]
    fn telemetry_cells_hold_semantics() {
        let t = TelemetryCells::new(
            vec![info("latest"), info("min"), info("max")],
            vec![Hold::Latest, Hold::Min, Hold::Max],
        );
        for v in [-3.0, -12.0, -1.0] {
            t.write(0, v);
            t.write(1, v);
            t.write(2, v);
        }
        assert!((t.read(0) + 1.0).abs() < f32::EPSILON);
        assert!((t.read(1) + 12.0).abs() < f32::EPSILON, "min held");
        assert!((t.read(2) + 1.0).abs() < f32::EPSILON, "max held");
        // New period after each read: the accumulator restarts.
        t.write(1, -2.0);
        t.write(2, -20.0);
        assert!((t.read(1) + 2.0).abs() < f32::EPSILON);
        assert!((t.read(2) + 20.0).abs() < f32::EPSILON);
        t.write(2, -30.0);
        t.write(2, -25.0);
        assert!(
            (t.read(2) + 25.0).abs() < f32::EPSILON,
            "max of the new period"
        );
        // No write since the last read: the value is held.
        assert!((t.read(2) + 25.0).abs() < f32::EPSILON);
        // Out of range: ignored / 0.0.
        t.write(9, 1.0);
        assert!(t.read(9).abs() < f32::EPSILON);
        assert_eq!(t.channels().len(), 3);
    }

    #[test]
    fn atomic_f32() {
        let a = AtomicF32::new(1.5);
        assert!((a.load() - 1.5).abs() < f32::EPSILON);
        a.store(-2.25);
        assert!((a.load() + 2.25).abs() < f32::EPSILON);
    }
}
