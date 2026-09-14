//! T-406 (SPEC-012 §2.7, ADR-005 §10/§12): `RackHost::apply_module_preset`/`reset_to_default`/
//! `slot_state`/`slot_module_id` — the pieces module & rack presets are built on.

#![allow(clippy::float_cmp)] // exact values
mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::*;
use vox_module_api::{ModuleRef, ModuleState};
use vox_modules::{Gain, NoiseReduction, NoiseReductionFactory};
use vox_rack::{RackError, RackHost, RackModel, RackNotice, RackOptions, Registry, SlotModel};

vox_module_api::install_test_allocator!();

fn nr_registry() -> Arc<Registry> {
    let mut factories = test_factories();
    factories.push(Arc::new(NoiseReductionFactory::new()));
    Arc::new(Registry::with_factories(factories).unwrap())
}

fn nr_slot(state: ModuleState) -> SlotModel {
    SlotModel::new(
        &ModuleRef {
            id: NoiseReduction::ID.into(),
            version: NoiseReduction::VERSION,
        },
        false,
        &state,
    )
}

fn host_with(registry: Arc<Registry>, model: &RackModel) -> RackHost {
    let (host, _live) =
        RackHost::new(registry, rt_config(), RackOptions::default(), model).unwrap();
    host
}

fn print_of(seed: u64, len: usize) -> Vec<u8> {
    let x = white(seed, len);
    vox_dsp::nr::capture_profile(&x, SR, &AtomicBool::new(false)).unwrap()
}

// --- slot_state / slot_module_id -------------------------------------------------------------

#[test]
fn slot_state_reports_the_committed_mirror_and_blob() {
    let registry = registry();
    let mut host = host_with(registry, &model(vec![gain_slot(-6.0)]));
    let state = host.slot_state(0).unwrap();
    assert_eq!(state.params.get("gain_db"), Some(&-6.0));
    assert_eq!(state.blob, None);
    assert_eq!(host.slot_module_id(0).as_deref(), Some(Gain::ID));

    host.set_param(0, Gain::GAIN_DB, 3.0).unwrap();
    assert_eq!(host.slot_state(0).unwrap().params["gain_db"], 3.0);
}

#[test]
fn slot_state_and_module_id_error_or_none_for_a_placeholder() {
    let registry = registry();
    let host = host_with(
        registry,
        &model(vec![SlotModel::new(
            &ModuleRef {
                id: "org.powervoice.no-such-module".into(),
                version: vox_module_api::Version::new(1, 0, 0),
            },
            false,
            &ModuleState::new(1),
        )]),
    );
    assert!(matches!(
        host.slot_state(0),
        Err(RackError::NotLoaded { index: 0 })
    ));
    assert_eq!(host.slot_module_id(0), None);
}

#[test]
fn out_of_range_index_is_index_out_of_range() {
    let registry = registry();
    let mut host = host_with(registry, &model(vec![gain_slot(0.0)]));
    assert!(matches!(
        host.slot_state(5),
        Err(RackError::IndexOutOfRange { index: 5, len: 1 })
    ));
    assert!(matches!(
        host.apply_module_preset(5, &ModuleState::new(1)),
        Err(RackError::IndexOutOfRange { index: 5, len: 1 })
    ));
    assert!(matches!(
        host.reset_to_default(5),
        Err(RackError::IndexOutOfRange { index: 5, len: 1 })
    ));
}

// --- apply_module_preset: parameter-only (no blob) -> smoothed, no restart --------------------

#[test]
fn parameter_only_preset_updates_the_mirror_without_a_restart_notice() {
    let registry = registry();
    let mut host = host_with(registry, &model(vec![gain_slot(0.0)]));
    host.take_notices();

    let mut preset = ModuleState::new(1);
    preset.params.insert("gain_db".into(), -9.0);
    host.apply_module_preset(0, &preset).unwrap();

    assert_eq!(host.param_value(0, Gain::GAIN_DB), Some(-9.0));
    let notices = host.take_notices();
    assert!(
        notices
            .iter()
            .all(|n| !matches!(n, RackNotice::SlotRestarted { .. })),
        "parameter-only preset must not replace the instance: {notices:?}"
    );
    assert!(
        notices
            .iter()
            .any(|n| matches!(n, RackNotice::ParamChanged { value, .. } if *value == -9.0)),
        "expected a ParamChanged notice: {notices:?}"
    );
}

#[test]
fn parameter_only_preset_on_a_noise_reduction_slot_keeps_the_print() {
    let registry = nr_registry();
    let print = print_of(1, 24_000);
    let mut initial = NoiseReduction::state_with_blob(Some(print.clone()));
    initial.params.insert("reduction_db".into(), 12.0);
    let mut host = host_with(registry, &model(vec![nr_slot(initial)]));

    // A factory preset like "Strong (20 dB)": parameter-only (SPEC-014 §2.4/§AC-22).
    let mut preset = ModuleState::new(1);
    preset.params.insert("reduction_db".into(), 20.0);
    host.take_notices();
    host.apply_module_preset(0, &preset).unwrap();

    assert_eq!(
        host.param_value(0, NoiseReduction::REDUCTION_DB),
        Some(20.0)
    );
    let state = host.slot_state(0).unwrap();
    assert_eq!(state.blob.as_deref(), Some(print.as_slice()), "print kept");
    assert!(
        host.take_notices()
            .iter()
            .all(|n| !matches!(n, RackNotice::SlotRestarted { .. }))
    );
}

/// A preset that omits a key leaves that parameter at its current (not default) value.
#[test]
fn parameter_only_preset_leaves_unmentioned_parameters_alone() {
    let registry = nr_registry();
    let mut initial = NoiseReduction::state_with_blob(None);
    initial.params.insert("sensitivity_db".into(), 7.0);
    let mut host = host_with(registry, &model(vec![nr_slot(initial)]));

    let mut preset = ModuleState::new(1);
    preset.params.insert("reduction_db".into(), 6.0);
    host.apply_module_preset(0, &preset).unwrap();

    assert_eq!(host.param_value(0, NoiseReduction::REDUCTION_DB), Some(6.0));
    assert_eq!(
        host.param_value(0, NoiseReduction::SENSITIVITY_DB),
        Some(7.0),
        "untouched by the preset"
    );
}

// --- apply_module_preset: a state with a blob -> replacement crossfade -----------------------

#[test]
fn preset_with_a_blob_replaces_the_instance_like_replace_state() {
    let registry = nr_registry();
    let mut host = host_with(
        registry,
        &model(vec![nr_slot(NoiseReduction::state_with_blob(None))]),
    );
    host.take_notices();

    let print = print_of(2, 24_000);
    let mut preset = NoiseReduction::state_with_blob(Some(print.clone()));
    preset.params.insert("reduction_db".into(), 18.0);
    host.apply_module_preset(0, &preset).unwrap();

    let state = host.slot_state(0).unwrap();
    assert_eq!(state.blob.as_deref(), Some(print.as_slice()));
    assert_eq!(
        host.param_value(0, NoiseReduction::REDUCTION_DB),
        Some(18.0)
    );
    assert!(
        host.take_notices()
            .iter()
            .any(|n| matches!(n, RackNotice::SlotRestarted { index: 0, .. })),
        "a preset with a blob must go through the replacement crossfade"
    );
}

// --- reset_to_default --------------------------------------------------------------------------

#[test]
fn reset_to_default_restores_every_writable_parameter_and_keeps_the_blob() {
    let registry = nr_registry();
    let print = print_of(3, 24_000);
    let mut initial = NoiseReduction::state_with_blob(Some(print.clone()));
    initial.params.insert("reduction_db".into(), 30.0);
    initial.params.insert("sensitivity_db".into(), -4.0);
    let mut host = host_with(registry, &model(vec![nr_slot(initial)]));

    host.reset_to_default(0).unwrap();

    let defaults = NoiseReduction::state_with_blob(None);
    for (key, default_value) in &defaults.params {
        let id = host
            .slot_info(0)
            .unwrap()
            .params
            .iter()
            .find(|p| &p.key == key)
            .unwrap()
            .id;
        assert_eq!(
            host.param_value(0, id),
            Some(*default_value),
            "{key} not reset"
        );
    }
    let state = host.slot_state(0).unwrap();
    assert_eq!(state.blob.as_deref(), Some(print.as_slice()), "print kept");
}
