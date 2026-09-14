//! Dev-dependency guard (T-406, MEMORY.md S3-06 pattern): `vox-presets` builds factory rack
//! presets from literal built-in module ids and parameter keys rather than depending on
//! `vox-modules` at compile time. This test uses `vox-modules` (dev-only) to check those literals
//! against the real modules: every module id matches an actual built-in, every overridden
//! parameter key exists in that module's real schema, and every overridden value is within the
//! parameter's real range.

use std::collections::HashMap;

use vox_module_api::ModuleState;
use vox_modules::builtin_factories;
use vox_presets::factory_rack_presets;
use vox_rack::Registry;

#[test]
fn every_factory_rack_preset_module_id_is_a_real_built_in() {
    let registry = Registry::with_factories(builtin_factories()).unwrap();
    for preset in factory_rack_presets() {
        for s in &preset.model.slots {
            let id = s.module_ref().unwrap().id;
            assert!(
                registry.get(&id).is_some(),
                "{}: `{id}` is not a registered built-in module",
                preset.key
            );
        }
    }
}

/// Every key/value a factory rack preset sets exists in the real module's parameter schema and
/// is within its `min..=max` (catches a typo'd key — which `prepare_state` would otherwise drop
/// silently, per ADR-005 §10 — and an out-of-range literal).
#[test]
fn every_factory_rack_preset_override_is_a_real_in_range_parameter() {
    let schemas: HashMap<String, Vec<vox_module_api::ParamInfo>> = builtin_factories()
        .iter()
        .map(|f| {
            let d = f.descriptor();
            (d.id.clone(), f.create().unwrap().params().to_vec())
        })
        .collect();

    for preset in factory_rack_presets() {
        for s in &preset.model.slots {
            let id = s.module_ref().unwrap().id;
            let params = schemas
                .get(&id)
                .unwrap_or_else(|| panic!("{}: unknown module `{id}`", preset.key));
            let state: ModuleState = serde_json::from_value(s.state.clone())
                .unwrap_or_else(|e| panic!("{}: {id}: invalid state JSON: {e}", preset.key));
            for (key, value) in &state.params {
                let p = params.iter().find(|p| &p.key == key).unwrap_or_else(|| {
                    panic!("{}: {id}: no parameter `{key}` (typo?)", preset.key)
                });
                assert!(
                    *value >= p.min && *value <= p.max,
                    "{}: {id}.{key} = {value} is outside {}..={}",
                    preset.key,
                    p.min,
                    p.max
                );
            }
        }
    }
}
