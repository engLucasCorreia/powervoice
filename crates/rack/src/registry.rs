//! Module registry (SPEC-012 §2.10, ADR-005 §2): module id → factory, one version per id.
//!
//! Interior-mutable (T-804, ADR-008 §6 Amendment 4): a background plugin scan can [`upsert`]
//! newly found modules into a `Registry` that's already shared (`Arc`) with a live rack, so the
//! Add-module list picks them up without a restart.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use vox_module_api::{
    ActivateConfig, Module, ModuleDescriptor, ModuleFactory, ModuleRef, ModuleState, ParamId,
    StateError, prepare_state, response_curve, validate_schema,
};

use crate::shim::DualMonoShim;
use crate::{RackError, RackModel, SlotModel};

/// Error registering a factory.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// Only one version per id is registered.
    #[error("module id `{0}` is already registered")]
    Duplicate(String),
}

/// What a stored slot resolves to.
pub enum Resolved {
    /// The instance, with the slot's state loaded and wrapped in the dual-mono shim if needed
    /// (inactive after [`Registry::resolve`], active after [`Registry::instantiate`]).
    Module(Box<dyn Module>),
    /// The module is missing (or its state is too new): dry, latency 0, kept verbatim.
    Placeholder {
        /// "Missing module ‹id@version›" or "‹id› requires a newer version".
        message: String,
        /// The module is installed but the state is newer than it supports.
        too_new: bool,
    },
}

/// What [`Registry::preview_response_curve`] returns — the same fields
/// `crate::RackHost`'s live-slot response curve carries, so the two are drop-in interchangeable
/// for whoever draws them.
#[derive(Clone, Debug)]
pub struct ResponseCurvePreview {
    /// The requested frequencies (Hz), unchanged and in the order given.
    pub freqs_hz: Vec<f64>,
    /// The rate the curve was evaluated at (`config.sample_rate`).
    pub sample_rate_hz: f64,
    /// Total response (dB) at each of `freqs_hz`.
    pub total_db: Vec<f64>,
    /// One row per component (band), in the module's `handles()` order; empty when the module
    /// reports zero components.
    pub components_db: Vec<Vec<f64>>,
}

impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Module(m) => write!(f, "Module({})", m.descriptor().id),
            Self::Placeholder { message, too_new } => f
                .debug_struct("Placeholder")
                .field("message", message)
                .field("too_new", too_new)
                .finish(),
        }
    }
}

/// id → factory. Resolution is by id; the stored version is informational.
#[derive(Default)]
pub struct Registry {
    factories: Mutex<BTreeMap<String, Arc<dyn ModuleFactory>>>,
    /// Bumped by every [`Self::register`]/[`Self::upsert`]/[`Self::remove`] that actually changed
    /// something (H-40, ADR-008 §6 Amendment: live recovery of Missing slots): a live
    /// [`crate::RackHost`] polls this once a control tick to know cheaply whether it's worth
    /// rechecking its placeholder slots against the registry again, without a channel or an
    /// observer list of its own.
    generation: AtomicU64,
}

fn lock(
    m: &Mutex<BTreeMap<String, Arc<dyn ModuleFactory>>>,
) -> std::sync::MutexGuard<'_, BTreeMap<String, Arc<dyn ModuleFactory>>> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Registry {
    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry holding `factories`.
    pub fn with_factories(
        factories: impl IntoIterator<Item = Arc<dyn ModuleFactory>>,
    ) -> Result<Self, RegistryError> {
        let r = Self::new();
        for f in factories {
            r.register(f)?;
        }
        Ok(r)
    }

    /// Registers a factory under its descriptor id; an id already present is an error (one
    /// version per id — use [`Self::upsert`] to replace or hot-add).
    pub fn register(&self, factory: Arc<dyn ModuleFactory>) -> Result<(), RegistryError> {
        let id = factory.descriptor().id.clone();
        let mut g = lock(&self.factories);
        if g.contains_key(&id) {
            return Err(RegistryError::Duplicate(id));
        }
        g.insert(id, factory);
        drop(g);
        self.generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Inserts or replaces a factory under its descriptor id, without erroring on a duplicate
    /// (T-804, ADR-008 §6 Amendment 4): a background scan hot-adding a newly found plugin into a
    /// `Registry` a live rack already shares. `true` if this added a new id, `false` if it
    /// replaced an existing one.
    pub fn upsert(&self, factory: Arc<dyn ModuleFactory>) -> bool {
        let id = factory.descriptor().id.clone();
        let added = lock(&self.factories).insert(id, factory).is_none();
        self.generation.fetch_add(1, Ordering::Relaxed);
        added
    }

    /// The factory for `id`.
    pub fn get(&self, id: &str) -> Option<Arc<dyn ModuleFactory>> {
        lock(&self.factories).get(id).cloned()
    }

    /// Removes `id` (H-29 "Uninstall…"): `true` if it was registered. A rack slot that already
    /// references it, live or in a document not yet reopened, is unaffected here — the next call
    /// to [`Self::resolve`] for that slot simply falls back to the "Missing module" placeholder,
    /// preserving the slot's own stored state untouched.
    pub fn remove(&self, id: &str) -> bool {
        let removed = lock(&self.factories).remove(id).is_some();
        if removed {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        removed
    }

    /// Bumped by every [`Self::register`]/[`Self::upsert`]/[`Self::remove`] that actually changed
    /// the registered set (H-40): lets a live [`crate::RackHost`] poll, once a control tick,
    /// whether it's worth rechecking its Missing/too-new placeholder slots — no observer list or
    /// channel needed for that side of it (the catalog's own hot-add to observed registries,
    /// T-804, is separate and still `Weak<Registry>`-based).
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// Registered ids, sorted.
    pub fn ids(&self) -> Vec<String> {
        lock(&self.factories).keys().cloned().collect()
    }

    /// Descriptors of the registered modules (the Add-module menu), sorted by id.
    pub fn descriptors(&self) -> Vec<ModuleDescriptor> {
        lock(&self.factories)
            .values()
            .map(|f| f.descriptor().clone())
            .collect()
    }

    /// Number of registered modules.
    pub fn len(&self) -> usize {
        lock(&self.factories).len()
    }

    /// True if nothing is registered.
    pub fn is_empty(&self) -> bool {
        lock(&self.factories).is_empty()
    }

    /// The module references of `model` whose id is not registered (or does not parse), in
    /// slot order, without duplicates.
    pub fn missing_ids(&self, model: &RackModel) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &model.slots {
            // A malformed slot (SPEC-018 §2.6.4) has no module id the user could install — it
            // isn't a "missing module" in that sense, just unreadable.
            if s.is_malformed() {
                continue;
            }
            let known = s
                .module_ref()
                .is_ok_and(|r| lock(&self.factories).contains_key(&r.id));
            if !known && !out.contains(&s.module) {
                out.push(s.module.clone());
            }
        }
        out
    }

    /// Creates an inactive instance for `slot` with its state loaded (through
    /// [`prepare_state`]), wrapped in the dual-mono shim if needed. Missing modules and too-new
    /// states resolve to [`Resolved::Placeholder`]; so does a slot whose source JSON wasn't even
    /// a well-formed slot object (SPEC-018 §2.6.4, [`SlotModel::is_malformed`]).
    pub fn resolve(&self, slot: &SlotModel) -> Result<Resolved, RackError> {
        if slot.is_malformed() {
            return Ok(Resolved::Placeholder {
                message: "Unreadable module".to_string(),
                too_new: false,
            });
        }
        let missing = || Resolved::Placeholder {
            message: format!("Missing module {}", slot.module),
            too_new: false,
        };
        let Ok(r) = slot.module_ref() else {
            return Ok(missing());
        };
        let Some(factory) = self.get(&r.id) else {
            return Ok(missing());
        };
        let mut module = factory.create().map_err(|source| RackError::Create {
            id: r.id.clone(),
            source,
        })?;
        let state = if slot.state.is_null() {
            module.save_state().map_err(|source| RackError::State {
                id: r.id.clone(),
                source,
            })?
        } else {
            serde_json::from_value::<ModuleState>(slot.state.clone()).map_err(|e| {
                RackError::InvalidState {
                    id: r.id.clone(),
                    message: e.to_string(),
                }
            })?
        };
        let state = match prepare_state(&*module, state) {
            Ok(s) => s,
            Err(StateError::TooNew { .. }) => {
                return Ok(Resolved::Placeholder {
                    message: format!("{} requires a newer version", r.id),
                    too_new: true,
                });
            }
            Err(source) => {
                return Err(RackError::State {
                    id: r.id.clone(),
                    source,
                });
            }
        };
        module
            .load_state(&state)
            .map_err(|source| RackError::State {
                id: r.id.clone(),
                source,
            })?;
        let module = DualMonoShim::adapt(module).map_err(|m| RackError::UnsupportedLayout {
            id: r.id.clone(),
            name: m.descriptor().name.text.clone(),
        })?;
        validate_schema(module.params(), module.groups()).map_err(|source| RackError::Schema {
            id: r.id.clone(),
            source,
        })?;
        Ok(Resolved::Module(module))
    }

    /// [`resolve`](Self::resolve), then activates the instance with `config`.
    pub fn instantiate(
        &self,
        slot: &SlotModel,
        config: &ActivateConfig,
        index: usize,
    ) -> Result<Resolved, RackError> {
        match self.resolve(slot)? {
            Resolved::Module(mut m) => {
                m.activate(config).map_err(|source| RackError::Activate {
                    index,
                    id: m.descriptor().id.clone(),
                    name: m.descriptor().name.text.clone(),
                    source,
                })?;
                Ok(Resolved::Module(m))
            }
            p => Ok(p),
        }
    }

    /// Evaluates `module_id`'s `ResponseCurve` extension at `freqs_hz`, from its own schema
    /// defaults with `overrides` applied on top — **never a live rack slot**: nothing a document
    /// keeps can be mutated by calling this (H-101, SPEC-015 §2.6.3 amendment — AC-17 stands, the
    /// UI still evaluates nothing, but a hypothetical filter can now be drawn before it is ever
    /// applied). An `overrides` id the module doesn't have is ignored; every other parameter
    /// stays at its schema default. Builds a fresh instance through the exact same
    /// [`Self::resolve`]/[`Self::instantiate`] path (`prepare_state`, `load_state`, the
    /// dual-mono shim, `activate`) a real slot goes through and reads the same `ResponseCurve`
    /// extension [`crate::RackHost::response_curve_extension`] does for a live one, so a preview
    /// and the applied result can never disagree; the instance is dropped when this returns.
    pub fn preview_response_curve(
        &self,
        module_id: &str,
        overrides: &[(ParamId, f64)],
        config: &ActivateConfig,
        freqs_hz: Vec<f64>,
    ) -> Result<ResponseCurvePreview, RackError> {
        let Some(factory) = self.get(module_id) else {
            return Err(RackError::UnknownModule(module_id.to_string()));
        };
        let descriptor = factory.descriptor().clone();
        // A throwaway, never-activated instance, just to read the id → key schema: overrides
        // arrive as `ParamId`s (the RT-safe, stable identifier), but `ModuleState` stores keys,
        // like the sidecar and every preset.
        let schema_probe = factory.create().map_err(|source| RackError::Create {
            id: module_id.to_string(),
            source,
        })?;
        let mut params = BTreeMap::new();
        for (id, value) in overrides {
            if let Some(p) = schema_probe.params().iter().find(|p| p.id == *id) {
                params.insert(p.key.clone(), *value);
            }
        }
        drop(schema_probe);
        let state = ModuleState {
            format_version: descriptor.state_format_version,
            params,
            blob: None,
        };
        let slot = SlotModel::new(&ModuleRef::of(&descriptor), false, &state);
        let module = match self.instantiate(&slot, config, 0)? {
            Resolved::Module(m) => m,
            Resolved::Placeholder { message, .. } => {
                return Err(RackError::PreviewUnavailable {
                    id: module_id.to_string(),
                    message,
                });
            }
        };
        let ext = response_curve(module.as_ref()).ok_or_else(|| RackError::NoResponseCurve {
            id: module_id.to_string(),
        })?;
        let values: Vec<f64> = module
            .params()
            .iter()
            .map(|p| module.param_value(p.id).unwrap_or(p.default))
            .collect();
        let sample_rate_hz = config.sample_rate;
        let mut total_db = vec![0.0; freqs_hz.len()];
        ext.magnitude_db(&values, sample_rate_hz, &freqs_hz, &mut total_db);
        let component_count = ext.component_count(&values);
        let mut components_db = Vec::with_capacity(component_count);
        for c in 0..component_count {
            let mut out = vec![0.0; freqs_hz.len()];
            ext.component_magnitude_db(c, &values, sample_rate_hz, &freqs_hz, &mut out);
            components_db.push(out);
        }
        Ok(ResponseCurvePreview {
            freqs_hz,
            sample_rate_hz,
            total_db,
            components_db,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vox_module_api::ModuleRef;
    use vox_module_api::test_util::{TestGain, TestGainFactory};

    /// H-29 "Uninstall…"/T-804 hot-remove: once a module's factory leaves the registry, a slot
    /// that still names it resolves to the "Missing module" placeholder instead of erroring —
    /// and the slot's own state (what an open document would keep) is completely untouched by
    /// either the removal or the resolve.
    #[test]
    fn removing_a_module_falls_back_to_the_missing_placeholder_and_keeps_the_slots_state() {
        let registry = Registry::new();
        let id = TestGain::ID.to_string();
        registry.register(Arc::new(TestGainFactory::new())).unwrap();

        let module_ref = ModuleRef {
            id: id.clone(),
            version: vox_module_api::Version::new(1, 0, 0),
        };
        let state = TestGain::state_with_gain_db(-6.0);
        let slot = SlotModel::new(&module_ref, false, &state);
        let before_state = slot.state.clone();
        assert!(matches!(
            registry.resolve(&slot).unwrap(),
            Resolved::Module(_)
        ));

        assert!(registry.remove(&id), "it was registered");
        assert!(!registry.remove(&id), "already gone");

        match registry.resolve(&slot).unwrap() {
            Resolved::Placeholder { message, too_new } => {
                assert!(message.contains(&id), "{message}");
                assert!(!too_new);
            }
            other => panic!("expected a placeholder, got {other:?}"),
        }
        // The slot itself — what a document's sidecar would keep — is never mutated by any of
        // this: resolving (successfully or not) only ever reads it.
        assert_eq!(slot.state, before_state);
    }

    fn eq_config() -> ActivateConfig {
        ActivateConfig {
            sample_rate: 48_000.0,
            max_block: 64,
            mode: vox_module_api::ProcessMode::Realtime,
            layout: vox_module_api::ChannelLayout::MONO,
        }
    }

    fn eq_registry() -> Registry {
        let registry = Registry::new();
        registry
            .register(Arc::new(vox_modules::ParametricEqFactory::new()))
            .unwrap();
        registry
    }

    /// H-101's whole point: a preview built from parameters alone must read exactly like a real
    /// instance loaded with the same parameters — the point of routing both through the same
    /// `ResponseCurve` extension instead of a second, UI-side implementation.
    #[test]
    fn preview_response_curve_matches_a_real_instance_with_the_same_overrides() {
        use vox_modules::ParametricEq;

        let registry = eq_registry();
        let config = eq_config();
        // Turn on peak band 1 (band index 2 of 0=hp,1=ls,2..6=b1..b5,7=hs,8=lp) at 1 kHz, Q 1,
        // +6 dB — exactly the shape `eqSuggest.ts::planEqAction` sends for a boost/cut move.
        let overrides = [
            (ParametricEq::param_id(2, ParametricEq::ON), 1.0),
            (ParametricEq::param_id(2, ParametricEq::FREQ), 1_000.0),
            (ParametricEq::param_id(2, ParametricEq::GAIN), 6.0),
            (ParametricEq::param_id(2, ParametricEq::Q), 1.0),
        ];
        let freqs_hz = vec![100.0, 500.0, 1_000.0, 2_000.0, 8_000.0];

        let preview = registry
            .preview_response_curve(ParametricEq::ID, &overrides, &config, freqs_hz.clone())
            .unwrap();

        // The reference: an instance built the ordinary way (a full, explicit `ModuleState`,
        // as a live rack slot would carry after the same parameter changes), read through the
        // exact same `response_curve`/`magnitude_db` calls `preview_response_curve` makes.
        let mut reference = vox_modules::ParametricEqFactory::new().create().unwrap();
        let mut params = std::collections::BTreeMap::new();
        for p in reference.params() {
            params.insert(p.key.clone(), p.default);
        }
        for (id, value) in overrides {
            let key = reference
                .params()
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .key
                .clone();
            params.insert(key, value);
        }
        reference
            .load_state(&ModuleState {
                format_version: ParametricEq::STATE_FORMAT_VERSION,
                params,
                blob: None,
            })
            .unwrap();
        reference.activate(&config).unwrap();
        let ext = response_curve(reference.as_ref()).unwrap();
        let values: Vec<f64> = reference
            .params()
            .iter()
            .map(|p| reference.param_value(p.id).unwrap_or(p.default))
            .collect();
        let mut expected_total = vec![0.0; freqs_hz.len()];
        ext.magnitude_db(&values, config.sample_rate, &freqs_hz, &mut expected_total);

        assert_eq!(preview.freqs_hz, freqs_hz);
        assert!((preview.sample_rate_hz - config.sample_rate).abs() < f64::EPSILON);
        assert_eq!(preview.total_db, expected_total);
        // freqs_hz = [100, 500, 1_000, 2_000, 8_000]: a boost at 1 kHz (index 2) reads back near
        // +6 dB there and near 0 dB far away from it (index 0, 100 Hz).
        assert!((preview.total_db[2] - 6.0).abs() < 0.1);
        assert!(preview.total_db[0].abs() < 0.5);
    }

    /// No overrides at all previews the module's own neutral defaults (a flat parametric EQ is
    /// bit-exact identity, SPEC-015 AC-2, so its response curve is 0 dB everywhere).
    #[test]
    fn preview_response_curve_with_no_overrides_reads_the_modules_own_defaults() {
        use vox_modules::ParametricEq;

        let registry = eq_registry();
        let config = eq_config();
        let freqs_hz = vec![20.0, 100.0, 1_000.0, 10_000.0, 20_000.0];

        let preview = registry
            .preview_response_curve(ParametricEq::ID, &[], &config, freqs_hz)
            .unwrap();

        for db in preview.total_db {
            assert!(db.abs() < 0.001, "expected ~0 dB, got {db}");
        }
    }

    /// Calling the preview never touches — let alone creates — any rack slot: the registry has
    /// none to mutate in the first place, which is the point (H-101: "with no rack slot and no
    /// mutation").
    #[test]
    fn preview_response_curve_never_creates_a_rack_slot() {
        use vox_modules::ParametricEq;

        let registry = eq_registry();
        let config = eq_config();
        let generation_before = registry.generation();

        registry
            .preview_response_curve(ParametricEq::ID, &[], &config, vec![1_000.0])
            .unwrap();

        // Nothing about the registered-factory set changed, and (more to the point) there is no
        // `RackModel`/`RackHost` anywhere in this test for a slot to have been added to.
        assert_eq!(registry.generation(), generation_before);
        assert_eq!(registry.ids(), vec![ParametricEq::ID.to_string()]);
    }

    #[test]
    fn preview_response_curve_rejects_an_unknown_module_id() {
        let registry = eq_registry();
        let err = registry
            .preview_response_curve("org.powervoice.does-not-exist", &[], &eq_config(), vec![])
            .unwrap_err();
        assert!(
            matches!(err, RackError::UnknownModule(id) if id == "org.powervoice.does-not-exist")
        );
    }

    /// A module with no `ResponseCurve` extension (Gain — `test_util::TestGain` has one, a flat
    /// `FlatCurve`, so it won't do for this case) previews as `NoResponseCurve`, not a panic or a
    /// silently empty curve.
    #[test]
    fn preview_response_curve_rejects_a_module_with_no_response_curve_extension() {
        let registry = Registry::new();
        registry
            .register(Arc::new(vox_modules::GainFactory::new()))
            .unwrap();
        let config = ActivateConfig {
            sample_rate: 48_000.0,
            max_block: 64,
            mode: vox_module_api::ProcessMode::Realtime,
            layout: vox_module_api::ChannelLayout::MONO,
        };
        let err = registry
            .preview_response_curve(vox_modules::Gain::ID, &[], &config, vec![1_000.0])
            .unwrap_err();
        assert!(matches!(err, RackError::NoResponseCurve { id } if id == vox_modules::Gain::ID));
    }

    /// An override naming a parameter id the module doesn't have is silently ignored rather than
    /// rejected — the caller (H-101's EQ overlay) can send the same override list regardless of
    /// which module id it targets.
    #[test]
    fn preview_response_curve_ignores_an_override_for_an_unknown_param_id() {
        use vox_modules::ParametricEq;

        let registry = eq_registry();
        let config = eq_config();
        let overrides = [(ParamId(999_999), 6.0)];
        let preview = registry
            .preview_response_curve(ParametricEq::ID, &overrides, &config, vec![1_000.0])
            .unwrap();
        assert!(preview.total_db[0].abs() < 0.001);
    }
}
