//! S3-06 (SPEC-014 §2.3, §2.9): Capture Noise Print's two engine steps — `nr_capture_prepare`
//! (target-slot resolution + the upstream model for the offline pre-render) and
//! `nr_capture_apply` (the live replacement) — round-tripped through the engine on the fake
//! backend, the same surface `src-tauri`'s IPC command wraps.

vox_module_api::install_test_allocator!();

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{
    EngineConfig, EngineEvent, HostId, ManualEngine, NOISE_REDUCTION_MODULE_ID, RackApiError,
    RackCommand,
};
use vox_modules::{Gain, NoiseReduction};
use vox_rack::{NoiseProfileStatus, RackNotice, Registry};

/// Seeded white noise in `[-amp, amp)` (xorshift64*, no extra test dependency).
fn white_noise(seed: u64, amp: f32, n: usize) -> Vec<f32> {
    let mut s = seed | 1;
    (0..n)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            let u = (s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64;
            amp * (2.0 * u - 1.0) as f32
        })
        .collect()
}

fn dev() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

fn rig() -> (ManualEngine, Arc<Mutex<Vec<EngineEvent>>>) {
    let fake = FakeBackend::new(7);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev().record_output()),
    );
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = EngineConfig::new(Arc::new(fake.clone()), registry);
    let ev = events.clone();
    cfg.events = Arc::new(move |e| ev.lock().unwrap().push(e));
    let clock = fake.clone();
    cfg.clock = Arc::new(move || clock.now_ns());
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    (eng, events)
}

/// Guards the engine crate's copy of the module id against silently drifting from the module
/// itself (`vox-engine` doesn't depend on `vox-modules` outside tests, MEMORY.md rationale in
/// `rack_api.rs`).
#[test]
fn nr_capture_module_id_matches_the_noise_reduction_module() {
    assert_eq!(NOISE_REDUCTION_MODULE_ID, NoiseReduction::ID);
}

/// No output stream open: `prepare` reports `Unavailable`, like every other rack command.
#[test]
fn prepare_is_unavailable_before_an_output_device_opens() {
    let fake = FakeBackend::new(1);
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let mut eng = ManualEngine::new(EngineConfig::new(Arc::new(fake), registry));
    eng.poll_devices();
    assert!(matches!(
        eng.nr_capture_prepare(None),
        Err(RackApiError::Unavailable)
    ));
}

/// With no NR slot, `prepare` inserts a default one at the top and reports it — mirroring every
/// other structural rack command's `RackChanged` broadcast.
#[test]
fn prepare_inserts_a_default_slot_when_none_exists_and_broadcasts_it() {
    let (mut eng, events) = rig();
    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();
    events.lock().unwrap().clear();

    let prep = eng.nr_capture_prepare(None).unwrap();
    assert_eq!(prep.index, 0);
    assert!(prep.inserted);
    assert!(
        prep.upstream_model.slots.is_empty(),
        "nothing precedes slot 0"
    );

    let snap = eng.rack_snapshot();
    assert_eq!(snap.slots.len(), 2);
    assert!(snap.slots[0].info.module.starts_with(NoiseReduction::ID));
    assert!(snap.slots[1].info.module.starts_with(Gain::ID));
    let saw_rack_changed = events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, EngineEvent::RackChanged(s) if s.slots.len() == 2));
    assert!(saw_rack_changed, "the insertion is broadcast");
}

/// An existing NR slot is found (not re-inserted); the upstream model carries the committed
/// state of the slots before it, omitting a placeholder.
#[test]
fn prepare_finds_an_existing_slot_and_builds_its_upstream_model() {
    let (mut eng, _events) = rig();
    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();
    eng.rack_command(RackCommand::SetParamText {
        index: 0,
        id: Gain::GAIN_DB,
        text: "-6.0".into(),
    })
    .unwrap();
    eng.rack_command(RackCommand::Add {
        module_id: NoiseReduction::ID.into(),
        index: 1,
    })
    .unwrap();

    let prep = eng.nr_capture_prepare(None).unwrap();
    assert_eq!(prep.index, 1);
    assert!(!prep.inserted);
    assert_eq!(prep.upstream_model.slots.len(), 1);
    assert_eq!(
        prep.upstream_model.slots[0].module,
        format!("{}@{}", Gain::ID, Gain::VERSION)
    );
    assert_eq!(
        prep.upstream_model.slots[0].state["params"]["gain_db"],
        serde_json::json!(-6.0)
    );
    assert_eq!(prep.values.len(), NoiseReduction::param_infos().len());
}

/// The hint picks a specific NR slot among several (SPEC-014 §2.3 "last-focused NR slot").
#[test]
fn prepare_honours_a_valid_hint_over_the_first_slot() {
    let (mut eng, _events) = rig();
    eng.rack_command(RackCommand::Add {
        module_id: NoiseReduction::ID.into(),
        index: 0,
    })
    .unwrap();
    eng.rack_command(RackCommand::Add {
        module_id: NoiseReduction::ID.into(),
        index: 1,
    })
    .unwrap();

    assert_eq!(eng.nr_capture_prepare(None).unwrap().index, 0);
    assert_eq!(eng.nr_capture_prepare(Some(1)).unwrap().index, 1);
    // An invalid hint (out of range) falls back to the first NR slot.
    assert_eq!(eng.nr_capture_prepare(Some(99)).unwrap().index, 0);
}

/// End to end: prepare, capture a print from white noise (no upstream slots), apply it — the
/// slot's status becomes `Loaded` and a `SlotRestarted` notice fires the live replacement.
#[test]
fn apply_installs_the_captured_print_live() {
    let (mut eng, events) = rig();
    let prep = eng.nr_capture_prepare(None).unwrap();
    assert!(prep.inserted);
    let noise = white_noise(1, 0.01, 48_000); // 1 s at 48 kHz, well above min_capture_samples
    let blob = prep
        .extension
        .capture(&noise, 48_000.0, &prep.values, &AtomicBool::new(false))
        .unwrap();

    events.lock().unwrap().clear();
    let snap = eng.nr_capture_apply(prep.index, blob).unwrap();
    assert_eq!(
        snap.slots[prep.index].info.noise_profile,
        Some(NoiseProfileStatus::Loaded)
    );
    let saw_restart = events.lock().unwrap().iter().any(|e| {
        matches!(
            e,
            EngineEvent::Rack(RackNotice::SlotRestarted { index, .. }) if *index == prep.index
        )
    });
    assert!(saw_restart, "apply is a live replacement");
}

/// `apply` to an out-of-range slot reports the rack's rejection rather than panicking (the race
/// between `prepare` and `apply` if the rack changed in between — MEMORY.md note).
#[test]
fn apply_to_a_stale_index_is_rejected_not_panicking() {
    let (mut eng, _events) = rig();
    let prep = eng.nr_capture_prepare(None).unwrap();
    eng.rack_command(RackCommand::Remove { index: prep.index })
        .unwrap();
    let err = eng.nr_capture_apply(prep.index, vec![0; 8]).unwrap_err();
    assert!(matches!(err, RackApiError::Rack(_)));
}
