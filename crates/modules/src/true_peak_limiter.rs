//! Built-in **True-Peak Limiter** (`org.powervoice.true-peak-limiter@1.0.0`, SPEC-017).
//!
//! The last module of a voice-over rack: guarantees the output's true peak stays under the
//! ceiling, and passes material below the ceiling bit-exactly (delayed by the latency).
//! DSP: [`vox_dsp::true_peak`] (4× detector + parabolic refinement, look-ahead hold, linear-dB
//! release, two moving averages).
//!
//! Parameters (all AUTOMATABLE, SPEC-017 §3):
//!
//! | id | key | range | default | smoothing |
//! |---|---|---|---|---|
//! | 0 | `input_gain_db` | −12 … +24 dB | 0 | 100 ms, linear in linear gain |
//! | 1 | `ceiling_dbtp` | −12 … 0 dBTP | −1 | 20 ms, linear in dB |
//! | 2 | `release_ms` | 10 … 1000 ms (log) | 100 | — (12 dB per release time) |
//! | 3 | `lookahead_ms` | 1 … 10 ms, step 0.5 | 5 | — (a change requests `Restart`) |
//!
//! Latency = L + 17 (SPEC-017 Amendment 2: the detector's D = 16 plus one sample so that every
//! output sample's gain covers both intervals it bounds exactly), with L = look-ahead in samples
//! rounded up to even (257 at 48 kHz / 5 ms).
//!
//! **Parameter timing (latency-aware, SPEC-017 §4.4).** An event at offset k changes nothing
//! before output sample k + latency. Events wait in preallocated queues (1024 per parameter;
//! on overflow the newest overwrites the last queued entry):
//! - `ceiling_dbtp` is attached to the input sample at k + L + 1 and travels with the audio
//!   (a sample's ceiling first reaches the gain computer through the requirement of the sample
//!   before it, D samples later);
//! - `input_gain_db` is applied to the input sample at k + latency (the detector's interpolator
//!   reads 16 samples ahead, and the per-sample requirement one more, so an earlier gain would
//!   move the gain computer before k + latency);
//! - `release_ms` reaches the gain computer at k + latency;
//! - `lookahead_ms` never changes the live instance: a new value requests
//!   `HostRequest::Restart` and the replacement is activated with it.
//!
//! Telemetry: channel 0 `gain_reduction_db` (min applied gain per block, held until read).
//! Tail: none. State: the four values (rule S1, no blob).

use std::collections::BTreeMap;
use std::sync::Arc;

use vox_dsp::true_peak::{
    REQUIREMENT_DELAY, TruePeakLimiterCore, lookahead_samples, release_coeff,
};
use vox_module_api::{
    ActivateConfig, ChannelLayout, Extension, ExtensionId, Hold, HostRequest, LocalizedText,
    MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleState,
    ParamFlags, ParamId, ParamInfo, ProcessContext, ProcessStatus, StateError, Tail, Taper,
    TelemetryCells, TelemetryInfo, TelemetryKind, Unit, Version, features,
};

/// Parameter events waiting for their sample, one FIFO per parameter (preallocated).
#[derive(Debug, Default)]
struct PendingQueue {
    at: Box<[u64]>,
    value: Box<[f64]>,
    head: usize,
    len: usize,
}

impl PendingQueue {
    fn with_capacity(cap: usize) -> Self {
        Self {
            at: vec![0; cap].into_boxed_slice(),
            value: vec![0.0; cap].into_boxed_slice(),
            head: 0,
            len: 0,
        }
    }

    fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    /// Queues `value` for sample `at` (non-decreasing). Full: the newest replaces the last
    /// queued entry (latest wins). RT-safe.
    fn push(&mut self, at: u64, value: f64) {
        let cap = self.at.len();
        if cap == 0 {
            return;
        }
        let slot = if self.len == cap {
            (self.head + self.len - 1) % cap
        } else {
            self.len += 1;
            (self.head + self.len - 1) % cap
        };
        self.at[slot] = at;
        self.value[slot] = value;
    }

    /// The oldest value due at or before sample `t`. RT-safe.
    fn pop_due(&mut self, t: u64) -> Option<f64> {
        if self.len == 0 || self.at[self.head] > t {
            return None;
        }
        let v = self.value[self.head];
        self.head = (self.head + 1) % self.at.len();
        self.len -= 1;
        Some(v)
    }
}

/// Linear ramp (TestGain convention): the first ramp sample already moves, the last equals the
/// target; a new target mid-ramp restarts from the current value.
#[derive(Clone, Copy, Debug)]
struct Ramp {
    len: u32,
    start: f64,
    target: f64,
    pos: u32,
}

impl Ramp {
    fn new(v: f64) -> Self {
        Self {
            len: 0,
            start: v,
            target: v,
            pos: u32::MAX,
        }
    }

    fn snap(&mut self, v: f64) {
        self.start = v;
        self.target = v;
        self.pos = u32::MAX;
    }

    fn is_ramping(&self) -> bool {
        self.pos < self.len
    }

    fn current(&self) -> f64 {
        if self.pos >= self.len {
            self.target
        } else {
            self.start + (self.target - self.start) * f64::from(self.pos) / f64::from(self.len)
        }
    }

    fn set_target(&mut self, v: f64) {
        self.start = self.current();
        self.target = v;
        self.pos = 0;
    }

    fn next(&mut self) -> f64 {
        if self.pos < self.len {
            self.pos += 1;
        }
        self.current()
    }
}

fn db_to_lin(db: f64) -> f64 {
    10f64.powf(db / 20.0)
}

fn ramp_samples(sample_rate: f64, ms: f32) -> u32 {
    (sample_rate * f64::from(ms) / 1000.0).round() as u32
}

const INPUT: usize = 0;
const CEILING: usize = 1;
const RELEASE: usize = 2;
const LOOKAHEAD: usize = 3;

/// The built-in True-Peak Limiter.
pub struct TruePeakLimiter {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    telemetry: Arc<TelemetryCells>,
    /// Latest (target) plain values, in `params()` order: `param_value` / `save_state`.
    values: [f64; 4],
    sample_rate: f64,
    /// Look-ahead the live instance runs with (set at activate).
    active_lookahead_ms: f64,
    lookahead: u64,
    core: Option<TruePeakLimiterCore>,
    /// Input gain, linear.
    input_gain: Ramp,
    /// Ceiling, dB.
    ceiling_db: Ramp,
    /// Linear ceiling of the current target (used while not ramping).
    ceiling_lin: f64,
    /// Pending events of input gain, ceiling and release.
    pending: [PendingQueue; 3],
    /// Input samples processed since activate.
    t: u64,
}

impl TruePeakLimiter {
    /// Module id (ADR-005 §2).
    pub const ID: &'static str = "org.powervoice.true-peak-limiter";
    /// Module version.
    pub const VERSION: Version = Version::new(1, 0, 0);
    /// State format this build writes.
    pub const STATE_FORMAT_VERSION: u32 = 1;
    /// `input_gain_db`.
    pub const INPUT_GAIN_DB: ParamId = ParamId(0);
    /// `ceiling_dbtp`.
    pub const CEILING_DBTP: ParamId = ParamId(1);
    /// `release_ms`.
    pub const RELEASE_MS: ParamId = ParamId(2);
    /// `lookahead_ms`.
    pub const LOOKAHEAD_MS: ParamId = ParamId(3);
    /// Declared smoothing of `input_gain_db`.
    pub const INPUT_GAIN_SMOOTHING_MS: f32 = 100.0;
    /// Declared smoothing of `ceiling_dbtp`.
    pub const CEILING_SMOOTHING_MS: f32 = 20.0;
    /// Telemetry channel index of the gain reduction.
    pub const TELEMETRY_GAIN_REDUCTION: usize = 0;
    /// Pending-event capacity per parameter.
    pub const PENDING_CAPACITY: usize = 1024;

    /// A new, inactive instance at the defaults.
    pub fn new() -> Self {
        let params = Self::param_infos();
        let values = [
            params[INPUT].default,
            params[CEILING].default,
            params[RELEASE].default,
            params[LOOKAHEAD].default,
        ];
        Self {
            descriptor: Self::descriptor_value(),
            telemetry: TelemetryCells::new(Self::telemetry_channels(), vec![Hold::Min]),
            params,
            values,
            sample_rate: 48_000.0,
            active_lookahead_ms: values[LOOKAHEAD],
            lookahead: 0,
            core: None,
            input_gain: Ramp::new(db_to_lin(values[INPUT])),
            ceiling_db: Ramp::new(values[CEILING]),
            ceiling_lin: db_to_lin(values[CEILING]),
            pending: Default::default(),
            t: 0,
        }
    }

    /// The descriptor every instance returns (SPEC-017 §2.1).
    pub fn descriptor_value() -> ModuleDescriptor {
        ModuleDescriptor {
            id: Self::ID.into(),
            version: Self::VERSION,
            name: LocalizedText::keyed("module.true_peak_limiter.name", "True-Peak Limiter"),
            vendor: "PowerVoice".into(),
            description: LocalizedText::keyed(
                "module.true_peak_limiter.description",
                "Keeps the true peak under the ceiling; transparent below it.",
            ),
            url: None,
            features: vec![
                features::LIMITER.into(),
                features::MASTERING.into(),
                features::MONO.into(),
            ],
            state_format_version: Self::STATE_FORMAT_VERSION,
            api_version: MODULE_API_VERSION,
        }
    }

    /// The parameter schema (SPEC-017 §3).
    pub fn param_infos() -> Vec<ParamInfo> {
        let db = |id: ParamId, key: &str, name: &str, unit: Unit, range: (f64, f64, f64), ms| {
            ParamInfo {
                id,
                key: key.into(),
                name: LocalizedText::keyed(format!("param.true_peak_limiter.{key}"), name),
                group: None,
                unit,
                min: range.0,
                max: range.1,
                default: range.2,
                taper: Taper::Db {
                    neg_inf_at_min: false,
                },
                step: None,
                enum_labels: Vec::new(),
                decimals: 1,
                smoothing_ms: ms,
                flags: ParamFlags::AUTOMATABLE,
            }
        };
        vec![
            db(
                Self::INPUT_GAIN_DB,
                "input_gain_db",
                "Input gain",
                Unit::Db,
                (-12.0, 24.0, 0.0),
                Self::INPUT_GAIN_SMOOTHING_MS,
            ),
            db(
                Self::CEILING_DBTP,
                "ceiling_dbtp",
                "Ceiling",
                Unit::Dbtp,
                (-12.0, 0.0, -1.0),
                Self::CEILING_SMOOTHING_MS,
            ),
            ParamInfo {
                id: Self::RELEASE_MS,
                key: "release_ms".into(),
                name: LocalizedText::keyed("param.true_peak_limiter.release_ms", "Release"),
                group: None,
                unit: Unit::Ms,
                min: 10.0,
                max: 1000.0,
                default: 100.0,
                taper: Taper::Log,
                step: None,
                enum_labels: Vec::new(),
                decimals: 0,
                smoothing_ms: 0.0,
                flags: ParamFlags::AUTOMATABLE,
            },
            ParamInfo {
                id: Self::LOOKAHEAD_MS,
                key: "lookahead_ms".into(),
                name: LocalizedText::keyed("param.true_peak_limiter.lookahead_ms", "Look-ahead"),
                group: None,
                unit: Unit::Ms,
                min: 1.0,
                max: 10.0,
                default: 5.0,
                taper: Taper::Linear,
                step: Some(0.5),
                enum_labels: Vec::new(),
                decimals: 1,
                smoothing_ms: 0.0,
                flags: ParamFlags::AUTOMATABLE | ParamFlags::STEPPED,
            },
        ]
    }

    /// Telemetry channels (SPEC-017 §3).
    pub fn telemetry_channels() -> Vec<TelemetryInfo> {
        vec![TelemetryInfo {
            id: 0,
            key: "gain_reduction_db".into(),
            name: LocalizedText::keyed(
                "telemetry.true_peak_limiter.gain_reduction_db",
                "Gain reduction",
            ),
            unit: Unit::Db,
            min: -24.0,
            max: 0.0,
            kind: TelemetryKind::GainReduction,
            group: None,
        }]
    }

    /// A state with the given plain values.
    pub fn state(
        input_gain_db: f64,
        ceiling_dbtp: f64,
        release_ms: f64,
        lookahead_ms: f64,
    ) -> ModuleState {
        ModuleState {
            format_version: Self::STATE_FORMAT_VERSION,
            params: BTreeMap::from([
                ("input_gain_db".to_owned(), input_gain_db),
                ("ceiling_dbtp".to_owned(), ceiling_dbtp),
                ("release_ms".to_owned(), release_ms),
                ("lookahead_ms".to_owned(), lookahead_ms),
            ]),
            blob: None,
        }
    }

    fn index_of(id: ParamId) -> Option<usize> {
        match id {
            Self::INPUT_GAIN_DB => Some(INPUT),
            Self::CEILING_DBTP => Some(CEILING),
            Self::RELEASE_MS => Some(RELEASE),
            Self::LOOKAHEAD_MS => Some(LOOKAHEAD),
            _ => None,
        }
    }
}

impl Default for TruePeakLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for TruePeakLimiter {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    fn activate(&mut self, config: &ActivateConfig) -> Result<(), ModuleError> {
        if config.layout != ChannelLayout::MONO {
            return Err(ModuleError::Unsupported(format!(
                "layout {:?}",
                config.layout
            )));
        }
        if !(config.sample_rate.is_finite() && config.sample_rate > 0.0) || config.max_block == 0 {
            return Err(ModuleError::Unsupported(format!("{config:?}")));
        }
        let fs = config.sample_rate;
        self.sample_rate = fs;
        self.active_lookahead_ms = self.values[LOOKAHEAD];
        let l = lookahead_samples(self.values[LOOKAHEAD], fs);
        self.lookahead = l as u64;
        self.core = Some(TruePeakLimiterCore::new(
            l,
            release_coeff(self.values[RELEASE], fs),
            db_to_lin(self.values[CEILING]),
        ));
        self.pending = std::array::from_fn(|_| PendingQueue::with_capacity(Self::PENDING_CAPACITY));
        self.input_gain.len = ramp_samples(fs, Self::INPUT_GAIN_SMOOTHING_MS);
        self.ceiling_db.len = ramp_samples(fs, Self::CEILING_SMOOTHING_MS);
        self.t = 0;
        self.reset();
        Ok(())
    }

    fn deactivate(&mut self) {
        self.core = None;
    }

    fn latency_samples(&self) -> u32 {
        self.core.as_ref().map_or(0, |c| c.latency() as u32)
    }

    fn tail(&self) -> Tail {
        Tail::Samples(0)
    }

    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let frames = ctx.frames as usize;
        let base = self.t;
        let l = self.lookahead;
        let latency = l + REQUIREMENT_DELAY as u64;
        for ev in ctx.events {
            let Some(i) = Self::index_of(ev.id) else {
                continue;
            };
            let v = self.params[i].clamp_quantize(ev.value);
            self.values[i] = v;
            let at = base + u64::from(ev.offset);
            match i {
                INPUT => self.pending[INPUT].push(at + latency, v),
                CEILING => self.pending[CEILING].push(at + l + 1, v),
                RELEASE => self.pending[RELEASE].push(at + latency, v),
                _ => {
                    if (v - self.active_lookahead_ms).abs() > 1e-9 {
                        ctx.request(HostRequest::Restart);
                    }
                }
            }
        }
        let (Some(core), Some(input), Some(output)) =
            (self.core.as_mut(), inputs.first(), outputs.first_mut())
        else {
            if let Some(out) = outputs.first_mut() {
                out.fill(0.0);
            }
            self.t += frames as u64;
            return ProcessStatus::Continue;
        };
        let fs = self.sample_rate;
        let mut min_gain = 1.0_f64;
        for (i, (o, &x)) in output.iter_mut().zip(input.iter()).take(frames).enumerate() {
            let t = base + i as u64;
            while let Some(v) = self.pending[INPUT].pop_due(t) {
                self.input_gain.set_target(db_to_lin(v));
            }
            while let Some(v) = self.pending[CEILING].pop_due(t) {
                self.ceiling_db.set_target(v);
                self.ceiling_lin = db_to_lin(v);
            }
            while let Some(v) = self.pending[RELEASE].pop_due(t) {
                core.set_release_coeff(release_coeff(v, fs));
            }
            let g_in = self.input_gain.next();
            let ceiling = if self.ceiling_db.is_ramping() {
                db_to_lin(self.ceiling_db.next())
            } else {
                self.ceiling_lin
            };
            let (y, g) = core.process(g_in * f64::from(x), ceiling);
            *o = y as f32;
            min_gain = min_gain.min(g);
        }
        self.t += frames as u64;
        let gr_db = if min_gain < 1.0 {
            (20.0 * min_gain.log10()) as f32
        } else {
            0.0
        };
        self.telemetry.write(Self::TELEMETRY_GAIN_REDUCTION, gr_db);
        ProcessStatus::Continue
    }

    fn reset(&mut self) {
        for q in &mut self.pending {
            q.clear();
        }
        self.input_gain.snap(db_to_lin(self.values[INPUT]));
        self.ceiling_db.snap(self.values[CEILING]);
        self.ceiling_lin = db_to_lin(self.values[CEILING]);
        if let Some(core) = self.core.as_mut() {
            core.set_release_coeff(release_coeff(self.values[RELEASE], self.sample_rate));
            core.reset(self.ceiling_lin);
        }
        self.telemetry.write(Self::TELEMETRY_GAIN_REDUCTION, 0.0);
    }

    fn param_value(&self, id: ParamId) -> Option<f64> {
        Self::index_of(id).map(|i| self.values[i])
    }

    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(Self::state(
            self.values[INPUT],
            self.values[CEILING],
            self.values[RELEASE],
            self.values[LOOKAHEAD],
        ))
    }

    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        for (i, p) in self.params.iter().enumerate() {
            self.values[i] =
                p.clamp_quantize(state.params.get(&p.key).copied().unwrap_or(p.default));
        }
        self.active_lookahead_ms = self.values[LOOKAHEAD];
        Ok(())
    }

    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        match id {
            ExtensionId::Telemetry => Some(Extension::Telemetry(self.telemetry.clone())),
            _ => None,
        }
    }
}

/// Factory for [`TruePeakLimiter`].
pub struct TruePeakLimiterFactory {
    descriptor: ModuleDescriptor,
}

impl TruePeakLimiterFactory {
    /// Creates the factory.
    pub fn new() -> Self {
        Self {
            descriptor: TruePeakLimiter::descriptor_value(),
        }
    }
}

impl Default for TruePeakLimiterFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleFactory for TruePeakLimiterFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(TruePeakLimiter::new()))
    }
}
