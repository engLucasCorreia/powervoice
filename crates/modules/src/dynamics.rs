//! Built-in **Dynamics** (`org.powervoice.dynamics@1.0.0`, SPEC-016).
//!
//! Four sections in processing order — **AutoGate → Expander → Compressor (+ makeup) → Limiter**
//! — driven by **one** detector on the module input (SPEC-016 §2.2, §4.5): a sliding maximum over
//! `W_pk + la` samples (peak; it reaches `la` samples ahead of the output sample) and a 20 ms
//! rectangular RMS window. Each section computes its gain from the level at *its own* input: the
//! detector level plus the gains of the sections before it. The AutoGate and the Limiter always
//! use the peak level; the Expander and the Compressor use the Peak/RMS crossfade.
//!
//! The AutoGate is the shared gate core (§4.7: 3 dB hysteresis, hold, raised-cosine opening,
//! exponential release to −∞). The Expander, Compressor and Limiter run their static gain
//! computer through attack/release ballistics in dB. Every section enable crossfades its
//! contribution over 20 ms; thresholds, knee, ratios (as `R − 1` and `1 − 1/R`) and makeup ramp
//! linearly over 20 ms. Output `y = x[n − la] · g`.
//!
//! **Look-ahead** (§4.9) delays the audio by `la = round(lookahead_ms · fs / 1000)` samples and is
//! the module's reported latency; it is fixed while the instance is active, so a `lookahead_ms`
//! event keeps the current delay and asks the host for a replacement
//! ([`HostRequest::Restart`], once per changed value).

use std::sync::Arc;

use vox_dsp::dynamics::ballistics::{AttackDirection, Ballistics};
use vox_dsp::dynamics::curves::{
    compressor_gain_db, compressor_slope, expander_gain_db, limiter_gain_db,
};
use vox_dsp::dynamics::detector::{DelayLine, SlidingPeak, SlidingRms, mean_square_to_db};
use vox_dsp::dynamics::gate::GateCore;
use vox_dsp::dynamics::ramp::LinearRamp;
use vox_dsp::dynamics::{
    AUTOGATE_HYSTERESIS_DB, LEVEL_FLOOR_DBFS, PEAK_WINDOW_MS, RAMP_MS, RMS_WINDOW_MS,
    SINE_PEAK_TO_RMS_DB, db_to_lin, lin_to_dbfs, ms_to_samples, ms_to_samples_or_zero,
    time_constant_alpha,
};
use vox_module_api::{
    ActivateConfig, ChannelLayout, CurveBranch, Extension, ExtensionId, GroupId, Hold, HostRequest,
    LocalizedText, MODULE_API_VERSION, Module, ModuleDescriptor, ModuleError, ModuleFactory,
    ModulePreset, ModuleState, ParamGroup, ParamId, ParamInfo, ProcessContext, ProcessStatus,
    StateError, Tail, Telemetry, TelemetryCells, TelemetryInfo, TelemetryKind, TransferCurve,
    TransferHandle, Unit, Version, features, segments,
};

use crate::schema::{self, ParamBuild, param, switch};

const PREFIX: &str = "module.dynamics";

/// Per-block telemetry accumulators (SPEC-016 §4.10).
struct BlockStats {
    total_db: f64,
    autogate_db: f64,
    expander_db: f64,
    compressor_db: f64,
    limiter_db: f64,
    level_dbfs: f64,
    autogate_open: bool,
}

impl BlockStats {
    /// Also the initial telemetry: GR 0, level −100 (clamped floor), lamp 0.
    fn new() -> Self {
        Self {
            total_db: 0.0,
            autogate_db: 0.0,
            expander_db: 0.0,
            compressor_db: 0.0,
            limiter_db: 0.0,
            level_dbfs: LEVEL_FLOOR_DBFS,
            autogate_open: false,
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
    /// `direction` is the way the gain moves when the level rises (§4.6).
    fn new(direction: AttackDirection) -> Self {
        Self {
            weight: LinearRamp::new(0.0),
            threshold: LinearRamp::new(0.0),
            ballistics: Ballistics::new(direction, 0.0, 0.0),
        }
    }
}

/// The AutoGate section: enable weight, the shared gate core and its (unramped) threshold.
struct AutoGate {
    weight: LinearRamp,
    gate: GateCore,
    threshold_db: f64,
}

impl AutoGate {
    fn new() -> Self {
        Self {
            weight: LinearRamp::new(0.0),
            gate: GateCore::new(1, 0, 0.0, 0.0),
            threshold_db: 0.0,
        }
    }
}

/// The built-in Dynamics module (SPEC-016).
pub struct Dynamics {
    descriptor: ModuleDescriptor,
    params: Vec<ParamInfo>,
    groups: Vec<ParamGroup>,
    /// Plain target values in `params()` order.
    values: Vec<f64>,
    telemetry: Arc<TelemetryCells>,
    /// The static transfer graph (SPEC-016 §4.11); stateless, shared with the UI thread.
    curve: Arc<DynamicsCurve>,
    sample_rate: f64,
    /// Look-ahead of the running instance, in samples (= the reported latency).
    lookahead: u32,
    /// Look-ahead the parameter asks for; a difference triggers a restart request (§4.9).
    lookahead_target_ms: f64,
    /// The look-ahead a `Restart` was already requested for (so it is requested once per value).
    restart_requested_for: Option<u32>,
    /// Audio delay line of `lookahead` samples.
    delay: DelayLine,
    peak: SlidingPeak,
    rms: SlidingRms,
    peak_max: f64,
    peak_dbfs: f64,
    /// Detection weight: 0 = Peak, 1 = RMS.
    detection: LinearRamp,
    knee: LinearRamp,
    autogate: AutoGate,
    expander: Section,
    /// Expander ratio in its ramped domain `R − 1` (§4.8).
    expander_ratio_minus_one: LinearRamp,
    compressor: Section,
    /// Compressor ratio in its ramped domain `1 − 1/R` (§4.8).
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
    /// Look-ahead in ms (the module's latency; a change requests a restart, §4.9).
    pub const LOOKAHEAD_MS: ParamId = ParamId(3);
    /// AutoGate enable.
    pub const AUTOGATE_ENABLED: ParamId = ParamId(10);
    /// AutoGate opening threshold (closes 3 dB below it, §4.7).
    pub const AUTOGATE_THRESHOLD_DB: ParamId = ParamId(11);
    /// AutoGate attack: the duration of the raised-cosine opening.
    pub const AUTOGATE_ATTACK_MS: ParamId = ParamId(12);
    /// AutoGate hold before the release.
    pub const AUTOGATE_HOLD_MS: ParamId = ParamId(13);
    /// AutoGate release time constant.
    pub const AUTOGATE_RELEASE_MS: ParamId = ParamId(14);
    /// Expander enable.
    pub const EXPANDER_ENABLED: ParamId = ParamId(20);
    /// Expander threshold (dBFS).
    pub const EXPANDER_THRESHOLD_DB: ParamId = ParamId(21);
    /// Expander ratio.
    pub const EXPANDER_RATIO: ParamId = ParamId(22);
    /// Expander attack time constant.
    pub const EXPANDER_ATTACK_MS: ParamId = ParamId(23);
    /// Expander release time constant.
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
    /// Telemetry: AutoGate gain reduction.
    pub const TELEMETRY_GR_AUTOGATE: usize = 1;
    /// Telemetry: Expander gain reduction.
    pub const TELEMETRY_GR_EXPANDER: usize = 2;
    /// Telemetry: compressor gain reduction.
    pub const TELEMETRY_GR_COMPRESSOR: usize = 3;
    /// Telemetry: limiter gain reduction.
    pub const TELEMETRY_GR_LIMITER: usize = 4;
    /// Telemetry: input peak level (dBFS).
    pub const TELEMETRY_INPUT_LEVEL: usize = 5;
    /// Telemetry: AutoGate open lamp.
    pub const TELEMETRY_AUTOGATE_OPEN: usize = 6;

    /// [`TransferCurve`] component: the AutoGate (SPEC-016 §4.11).
    pub const COMPONENT_AUTOGATE: usize = 0;
    /// [`TransferCurve`] component: the Expander.
    pub const COMPONENT_EXPANDER: usize = 1;
    /// [`TransferCurve`] component: the Compressor, makeup included.
    pub const COMPONENT_COMPRESSOR: usize = 2;
    /// [`TransferCurve`] component: the Limiter.
    pub const COMPONENT_LIMITER: usize = 3;
    /// Number of [`TransferCurve`] components (one per section).
    pub const CURVE_COMPONENTS: usize = 4;

    /// A new, inactive instance with default values.
    pub fn new() -> Self {
        let params = Self::schema();
        let values = params.iter().map(|p| p.default).collect();
        let curve = Arc::new(DynamicsCurve::new(params.clone()));
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
            curve,
            sample_rate: 48_000.0,
            lookahead: 0,
            lookahead_target_ms: 0.0,
            restart_requested_for: None,
            delay: DelayLine::new(0),
            peak: SlidingPeak::new(1),
            rms: SlidingRms::new(1),
            peak_max: 0.0,
            peak_dbfs: LEVEL_FLOOR_DBFS,
            detection: LinearRamp::new(0.0),
            knee: LinearRamp::new(0.0),
            autogate: AutoGate::new(),
            expander: Section::new(AttackDirection::Up),
            expander_ratio_minus_one: LinearRamp::new(0.0),
            compressor: Section::new(AttackDirection::Down),
            slope: LinearRamp::new(0.0),
            makeup: LinearRamp::new(0.0),
            limiter: Section::new(AttackDirection::Down),
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
                .decimals(0),
            p(10, "autogate_enabled", "AutoGate")
                .toggle(false)
                .in_group(1)
                .smoothing(20.0),
            p(11, "autogate_threshold_db", "Threshold")
                .db(Unit::Dbfs, -80.0, 0.0, -50.0)
                .in_group(1),
            ms(12, "autogate_attack_ms", "Attack", 1, 0.1, 100.0, 2.0),
            ms(13, "autogate_hold_ms", "Hold", 1, 0.1, 1000.0, 50.0),
            ms(14, "autogate_release_ms", "Release", 1, 1.0, 2000.0, 100.0),
            p(20, "expander_enabled", "Expander")
                .toggle(false)
                .in_group(2)
                .smoothing(20.0),
            p(21, "expander_threshold_db", "Threshold")
                .db(Unit::Dbfs, -80.0, 0.0, -45.0)
                .in_group(2)
                .smoothing(20.0),
            p(22, "expander_ratio", "Ratio")
                .log(Unit::Ratio, 1.0, 30.0, 2.0)
                .in_group(2)
                .smoothing(20.0),
            ms(23, "expander_attack_ms", "Attack", 2, 0.1, 100.0, 2.0),
            ms(24, "expander_release_ms", "Release", 2, 1.0, 2000.0, 100.0),
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

    fn ramps(&mut self) -> [&mut LinearRamp; 12] {
        [
            &mut self.detection,
            &mut self.knee,
            &mut self.autogate.weight,
            &mut self.expander.weight,
            &mut self.expander.threshold,
            &mut self.expander_ratio_minus_one,
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
            // Latency is fixed while active (§4.9): only the restart target moves.
            Self::LOOKAHEAD_MS => self.lookahead_target_ms = v,
            Self::AUTOGATE_ENABLED => self.autogate.weight.set_target(switch(v)),
            Self::AUTOGATE_THRESHOLD_DB => self.autogate.threshold_db = v,
            Self::AUTOGATE_ATTACK_MS => self
                .autogate
                .gate
                .set_attack(u32::try_from(ms_to_samples(v, fs)).unwrap_or(u32::MAX)),
            Self::AUTOGATE_HOLD_MS => self.autogate.gate.set_hold(ms_to_samples_or_zero(v, fs)),
            Self::AUTOGATE_RELEASE_MS => self.autogate.gate.set_release(time_constant_alpha(v, fs)),
            Self::EXPANDER_ENABLED => self.expander.weight.set_target(switch(v)),
            Self::EXPANDER_THRESHOLD_DB => self.expander.threshold.set_target(v),
            Self::EXPANDER_RATIO => self.expander_ratio_minus_one.set_target(v - 1.0),
            Self::EXPANDER_ATTACK_MS => self
                .expander
                .ballistics
                .set_attack(time_constant_alpha(v, fs)),
            Self::EXPANDER_RELEASE_MS => self
                .expander
                .ballistics
                .set_release(time_constant_alpha(v, fs)),
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

    /// The `lookahead_ms` target value (plain ms).
    fn lookahead_value(&self) -> f64 {
        schema::index_of(&self.params, Self::LOOKAHEAD_MS).map_or(0.0, |i| self.values[i])
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
        t.write(Self::TELEMETRY_GR_AUTOGATE, gr(s.autogate_db));
        t.write(Self::TELEMETRY_GR_EXPANDER, gr(s.expander_db));
        t.write(Self::TELEMETRY_GR_COMPRESSOR, gr(s.compressor_db));
        t.write(Self::TELEMETRY_GR_LIMITER, gr(s.limiter_db));
        t.write(
            Self::TELEMETRY_INPUT_LEVEL,
            s.level_dbfs.clamp(-100.0, 6.0) as f32,
        );
        t.write(
            Self::TELEMETRY_AUTOGATE_OPEN,
            f32::from(u8::from(s.autogate_open)),
        );
    }

    /// One output sample: the level-domain composition of SPEC-016 §4.5, in processing order.
    ///
    /// `x` is the **input** sample (it feeds the detectors and the look-ahead delay line); the
    /// returned sample is the one `lookahead` samples older, scaled by the composed gain.
    fn tick(&mut self, x: f32, stats: &mut BlockStats) -> f32 {
        let s = f64::from(x);
        let pk = self.peak.push(s.abs());
        if pk.to_bits() != self.peak_max.to_bits() {
            self.peak_max = pk;
            self.peak_dbfs = lin_to_dbfs(pk);
        }
        let level_pk = self.peak_dbfs;
        let level_rms = mean_square_to_db(self.rms.push(s));
        let delayed = self.delay.push(x);
        let w_det = self.detection.advance();
        let level_mode = (1.0 - w_det) * level_pk + w_det * level_rms;
        let knee = self.knee.advance();

        // AutoGate: always peak, a linear gain from the gate core, crossfaded toward unity.
        let w_ag = self.autogate.weight.advance();
        let t_ag = self.autogate.threshold_db;
        let g_ag = self.autogate.gate.process(
            level_pk,
            t_ag,
            t_ag - AUTOGATE_HYSTERESIS_DB,
            0.0, // closed = −∞ (SPEC-016 §2.3)
        );
        // Exactly 1.0 (bit-exact passthrough) whenever the section is off.
        let g_ag_eff = 1.0 + w_ag * (g_ag - 1.0);
        let gain_ag_db = lin_to_dbfs(g_ag_eff);

        // Expander: the mode level shifted by the AutoGate's gain.
        let w_ex = self.expander.weight.advance();
        let t_ex = self.expander.threshold.advance();
        let ratio_minus_one = self.expander_ratio_minus_one.advance();
        let level_ex = (level_mode + gain_ag_db).max(LEVEL_FLOOR_DBFS);
        let g_ex = self.expander.ballistics.process(expander_gain_db(
            level_ex,
            t_ex,
            ratio_minus_one,
            knee,
        ));
        let g_ex_eff = w_ex * g_ex;

        let w_co = self.compressor.weight.advance();
        let t_co = self.compressor.threshold.advance();
        let slope = self.slope.advance();
        let makeup = self.makeup.advance();
        let level_co = (level_mode + gain_ag_db + g_ex_eff).max(LEVEL_FLOOR_DBFS);
        let g_co = self
            .compressor
            .ballistics
            .process(compressor_gain_db(level_co, t_co, slope, knee));
        let g_co_eff = w_co * g_co;
        let makeup_eff = w_co * makeup;

        let w_li = self.limiter.weight.advance();
        let t_li = self.limiter.threshold.advance();
        let level_li =
            (level_pk + gain_ag_db + g_ex_eff + g_co_eff + makeup_eff).max(LEVEL_FLOOR_DBFS);
        let g_li = self
            .limiter
            .ballistics
            .process(limiter_gain_db(level_li, t_li));
        let g_li_eff = w_li * g_li;

        let reduction = g_ex_eff + g_co_eff + g_li_eff;
        stats.total_db = stats.total_db.min(gain_ag_db + reduction);
        stats.autogate_db = stats.autogate_db.min(gain_ag_db);
        stats.expander_db = stats.expander_db.min(g_ex_eff);
        stats.compressor_db = stats.compressor_db.min(g_co_eff);
        stats.limiter_db = stats.limiter_db.min(g_li_eff);
        stats.level_dbfs = stats.level_dbfs.max(level_pk);
        stats.autogate_open |= w_ag > 0.0 && self.autogate.gate.is_open();
        // All weights 0 and no ramp running: exactly 1.0 · 10^0 = 1.0, a bit-exact passthrough.
        let g = g_ag_eff * db_to_lin(reduction + makeup_eff);
        delayed * (g as f32)
    }

    /// Diagnostic (tests, SPEC-016 AC-18): true if any persistent state value — the detectors,
    /// the AutoGate's linear gain or a section's dB ballistics — is a subnormal `f32`/`f64`. Not
    /// on the RT path and not part of the public API surface.
    #[doc(hidden)]
    pub fn state_has_subnormals(&self) -> bool {
        let vals = [
            self.peak_max,
            self.peak_dbfs,
            self.rms.mean_square(),
            self.autogate.gate.gain(),
            self.expander.ballistics.gain_db(),
            self.compressor.ballistics.gain_db(),
            self.limiter.ballistics.gain_db(),
        ];
        vals.iter().any(|v| v.is_subnormal())
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
        // Latency is fixed for the lifetime of the instance (ADR-005 §8, SPEC-016 §4.9).
        let la = ms_to_samples_or_zero(self.lookahead_value(), fs);
        self.lookahead = la;
        self.restart_requested_for = None;
        self.delay = DelayLine::new(la as usize);
        // The peak window reaches `la` ahead and W_pk behind the output sample (§4.3).
        self.peak = SlidingPeak::new(ms_to_samples(PEAK_WINDOW_MS, fs) + la as usize);
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
        self.lookahead
    }

    fn tail(&self) -> Tail {
        Tail::Samples(u64::from(self.lookahead))
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
        // §4.9: the running instance keeps its look-ahead; the host builds a replacement. Asked
        // for once per changed value, so a value that comes back to the active one asks for
        // nothing.
        let wanted = ms_to_samples_or_zero(self.lookahead_target_ms, self.sample_rate);
        if wanted == self.lookahead {
            self.restart_requested_for = None;
        } else if self.restart_requested_for != Some(wanted) {
            self.restart_requested_for = Some(wanted);
            ctx.request(HostRequest::Restart);
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
        self.delay.clear();
        self.peak_max = 0.0;
        self.peak_dbfs = LEVEL_FLOOR_DBFS;
        self.autogate.gate.reset(0.0);
        self.expander.ballistics.reset();
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
            ExtensionId::TransferCurve => {
                let c: Arc<dyn TransferCurve> = self.curve.clone();
                Some(Extension::TransferCurve(c))
            }
            _ => None,
        }
    }
}

/// The target values the static curve needs, read (and clamp-quantized) once per call.
struct CurveValues {
    /// `detection` = RMS: the Expander and Compressor read `in − 3.0103 dB` (§4.11).
    rms: bool,
    knee: f64,
    w_ag: f64,
    t_ag: f64,
    w_ex: f64,
    t_ex: f64,
    ratio_minus_one: f64,
    w_co: f64,
    t_co: f64,
    slope: f64,
    makeup: f64,
    w_li: f64,
    t_li: f64,
}

/// The Dynamics [`TransferCurve`] (SPEC-016 §4.11): stateless, so it is callable from any
/// non-audio thread while `process()` runs. It is the §4.5 composition of [`Dynamics::tick`] with
/// every ramp at its target and every ballistic settled, through the same
/// `vox_dsp::dynamics::curves` gain computers, so the graph is what the module does (AC-17).
struct DynamicsCurve {
    params: Vec<ParamInfo>,
    handles: Vec<TransferHandle>,
}

impl DynamicsCurve {
    fn new(params: Vec<ParamInfo>) -> Self {
        let handle = |component, threshold, enable| TransferHandle {
            component,
            threshold,
            enable: Some(enable),
        };
        Self {
            params,
            handles: vec![
                handle(
                    Dynamics::COMPONENT_AUTOGATE,
                    Dynamics::AUTOGATE_THRESHOLD_DB,
                    Dynamics::AUTOGATE_ENABLED,
                ),
                handle(
                    Dynamics::COMPONENT_EXPANDER,
                    Dynamics::EXPANDER_THRESHOLD_DB,
                    Dynamics::EXPANDER_ENABLED,
                ),
                handle(
                    Dynamics::COMPONENT_COMPRESSOR,
                    Dynamics::COMPRESSOR_THRESHOLD_DB,
                    Dynamics::COMPRESSOR_ENABLED,
                ),
                handle(
                    Dynamics::COMPONENT_LIMITER,
                    Dynamics::LIMITER_THRESHOLD_DB,
                    Dynamics::LIMITER_ENABLED,
                ),
            ],
        }
    }

    /// Clamp-quantized value of `id` (missing → its default), as the host would set it.
    fn value(&self, values: &[f64], id: ParamId) -> f64 {
        schema::index_of(&self.params, id).map_or(0.0, |i| {
            let p = &self.params[i];
            p.clamp_quantize(values.get(i).copied().unwrap_or(p.default))
        })
    }

    fn read(&self, values: &[f64]) -> CurveValues {
        let v = |id| self.value(values, id);
        CurveValues {
            rms: switch(v(Dynamics::DETECTION)) >= 0.5,
            knee: v(Dynamics::KNEE_DB),
            w_ag: switch(v(Dynamics::AUTOGATE_ENABLED)),
            t_ag: v(Dynamics::AUTOGATE_THRESHOLD_DB),
            w_ex: switch(v(Dynamics::EXPANDER_ENABLED)),
            t_ex: v(Dynamics::EXPANDER_THRESHOLD_DB),
            ratio_minus_one: v(Dynamics::EXPANDER_RATIO) - 1.0,
            w_co: switch(v(Dynamics::COMPRESSOR_ENABLED)),
            t_co: v(Dynamics::COMPRESSOR_THRESHOLD_DB),
            slope: compressor_slope(v(Dynamics::COMPRESSOR_RATIO)),
            makeup: v(Dynamics::COMPRESSOR_MAKEUP_DB),
            w_li: switch(v(Dynamics::LIMITER_ENABLED)),
            t_li: v(Dynamics::LIMITER_THRESHOLD_DB),
        }
    }

    /// The four component gains in dB for one input peak level, in processing order. The AutoGate
    /// entry is `-inf` when the gate mutes (`tick` multiplies by exactly 0.0 there), so the
    /// components always sum to `output − input`.
    fn gains(&self, v: &CurveValues, branch: CurveBranch, in_dbfs: f64) -> [f64; 4] {
        let level_pk = in_dbfs;
        let level_mode = if v.rms {
            in_dbfs - SINE_PEAK_TO_RMS_DB
        } else {
            in_dbfs
        };

        // AutoGate: peak detection, fixed 3 dB hysteresis, closed = −∞ (§2.3, §4.7).
        let open_at = match branch {
            CurveBranch::Rising => v.t_ag,
            CurveBranch::Falling => v.t_ag - AUTOGATE_HYSTERESIS_DB,
        };
        let g_ag = f64::from(u8::from(level_pk >= open_at));
        let g_ag_eff = 1.0 + v.w_ag * (g_ag - 1.0);
        // The floor `tick` feeds the sections below when the gate is shut.
        let gain_ag_db = lin_to_dbfs(g_ag_eff);

        let level_ex = (level_mode + gain_ag_db).max(LEVEL_FLOOR_DBFS);
        let g_ex_eff = v.w_ex * expander_gain_db(level_ex, v.t_ex, v.ratio_minus_one, v.knee);

        let level_co = (level_mode + gain_ag_db + g_ex_eff).max(LEVEL_FLOOR_DBFS);
        let g_co_eff = v.w_co * compressor_gain_db(level_co, v.t_co, v.slope, v.knee);
        let makeup_eff = v.w_co * v.makeup;

        let level_li =
            (level_pk + gain_ag_db + g_ex_eff + g_co_eff + makeup_eff).max(LEVEL_FLOOR_DBFS);
        let g_li_eff = v.w_li * limiter_gain_db(level_li, v.t_li);

        let autogate = if g_ag_eff > 0.0 {
            gain_ag_db
        } else {
            f64::NEG_INFINITY
        };
        [autogate, g_ex_eff, g_co_eff + makeup_eff, g_li_eff]
    }
}

impl TransferCurve for DynamicsCurve {
    fn output_dbfs(
        &self,
        values: &[f64],
        branch: CurveBranch,
        in_dbfs: &[f64],
        out_dbfs: &mut [f64],
    ) {
        let v = self.read(values);
        for (o, &x) in out_dbfs.iter_mut().zip(in_dbfs) {
            let g = self.gains(&v, branch, x);
            *o = x + g[0] + g[1] + g[2] + g[3];
        }
    }

    fn has_hysteresis(&self, values: &[f64]) -> bool {
        switch(self.value(values, Dynamics::AUTOGATE_ENABLED)) >= 0.5
    }

    fn component_count(&self) -> usize {
        Dynamics::CURVE_COMPONENTS
    }

    fn component_group(&self, component: usize) -> Option<GroupId> {
        match component {
            Dynamics::COMPONENT_AUTOGATE => Some(Dynamics::GROUP_AUTOGATE),
            Dynamics::COMPONENT_EXPANDER => Some(Dynamics::GROUP_EXPANDER),
            Dynamics::COMPONENT_COMPRESSOR => Some(Dynamics::GROUP_COMPRESSOR),
            Dynamics::COMPONENT_LIMITER => Some(Dynamics::GROUP_LIMITER),
            _ => None,
        }
    }

    fn component_gain_db(
        &self,
        component: usize,
        values: &[f64],
        in_dbfs: &[f64],
        out_db: &mut [f64],
    ) {
        if component >= Dynamics::CURVE_COMPONENTS {
            out_db.fill(0.0);
            return;
        }
        let v = self.read(values);
        for (o, &x) in out_db.iter_mut().zip(in_dbfs) {
            *o = self.gains(&v, CurveBranch::Rising, x)[component];
        }
    }

    fn handles(&self) -> &[TransferHandle] {
        &self.handles
    }

    /// The Expander and Compressor are the only RMS-detected sections (§2.4), so only their
    /// handles move by a sine's peak-to-RMS distance when `detection` is RMS.
    fn handle_offset_db(&self, handle: usize, values: &[f64]) -> f64 {
        let rms = switch(self.value(values, Dynamics::DETECTION)) >= 0.5;
        match self.handles.get(handle).map(|h| h.component) {
            Some(Dynamics::COMPONENT_EXPANDER | Dynamics::COMPONENT_COMPRESSOR) if rms => {
                SINE_PEAK_TO_RMS_DB
            }
            _ => 0.0,
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
