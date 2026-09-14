//! T-803: modules whose factory `loads_async` (out-of-process plugins) are created and
//! activated off the rack's control thread. The slot shows "Loading" (dry, latency 0, written
//! back verbatim) until the instance arrives at a later `tick`; a failed load leaves a failed
//! slot with the reason (Retry loads again); a removed slot's late instance is discarded; a
//! replacement (restart) keeps the current instance playing and keeps edits made meanwhile.

vox_module_api::install_test_allocator!();

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use vox_module_api::{
    ActivateConfig, ChannelLayout, Module, ModuleDescriptor, ModuleError, ModuleFactory, ModuleRef,
    ProcessMode, Transport,
};
use vox_modules::Gain;
use vox_rack::{
    LiveRack, MAX_BLOCK, RackHost, RackModel, RackNotice, RackOptions, Registry, SlotModel,
    SlotStatus,
};

const RATE: f64 = 48_000.0;
const BLOCK: usize = 256;

/// The built-in Gain behind a slow (optionally failing) asynchronous factory.
struct SlowGain {
    descriptor: ModuleDescriptor,
    delay: Duration,
    fail: AtomicBool,
    created: AtomicUsize,
}

impl ModuleFactory for SlowGain {
    fn descriptor(&self) -> &ModuleDescriptor {
        &self.descriptor
    }

    fn create(&self) -> Result<Box<dyn Module>, ModuleError> {
        std::thread::sleep(self.delay);
        self.created.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(ModuleError::External("the plugin refused to load".into()));
        }
        Ok(Box::new(Gain::new()))
    }

    fn loads_async(&self) -> bool {
        true
    }
}

fn slow(delay_ms: u64, fail: bool) -> Arc<SlowGain> {
    Arc::new(SlowGain {
        descriptor: Gain::new().descriptor().clone(),
        delay: Duration::from_millis(delay_ms),
        fail: AtomicBool::new(fail),
        created: AtomicUsize::new(0),
    })
}

fn registry(f: &Arc<SlowGain>) -> Arc<Registry> {
    Arc::new(Registry::with_factories([f.clone() as Arc<dyn ModuleFactory>]).unwrap())
}

fn rt() -> ActivateConfig {
    ActivateConfig {
        sample_rate: RATE,
        max_block: MAX_BLOCK,
        mode: ProcessMode::Realtime,
        layout: ChannelLayout::MONO,
    }
}

fn slot(gain_db: f64) -> SlotModel {
    SlotModel::new(
        &ModuleRef::of(Gain::new().descriptor()),
        false,
        &Gain::state_with_gain_db(gain_db),
    )
}

fn new_rack(f: &Arc<SlowGain>, slots: Vec<SlotModel>) -> (RackHost, LiveRack) {
    RackHost::new(
        registry(f),
        rt(),
        RackOptions::default(),
        &RackModel { slots },
    )
    .unwrap()
}

/// One block of ones through `live`, then a tick; returns the block's output.
fn step(host: &mut RackHost, live: &mut LiveRack, notices: &mut Vec<RackNotice>) -> Vec<f32> {
    let x = vec![1.0f32; BLOCK];
    let mut y = vec![0.0f32; BLOCK];
    live.process(
        Transport {
            playing: true,
            position_samples: None,
        },
        &x,
        &mut y,
    );
    notices.extend(host.tick());
    y
}

/// Processes and ticks until no background load is in flight (5 s at most).
fn wait_loaded(host: &mut RackHost, live: &mut LiveRack) -> Vec<RackNotice> {
    let mut notices = Vec::new();
    let t0 = Instant::now();
    while host.is_loading() {
        assert!(t0.elapsed() < Duration::from_secs(5), "load never finished");
        step(host, live, &mut notices);
        std::thread::sleep(Duration::from_millis(2));
    }
    step(host, live, &mut notices);
    notices
}

fn status(host: &RackHost) -> SlotStatus {
    host.slot_info(0).unwrap().status
}

#[test]
fn insert_returns_at_once_and_the_slot_loads_in_the_background() {
    let f = slow(150, false);
    let (mut host, mut live) = new_rack(&f, Vec::new());
    let t0 = Instant::now();
    host.insert(0, slot(-6.0)).unwrap();
    assert!(t0.elapsed() < Duration::from_millis(100), "insert blocked");
    let info = host.slot_info(0).unwrap();
    assert_eq!(info.status, SlotStatus::Loading);
    assert_eq!(info.name, "Gain");
    assert!(info.params.is_empty());
    assert_eq!(host.total_latency_samples(), 0);
    // Saved while loading: the stored slot, verbatim.
    assert_eq!(host.model().slots, vec![slot(-6.0)]);
    assert!(host.slot_state(0).is_err());

    let notices = wait_loaded(&mut host, &mut live);
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotLoaded { index: 0, .. })),
        "{notices:?}"
    );
    assert_eq!(status(&host), SlotStatus::Active);
    assert_eq!(host.param_value(0, Gain::GAIN_DB), Some(-6.0));
    assert_eq!(f.created.load(Ordering::SeqCst), 1);
    // The instance is heard once its fade-in is over.
    let mut n = Vec::new();
    let mut y = Vec::new();
    for _ in 0..20 {
        y = step(&mut host, &mut live, &mut n);
    }
    let want = Gain::db_to_gain(-6.0) as f32;
    assert!(y.iter().all(|v| (v - want).abs() < 1e-6), "{:?}", &y[..4]);
    host.teardown(live);
}

#[test]
fn a_failed_background_load_leaves_a_failed_slot_and_retry_loads_again() {
    let f = slow(20, true);
    let (mut host, mut live) = new_rack(&f, Vec::new());
    host.insert(0, slot(-3.0)).unwrap();
    let notices = wait_loaded(&mut host, &mut live);
    let SlotStatus::Failed { message } = status(&host) else {
        panic!("not failed: {:?}", status(&host));
    };
    assert!(message.contains("the plugin refused to load"), "{message}");
    assert!(notices.iter().any(|n| matches!(
        n,
        RackNotice::SlotFailed { index: 0, message: m, .. } if *m == message
    )));
    assert_eq!(host.model().slots, vec![slot(-3.0)], "kept verbatim");

    f.fail.store(false, Ordering::SeqCst);
    host.restart(0).unwrap();
    assert_eq!(status(&host), SlotStatus::Loading);
    wait_loaded(&mut host, &mut live);
    assert_eq!(status(&host), SlotStatus::Active);
    assert_eq!(host.param_value(0, Gain::GAIN_DB), Some(-3.0));
    host.teardown(live);
}

#[test]
fn removing_a_loading_slot_discards_its_late_instance() {
    let f = slow(80, false);
    let (mut host, mut live) = new_rack(&f, Vec::new());
    host.insert(0, slot(0.0)).unwrap();
    host.remove(0).unwrap();
    assert!(host.is_empty());
    let mut notices = Vec::new();
    let t0 = Instant::now();
    while f.created.load(Ordering::SeqCst) == 0 || t0.elapsed() < Duration::from_millis(150) {
        assert!(t0.elapsed() < Duration::from_secs(5));
        step(&mut host, &mut live, &mut notices);
        std::thread::sleep(Duration::from_millis(5));
    }
    step(&mut host, &mut live, &mut notices);
    assert!(host.is_empty());
    assert!(!host.is_loading());
    assert!(
        !notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotLoaded { .. })),
        "{notices:?}"
    );
    host.teardown(live);
}

#[test]
fn a_rack_opened_with_async_slots_loads_them_in_the_background() {
    let f = slow(60, false);
    let (mut host, mut live) = new_rack(&f, vec![slot(-9.0)]);
    assert_eq!(status(&host), SlotStatus::Loading);
    assert_eq!(host.total_latency_samples(), 0);
    let notices = wait_loaded(&mut host, &mut live);
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotLoaded { .. }))
    );
    assert_eq!(status(&host), SlotStatus::Active);
    assert_eq!(host.param_value(0, Gain::GAIN_DB), Some(-9.0));
    // Documents opened later go the same way.
    host.load_model(&RackModel {
        slots: vec![slot(-1.0), slot(-2.0)],
    })
    .unwrap();
    assert_eq!(host.slot_info(1).unwrap().status, SlotStatus::Loading);
    wait_loaded(&mut host, &mut live);
    assert_eq!(host.param_value(1, Gain::GAIN_DB), Some(-2.0));
    host.teardown(live);
}

#[test]
fn a_replacement_loads_off_thread_while_the_old_instance_plays_and_keeps_edits() {
    let f = slow(120, false);
    let (mut host, mut live) = new_rack(&f, vec![slot(-6.0)]);
    wait_loaded(&mut host, &mut live);
    let t0 = Instant::now();
    host.restart(0).unwrap();
    assert!(t0.elapsed() < Duration::from_millis(80), "restart blocked");
    assert!(host.is_loading());
    // A working slot keeps showing Active while its replacement loads.
    assert_eq!(status(&host), SlotStatus::Active);
    // An edit made meanwhile wins over the replacement's (older) state.
    host.set_param(0, Gain::GAIN_DB, -12.0).unwrap();
    let notices = wait_loaded(&mut host, &mut live);
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::SlotRestarted { .. }))
    );
    assert_eq!(host.param_value(0, Gain::GAIN_DB), Some(-12.0));
    assert_eq!(f.created.load(Ordering::SeqCst), 2);
    let mut n = Vec::new();
    let mut y = Vec::new();
    for _ in 0..30 {
        y = step(&mut host, &mut live, &mut n);
    }
    let want = Gain::db_to_gain(-12.0) as f32;
    assert!(y.iter().all(|v| (v - want).abs() < 1e-6), "{:?}", &y[..4]);
    host.teardown(live);
}
