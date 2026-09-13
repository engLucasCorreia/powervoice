//! Shared helpers for the rack tests: a registry with the built-in Gain and the test modules,
//! slot builders, a probe module that logs the events it receives, and a block-feeding driver
//! for the live rack (seeded random block sizes 1…1024 with 0-length flushes, a control tick
//! every ~16 ms, every `LiveRack::process` under the allocation checker).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use vox_module_api::test_util::{TestNaN, TestReporter, TestRestart, TestRng, no_alloc};
use vox_module_api::{
    ActivateConfig, ChannelLayout, LocalizedText, MODULE_API_VERSION, Module, ModuleDescriptor,
    ModuleError, ModuleFactory, ModuleRef, ModuleState, ParamEvent, ParamFlags, ParamId, ParamInfo,
    ProcessContext, ProcessMode, ProcessStatus, StateError, Tail, Taper, Transport, Unit, Version,
    segments,
};
use vox_modules::{Gain, GainFactory};
use vox_rack::{
    LiveRack, MAX_BLOCK, RackHost, RackModel, RackNotice, RackOptions, Registry, SlotModel, SlotUid,
};

pub const SR: f64 = 48_000.0;
/// 15 ms at 48 kHz.
pub const XF: usize = 720;
/// 65 ms at 48 kHz.
pub const MS65: usize = 3120;
/// 100 ms at 48 kHz.
pub const MS100: usize = 4800;

pub fn rt_config() -> ActivateConfig {
    ActivateConfig {
        sample_rate: SR,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

pub fn cfg(max_block: u32) -> ActivateConfig {
    ActivateConfig {
        max_block,
        ..rt_config()
    }
}

// ---------------------------------------------------------------------------------------------
// Factories and slots.

pub struct FnFactory {
    desc: ModuleDescriptor,
    make: fn() -> Box<dyn Module>,
}

impl ModuleFactory for FnFactory {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        Ok((self.make)())
    }
}

pub fn factory(make: fn() -> Box<dyn Module>) -> Arc<dyn ModuleFactory> {
    Arc::new(FnFactory {
        desc: make().descriptor().clone(),
        make,
    })
}

/// NaN from absolute sample 5000.
pub const NAN_FROM: u64 = 5000;
/// Reporter period: 50 ms.
pub const REPORT_PERIOD: u64 = 2400;

pub fn test_factories() -> Vec<Arc<dyn ModuleFactory>> {
    vec![
        Arc::new(GainFactory::new()),
        factory(|| Box::new(TestRestart::new(0))),
        factory(|| Box::new(TestNaN::new(NAN_FROM))),
        factory(|| Box::new(TestReporter::new(REPORT_PERIOD))),
        factory(|| Box::new(Sentinel::new())),
    ]
}

pub fn registry() -> Arc<Registry> {
    Arc::new(Registry::with_factories(test_factories()).unwrap())
}

pub fn slot(id: &str, params: &[(&str, f64)]) -> SlotModel {
    SlotModel::new(
        &ModuleRef {
            id: id.into(),
            version: Version::new(1, 0, 0),
        },
        false,
        &ModuleState {
            format_version: 1,
            params: params
                .iter()
                .map(|(k, v)| ((*k).to_owned(), *v))
                .collect::<BTreeMap<_, _>>(),
            blob: None,
        },
    )
}

pub fn gain_slot(db: f64) -> SlotModel {
    slot(Gain::ID, &[("gain_db", db)])
}

/// "TestDelay(n)": the exact delay `TestRestart` with `latency_samples = n`.
pub fn delay_slot(n: u32) -> SlotModel {
    slot(TestRestart::ID, &[("latency_samples", f64::from(n))])
}

pub fn bypassed(mut s: SlotModel) -> SlotModel {
    s.bypass = true;
    s
}

pub fn model(slots: Vec<SlotModel>) -> RackModel {
    RackModel { slots }
}

pub fn white(seed: u64, len: usize) -> Vec<f32> {
    vox_testkit::signal::white_noise(seed, -20.0, len as f64 / SR, 48_000).unwrap()
}

pub fn rms_db(x: &[f32]) -> f64 {
    let ms = x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len() as f64;
    10.0 * ms.log10()
}

pub fn max_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

pub fn bit_identical(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

// ---------------------------------------------------------------------------------------------
// Driver.

/// An automation event at an absolute input position (realtime driver).
#[derive(Clone, Copy, Debug)]
pub struct RtEvent {
    pub position: usize,
    pub slot: SlotUid,
    pub id: ParamId,
    pub value: f64,
}

pub struct Driver {
    pub host: RackHost,
    pub live: LiveRack,
    rng: TestRng,
    pub pos: usize,
    pub out: Vec<f32>,
    since_tick: usize,
    /// `(sample position at the tick, notice)`.
    pub notices: Vec<(usize, RackNotice)>,
    pub automation: Vec<RtEvent>,
    next_auto: usize,
}

impl Driver {
    pub fn new(model: &RackModel, seed: u64, len: usize) -> Self {
        Self::with_registry(registry(), model, seed, len)
    }

    pub fn with_registry(
        registry: Arc<Registry>,
        model: &RackModel,
        seed: u64,
        len: usize,
    ) -> Self {
        assert!(
            vox_module_api::test_util::alloc_checks_active(),
            "install_test_allocator!() missing: RT checks would pass silently"
        );
        let (host, live) =
            RackHost::new(registry, rt_config(), RackOptions::default(), model).unwrap();
        Self {
            host,
            live,
            rng: TestRng::new(seed),
            pos: 0,
            out: vec![0.0; len],
            since_tick: 0,
            notices: Vec::new(),
            automation: Vec::new(),
            next_auto: 0,
        }
    }

    pub fn tick(&mut self) {
        let pos = self.pos;
        let notices = self.host.tick();
        self.notices.extend(notices.into_iter().map(|n| (pos, n)));
    }

    /// Processes `input[pos..until]`.
    pub fn run_until(&mut self, input: &[f32], until: usize) {
        while self.pos < until {
            let n = if self.rng.chance(0.05) {
                0
            } else {
                self.rng.range_u32(1, 1024) as usize
            }
            .min(until - self.pos);
            let pos = self.pos;
            if n > 0 {
                while let Some(e) = self.automation.get(self.next_auto)
                    && e.position < pos + n
                {
                    let ev = ParamEvent {
                        offset: (e.position - pos) as u32,
                        id: e.id,
                        value: e.value,
                    };
                    self.live.push_event(e.slot, ev).unwrap();
                    self.next_auto += 1;
                }
            }
            let (i, o) = (&input[pos..pos + n], &mut self.out[pos..pos + n]);
            let live = &mut self.live;
            let t = Transport {
                playing: true,
                position_samples: Some(pos as u64),
            };
            no_alloc(|| live.process(t, i, o)).expect("LiveRack::process allocated");
            self.pos += n;
            self.since_tick += n;
            if self.since_tick >= 768 {
                self.since_tick = 0;
                self.tick();
            }
        }
    }

    pub fn run_to_end(&mut self, input: &[f32]) {
        self.run_until(input, input.len());
        self.tick();
    }
}

/// Renders `input` through `model` without edits.
pub fn uninterrupted(model: &RackModel, input: &[f32], seed: u64) -> Vec<f32> {
    let mut d = Driver::new(model, seed, input.len());
    d.run_to_end(input);
    d.out
}

// ---------------------------------------------------------------------------------------------
// Sentinel: pass-through that records whether it ever received a non-finite sample.

pub static SENTINEL_SAW_NON_FINITE: AtomicBool = AtomicBool::new(false);
pub static SENTINEL_SAMPLES: AtomicUsize = AtomicUsize::new(0);

pub struct Sentinel {
    desc: ModuleDescriptor,
}

impl Sentinel {
    pub const ID: &'static str = "org.powervoice.test-sentinel";
    pub fn new() -> Self {
        Self {
            desc: descriptor(Self::ID, "Sentinel"),
        }
    }
}

pub fn descriptor(id: &str, name: &str) -> ModuleDescriptor {
    ModuleDescriptor {
        id: id.into(),
        version: Version::new(1, 0, 0),
        name: LocalizedText::plain(name),
        vendor: "PowerVoice".into(),
        description: LocalizedText::plain("test"),
        url: None,
        features: Vec::new(),
        state_format_version: 1,
        api_version: MODULE_API_VERSION,
    }
}

impl Module for Sentinel {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn params(&self) -> &[ParamInfo] {
        &[]
    }
    fn activate(&mut self, _config: &ActivateConfig) -> Result<(), ModuleError> {
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
        _ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        if inputs[0].iter().any(|x| !x.is_finite()) {
            SENTINEL_SAW_NON_FINITE.store(true, Ordering::Relaxed);
        }
        SENTINEL_SAMPLES.fetch_add(inputs[0].len(), Ordering::Relaxed);
        outputs[0].copy_from_slice(inputs[0]);
        ProcessStatus::Continue
    }
    fn reset(&mut self) {}
    fn param_value(&self, _id: ParamId) -> Option<f64> {
        None
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        Ok(ModuleState::new(1))
    }
    fn load_state(&mut self, _state: &ModuleState) -> Result<(), StateError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------
// Probe: fixed delay (declared latency) + an event log with absolute sample positions.

pub type Log = Arc<Mutex<Vec<(u64, ParamId, f64)>>>;

pub struct Probe {
    desc: ModuleDescriptor,
    pub params: Vec<ParamInfo>,
    latency: u32,
    line: Vec<f32>,
    pos: usize,
    log: Vec<(u64, ParamId, f64)>,
    pub shared_log: Log,
    values: [f64; 2],
    pub layouts: Vec<ChannelLayout>,
    pub fail_activate: bool,
    pub deactivations: Arc<AtomicUsize>,
    /// Output NaN from this instance's `steady_time` on (non-finite guard tests).
    pub nan_from: Option<u64>,
}

impl Probe {
    pub const VALUE: ParamId = ParamId(0);
    pub const OTHER: ParamId = ParamId(1);

    fn param(id: ParamId, key: &str) -> ParamInfo {
        ParamInfo {
            id,
            key: key.into(),
            name: LocalizedText::plain(key),
            group: None,
            unit: Unit::None,
            min: -1e6,
            max: 1e6,
            default: 0.0,
            taper: Taper::Linear,
            step: None,
            enum_labels: Vec::new(),
            decimals: 3,
            smoothing_ms: 0.0,
            flags: ParamFlags::AUTOMATABLE,
        }
    }

    pub fn new(latency: u32) -> Self {
        Self {
            desc: descriptor("org.powervoice.test-probe", "Probe"),
            params: vec![
                Self::param(Self::VALUE, "value"),
                Self::param(Self::OTHER, "other"),
            ],
            latency,
            line: Vec::new(),
            pos: 0,
            log: Vec::new(),
            shared_log: Log::default(),
            values: [0.0; 2],
            layouts: vec![ChannelLayout::MONO],
            fail_activate: false,
            deactivations: Arc::default(),
            nan_from: None,
        }
    }
}

impl Module for Probe {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.desc
    }
    fn params(&self) -> &[ParamInfo] {
        &self.params
    }
    fn supported_layouts(&self) -> &[ChannelLayout] {
        &self.layouts
    }
    fn activate(&mut self, _config: &ActivateConfig) -> Result<(), ModuleError> {
        if self.fail_activate {
            return Err(ModuleError::Resource("nope".into()));
        }
        self.line = vec![0.0; self.latency as usize];
        self.log = Vec::with_capacity(16_384);
        self.reset();
        Ok(())
    }
    fn deactivate(&mut self) {
        self.shared_log.lock().unwrap().extend_from_slice(&self.log);
        self.log.clear();
        self.deactivations.fetch_add(1, Ordering::Relaxed);
    }
    fn latency_samples(&self) -> u32 {
        self.latency
    }
    fn tail(&self) -> Tail {
        Tail::Samples(u64::from(self.latency))
    }
    fn process(
        &mut self,
        ctx: &mut ProcessContext<'_>,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
    ) -> ProcessStatus {
        let (input, output) = (inputs[0], &mut *outputs[0]);
        for seg in segments(ctx.frames, ctx.events) {
            for ev in seg.events {
                if self.log.len() < self.log.capacity() {
                    self.log
                        .push((ctx.steady_time + u64::from(seg.start), ev.id, ev.value));
                }
                if let Some(v) = self.values.get_mut(ev.id.0 as usize) {
                    *v = ev.value;
                }
            }
            for i in seg.start as usize..(seg.start + seg.len) as usize {
                if self.line.is_empty() {
                    output[i] = input[i];
                } else {
                    output[i] = self.line[self.pos];
                    self.line[self.pos] = input[i];
                    self.pos = (self.pos + 1) % self.line.len();
                }
            }
        }
        if let Some(from) = self.nan_from {
            for (i, o) in output.iter_mut().enumerate() {
                if ctx.steady_time + i as u64 >= from {
                    *o = f32::NAN;
                }
            }
        }
        ProcessStatus::Continue
    }
    fn reset(&mut self) {
        self.line.fill(0.0);
        self.pos = 0;
    }
    fn param_value(&self, id: ParamId) -> Option<f64> {
        self.values.get(id.0 as usize).copied()
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        let mut s = ModuleState::new(1);
        s.params.insert("value".into(), self.values[0]);
        s.params.insert("other".into(), self.values[1]);
        Ok(s)
    }
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        self.values[0] = state.params.get("value").copied().unwrap_or(0.0);
        self.values[1] = state.params.get("other").copied().unwrap_or(0.0);
        Ok(())
    }
}

pub fn value_event(offset: u32, value: f64) -> ParamEvent {
    ParamEvent {
        offset,
        id: Probe::VALUE,
        value,
    }
}

/// A probe whose log is published into the returned handle on deactivate.
pub fn probe_with_log(latency: u32) -> (Box<dyn Module>, Log) {
    let p = Probe::new(latency);
    let log = p.shared_log.clone();
    (Box::new(p), log)
}
