//! T-005 rack skeleton tests: chain processing, event routing across sub-blocks, latency sum,
//! chain edits, RT safety.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use vox_module_api::test_util::{TestGain, TestRng, no_alloc};
use vox_module_api::{
    ActivateConfig, ChannelLayout, EventListError, LocalizedText, MODULE_API_VERSION, Module,
    ModuleDescriptor, ModuleError, ModuleState, ParamEvent, ParamFlags, ParamId, ParamInfo,
    ProcessContext, ProcessMode, ProcessStatus, StateError, Tail, Taper, Transport, Unit, Version,
    prepare_state, segments,
};
use vox_rack::{PushEventError, Rack, RackError, RackOptions, swap_chain};

vox_module_api::install_test_allocator!();

const SR: f64 = 48_000.0;

fn cfg(max_block: u32) -> ActivateConfig {
    ActivateConfig {
        sample_rate: SR,
        max_block,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

fn gain(db: f64) -> Box<dyn Module> {
    let mut m = TestGain::new();
    m.load_state(&prepare_state(&m, TestGain::state_with_gain_db(db)).unwrap())
        .unwrap();
    Box::new(m)
}

fn gain_event(offset: u32, db: f64) -> ParamEvent {
    ParamEvent {
        offset,
        id: TestGain::GAIN_DB,
        value: db,
    }
}

fn sine_1k(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * 1000.0 * i as f64 / SR).sin()) as f32)
        .collect()
}

fn noise(seed: u64, n: usize) -> Vec<f32> {
    let mut rng = TestRng::new(seed);
    (0..n).map(|_| 0.8 * rng.bipolar_f32()).collect()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>() / x.len() as f64).sqrt()
}

/// Runs `input` through `rack` in host blocks of `block` frames, under the allocation checker.
fn run(rack: &mut Rack, input: &[f32], block: usize) -> Vec<f32> {
    let mut out = vec![0.0; input.len()];
    for (i, o) in input.chunks(block).zip(out.chunks_mut(block)) {
        let status =
            no_alloc(|| rack.process(Transport::default(), i, o)).expect("rack.process allocated");
        assert_eq!(status, ProcessStatus::Continue);
    }
    out
}

fn bit_identical(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

// ---------------------------------------------------------------------------------------------
// Probe module: fixed delay (declared latency) + an event log with absolute sample positions.

type Log = Arc<std::sync::Mutex<Vec<(u64, f64)>>>;

struct Probe {
    desc: ModuleDescriptor,
    params: Vec<ParamInfo>,
    latency: u32,
    line: Vec<f32>,
    pos: usize,
    log: Vec<(u64, f64)>,
    shared_log: Log,
    value: f64,
    layouts: Vec<ChannelLayout>,
    fail_activate: bool,
    deactivations: Arc<AtomicUsize>,
}

impl Probe {
    const VALUE: ParamId = ParamId(0);

    fn new(latency: u32) -> Self {
        let mut value = TestGain::gain_param();
        value.key = "value".into();
        value.unit = Unit::None;
        value.taper = Taper::Linear;
        value.min = 0.0;
        value.max = 1000.0;
        value.smoothing_ms = 0.0;
        Self {
            desc: ModuleDescriptor {
                id: "org.powervoice.test-probe".into(),
                version: Version::new(0, 1, 0),
                name: LocalizedText::plain("Probe"),
                vendor: "PowerVoice".into(),
                description: LocalizedText::plain("delay + event log"),
                url: None,
                features: Vec::new(),
                state_format_version: 1,
                api_version: MODULE_API_VERSION,
            },
            params: vec![value],
            latency,
            line: Vec::new(),
            pos: 0,
            log: Vec::new(),
            shared_log: Log::default(),
            value: 0.0,
            layouts: vec![ChannelLayout::MONO],
            fail_activate: false,
            deactivations: Arc::default(),
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
        self.log = Vec::with_capacity(4096);
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
                        .push((ctx.steady_time + u64::from(seg.start), ev.value));
                }
                self.value = ev.value;
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
        ProcessStatus::Continue
    }
    fn reset(&mut self) {
        self.line.fill(0.0);
        self.pos = 0;
    }
    fn param_value(&self, id: ParamId) -> Option<f64> {
        (id == Self::VALUE).then_some(self.value)
    }
    fn save_state(&self) -> Result<ModuleState, StateError> {
        let mut s = ModuleState::new(1);
        s.params.insert("value".into(), self.value);
        Ok(s)
    }
    fn load_state(&mut self, state: &ModuleState) -> Result<(), StateError> {
        self.value = state.params.get("value").copied().unwrap_or(0.0);
        Ok(())
    }
}

fn value_event(offset: u32, value: f64) -> ParamEvent {
    ParamEvent {
        offset,
        id: Probe::VALUE,
        value,
    }
}

thread_local! {
    /// Log of the probe created by `probe_with_thread_log` on this test thread.
    static PROBE_LOG: Log = Log::default();
}

/// Deactivates `rack` (the probe publishes its log on deactivate) and returns the log:
/// `(absolute sample position, value)` per received event.
fn probe_log(rack: &mut Rack) -> Vec<(u64, f64)> {
    rack.deactivate();
    PROBE_LOG.with(|l| l.lock().unwrap().clone())
}

fn probe_with_thread_log(latency: u32) -> Box<dyn Module> {
    let mut p = Probe::new(latency);
    p.shared_log = PROBE_LOG.with(|l| l.clone());
    p.shared_log.lock().unwrap().clear();
    Box::new(p)
}

// ---------------------------------------------------------------------------------------------

#[test]
fn three_test_gains_at_minus_6_db_give_minus_18_db() {
    let x = sine_1k(48_000);
    let settle = TestGain::ramp_samples(SR) as usize;

    // (a) gains from state; (b) gains from events at offset 0 (after the 5 ms ramp).
    let mut from_state = Rack::new();
    let mut from_events = Rack::new();
    for _ in 0..3 {
        from_state.push(gain(-6.0)).unwrap();
        from_events.push(gain(0.0)).unwrap();
    }
    from_state.activate(&cfg(256)).unwrap();
    from_events.activate(&cfg(256)).unwrap();
    for slot in 0..3 {
        from_events.push_event(slot, gain_event(0, -6.0)).unwrap();
    }
    for rack in [&mut from_state, &mut from_events] {
        let y = run(rack, &x, 480);
        let db = 20.0 * (rms(&y[settle..]) / rms(&x[settle..])).log10();
        assert!((db + 18.0).abs() <= 0.01, "{db} dB");
    }
}

#[test]
fn events_split_at_sub_block_boundaries_keep_their_sample_position() {
    let x = noise(1, 1000);
    let events = [
        gain_event(0, -20.0),
        gain_event(10, -3.0),
        gain_event(63, -40.0),
        gain_event(64, 6.0),
        gain_event(500, -60.0),
        gain_event(999, 0.0),
    ];
    // Reference: TestGain processed directly in one 1000-frame block.
    let mut direct = TestGain::new();
    direct.activate(&cfg(1024)).unwrap();
    let mut expected = vec![0.0; 1000];
    let mut out_events = vox_module_api::OutputEvents::with_capacity(8);
    let mut ctx = ProcessContext::new(1000, 0, Transport::default(), &events, &mut out_events);
    direct.process(&mut ctx, &[&x], &mut [&mut expected]);

    // Rack with max_block 64: the 1000-frame host block is split into 16 sub-blocks.
    let mut rack = Rack::new();
    rack.push(gain(0.0)).unwrap();
    rack.activate(&cfg(64)).unwrap();
    for e in events {
        rack.push_event(0, e).unwrap();
    }
    let got = run(&mut rack, &x, 1000);
    assert!(bit_identical(&got, &expected));
    assert_eq!(
        rack.module(0)
            .unwrap()
            .param_value(TestGain::GAIN_DB)
            .map(f64::to_bits),
        Some(0.0_f64.to_bits())
    );
}

#[test]
fn chain_matches_modules_processed_in_sequence() {
    let x = noise(2, 3000);
    let (g1, g2, g3) = (-3.0, 4.5, -10.0);
    let mut expected = x.clone();
    for db in [g1, g2, g3] {
        let mut m = gain(db);
        m.activate(&cfg(4096)).unwrap();
        let input = expected.clone();
        let mut out_events = vox_module_api::OutputEvents::with_capacity(8);
        let mut ctx = ProcessContext::new(3000, 0, Transport::default(), &[], &mut out_events);
        m.process(&mut ctx, &[&input], &mut [&mut expected]);
    }
    for (n_slots_block, max_block) in [(1usize, 1u32), (37, 16), (3000, 1024), (1024, 1024)] {
        let mut rack = Rack::new();
        for db in [g1, g2, g3] {
            rack.push(gain(db)).unwrap();
        }
        rack.activate(&cfg(max_block)).unwrap();
        let got = run(&mut rack, &x, n_slots_block);
        assert!(
            bit_identical(&got, &expected),
            "host block {n_slots_block}, max_block {max_block}"
        );
    }
}

#[test]
fn full_event_lists_carry_over_without_losing_values() {
    // 4 events per process() call, 16 queued.
    let options = RackOptions {
        event_capacity: 4,
        queue_capacity: 16,
    };
    let mut rack = Rack::with_options(options);
    rack.push(probe_with_thread_log(0)).unwrap();
    rack.activate(&cfg(10)).unwrap();
    // 30-frame host block → sub-blocks [0,10) [10,20) [20,30).
    for (offset, v) in [
        (0, 1.0),
        (0, 2.0),
        (0, 3.0),
        (0, 4.0),
        (0, 5.0),
        (0, 6.0),
        (15, 7.0),
        (29, 8.0),
        (29, 9.0),
        (29, 10.0),
        (29, 11.0),
        (29, 12.0),
    ] {
        rack.push_event(0, value_event(offset, v)).unwrap();
    }
    let x = vec![0.0; 30];
    run(&mut rack, &x, 30);
    // Next call: the leftovers arrive at its offset 0 (absolute sample 30).
    run(&mut rack, &x, 30);
    assert_eq!(
        rack.module(0)
            .unwrap()
            .param_value(Probe::VALUE)
            .map(f64::to_bits),
        Some(12.0_f64.to_bits())
    );
    let log = probe_log(&mut rack);
    let expected: Vec<(u64, f64)> = vec![
        (0, 1.0),
        (0, 2.0),
        (0, 3.0),
        (0, 4.0), // sub-block 0 full
        (10, 5.0),
        (10, 6.0),
        (15, 7.0), // carried to the start of sub-block 1, then on time
        (29, 8.0),
        (29, 9.0),
        (29, 10.0),
        (29, 11.0), // sub-block 2 full
        (30, 12.0), // carried to the next call
    ];
    assert_eq!(log.len(), expected.len(), "{log:?}");
    for ((pos, v), (epos, ev)) in log.iter().zip(&expected) {
        assert_eq!((*pos, v.to_bits()), (*epos, ev.to_bits()), "{log:?}");
    }
}

#[test]
fn push_event_errors() {
    let mut rack = Rack::with_options(RackOptions {
        event_capacity: 2,
        queue_capacity: 2,
    });
    rack.push(gain(0.0)).unwrap();
    assert_eq!(
        rack.push_event(1, gain_event(0, 0.0)),
        Err(PushEventError::NoSuchSlot(1))
    );
    rack.push_event(0, gain_event(5, 0.0)).unwrap();
    assert_eq!(
        rack.push_event(0, gain_event(4, 0.0)),
        Err(PushEventError::List(EventListError::OutOfOrder))
    );
    rack.push_event(0, gain_event(5, 0.0)).unwrap();
    assert_eq!(
        rack.push_event(0, gain_event(6, 0.0)),
        Err(PushEventError::List(EventListError::Full))
    );
}

#[test]
fn zero_length_block_flushes_events() {
    let mut rack = Rack::new();
    rack.push(gain(0.0)).unwrap();
    rack.activate(&cfg(64)).unwrap();
    rack.push_event(0, gain_event(0, -9.0)).unwrap();
    no_alloc(|| rack.process(Transport::default(), &[], &mut [])).unwrap();
    assert_eq!(
        rack.module(0)
            .unwrap()
            .param_value(TestGain::GAIN_DB)
            .map(f64::to_bits),
        Some((-9.0_f64).to_bits())
    );
}

#[test]
fn latency_is_the_sum_of_slot_latencies() {
    let mut rack = Rack::new();
    rack.push(Box::new(Probe::new(3))).unwrap();
    rack.push(gain(0.0)).unwrap();
    rack.push(Box::new(Probe::new(5))).unwrap();
    assert_eq!(rack.latency_samples(), 0, "unknown until activated");
    rack.activate(&cfg(32)).unwrap();
    assert_eq!(rack.latency_samples(), 8);
    assert_eq!(rack.slot_latency_samples(2), Some(5));
    assert_eq!(rack.slot_tail(0), Some(Tail::Samples(3)));
    // The chain output is the input delayed by the total latency (reporting only, no compensation).
    let x = noise(3, 200);
    let y = run(&mut rack, &x, 50);
    assert!(y[..8].iter().all(|&s| s == 0.0));
    assert!(bit_identical(&y[8..], &x[..192]));
    // Inserting into an active rack activates the module; latency follows.
    rack.insert(1, Box::new(Probe::new(10))).unwrap();
    assert_eq!(rack.latency_samples(), 18);
    rack.remove(1).unwrap();
    assert_eq!(rack.latency_samples(), 8);
}

#[test]
fn insert_remove_reorder() {
    let mut rack = Rack::new();
    rack.push(gain(-1.0)).unwrap();
    rack.push(gain(-2.0)).unwrap();
    rack.insert(0, gain(-3.0)).unwrap();
    let order = |r: &Rack| -> Vec<f64> {
        (0..r.len())
            .map(|i| r.module(i).unwrap().param_value(TestGain::GAIN_DB).unwrap())
            .collect()
    };
    assert_eq!(order(&rack), vec![-3.0, -1.0, -2.0]);
    rack.move_slot(0, 2).unwrap();
    assert_eq!(order(&rack), vec![-1.0, -2.0, -3.0]);
    rack.move_slot(2, 1).unwrap();
    assert_eq!(order(&rack), vec![-1.0, -3.0, -2.0]);
    let removed = rack.remove(1).unwrap();
    assert_eq!(removed.param_value(TestGain::GAIN_DB), Some(-3.0));
    assert_eq!(order(&rack), vec![-1.0, -2.0]);
    assert!(matches!(
        rack.remove(5),
        Err(RackError::IndexOutOfRange { index: 5, len: 2 })
    ));
    assert!(matches!(
        rack.move_slot(0, 2),
        Err(RackError::IndexOutOfRange { index: 2, .. })
    ));
    assert!(matches!(
        rack.insert(3, gain(0.0)),
        Err(RackError::IndexOutOfRange { .. })
    ));

    // Removing from an active rack deactivates the module.
    let mut p = Probe::new(0);
    let deactivations = p.deactivations.clone();
    p.shared_log = Log::default();
    rack.push(Box::new(p)).unwrap();
    rack.activate(&cfg(16)).unwrap();
    rack.remove(2).unwrap();
    assert_eq!(deactivations.load(Ordering::Relaxed), 1);
}

#[test]
fn rejects_non_mono_modules_and_invalid_schemas() {
    let mut rack = Rack::new();
    let mut stereo = Probe::new(0);
    stereo.layouts = vec![ChannelLayout::STEREO];
    assert!(matches!(
        rack.push(Box::new(stereo)),
        Err(RackError::UnsupportedLayout { .. })
    ));
    let mut bad = Probe::new(0);
    bad.params[0].flags = ParamFlags::READ_ONLY | ParamFlags::AUTOMATABLE;
    assert!(matches!(
        rack.push(Box::new(bad)),
        Err(RackError::Schema { .. })
    ));
    assert!(rack.is_empty());
}

#[test]
fn activation_failure_rolls_back() {
    let mut rack = Rack::new();
    let ok = Probe::new(0);
    let deactivations = ok.deactivations.clone();
    rack.push(Box::new(ok)).unwrap();
    let mut failing = Probe::new(0);
    failing.fail_activate = true;
    rack.push(Box::new(failing)).unwrap();
    assert!(matches!(
        rack.activate(&cfg(16)),
        Err(RackError::Activate { index: 1, .. })
    ));
    assert!(!rack.is_active());
    assert_eq!(
        deactivations.load(Ordering::Relaxed),
        1,
        "slot 0 deactivated again"
    );
    let mut rack = Rack::new();
    assert!(matches!(
        rack.activate(&ActivateConfig {
            max_block: 0,
            ..cfg(1)
        }),
        Err(RackError::InvalidConfig(_))
    ));
    assert!(matches!(
        rack.activate(&ActivateConfig {
            layout: ChannelLayout::STEREO,
            ..cfg(1)
        }),
        Err(RackError::InvalidConfig(_))
    ));
    rack.activate(&cfg(8)).unwrap();
    assert!(matches!(
        rack.activate(&cfg(8)),
        Err(RackError::AlreadyActive)
    ));
}

#[test]
fn inactive_or_empty_rack_passes_through() {
    let x = noise(4, 100);
    let mut rack = Rack::new();
    let mut y = vec![0.0; 100];
    rack.process(Transport::default(), &x, &mut y);
    assert!(bit_identical(&x, &y));
    rack.activate(&cfg(16)).unwrap();
    let y = run(&mut rack, &x, 33);
    assert!(bit_identical(&x, &y));
}

#[test]
fn process_reset_and_push_are_allocation_free() {
    let mut rack = Rack::new();
    for db in [-1.0, -2.0, -3.0, -4.0] {
        rack.push(gain(db)).unwrap();
    }
    rack.activate(&cfg(128)).unwrap();
    let x = noise(5, 5000);
    let mut y = vec![0.0; 5000];
    let mut rng = TestRng::new(9);
    no_alloc(|| {
        let mut pos = 0;
        while pos < x.len() {
            let n = (rng.range_u32(0, 700) as usize).min(x.len() - pos);
            let _ = rack.push_event(rng.below(4) as usize, gain_event(0, -rng.unit_f64() * 30.0));
            rack.process(Transport::default(), &x[pos..pos + n], &mut y[pos..pos + n]);
            if rng.chance(0.05) {
                rack.reset();
            }
            pos += n;
        }
    })
    .expect("rack RT path allocated");
    assert!(y.iter().all(|s| s.is_finite()));
}

#[test]
fn swap_chain_is_rt_safe_and_returns_the_retired_chain() {
    let mut live = Box::new(Rack::new());
    live.push(gain(-6.0)).unwrap();
    live.activate(&cfg(64)).unwrap();
    let mut next = Box::new(Rack::new());
    next.push(gain(-12.0)).unwrap();
    next.activate(&cfg(64)).unwrap();

    let mut retired = no_alloc(|| swap_chain(&mut live, next)).expect("swap allocated");
    let x = vec![1.0_f32; 64];
    let y = run(&mut live, &x, 64);
    assert_eq!(y[63].to_bits(), TestGain::db_to_gain(-12.0).to_bits());
    // Off the audio thread: deactivate and drop the retired chain.
    assert_eq!(
        retired.module(0).unwrap().param_value(TestGain::GAIN_DB),
        Some(-6.0)
    );
    retired.deactivate();
    drop(retired);
}

#[test]
fn rack_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Rack>();
    assert_send::<Box<Rack>>();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "differ from the live chain")]
fn swap_chain_rejects_a_mismatched_chain_in_debug_builds() {
    let mut live = Box::new(Rack::new());
    live.activate(&cfg(64)).unwrap();
    let mut next = Box::new(Rack::new());
    next.activate(&cfg(128)).unwrap();
    let _retired = swap_chain(&mut live, next);
}
