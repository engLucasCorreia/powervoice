//! Built-in **Dynamics** (`org.powervoice.dynamics@1.0.0`, SPEC-016), Slice 3 subset (S3-02).
//!
//! Per sample (SPEC-016 §4.5): one peak detector (5 ms sliding maximum) and one RMS detector
//! (20 ms rectangular window) on the input. The compressor sees the detection-mode level (the
//! Peak/RMS crossfade), its static gain goes through attack/release ballistics in dB, and makeup
//! follows it. The limiter sees the peak level plus the compressor's effective gain and makeup.
//! Every section enable crossfades its contribution over 20 ms; thresholds, knee, ratio (as
//! `1 − 1/R`) and makeup ramp linearly over 20 ms. Output `y = x · g`.
//!
//! **Not yet available in this slice:** the AutoGate and Expander sections (T-407) and the
//! look-ahead. Their parameters are part of the permanent schema (ids, keys, state) but are
//! flagged `HIDDEN` and have no effect; latency is always 0.

use std::sync::Arc;

use vox_dsp::dynamics::ballistics::{AttackDirection, Ballistics};
use vox_dsp::dynamics::curves::{compressor_gain_db, compressor_slope, limiter_gain_db};
use vox_dsp::dynamics::detector::{SlidingPeak, SlidingRms, mean_square_to_db};
use vox_dsp::dynamics::ramp::LinearRamp;
use vox_dsp::dynamics::{
    LEVEL_FLOOR_DBFS, PEAK_WINDOW_MS, RAMP_MS, RMS_WINDOW_MS, db_to_lin, lin_to_dbfs,
    ms_to_samples, time_constant_alpha,
};
use vox_module_api::{
    ActivateConfig, ChannelLayout, Extension, ExtensionId, GroupId, Hold, LocalizedText,
    MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError, ModuleFactory, ModulePreset,
    ModuleState, ParamGroup, ParamId, ParamInfo, ProcessContext, ProcessStatus, StateError, Tail,
    Telemetry, TelemetryCells, TelemetryInfo, TelemetryKind, Unit, Version, features, segments,
};

use crate::schema::{self, ParamBuild, param, switch};

const PREFIX: &str = "module.dynamics";

/// Per-block telemetry accumulators (SPEC-016 §4.10).
struct BlockStats {
    total_db: f64,
    compressor_db: f64,
    limiter_db: f64,
    level_dbfs: f64,
}

impl BlockStats {
    /// Also the initial telemetry: GR 0, level −100 (clamped floor), lamp 0.
    fn new() -> Self {
        Self {
            total_db: 0.0,
            compressor_db: 0.0,
            limiter_db: 0.0,
            level_dbfs: LEVEL_FLOOR_DBFS,
        }
    }
}

/// One dB-domain section: enable weight, threshold ramp, ballistics.
struct Section {
    weight: LinearRamp,
    threshold: LinearRamp,
    ballistics: Ballistics,
}

impl Section {
    fn new() -> Self {
        Self {
            weight: LinearRamp::new(0.0),
            threshold: LinearRamp::new(0.0),
            ballistics: Ballistics::new(AttackDirection::Down, 0.0, 0.0),
        }
    }
}

/// The built-in Dynamics module (compressor + limiter live in S3-02).
pub struct Dynamics {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    /// Plain target values in `params()` order.
    values: Vec<f64>,
    telemetry: Arc<TelemetryCells>,
    sample_rate: f64,
    peak: SlidingPeak,
    rms: SlidingRms,
    peak_max: f64,
    peak_dbfs: f64,
    /// Detection weight: 0 = Peak, 1 = RMS.
    detection: LinearRamp,
    knee: LinearRamp,
    compressor: Section,
    slope: LinearRamp,
    makeup: LinearRamp,
    limiter: Section,
}

impl Dynamics {
    /// Module id.
    pub const ID: &'static str = "org.powervoice.dynamics";
    /// Module version.
    pub const VERSION: Version = Version::new(1, 0, 0);
    /// State format this build writes.
    pub const STATE_FORMAT_VERSION: u32 = 1;

    /// Peak / RMS (enum 0 / 1).
    pub const DETECTION: ParamId = ParamId(1);
    /// Knee width in dB (0 = hard).
    pub const KNEE_DB: ParamId = ParamId(2);
    /// Look-ahead (not yet available: no effect, latency 0).
    pub const LOOKAHEAD_MS: ParamId = ParamId(3);
    /// AutoGate enable (not yet available).
    pub const AUTOGATE_ENABLED: ParamId = ParamId(10);
    /// AutoGate threshold (not yet available).
    pub const AUTOGATE_THRESHOLD_DB: ParamId = ParamId(11);
    /// AutoGate attack (not yet available).
    pub const AUTOGATE_ATTACK_MS: ParamId = ParamId(12);
    /// AutoGate hold (not yet available).
    pub const AUTOGATE_HOLD_MS: ParamId = ParamId(13);
    /// AutoGate release (not yet available).
    pub const AUTOGATE_RELEASE_MS: ParamId = ParamId(14);
    /// Expander enable (not yet available).
    pub const EXPANDER_ENABLED: ParamId = ParamId(20);
    /// Expander threshold (not yet available).
    pub const EXPANDER_THRESHOLD_DB: ParamId = ParamId(21);
    /// Expander ratio (not yet available).
    pub const EXPANDER_RATIO: ParamId = ParamId(22);
    /// Expander attack (not yet available).
    pub const EXPANDER_ATTACK_MS: ParamId = ParamId(23);
    /// Expander release (not yet available).
    pub const EXPANDER_RELEASE_MS: ParamId = ParamId(24);
    /// Compressor enable.
    pub const COMPRESSOR_ENABLED: ParamId = ParamId(30);
    /// Compressor threshold (dBFS).
    pub const COMPRESSOR_THRESHOLD_DB: ParamId = ParamId(31);
    /// Compressor ratio.
    pub const COMPRESSOR_RATIO: ParamId = ParamId(32);
    /// Compressor attack time constant.
    pub const COMPRESSOR_ATTACK_MS: ParamId = ParamId(33);
    /// Compressor release time constant.
    pub const COMPRESSOR_RELEASE_MS: ParamId = ParamId(34);
    /// Makeup gain (belongs to the compressor).
    pub const COMPRESSOR_MAKEUP_DB: ParamId = ParamId(35);
    /// Limiter enable.
    pub const LIMITER_ENABLED: ParamId = ParamId(40);
    /// Limiter threshold (sample-peak ceiling, dBFS).
    pub const LIMITER_THRESHOLD_DB: ParamId = ParamId(41);
    /// Limiter attack time constant.
    pub const LIMITER_ATTACK_MS: ParamId = ParamId(42);
    /// Limiter release time constant.
    pub const LIMITER_RELEASE_MS: ParamId = ParamId(43);

    /// Parameters that have no effect in this slice (flagged `HIDDEN`).
    pub const NOT_YET_AVAILABLE: [ParamId; 11] = [
        Self::LOOKAHEAD_MS,
        Self::AUTOGATE_ENABLED,
        Self::AUTOGATE_THRESHOLD_DB,
        Self::AUTOGATE_ATTACK_MS,
        Self::AUTOGATE_HOLD_MS,
        Self::AUTOGATE_RELEASE_MS,
        Self::EXPANDER_ENABLED,
        Self::EXPANDER_THRESHOLD_DB,
        Self::EXPANDER_RATIO,
        Self::EXPANDER_ATTACK_MS,
        Self::EXPANDER_RELEASE_MS,
    ];

    /// AutoGate group.
    pub const GROUP_AUTOGATE: GroupId = GroupId(1);
    /// Expander group.
    pub const GROUP_EXPANDER: GroupId = GroupId(2);
    /// Compressor group.
    pub const GROUP_COMPRESSOR: GroupId = GroupId(3);
    /// Limiter group.
    pub const GROUP_LIMITER: GroupId = GroupId(4);

    /// Telemetry: total gain reduction (dB, makeup excluded).
    pub const TELEMETRY_GR_TOTAL: usize = 0;
    /// Telemetry: AutoGate gain reduction (0 in this slice).
    pub const TELEMETRY_GR_AUTOGATE: usize = 1;
    /// Telemetry: Expander gain reduction (0 in this slice).
    pub const TELEMETRY_GR_EXPANDER: usize = 2;
    /// Telemetry: compressor gain reduction.
    pub const TELEMETRY_GR_COMPRESSOR: usize = 3;
    /// Telemetry: limiter gain reduction.
    pub const TELEMETRY_GR_LIMITER: usize = 4;
    /// Telemetry: input peak level (dBFS).
    pub const TELEMETRY_INPUT_LEVEL: usize = 5;
    /// Telemetry: AutoGate open lamp (0 in this slice).
    pub const TELEMETRY_AUTOGATE_OPEN: usize = 6;

    /// A new, inactive instance with default values.
    pub fn new() -> Self {
        let params = Self::schema();
        let values = params.iter().map(|p| p.default).collect();
        let mut m = Self {
            descriptor: Self::descriptor_value(),
            params,
            groups: Self::groups_value(),
            values,
            telemetry: TelemetryCells::new(
                Self::telemetry_channels(),
                vec![
                    Hold::Min,
                    Hold::Min,
                    Hold::Min,
                    Hold::Min,
                    Hold::Min,
                    Hold::Max,
                    Hold::Max,
                ],
            ),
            sample_rate: 48_000.0,
            peak: SlidingPeak::new(1),
            rms: SlidingRms::new(1),
            peak_max: 0.0,
            peak_dbfs: LEVEL_FLOOR_DBFS,
            detection: LinearRamp::new(0.0),
            knee: LinearRamp::new(0.0),
            compressor: Section::new(),
            slope: LinearRamp::new(0.0),
            makeup: LinearRamp::new(0.0),
            limiter: Section::new(),
        };
        m.configure();
        m
    }

    /// The descriptor every instance returns (SPEC-016 §2.1).
    pub fn descriptor_value() -> ModuleDescriptor {
        ModuleDescriptor {
            id: Self::ID.into(),
            version: Self::VERSION,
            name: LocalizedText::keyed("module.dynamics.name", "Dynamics"),
            vendor: "PowerVoice".into(),
            description: LocalizedText::keyed(
                "module.dynamics.description",
                "Compressor and limiter for voice levels.",
            ),
            url: None,
            features: vec![
                features::AUDIO_EFFECT.into(),
                features::COMPRESSOR.into(),
                features::EXPANDER.into(),
                features::GATE.into(),
                features::LIMITER.into(),
                features::MONO.into(),
            ],
            state_format_version: Self::STATE_FORMAT_VERSION,
            api_version: MODULE_API_VERSION,
        }
    }

    /// Parameter schema (SPEC-016 §3).
    pub fn schema() -> Vec<ParamInfo> {
        let p = |id, key, name| param(PREFIX, id, key, name);
        let ms = |id, key, name, group, min, max, default| {
            param(PREFIX, id, key, name)
                .log(Unit::Ms, min, max, default)
                .in_group(group)
        };
        vec![
            p(1, "detection", "Detection")
                .choice(&[("peak", "Peak"), ("rms", "RMS")], 1.0)
                .smoothing(20.0),
            p(2, "knee_db", "Knee")
                .db(Unit::Db, 0.0, 20.0, 6.0)
                .smoothing(20.0),
            p(3, "lookahead_ms", "Look-ahead")
                .stepped(Unit::Ms, 0.0, 20.0, 0.0, 1.0)
                .decimals(0)
                .hidden(),
            p(10, "autogate_enabled", "AutoGate")
                .toggle(false)
                .in_group(1)
                .smoothing(20.0)
                .hidden(),
            p(11, "autogate_threshold_db", "Threshold")
                .db(Unit::Dbfs, -80.0, 0.0, -50.0)
                .in_group(1)
                .hidden(),
            ms(12, "autogate_attack_ms", "Attack", 1, 0.1, 100.0, 2.0).hidden(),
            ms(13, "autogate_hold_ms", "Hold", 1, 0.1, 1000.0, 50.0).hidden(),
            ms(14, "autogate_release_ms", "Release", 1, 1.0, 2000.0, 100.0).hidden(),
            p(20, "expander_enabled", "Expander")
                .toggle(false)
                .in_group(2)
                .smoothing(20.0)
                .hidden(),
            p(21, "expander_threshold_db", "Threshold")
                .db(Unit::Dbfs, -80.0, 0.0, -45.0)
                .in_group(2)
                .smoothing(20.0)
                .hidden(),
            p(22, "expander_ratio", "Ratio")
                .log(Unit::Ratio, 1.0, 30.0, 2.0)
                .in_group(2)
                .smoothing(20.0)
                .hidden(),
            ms(23, "expander_attack_ms", "Attack", 2, 0.1, 100.0, 2.0).hidden(),
            ms(24, "expander_release_ms", "Release", 2, 1.0, 2000.0, 100.0).hidden(),
            p(30, "compressor_enabled", "Compressor")
                .toggle(true)
                .in_group(3)
                .smoothing(20.0),
            p(31, "compressor_threshold_db", "Threshold")
                .db(Unit::Dbfs, -60.0, 0.0, -20.0)
                .in_group(3)
                .smoothing(20.0),
            p(32, "compressor_ratio", "Ratio")
                .log(Unit::Ratio, 1.0, 30.0, 3.0)
                .in_group(3)
                .smoothing(20.0),
            ms(33, "compressor_attack_ms", "Attack", 3, 0.1, 200.0, 10.0),
            ms(
                34,
                "compressor_release_ms",
                "Release",
                3,
                1.0,
                2000.0,
                100.0,
            ),
            p(35, "compressor_makeup_db", "Makeup")
                .db(Unit::Db, 0.0, 30.0, 0.0)
                .in_group(3)
                .smoothing(20.0),
            p(40, "limiter_enabled", "Limiter")
                .toggle(false)
                .in_group(4)
                .smoothing(20.0),
            p(41, "limiter_threshold_db", "Threshold")
                .db(Unit::Dbfs, -30.0, 0.0, -1.0)
                .in_group(4)
                .smoothing(20.0),
            ms(42, "limiter_attack_ms", "Attack", 4, 0.1, 50.0, 1.0),
            ms(43, "limiter_release_ms", "Release", 4, 1.0, 2000.0, 100.0),
        ]
    }

    /// Groups (SPEC-016 §3), in processing order.
    pub fn groups_value() -> Vec<ParamGroup> {
        vec![
            schema::group(PREFIX, 1, "autogate", "AutoGate", Some(10), true),
            schema::group(PREFIX, 2, "expander", "Expander", Some(20), true),
            schema::group(PREFIX, 3, "compressor", "Compressor", Some(30), false),
            schema::group(PREFIX, 4, "limiter", "Limiter", Some(40), true),
        ]
    }

    /// Telemetry channels (SPEC-016 §3).
    pub fn telemetry_channels() -> Vec<TelemetryInfo> {
        use TelemetryKind::{GainReduction, Indicator, Level};
        let t = |id, key, name, kind, group| schema::telemetry(PREFIX, id, key, name, kind, group);
        vec![
            t(0, "gr_total_db", "Gain reduction", GainReduction, None),
            t(1, "gr_autogate_db", "AutoGate GR", GainReduction, Some(1)),
            t(2, "gr_expander_db", "Expander GR", GainReduction, Some(2)),
            t(
                3,
                "gr_compressor_db",
                "Compressor GR",
                GainReduction,
                Some(3),
            ),
            t(4, "gr_limiter_db", "Limiter GR", GainReduction, Some(4)),
            t(5, "input_level_dbfs", "Input level", Level, None),
            t(6, "autogate_open", "Gate open", Indicator, Some(1)),
        ]
    }

    fn ramps(&mut self) -> [&mut LinearRamp; 8] {
        [
            &mut self.detection,
            &mut self.knee,
            &mut self.compressor.weight,
            &mut self.compressor.threshold,
            &mut self.slope,
            &mut self.makeup,
            &mut self.limiter.weight,
            &mut self.limiter.threshold,
        ]
    }

    /// Routes one plain value to the DSP (starting a ramp where the parameter is ramped).
    fn apply(&mut self, id: ParamId, v: f64) {
        let fs = self.sample_rate;
        match id {
            Self::DETECTION => self.detection.set_target(switch(v)),
            Self::KNEE_DB => self.knee.set_target(v),
            Self::COMPRESSOR_ENABLED => self.compressor.weight.set_target(switch(v)),
            Self::COMPRESSOR_THRESHOLD_DB => self.compressor.threshold.set_target(v),
            Self::COMPRESSOR_RATIO => self.slope.set_target(compressor_slope(v)),
            Self::COMPRESSOR_ATTACK_MS => self
                .compressor
                .ballistics
                .set_attack(time_constant_alpha(v, fs)),
            Self::COMPRESSOR_RELEASE_MS => self
                .compressor
                .ballistics
                .set_release(time_constant_alpha(v, fs)),
            Self::COMPRESSOR_MAKEUP_DB => self.makeup.set_target(v),
            Self::LIMITER_ENABLED => self.limiter.weight.set_target(switch(v)),
            Self::LIMITER_THRESHOLD_DB => self.limiter.threshold.set_target(v),
            Self::LIMITER_ATTACK_MS => self
                .limiter
                .ballistics
                .set_attack(time_constant_alpha(v, fs)),
            Self::LIMITER_RELEASE_MS => self
                .limiter
                .ballistics
                .set_release(time_constant_alpha(v, fs)),
            // Look-ahead, AutoGate, Expander: stored only (not yet available).
            _ => {}
        }
    }

    /// A parameter event: store the clamp-quantized value, route it. RT-safe.
    fn set_param(&mut self, id: ParamId, value: f64) {
        if let Some(i) = schema::index_of(&self.params, id) {
            let v = self.params[i].clamp_quantize(value);
            self.values[i] = v;
            self.apply(id, v);
        }
    }

    /// Every value routed, ramps at their targets.
    fn configure(&mut self) {
        for i in 0..self.params.len() {
            let (id, v) = (self.params[i].id, self.values[i]);
            self.apply(id, v);
        }
        for r in self.ramps() {
            r.snap();
        }
    }

    fn write_telemetry(&self, s: &BlockStats) {
        let t = &self.telemetry;
        let gr = |db: f64| db.clamp(-60.0, 0.0) as f32;
        t.write(Self::TELEMETRY_GR_TOTAL, gr(s.total_db));
        t.write(Self::TELEMETRY_GR_AUTOGATE, 0.0);
        t.write(Self::TELEMETRY_GR_EXPANDER, 0.0);
        t.write(Self::TELEMETRY_GR_COMPRESSOR, gr(s.compressor_db));
        t.write(Self::TELEMETRY_GR_LIMITER, gr(s.limiter_db));
        t.write(
            Self::TELEMETRY_INPUT_LEVEL,
            s.level_dbfs.clamp(-100.0, 6.0) as f32,
        );
        t.write(Self::TELEMETRY_AUTOGATE_OPEN, 0.0);
    }

    /// One output sample (SPEC-016 §4.5 with the AutoGate and Expander at 0 dB).
    fn tick(&mut self, x: f32, stats: &mut BlockStats) -> f32 {
        let s = f64::from(x);
        let pk = self.peak.push(s.abs());
        if pk.to_bits() != self.peak_max.to_bits() {
            self.peak_max = pk;
            self.peak_dbfs = lin_to_dbfs(pk);
        }
        let level_pk = self.peak_dbfs;
        let level_rms = mean_square_to_db(self.rms.push(s));
        let w_det = self.detection.advance();
        let level_mode = (1.0 - w_det) * level_pk + w_det * level_rms;
        let knee = self.knee.advance();

        let w_co = self.compressor.weight.advance();
        let t_co = self.compressor.threshold.advance();
        let slope = self.slope.advance();
        let makeup = self.makeup.advance();
        let g_co = self
            .compressor
            .ballistics
            .process(compressor_gain_db(level_mode, t_co, slope, knee));
        let g_co_eff = w_co * g_co;
        let makeup_eff = w_co * makeup;

        let w_li = self.limiter.weight.advance();
        let t_li = self.limiter.threshold.advance();
        let level_li = (level_pk + g_co_eff + makeup_eff).max(LEVEL_FLOOR_DBFS);
        let g_li = self
            .limiter
            .ballistics
            .process(limiter_gain_db(level_li, t_li));
        let g_li_eff = w_li * g_li;

        let reduction = g_co_eff + g_li_eff;
        stats.total_db = stats.total_db.min(reduction);
        stats.compressor_db = stats.compressor_db.min(g_co_eff);
        stats.limiter_db = stats.limiter_db.min(g_li_eff);
        stats.level_dbfs = stats.level_dbfs.max(level_pk);
        // All weights 0 and no ramp running: exactly 10^0 = 1.0, a bit-exact passthrough.
        let g = db_to_lin(reduction + makeup_eff);
        x * (g as f32)
    }
}

impl Default for Dynamics {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for Dynamics {
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
        self.rms = SlidingRms::new(ms_to_samples(RMS_WINDOW_MS, fs));
        let ramp = u32::try_from(ms_to_samples(RAMP_MS, fs)).unwrap_or(u32::MAX);
        for r in self.ramps() {
            r.set_len(ramp);
        }
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
        let mut stats = BlockStats::new();
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
        for r in self.ramps() {
            r.snap();
        }
        self.peak.reset();
        self.rms.reset();
        self.peak_max = 0.0;
        self.peak_dbfs = LEVEL_FLOOR_DBFS;
        self.compressor.ballistics.reset();
        self.limiter.ballistics.reset();
        self.write_telemetry(&BlockStats::new());
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

/// Factory for [`Dynamics`].
pub struct DynamicsFactory {
    descriptor: ModuleDescriptor,
}

impl DynamicsFactory {
    /// Creates the factory.
    pub fn new() -> Self {
        Self {
            descriptor: Dynamics::descriptor_value(),
        }
    }
}

impl Default for DynamicsFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ModuleFactory for DynamicsFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(Dynamics::new()))
    }

    /// Factory presets (T-406; SPEC-016 §7 leaves the exact values to this ticket). The limiter
    /// section stays off (the dedicated True-Peak Limiter module owns the final ceiling).
    fn presets(&self) -> Vec<ModulePreset> {
        let params = Dynamics::schema();
        let state = |overrides: &[(&str, f64)]| {
            schema::preset_state(&params, Dynamics::STATE_FORMAT_VERSION, overrides)
        };
        vec![
            ModulePreset {
                key: "gentle_voice".into(),
                name: LocalizedText::keyed("module.dynamics.preset.gentle_voice", "Gentle voice"),
                state: state(&[
                    ("compressor_enabled", 1.0),
                    ("compressor_threshold_db", -22.0),
                    ("compressor_ratio", 2.0),
                    ("compressor_attack_ms", 15.0),
                    ("compressor_release_ms", 150.0),
                    ("compressor_makeup_db", 2.0),
                ]),
            },
            ModulePreset {
                key: "podcast".into(),
                name: LocalizedText::keyed("module.dynamics.preset.podcast", "Podcast"),
                state: state(&[
                    ("compressor_enabled", 1.0),
                    ("compressor_threshold_db", -20.0),
                    ("compressor_ratio", 3.5),
                    ("compressor_attack_ms", 10.0),
                    ("compressor_release_ms", 120.0),
                    ("compressor_makeup_db", 3.0),
                ]),
            },
            ModulePreset {
                key: "broadcast_punch".into(),
                name: LocalizedText::keyed(
                    "module.dynamics.preset.broadcast_punch",
                    "Broadcast punch",
                ),
                state: state(&[
                    ("compressor_enabled", 1.0),
                    ("compressor_threshold_db", -18.0),
                    ("compressor_ratio", 4.0),
                    ("compressor_attack_ms", 5.0),
                    ("compressor_release_ms", 80.0),
                    ("compressor_makeup_db", 4.0),
                ]),
            },
        ]
    }
}
