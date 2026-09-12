//! `ModuleTestHost`: runs the ADR-005 §15 obligations against any `Module`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use super::{TestRng, alloc_checks_active, no_alloc};
use crate::{
    ActivateConfig, ChannelLayout, DEFAULT_EVENT_CAPACITY, EventList, ExtensionId,
    MODULE_API_VERSION, Module, ModuleFactory, ModuleState, OutputEvents, ParamEvent, ParamFlags,
    ParamId, ParamInfo, ProcessContext, ProcessMode, ProcessStatus, Transport, prepare_state,
    validate_schema,
};

/// Output differences at or below this are "no effect" in the timing/flush probes
/// (≈ −120 dBFS, the ADR-001 §5 cross-path tolerance).
const EFFECT_EPS: f32 = 1e-6;
const SAMPLE_RATES: [f64; 3] = [44_100.0, 48_000.0, 96_000.0];
const MODES: [ProcessMode; 2] = [ProcessMode::Realtime, ProcessMode::Offline];
const PROBE_RATE: f64 = 48_000.0;

/// A failed [`ModuleTestHost`] check.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("ModuleTestHost: `{module}` failed check `{check}` (seed {seed:#x}): {detail}")]
pub struct HostFailure {
    /// Module id.
    pub module: String,
    /// Check name (`"schema"`, `"activate"`, `"process"`, `"reset"`, `"param_flush"`,
    /// `"event_timing"`, `"state_roundtrip"`, `"determinism"`, `"text_roundtrip"`,
    /// `"extensions"`, `"alloc_checker"`).
    pub check: &'static str,
    /// What went wrong.
    pub detail: String,
    /// Seed to reproduce.
    pub seed: u64,
}

/// Where a parameter event first changed the output (event-timing probe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectProbe {
    /// Parameter key.
    pub key: String,
    /// Event offset within the probe block.
    pub offset: u32,
    /// First differing sample, relative to the start of the event's block (the check expects
    /// `offset + latency_samples()`). `None`: no observable effect on the probe signal.
    pub first_difference: Option<u32>,
}

/// Statistics of a passing run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostReport {
    /// Allocation checks were active (the checker is the global allocator).
    pub alloc_checked: bool,
    /// `process` calls.
    pub blocks: u64,
    /// `process` calls with `frames == 0`.
    pub zero_length_blocks: u64,
    /// Frames processed.
    pub frames: u64,
    /// Parameter events sent.
    pub events: u64,
    /// `reset` calls interleaved with processing.
    pub resets: u64,
    /// Event-timing probe results.
    pub first_effect: Vec<EffectProbe>,
}

/// Hammers a module with the Module API test obligations (ADR-005 §15):
/// schema validation; activation at 44.1/48/96 kHz (realtime and offline); `process` with random
/// block sizes in `0..=max_block` (including 0), random valid events at random offsets and varied
/// signals, under the allocation checker, with finite output; `reset` under the checker; the
/// parameter mirror (`param_value`); zero-length parameter flushes; event timing (the first
/// change exactly at `offset + latency_samples()`, unless declared with
/// [`allow_delayed_effect`](Self::allow_delayed_effect)); state round-trip (bit-identical
/// output); determinism of two instances; text and normalized round-trips; extension variants.
///
/// The bit-exact comparisons (flush, timing, state round-trip, determinism) run in
/// [`ProcessMode::Offline`], where adapters must not substitute dry audio on deadline misses.
///
/// ```ignore
/// vox_module_api::install_test_allocator!();
/// let report = ModuleTestHost::new(|| Box::new(TestGain::new())).assert_passes();
/// ```
pub struct ModuleTestHost {
    make: Box<dyn Fn() -> Box<dyn Module>>,
    module_id: String,
    seed: u64,
    max_block: u32,
    blocks: usize,
    sample_rates: Vec<f64>,
    /// Parameters whose effect is deliberately not immediate (opt-out of the exact-timing check).
    delayed: Vec<ParamId>,
    stats: RefCell<HostReport>,
}

/// One block of a random script.
struct BlockSpec {
    frames: u32,
    events: Vec<ParamEvent>,
    reset_before: bool,
}

/// Random blocks plus the concatenated input they consume.
struct Script {
    blocks: Vec<BlockSpec>,
    input: Vec<f32>,
}

/// One instance with preallocated host buffers.
struct Runner {
    module: Box<dyn Module>,
    output: Vec<f32>,
    events: EventList,
    out_events: OutputEvents,
    steady_time: u64,
}

impl Runner {
    fn new(module: Box<dyn Module>, max_block: u32) -> Self {
        Self {
            module,
            output: vec![0.0; max_block as usize],
            events: EventList::with_capacity(DEFAULT_EVENT_CAPACITY),
            out_events: OutputEvents::with_capacity(DEFAULT_EVENT_CAPACITY),
            steady_time: 0,
        }
    }

    /// Processes one block (`input.len() <= max_block`) under the allocation checker and checks
    /// the status and that every output sample is finite (the output is pre-filled with NaN).
    fn block(
        &mut self,
        input: &[f32],
        events: &[ParamEvent],
        reset_before: bool,
    ) -> Result<&[f32], String> {
        let frames = input.len();
        if frames > self.output.len() {
            return Err(format!("host bug: block of {frames} frames > max_block"));
        }
        self.events.clear();
        for e in events {
            self.events
                .push(*e)
                .map_err(|err| format!("host bug: {err}"))?;
        }
        self.out_events.clear();
        self.output[..frames].fill(f32::NAN);
        let Runner {
            module,
            output,
            events,
            out_events,
            steady_time,
        } = self;
        let out = &mut output[..frames];
        let transport = Transport {
            playing: true,
            position_samples: Some(*steady_time),
        };
        let status = no_alloc(|| {
            if reset_before {
                module.reset();
            }
            let mut ctx = ProcessContext::new(
                frames as u32,
                *steady_time,
                transport,
                events.as_slice(),
                out_events,
            );
            module.process(&mut ctx, &[input], &mut [out])
        });
        match status {
            Err(n) => {
                let what = if reset_before {
                    "reset + process"
                } else {
                    "process"
                };
                return Err(format!(
                    "{n} heap (de)allocation(s) during {what} ({frames} frames)"
                ));
            }
            Ok(ProcessStatus::Error) => {
                return Err(
                    "process returned ProcessStatus::Error (built-in modules never do)".into(),
                );
            }
            Ok(ProcessStatus::Continue) => {}
        }
        let out = &self.output[..frames];
        if let Some(i) = out.iter().position(|x| !x.is_finite()) {
            return Err(format!(
                "non-finite output {} at sample {i} of a {frames}-frame block (unwritten samples stay NaN)",
                out[i]
            ));
        }
        self.steady_time += frames as u64;
        Ok(out)
    }
}

/// Index of the first sample where `a` and `b` differ by more than `eps`.
fn first_diff(a: &[f32], b: &[f32], eps: f32) -> Option<usize> {
    a.iter()
        .zip(b)
        .position(|(x, y)| (x - y).abs() > eps || x.is_finite() != y.is_finite())
}

/// Index of the first sample where `a` and `b` are not bit-identical.
fn first_bit_diff(a: &[f32], b: &[f32]) -> Option<usize> {
    if a.len() != b.len() {
        return Some(a.len().min(b.len()));
    }
    a.iter()
        .zip(b)
        .position(|(x, y)| x.to_bits() != y.to_bits())
}

/// Exact equality of plain values (`±0` equal).
fn exact(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.0
}

fn non_read_only(params: &[ParamInfo]) -> Vec<ParamInfo> {
    params
        .iter()
        .filter(|p| !p.flags.contains(ParamFlags::READ_ONLY))
        .cloned()
        .collect()
}

fn random_value(rng: &mut TestRng, p: &ParamInfo) -> f64 {
    let v = match rng.below(8) {
        0 => p.min,
        1 => p.max,
        2 => p.default,
        _ => p.from_normalized(rng.unit_f64()),
    };
    p.clamp_quantize(v)
}

/// The value at the other end of the range from `current`.
fn far_value(p: &ParamInfo, current: f64) -> f64 {
    p.clamp_quantize(if p.to_normalized(current) < 0.5 {
        p.max
    } else {
        p.min
    })
}

/// Varied test signal: silence, sines, noise, impulses, DC and hot squares (up to ±4).
fn test_signal(rng: &mut TestRng, n: usize, sample_rate: f64) -> Vec<f32> {
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0_f64;
    while out.len() < n {
        let len = (rng.range_u32(64, 4096) as usize).min(n - out.len());
        let amp = rng.unit_f64();
        match rng.below(6) {
            0 => out.extend(std::iter::repeat_n(0.0, len)),
            1 => {
                let freq = 20.0 + rng.unit_f64() * (sample_rate * 0.45 - 20.0);
                let inc = std::f64::consts::TAU * freq / sample_rate;
                for _ in 0..len {
                    out.push((amp * phase.sin()) as f32);
                    phase = (phase + inc) % std::f64::consts::TAU;
                }
            }
            2 => out.extend((0..len).map(|_| amp as f32 * rng.bipolar_f32())),
            3 => {
                let every = rng.range_u32(1, 512) as usize;
                out.extend((0..len).map(|i| if i % every == 0 { 1.0 } else { 0.0 }));
            }
            4 => out.extend(std::iter::repeat_n(rng.bipolar_f32(), len)),
            _ => {
                let level = (1.0 + 3.0 * amp) as f32;
                let half = rng.range_u32(1, 200) as usize;
                out.extend((0..len).map(|i| {
                    if (i / half).is_multiple_of(2) {
                        level
                    } else {
                        -level
                    }
                }));
            }
        }
    }
    out
}

/// Probe signal, never near zero: `0.5 + 0.25·sin(2π·997 Hz·t) + 0.1·noise` ∈ \[0.15, 0.85\].
/// `start` keeps the sine continuous across blocks.
fn probe_signal(rng: &mut TestRng, start: usize, n: usize, sample_rate: f64) -> Vec<f32> {
    (start..start + n)
        .map(|i| {
            let s = (std::f64::consts::TAU * 997.0 * i as f64 / sample_rate).sin();
            (0.5 + 0.25 * s) as f32 + 0.1 * rng.bipolar_f32()
        })
        .collect()
}

impl ModuleTestHost {
    /// Host for modules built by `make` (called once per instance the checks need).
    pub fn new(make: impl Fn() -> Box<dyn Module> + 'static) -> Self {
        let module_id = make().descriptor().id.clone();
        Self {
            make: Box::new(make),
            module_id,
            seed: 0x5EED_7005,
            max_block: 1024,
            blocks: 256,
            sample_rates: SAMPLE_RATES.to_vec(),
            delayed: Vec::new(),
            stats: RefCell::default(),
        }
    }

    /// Host for a factory's modules.
    ///
    /// # Panics
    /// When a check needs an instance and `factory.create()` fails.
    pub fn from_factory(factory: Arc<dyn ModuleFactory>) -> Self {
        Self::new(move || {
            factory
                .create()
                .unwrap_or_else(|e| panic!("ModuleFactory::create failed: {e}"))
        })
    }

    /// Random seed (default fixed, so runs are reproducible).
    pub fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// `max_block` passed to `activate` (default 1024, the realtime sub-block size).
    ///
    /// # Panics
    /// If `max_block == 0`.
    pub fn max_block(mut self, max_block: u32) -> Self {
        assert!(max_block >= 1, "max_block must be >= 1");
        self.max_block = max_block;
        self
    }

    /// Random blocks per (sample rate, mode) run (default 256).
    pub fn blocks(mut self, blocks: usize) -> Self {
        self.blocks = blocks;
        self
    }

    /// Sample rates for activation and random processing (default 44.1/48/96 kHz).
    pub fn sample_rates(mut self, rates: &[f64]) -> Self {
        self.sample_rates = rates.to_vec();
        self
    }

    /// Opts `id` out of the exact-timing check. By default an event at offset k must change the
    /// output **exactly** at sample `k + latency_samples()` (on the probe signal, a positive
    /// 997 Hz tone plus noise). Parameters whose effect is deliberately delayed or not
    /// observable there (time constants, thresholds, enables of neutral sections) must be
    /// declared here; for them the check only requires "no change before
    /// `k + latency_samples()`".
    pub fn allow_delayed_effect(mut self, id: ParamId) -> Self {
        self.delayed.push(id);
        self
    }

    /// Runs every check in order and returns the statistics of the run.
    pub fn run(&self) -> Result<HostReport, HostFailure> {
        *self.stats.borrow_mut() = HostReport::default();
        self.check_alloc_checker()?;
        self.check_schema()?;
        self.check_activate()?;
        self.check_process()?;
        self.check_reset()?;
        self.check_param_flush()?;
        self.check_event_timing()?;
        self.check_state_roundtrip()?;
        self.check_determinism()?;
        self.check_text_roundtrip()?;
        self.check_extensions()?;
        Ok(self.stats.borrow().clone())
    }

    /// [`run`](Self::run), panicking with the failure.
    ///
    /// # Panics
    /// On the first failed check.
    pub fn assert_passes(&self) -> HostReport {
        self.run().unwrap_or_else(|e| panic!("{e}"))
    }

    fn fail(&self, check: &'static str, detail: impl Into<String>) -> HostFailure {
        HostFailure {
            module: self.module_id.clone(),
            check,
            detail: detail.into(),
            seed: self.seed,
        }
    }

    fn config(&self, sample_rate: f64, mode: ProcessMode) -> ActivateConfig {
        ActivateConfig {
            sample_rate,
            max_block: self.max_block,
            mode,
            layout: ChannelLayout::MONO,
        }
    }

    /// New instance, optionally loaded with `state` (through [`prepare_state`]), activated.
    fn instance(
        &self,
        cfg: &ActivateConfig,
        state: Option<&ModuleState>,
    ) -> Result<Box<dyn Module>, String> {
        let mut m = (self.make)();
        if let Some(s) = state {
            let s = prepare_state(&*m, s.clone()).map_err(|e| format!("prepare_state: {e}"))?;
            m.load_state(&s).map_err(|e| format!("load_state: {e}"))?;
        }
        m.activate(cfg)
            .map_err(|e| format!("activate({cfg:?}): {e}"))?;
        Ok(m)
    }

    fn runner(&self, cfg: &ActivateConfig, state: Option<&ModuleState>) -> Result<Runner, String> {
        Ok(Runner::new(self.instance(cfg, state)?, self.max_block))
    }

    /// Random non-default state: every non-READ_ONLY key random, the fresh instance's blob.
    fn random_state(&self, rng: &mut TestRng) -> ModuleState {
        let m = (self.make)();
        let params = non_read_only(m.params())
            .iter()
            .map(|p| (p.key.clone(), random_value(rng, p)))
            .collect();
        ModuleState {
            format_version: m.descriptor().state_format_version,
            params,
            blob: m.save_state().ok().and_then(|s| s.blob),
        }
    }

    fn script(
        &self,
        rng: &mut TestRng,
        params: &[ParamInfo],
        sample_rate: f64,
        blocks: usize,
    ) -> Script {
        let eligible = non_read_only(params);
        let mut specs = Vec::with_capacity(blocks);
        let mut total = 0usize;
        for _ in 0..blocks {
            let frames = match rng.below(10) {
                0 => 0,
                1 => 1,
                2 => self.max_block,
                _ => rng.range_u32(0, self.max_block),
            };
            let mut events = Vec::new();
            if !eligible.is_empty() && rng.chance(0.5) {
                let count = if rng.chance(0.05) {
                    rng.range_u32(8, 64)
                } else {
                    rng.range_u32(1, 4)
                };
                let mut offsets: Vec<u32> = (0..count)
                    .map(|_| {
                        if frames == 0 {
                            0
                        } else {
                            rng.range_u32(0, frames - 1)
                        }
                    })
                    .collect();
                offsets.sort_unstable();
                for offset in offsets {
                    let p = &eligible[rng.below(eligible.len() as u64) as usize];
                    events.push(ParamEvent {
                        offset,
                        id: p.id,
                        value: random_value(rng, p),
                    });
                }
            }
            specs.push(BlockSpec {
                frames,
                events,
                reset_before: rng.chance(0.03),
            });
            total += frames as usize;
        }
        Script {
            blocks: specs,
            input: test_signal(rng, total, sample_rate),
        }
    }

    fn run_script(&self, r: &mut Runner, script: &Script) -> Result<Vec<f32>, String> {
        let mut out = Vec::with_capacity(script.input.len());
        let mut pos = 0;
        for b in &script.blocks {
            let n = b.frames as usize;
            out.extend_from_slice(r.block(
                &script.input[pos..pos + n],
                &b.events,
                b.reset_before,
            )?);
            pos += n;
            let mut s = self.stats.borrow_mut();
            s.blocks += 1;
            s.frames += u64::from(b.frames);
            s.events += b.events.len() as u64;
            s.zero_length_blocks += u64::from(b.frames == 0);
            s.resets += u64::from(b.reset_before);
        }
        Ok(out)
    }

    /// The allocation checker must be installed (otherwise every RT check would silently pass).
    pub fn check_alloc_checker(&self) -> Result<(), HostFailure> {
        let active = alloc_checks_active();
        self.stats.borrow_mut().alloc_checked = active;
        if !active {
            return Err(self.fail(
                "alloc_checker",
                "the allocation checker is not the global allocator: add \
                 `vox_module_api::install_test_allocator!();` to the test crate",
            ));
        }
        Ok(())
    }

    /// `validate_schema`, `api_version`, MONO layout support.
    pub fn check_schema(&self) -> Result<(), HostFailure> {
        let m = (self.make)();
        validate_schema(m.params(), m.groups()).map_err(|e| self.fail("schema", e.to_string()))?;
        let api = m.descriptor().api_version;
        if api != MODULE_API_VERSION {
            return Err(self.fail(
                "schema",
                format!("api_version {api} != MODULE_API_VERSION {MODULE_API_VERSION}"),
            ));
        }
        if !m.supported_layouts().contains(&ChannelLayout::MONO) {
            return Err(self.fail(
                "schema",
                "supported_layouts() lacks MONO (the test host drives MONO only)",
            ));
        }
        Ok(())
    }

    /// Activation at every sample rate in both modes; latency/tail callable and constant;
    /// deactivate + re-activate.
    pub fn check_activate(&self) -> Result<(), HostFailure> {
        for &sr in &self.sample_rates {
            for mode in MODES {
                let cfg = self.config(sr, mode);
                let mut m = self
                    .instance(&cfg, None)
                    .map_err(|e| self.fail("activate", e))?;
                let (latency, tail) = (m.latency_samples(), m.tail());
                if (m.latency_samples(), m.tail()) != (latency, tail) {
                    return Err(
                        self.fail("activate", "latency_samples()/tail() changed while active")
                    );
                }
                m.deactivate();
                m.activate(&cfg)
                    .map_err(|e| self.fail("activate", format!("re-activate({cfg:?}): {e}")))?;
                m.deactivate();
            }
        }
        Ok(())
    }

    /// Random blocks (0..=max_block), random events, varied signals, occasional resets, all under
    /// the allocation checker with finite output; then the parameter mirror (`param_value` equals
    /// the last event value; `None` for an unknown id).
    pub fn check_process(&self) -> Result<(), HostFailure> {
        for (i, &sr) in self.sample_rates.iter().enumerate() {
            for (j, mode) in MODES.into_iter().enumerate() {
                let mut rng = TestRng::new(self.seed ^ ((i as u64) << 8 | j as u64));
                let cfg = self.config(sr, mode);
                let mut r = self
                    .runner(&cfg, None)
                    .map_err(|e| self.fail("process", e))?;
                let params = r.module.params().to_vec();
                let script = self.script(&mut rng, &params, sr, self.blocks);
                self.run_script(&mut r, &script)
                    .map_err(|e| self.fail("process", format!("{e} ({sr} Hz, {mode:?})")))?;
                let mut expected = BTreeMap::new();
                for e in script.blocks.iter().flat_map(|b| &b.events) {
                    expected.insert(e.id, e.value);
                }
                for (id, v) in expected {
                    match r.module.param_value(id) {
                        Some(got) if exact(got, v) => {}
                        got => {
                            return Err(self.fail(
                                "process",
                                format!("param_value({id:?}) = {got:?}, expected the last event value {v}"),
                            ));
                        }
                    }
                }
                let unknown = ParamId(
                    params
                        .iter()
                        .map(|p| p.id.0)
                        .max()
                        .map_or(0, |m| m.wrapping_add(1)),
                );
                if !params.iter().any(|p| p.id == unknown)
                    && r.module.param_value(unknown).is_some()
                {
                    return Err(self.fail(
                        "process",
                        format!("param_value({unknown:?}) of an unknown id is Some"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// `reset` under the allocation checker after processing; output finite afterwards.
    pub fn check_reset(&self) -> Result<(), HostFailure> {
        let mut rng = TestRng::new(self.seed ^ 0x2E5E7);
        let cfg = self.config(PROBE_RATE, ProcessMode::Realtime);
        let mut r = self.runner(&cfg, None).map_err(|e| self.fail("reset", e))?;
        let params = r.module.params().to_vec();
        let script = self.script(&mut rng, &params, PROBE_RATE, 32);
        self.run_script(&mut r, &script)
            .map_err(|e| self.fail("reset", e))?;
        let module = &mut r.module;
        if let Err(n) = no_alloc(|| module.reset()) {
            return Err(self.fail("reset", format!("{n} heap (de)allocation(s) in reset()")));
        }
        let x = test_signal(&mut rng, self.max_block as usize, PROBE_RATE);
        r.block(&x, &[], false)
            .map_err(|e| self.fail("reset", format!("after reset: {e}")))?;
        Ok(())
    }

    /// A zero-length block must apply its events: `param_value` updates, and the following
    /// blocks (enough to cover `latency_samples()`) sound like blocks where the same event came
    /// at offset 0. Offline mode.
    pub fn check_param_flush(&self) -> Result<(), HostFailure> {
        let cfg = self.config(PROBE_RATE, ProcessMode::Offline);
        let n = self.max_block.min(256) as usize;
        let params = non_read_only((self.make)().params());
        for p in &params {
            let fail = |d: String| self.fail("param_flush", format!("`{}`: {d}", p.key));
            let mut rng = TestRng::new(self.seed ^ 0xF1A5 ^ u64::from(p.id.0));
            let mut a = self.runner(&cfg, None).map_err(fail)?;
            let mut b = self.runner(&cfg, None).map_err(fail)?;
            let warm = probe_signal(&mut rng, 0, n, PROBE_RATE);
            a.block(&warm, &[], false).map_err(fail)?;
            b.block(&warm, &[], false).map_err(fail)?;
            let v = far_value(p, a.module.param_value(p.id).unwrap_or(p.default));
            let ev = ParamEvent {
                offset: 0,
                id: p.id,
                value: v,
            };
            a.block(&[], &[ev], false).map_err(fail)?;
            match a.module.param_value(p.id) {
                Some(got) if exact(got, v) => {}
                got => {
                    return Err(fail(format!(
                        "after a zero-length block with an event to {v}, param_value = {got:?}"
                    )));
                }
            }
            let blocks = (a.module.latency_samples() as usize).div_ceil(n) + 1;
            let first = [ev];
            let (mut out_a, mut out_b) = (Vec::new(), Vec::new());
            for i in 0..blocks {
                let x = probe_signal(&mut rng, (1 + i) * n, n, PROBE_RATE);
                let events: &[ParamEvent] = if i == 0 { &first } else { &[] };
                out_a.extend_from_slice(a.block(&x, &[], false).map_err(fail)?);
                out_b.extend_from_slice(b.block(&x, events, false).map_err(fail)?);
            }
            if let Some(i) = first_diff(&out_a, &out_b, EFFECT_EPS) {
                return Err(fail(format!(
                    "an event in a zero-length block was not applied like the same event at offset 0 \
                     of the next block (first difference at sample {i}: {} vs {})",
                    out_a[i], out_b[i]
                )));
            }
        }
        Ok(())
    }

    /// An event at offset k must change the output first at exactly sample
    /// `k + latency_samples()` (never earlier); parameters declared with
    /// [`allow_delayed_effect`](Self::allow_delayed_effect) may change it later or not at all.
    /// Observes `ceil((k + latency) / n) + 1` blocks of n frames. Offline mode.
    pub fn check_event_timing(&self) -> Result<(), HostFailure> {
        if self.max_block < 2 {
            return Ok(());
        }
        let cfg = self.config(PROBE_RATE, ProcessMode::Offline);
        let n = self.max_block.min(256);
        let mut offsets = vec![1, n / 2, n - 1];
        offsets.dedup();
        let params = non_read_only((self.make)().params());
        for p in &params {
            for &k in &offsets {
                let fail = |d: String| {
                    self.fail("event_timing", format!("`{}` at offset {k}: {d}", p.key))
                };
                let mut rng =
                    TestRng::new(self.seed ^ 0x7131 ^ (u64::from(p.id.0) << 16 | u64::from(k)));
                let mut a = self.runner(&cfg, None).map_err(fail)?;
                let mut b = self.runner(&cfg, None).map_err(fail)?;
                let nn = n as usize;
                for w in 0..2 {
                    let warm = probe_signal(&mut rng, w * nn, nn, PROBE_RATE);
                    a.block(&warm, &[], false).map_err(fail)?;
                    b.block(&warm, &[], false).map_err(fail)?;
                }
                let v = far_value(p, a.module.param_value(p.id).unwrap_or(p.default));
                let ev = ParamEvent {
                    offset: k,
                    id: p.id,
                    value: v,
                };
                let latency = a.module.latency_samples() as usize;
                let expected = k as usize + latency;
                let blocks = expected.div_ceil(nn) + 1;
                let first = [ev];
                let (mut out_a, mut out_b) = (Vec::new(), Vec::new());
                for i in 0..blocks {
                    let x = probe_signal(&mut rng, (2 + i) * nn, nn, PROBE_RATE);
                    let events: &[ParamEvent] = if i == 0 { &first } else { &[] };
                    out_a.extend_from_slice(a.block(&x, events, false).map_err(fail)?);
                    out_b.extend_from_slice(b.block(&x, &[], false).map_err(fail)?);
                }
                let d = first_diff(&out_a, &out_b, EFFECT_EPS);
                if let Some(d) = d
                    && d < expected
                {
                    return Err(fail(format!(
                        "output changed at sample {d}, before offset {k} + latency {latency} \
                         ({} vs {})",
                        out_a[d], out_b[d]
                    )));
                }
                if !self.delayed.contains(&p.id) && d != Some(expected) {
                    return Err(fail(format!(
                        "expected the first change exactly at sample {expected} (offset {k} + \
                         latency {latency}), got {d:?}; declare deliberately delayed parameters \
                         with allow_delayed_effect"
                    )));
                }
                self.stats.borrow_mut().first_effect.push(EffectProbe {
                    key: p.key.clone(),
                    offset: k,
                    first_difference: d.map(|d| d as u32),
                });
            }
        }
        Ok(())
    }

    /// Save → load into a fresh instance → activate: same parameter values, `save_state` is
    /// idempotent, and bit-identical output versus the original (re-activated) instance on the
    /// same input and events. Also checks the saved state's shape (format version, every
    /// non-READ_ONLY key, finite values equal to `param_value`).
    pub fn check_state_roundtrip(&self) -> Result<(), HostFailure> {
        let fail = |d: String| self.fail("state_roundtrip", d);
        let mut rng = TestRng::new(self.seed ^ 0x57A7E);
        let cfg = self.config(PROBE_RATE, ProcessMode::Offline);
        let s0 = self.random_state(&mut rng);
        let mut a = self.runner(&cfg, Some(&s0)).map_err(fail)?;
        let params = a.module.params().to_vec();
        let script1 = self.script(&mut rng, &params, PROBE_RATE, 64);
        self.run_script(&mut a, &script1).map_err(fail)?;

        let saved = a
            .module
            .save_state()
            .map_err(|e| fail(format!("save_state: {e}")))?;
        let format = a.module.descriptor().state_format_version;
        if saved.format_version != format {
            return Err(fail(format!(
                "saved format_version {} != descriptor state_format_version {format}",
                saved.format_version
            )));
        }
        let keys = non_read_only(&params);
        if saved.params.len() != keys.len() {
            return Err(fail(format!(
                "saved {} params, expected exactly the {} non-READ_ONLY keys",
                saved.params.len(),
                keys.len()
            )));
        }
        for p in &keys {
            match (saved.params.get(&p.key), a.module.param_value(p.id)) {
                (Some(&v), Some(cur)) if v.is_finite() && exact(v, cur) => {}
                (v, cur) => {
                    return Err(fail(format!(
                        "`{}`: saved {v:?}, param_value {cur:?} (rule S1)",
                        p.key
                    )));
                }
            }
        }

        a.module.deactivate();
        a.module
            .activate(&cfg)
            .map_err(|e| fail(format!("re-activate: {e}")))?;
        a.steady_time = 0;
        let mut b = self.runner(&cfg, Some(&saved)).map_err(fail)?;
        let prepared = prepare_state(&*b.module, saved.clone())
            .map_err(|e| fail(format!("prepare_state: {e}")))?;
        let resaved = b
            .module
            .save_state()
            .map_err(|e| fail(format!("save_state: {e}")))?;
        if resaved.format_version != prepared.format_version
            || resaved.blob != prepared.blob
            || resaved.params.len() != prepared.params.len()
            || resaved
                .params
                .iter()
                .any(|(k, v)| prepared.params.get(k).is_none_or(|w| !exact(*v, *w)))
        {
            return Err(fail(format!(
                "save → load → save changed the state: {prepared:?} → {resaved:?}"
            )));
        }
        for p in &keys {
            let (va, vb) = (a.module.param_value(p.id), b.module.param_value(p.id));
            if !matches!((va, vb), (Some(x), Some(y)) if exact(x, y)) {
                return Err(fail(format!(
                    "`{}`: param_value {va:?} (original) vs {vb:?} (loaded)",
                    p.key
                )));
            }
        }
        let script2 = self.script(&mut rng, &params, PROBE_RATE, 64);
        let out_a = self.run_script(&mut a, &script2).map_err(fail)?;
        let out_b = self.run_script(&mut b, &script2).map_err(fail)?;
        if let Some(i) = first_bit_diff(&out_a, &out_b) {
            return Err(fail(format!(
                "output differs at sample {i} after the round-trip: {:?} vs {:?}",
                out_a.get(i),
                out_b.get(i)
            )));
        }
        Ok(())
    }

    /// Two instances with the same state, config, input and events are bit-identical.
    pub fn check_determinism(&self) -> Result<(), HostFailure> {
        let fail = |d: String| self.fail("determinism", d);
        for (i, &sr) in self.sample_rates.iter().enumerate() {
            let mut rng = TestRng::new(self.seed ^ 0xDE7 ^ i as u64);
            let cfg = self.config(sr, ProcessMode::Offline);
            let state = self.random_state(&mut rng);
            let mut a = self.runner(&cfg, Some(&state)).map_err(fail)?;
            let mut b = self.runner(&cfg, Some(&state)).map_err(fail)?;
            let params = a.module.params().to_vec();
            let script = self.script(&mut rng, &params, sr, 64);
            let out_a = self.run_script(&mut a, &script).map_err(fail)?;
            let out_b = self.run_script(&mut b, &script).map_err(fail)?;
            if let Some(i) = first_bit_diff(&out_a, &out_b) {
                return Err(fail(format!(
                    "two instances differ at sample {i} ({sr} Hz): {:?} vs {:?}",
                    out_a.get(i),
                    out_b.get(i)
                )));
            }
        }
        Ok(())
    }

    /// `text_to_value(value_to_text(v))` equals `clamp_quantize(v)` within half a displayed unit
    /// (exact for stepped, enum and bool); enum labels parse to their index; the normalized
    /// mapping round-trips.
    pub fn check_text_roundtrip(&self) -> Result<(), HostFailure> {
        let m = (self.make)();
        let mut rng = TestRng::new(self.seed ^ 0x7E47);
        for p in m.params() {
            let fail = |d: String| self.fail("text_roundtrip", format!("`{}`: {d}", p.key));
            let exact_kind = p.is_stepped() || p.is_enum() || p.flags.contains(ParamFlags::BOOL);
            let mut values = vec![p.min, p.max, p.default];
            values.extend((0..=32).map(|i| p.from_normalized(f64::from(i) / 32.0)));
            values.extend((0..64).map(|_| p.from_normalized(rng.unit_f64())));
            for v in values {
                let q = p.clamp_quantize(v);
                let text = p.value_to_text(v);
                let Some(back) = p.text_to_value(&text) else {
                    return Err(fail(format!("{v} → {text:?} does not parse back")));
                };
                let tol = if exact_kind {
                    0.0
                } else {
                    0.5 * p.text_resolution(v) * (1.0 + 1e-9) + 1e-9 * q.abs().max(1.0)
                };
                let ok = (back - q).abs() <= tol;
                if !ok {
                    return Err(fail(format!(
                        "{v} → {text:?} → {back}, expected {q} ± {tol}"
                    )));
                }
                let back_n = p.from_normalized(p.to_normalized(q));
                let tol_n = if exact_kind {
                    0.0
                } else {
                    1e-9 * (p.max - p.min).abs().max(q.abs())
                };
                let ok = (back_n - q).abs() <= tol_n;
                if !ok {
                    return Err(fail(format!("normalized round-trip {q} → {back_n}")));
                }
            }
            for (i, label) in p.enum_labels.iter().enumerate() {
                let got = p.text_to_value(&label.text);
                if !got.is_some_and(|g| exact(g, i as f64)) {
                    return Err(fail(format!(
                        "enum label {:?} parses to {got:?}, expected {i}",
                        label.text
                    )));
                }
            }
        }
        Ok(())
    }

    /// `extension(id)` returns `None` or the variant matching `id`.
    pub fn check_extensions(&self) -> Result<(), HostFailure> {
        let cfg = self.config(PROBE_RATE, ProcessMode::Realtime);
        let m = self
            .instance(&cfg, None)
            .map_err(|e| self.fail("extensions", e))?;
        for id in ExtensionId::ALL {
            if let Some(ext) = m.extension(id)
                && ext.id() != id
            {
                return Err(self.fail("extensions", format!("extension({id:?}) returned {ext:?}")));
            }
        }
        Ok(())
    }
}
