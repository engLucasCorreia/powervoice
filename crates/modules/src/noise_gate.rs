//! Built-in **Noise Gate** (`org.powervoice.noise-gate@1.0.0`, SPEC-013), Slice 3 subset (S3-02).
//!
//! Per sample: the sidechain `s` is the input, or its 24 dB/oct Butterworth high-pass when
//! `sc_hpf_enabled` (the filter always runs, so re-enabling is warm; the audio never passes
//! through it). A 5 ms sliding peak of `s` drives the SPEC-016 §4.7 gate core: opens at
//! `threshold`, closes after `hold` below `threshold − hysteresis`, raised-cosine opening of
//! exactly `attack`, exponential release toward the range floor. The range floor ramps over
//! 20 ms in linear gain (−100 dB = −∞). Output `y = x · g`; open means `g = 1.0` exactly.
//!
//! **Not yet available in this slice:** the look-ahead (`lookahead_ms` is in the permanent
//! schema, flagged `HIDDEN`, no effect; latency is always 0).

use std::sync::Arc;

use vox_dsp::dynamics::detector::SlidingPeak;
use vox_dsp::dynamics::gate::GateCore;
use vox_dsp::dynamics::ramp::LinearRamp;
use vox_dsp::dynamics::sidechain::ButterworthHighpass4;
use vox_dsp::dynamics::{
    LEVEL_FLOOR_DBFS, PEAK_WINDOW_MS, RAMP_MS, db_to_lin, lin_to_dbfs, ms_to_samples,
    ms_to_samples_or_zero, time_constant_alpha,
};
use vox_module_api::{
    ActivateConfig, ChannelLayout, Extension, ExtensionId, GroupId, Hold, LocalizedText,
    MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError, ModuleFactory, ModulePreset,
    ModuleState, ParamGroup, ParamId, ParamInfo, ProcessContext, ProcessStatus, StateError, Tail,
    Telemetry, TelemetryCells, TelemetryInfo, TelemetryKind, Unit, Version, features, segments,
};

use crate::schema::{self, ParamBuild, param};

const PREFIX: &str = "module.noise_gate";

/// Per-block telemetry accumulators (SPEC-013 §3).
struct BlockStats {
    open: bool,
    min_gain: f64,
    level_dbfs: f64,
}

/// The built-in Noise Gate module.
pub struct NoiseGate {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    /// Plain target values in `params()` order.
    values: Vec<f64>,
    telemetry: Arc<TelemetryCells>,
    sample_rate: f64,
    threshold_db: f64,
    hysteresis_db: f64,
    hpf_enabled: bool,
    hpf: ButterworthHighpass4,
    peak: SlidingPeak,
    peak_max: f64,
    peak_dbfs: f64,
    /// Closed gain (linear), ramped.
    range: LinearRamp,
    gate: GateCore,
}

impl NoiseGate {
    /// Module id (ADR-005 §2).
    pub const ID: &'static str = "org.powervoice.noise-gate";
    /// Module version.
    pub const VERSION: Version = Version::new(1, 0, 0);
    /// State format this build writes.
    pub const STATE_FORMAT_VERSION: u32 = 1;

    /// Opening threshold (dBFS).
    pub const THRESHOLD_DB: ParamId = ParamId(1);
    /// Close threshold = threshold − hysteresis.
    pub const HYSTERESIS_DB: ParamId = ParamId(2);
    /// Raised-cosine opening duration.
    pub const ATTACK_MS: ParamId = ParamId(3);
    /// Hold before the release.
    pub const HOLD_MS: ParamId = ParamId(4);
    /// Release time constant.
    pub const RELEASE_MS: ParamId = ParamId(5);
    /// Closed gain (−100 = −∞).
    pub const RANGE_DB: ParamId = ParamId(6);
    /// Sidechain high-pass switch (group enable).
    pub const SC_HPF_ENABLED: ParamId = ParamId(7);
    /// Sidechain high-pass corner.
    pub const SC_HPF_HZ: ParamId = ParamId(8);
    /// Look-ahead (not yet available: no effect, latency 0).
    pub const LOOKAHEAD_MS: ParamId = ParamId(9);

    /// Sidechain group.
    pub const GROUP_SIDECHAIN: GroupId = GroupId(1);

    /// Telemetry: gate open lamp.
    pub const TELEMETRY_GATE_OPEN: usize = 0;
    /// Telemetry: the block's minimum gate gain in dB (−100 for −∞).
    pub const TELEMETRY_GAIN_DB: usize = 1;
    /// Telemetry: the block's maximum detector level (after the HPF).
    pub const TELEMETRY_SIDECHAIN_LEVEL: usize = 2;

    /// `range_db` at or below this is −∞ (linear 0).
    pub const RANGE_MIN_DB: f64 = -100.0;

    /// A new, inactive instance with default values.
    pub fn new() -> Self {
        let params = Self::schema();
        let values = params.iter().map(|p| p.default).collect();
        let mut m = Self {
            descriptor: Self::descriptor_value(),
            params,
            groups: vec![schema::group(
                PREFIX,
                1,
                "sidechain",
                "Sidechain",
                Some(7),
                false,
            )],
            values,
            telemetry: TelemetryCells::new(
                Self::telemetry_channels(),
                vec![Hold::Max, Hold::Min, Hold::Max],
            ),
            sample_rate: 48_000.0,
            threshold_db: -40.0,
            hysteresis_db: 6.0,
            hpf_enabled: true,
            hpf: ButterworthHighpass4::new(100.0, 48_000.0),
            peak: SlidingPeak::new(1),
            peak_max: 0.0,
            peak_dbfs: LEVEL_FLOOR_DBFS,
            range: LinearRamp::new(0.0),
            gate: GateCore::new(1, 0, 0.0, 0.0),
        };
        m.configure();
        m
    }

    /// The descriptor every instance returns (SPEC-013 §2.1).
    pub fn descriptor_value() -> ModuleDescriptor {
        ModuleDescriptor {
            id: Self::ID.into(),
            version: Self::VERSION,
            name: LocalizedText::keyed("module.noise_gate.name", "Noise Gate"),
            vendor: "PowerVoice".into(),
            description: LocalizedText::keyed(
                "module.noise_gate.description",
                "Turns room noise down between phrases.",
            ),
            url: None,
            features: vec![
                features::AUDIO_EFFECT.into(),
                features::GATE.into(),
                features::MONO.into(),
            ],
            state_format_version: Self::STATE_FORMAT_VERSION,
            api_version: MODULE_API_VERSION,
        }
    }

    /// Parameter schema (SPEC-013 §3).
    pub fn schema() -> Vec<ParamInfo> {
        let p = |id, key, name| param(PREFIX, id, key, name);
        vec![
            p(1, "threshold_db", "Threshold").db(Unit::Dbfs, -80.0, 0.0, -40.0),
            p(2, "hysteresis_db", "Hysteresis").db(Unit::Db, 0.0, 20.0, 6.0),
            p(3, "attack_ms", "Attack").log(Unit::Ms, 0.1, 100.0, 2.0),
            p(4, "hold_ms", "Hold").log(Unit::Ms, 0.1, 1000.0, 50.0),
            p(5, "release_ms", "Release").log(Unit::Ms, 1.0, 2000.0, 100.0),
            p(6, "range_db", "Range")
                .db(Unit::Db, Self::RANGE_MIN_DB, 0.0, -30.0)
                .neg_inf_at_min()
                .smoothing(20.0),
            p(7, "sc_hpf_enabled", "Sidechain HPF")
                .toggle(true)
                .in_group(1),
            p(8, "sc_hpf_hz", "Frequency")
                .log(Unit::Hz, 20.0, 2000.0, 100.0)
                .decimals(0)
                .in_group(1),
            p(9, "lookahead_ms", "Look-ahead")
                .stepped(Unit::Ms, 0.0, 20.0, 0.0, 1.0)
                .decimals(0)
                .hidden(),
        ]
    }

    /// Telemetry channels (SPEC-013 §3).
    pub fn telemetry_channels() -> Vec<TelemetryInfo> {
        vec![
            schema::telemetry(
                PREFIX,
                0,
                "gate_open",
                "Gate open",
                TelemetryKind::Indicator,
                None,
            ),
            TelemetryInfo {
                min: -100.0,
                ..schema::telemetry(
                    PREFIX,
                    1,
                    "gain_db",
                    "Gate gain",
                    TelemetryKind::GainReduction,
                    None,
                )
            },
            schema::telemetry(
                PREFIX,
                2,
                "sidechain_level_dbfs",
                "Sidechain level",
                TelemetryKind::Level,
                Some(1),
            ),
        ]
    }

    /// Closed gain for `range_db` (−100 → 0.0, i.e. −∞).
    pub fn range_gain(range_db: f64) -> f64 {
        if range_db <= Self::RANGE_MIN_DB {
            0.0
        } else {
            db_to_lin(range_db)
        }
    }

    fn apply(&mut self, id: ParamId, v: f64) {
        let fs = self.sample_rate;
        match id {
            Self::THRESHOLD_DB => self.threshold_db = v,
            Self::HYSTERESIS_DB => self.hysteresis_db = v,
            Self::ATTACK_MS => self
                .gate
                .set_attack(u32::try_from(ms_to_samples(v, fs)).unwrap_or(u32::MAX)),
            Self::HOLD_MS => self.gate.set_hold(ms_to_samples_or_zero(v, fs)),
            Self::RELEASE_MS => self.gate.set_release(time_constant_alpha(v, fs)),
            Self::RANGE_DB => self.range.set_target(Self::range_gain(v)),
            Self::SC_HPF_ENABLED => self.hpf_enabled = v >= 0.5,
            Self::SC_HPF_HZ => self.hpf.set_freq(v, fs),
            // Look-ahead: stored only (not yet available).
            _ => {}
        }
    }

    fn set_param(&mut self, id: ParamId, value: f64) {
        if let Some(i) = schema::index_of(&self.params, id) {
            let v = self.params[i].clamp_quantize(value);
            self.values[i] = v;
            self.apply(id, v);
        }
    }

    fn configure(&mut self) {
        for i in 0..self.params.len() {
            let (id, v) = (self.params[i].id, self.values[i]);
            self.apply(id, v);
        }
        self.range.snap();
    }

    fn write_telemetry(&self, s: &BlockStats) {
        let t = &self.telemetry;
        t.write(Self::TELEMETRY_GATE_OPEN, if s.open { 1.0 } else { 0.0 });
        let gain_db = if s.min_gain > 0.0 {
            (20.0 * s.min_gain.log10()).clamp(-100.0, 0.0)
        } else {
            -100.0
        };
        t.write(Self::TELEMETRY_GAIN_DB, gain_db as f32);
        t.write(
            Self::TELEMETRY_SIDECHAIN_LEVEL,
            s.level_dbfs.clamp(-100.0, 6.0) as f32,
        );
    }

    fn tick(&mut self, x: f32, stats: &mut BlockStats) -> f32 {
        let s = f64::from(x);
        let filtered = self.hpf.process(s);
        let detect = if self.hpf_enabled { filtered } else { s };
        let pk = self.peak.push(detect.abs());
        if pk.to_bits() != self.peak_max.to_bits() {
            self.peak_max = pk;
            self.peak_dbfs = lin_to_dbfs(pk);
        }
        let floor = self.range.advance();
        let g = self.gate.process(
            self.peak_dbfs,
            self.threshold_db,
            self.threshold_db - self.hysteresis_db,
            floor,
        );
        stats.open |= self.gate.is_open();
        stats.min_gain = stats.min_gain.min(g);
        stats.level_dbfs = stats.level_dbfs.max(self.peak_dbfs);
        x * (g as f32)
    }
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for NoiseGate {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn params(&self) -> &[ParamInfo] {
        &self.params
    }

    fn groups(&self) -> &[ParamGroup] {
        &self.groups
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
        self.peak = SlidingPeak::new(ms_to_samples(PEAK_WINDOW_MS, fs));
        self.range
            .set_len(u32::try_from(ms_to_samples(RAMP_MS, fs)).unwrap_or(u32::MAX));
        self.configure();
        self.reset();
        Ok(())
    }

    fn deactivate(&mut self) {}

    fn latency_samples(&self) -> u32 {
        0
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
        let (Some(input), Some(output)) = (inputs.first(), outputs.first_mut()) else {
            return ProcessStatus::Continue;
        };
        let mut stats = BlockStats {
            open: false,
            min_gain: f64::INFINITY,
            level_dbfs: LEVEL_FLOOR_DBFS,
        };
        for seg in segments(ctx.frames, ctx.events) {
            for ev in seg.events {
                self.set_param(ev.id, ev.value);
            }
            let r = seg.start as usize..(seg.start + seg.len) as usize;
            let (Some(out), Some(inp)) = (output.get_mut(r.clone()), input.get(r)) else {
                continue;
            };
            for (o, &x) in out.iter_mut().zip(inp) {
                *o = self.tick(x, &mut stats);
            }
        }
        if ctx.frames > 0 {
            self.write_telemetry(&stats);
        }
        ProcessStatus::Continue
    }

    fn reset(&mut self) {
        self.range.snap();
        let floor = self.range.target();
        self.gate.reset(floor);
        self.hpf.reset();
        self.peak.reset();
        self.peak_max = 0.0;
        self.peak_dbfs = LEVEL_FLOOR_DBFS;
        self.write_telemetry(&BlockStats {
            open: false,
            min_gain: floor,
            level_dbfs: LEVEL_FLOOR_DBFS,
        });
    }

    fn param_value(&self, id: ParamId) -> Option<f64> {
        schema::index_of(&self.params, id).map(|i| self.values[i])
    }

    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(schema::save_values(
            &self.params,
            &self.values,
            Self::STATE_FORMAT_VERSION,
        ))
    }

    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        schema::load_values(&self.params, state, &mut self.values);
        self.configure();
        Ok(())
    }

    fn extension(&self, id: ExtensionId) -> Option<Extension> {
        match id {
            ExtensionId::Telemetry => {
                let t: Arc<dyn Telemetry> = self.telemetry.clone();
                Some(Extension::Telemetry(t))
            }
            _ => None,
        }
    }
}

/// Factory for [`NoiseGate`].
pub struct NoiseGateFactory {
    descriptor: ModuleDescriptor,
}

impl NoiseGateFactory {
    /// Creates the factory.
    pub fn new() -> Self {
        Self {
            descriptor: NoiseGate::descriptor_value(),
        }
    }
}

impl Default for NoiseGateFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleFactory for NoiseGateFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(NoiseGate::new()))
    }

    /// Factory presets (T-406; SPEC-013 §7 leaves the exact values to this ticket).
    fn presets(&self) -> Vec<ModulePreset> {
        let params = NoiseGate::schema();
        let state = |overrides: &[(&str, f64)]| {
            schema::preset_state(&params, NoiseGate::STATE_FORMAT_VERSION, overrides)
        };
        vec![
            ModulePreset {
                key: "light".into(),
                name: LocalizedText::keyed("module.noise_gate.preset.light", "Light"),
                state: state(&[
                    ("threshold_db", -50.0),
                    ("hysteresis_db", 4.0),
                    ("range_db", -18.0),
                ]),
            },
            ModulePreset {
                key: "aggressive".into(),
                name: LocalizedText::keyed("module.noise_gate.preset.aggressive", "Aggressive"),
                state: state(&[
                    ("threshold_db", -35.0),
                    ("hysteresis_db", 8.0),
                    ("range_db", -60.0),
                ]),
            },
        ]
    }
}
