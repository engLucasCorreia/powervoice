//! Module registry (SPEC-012 §2.10, ADR-005 §2): module id → factory, one version per id.
//!
//! Interior-mutable (T-804, ADR-008 §6 Amendment 4): a background plugin scan can [`upsert`]
//! newly found modules into a `Registry` that's already shared (`Arc`) with a live rack, so the
//! Add-module list picks them up without a restart.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use vox_module_api::{
    ActivateConfig, Module, ModuleDescriptor, ModuleFactory, ModuleState, StateError,
    prepare_state, validate_schema,
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
}
