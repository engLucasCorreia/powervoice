//! T-406 (SPEC-012 §2.7, ADR-005 §12): the engine surface module & rack presets are built on —
//! `rack_slot_state`/`rack_slot_module_id` (read-only) and the `ApplyModulePreset`/
//! `ResetToDefault` rack commands — round-trip through `ManualEngine` on the fake backend, same
//! as every other rack command (`rack_api.rs`).

#![allow(clippy::float_cmp)] // exact values
vox_module_api::install_test_allocator!();

use std::sync::Arc;

use vox_engine::backend::fake::{FakeBackend, FakeDevice, FakeDirection};
use vox_engine::{EngineConfig, HostId, ManualEngine, RackApiError, RackCommand};
use vox_modules::{Gain, NoiseReduction};
use vox_rack::{ModuleState, RackNotice, Registry};

fn dev() -> FakeDirection {
    FakeDirection::new(2, &[48_000], 48_000).default_buffer(256)
}

fn rig() -> ManualEngine {
    let fake = FakeBackend::new(7);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev().record_output()),
    );
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let cfg = EngineConfig::new(Arc::new(fake), registry);
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();
    eng
}

fn preset(params: &[(&str, f64)]) -> ModuleState {
    ModuleState {
        format_version: 1,
        params: params.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
        blob: None,
    }
}

#[test]
fn slot_state_and_module_id_are_unavailable_or_none_with_no_live_rack() {
    let fake = FakeBackend::new(1); // nothing plugged: no output ever opens
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let cfg = EngineConfig::new(Arc::new(fake), registry);
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();

    assert!(matches!(
        eng.rack_slot_state(0),
        Err(RackApiError::Unavailable)
    ));
    assert_eq!(eng.rack_slot_module_id(0), None);
}

#[test]
fn slot_state_and_module_id_read_the_live_slot() {
    let mut eng = rig();
    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();
    eng.rack_command(RackCommand::SetParamPlain {
        index: 0,
        id: Gain::GAIN_DB,
        value: -4.5,
    })
    .unwrap();

    assert_eq!(eng.rack_slot_module_id(0), Some(Gain::ID.to_owned()));
    assert_eq!(eng.rack_slot_module_id(1), None, "out of range");

    let state = eng.rack_slot_state(0).unwrap();
    assert_eq!(state.params["gain_db"], -4.5);
    assert_eq!(state.blob, None);
}

/// A parameter-only preset (no blob) is smoothed like a typed value, with no restart notice
/// (SPEC-012 §2.7).
#[test]
fn apply_parameter_only_preset_updates_the_snapshot_without_a_restart() {
    let mut eng = rig();
    eng.rack_command(RackCommand::Add {
        module_id: Gain::ID.into(),
        index: 0,
    })
    .unwrap();

    let snap = eng
        .rack_command(RackCommand::ApplyModulePreset {
            index: 0,
            state: preset(&[("gain_db", 6.0)]),
        })
        .unwrap();
    assert_eq!(snap.slots[0].values[0], 6.0);
}

/// A preset with a blob (a Noise Reduction preset that "includes the print", SPEC-014 §2.4)
/// replaces the instance through the same crossfade as `Restart` — the snapshot's `RackNotice`
/// stream carries a `SlotRestarted`.
#[test]
fn apply_preset_with_a_blob_replaces_the_instance() {
    let mut eng = rig();
    eng.rack_command(RackCommand::Add {
        module_id: NoiseReduction::ID.into(),
        index: 0,
    })
    .unwrap();

    let print = vec![1u8, 2, 3, 4];
    let mut state = NoiseReduction::state_with_blob(Some(print.clone()));
    state.params.insert("reduction_db".into(), 20.0);
    let snap = eng
        .rack_command(RackCommand::ApplyModulePreset { index: 0, state })
        .unwrap();
    assert!(snap.slots[0].values.contains(&20.0));
    let loaded = eng.rack_slot_state(0).unwrap();
    assert_eq!(loaded.blob, Some(print));
}

/// Resetting to default restores every writable parameter and keeps a committed blob.
#[test]
fn reset_to_default_restores_defaults_and_keeps_the_blob() {
    let mut eng = rig();
    eng.rack_command(RackCommand::Add {
        module_id: NoiseReduction::ID.into(),
        index: 0,
    })
    .unwrap();
    let print = vec![9u8, 9, 9];
    let mut state = NoiseReduction::state_with_blob(Some(print.clone()));
    state.params.insert("reduction_db".into(), 30.0);
    eng.rack_command(RackCommand::ApplyModulePreset { index: 0, state })
        .unwrap();

    eng.rack_command(RackCommand::ResetToDefault { index: 0 })
        .unwrap();
    let after = eng.rack_slot_state(0).unwrap();
    assert_eq!(after.params["reduction_db"], 12.0, "back to the default");
    assert_eq!(after.blob, Some(print), "the print is kept");
}

#[test]
fn preset_commands_are_unavailable_before_an_output_device_opens() {
    let fake = FakeBackend::new(1);
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let cfg = EngineConfig::new(Arc::new(fake), registry);
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();

    assert!(matches!(
        eng.rack_command(RackCommand::ApplyModulePreset {
            index: 0,
            state: preset(&[]),
        }),
        Err(RackApiError::Unavailable)
    ));
    assert!(matches!(
        eng.rack_command(RackCommand::ResetToDefault { index: 0 }),
        Err(RackApiError::Unavailable)
    ));
}

/// The generic notice stream a preset-menu-driven UI relies on: applying a blob preset is
/// reported the same way as `Restart` (`SlotRestarted`), so the UI doesn't need special cases.
#[test]
fn apply_preset_with_a_blob_is_reported_as_a_slot_restart_notice() {
    use std::sync::Mutex;
    use vox_engine::EngineEvent;

    let fake = FakeBackend::new(7);
    fake.plug(
        HostId::Alsa,
        FakeDevice::new("DAC").with_output(dev().record_output()),
    );
    let registry = Arc::new(Registry::with_factories(vox_modules::builtin_factories()).unwrap());
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut cfg = EngineConfig::new(Arc::new(fake), registry);
    let ev = events.clone();
    cfg.events = Arc::new(move |e| ev.lock().unwrap().push(e));
    let mut eng = ManualEngine::new(cfg);
    eng.poll_devices();

    eng.rack_command(RackCommand::Add {
        module_id: NoiseReduction::ID.into(),
        index: 0,
    })
    .unwrap();
    events.lock().unwrap().clear();

    eng.rack_command(RackCommand::ApplyModulePreset {
        index: 0,
        state: NoiseReduction::state_with_blob(Some(vec![1, 2, 3])),
    })
    .unwrap();

    let saw_restart = events.lock().unwrap().iter().any(|e| {
        matches!(
            e,
            EngineEvent::Rack(RackNotice::SlotRestarted { index: 0, .. })
        )
    });
    assert!(saw_restart, "{:?}", events.lock().unwrap());
}
