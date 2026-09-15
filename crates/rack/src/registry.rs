//! Module registry (SPEC-012 §2.10, ADR-005 §2): module id → factory, one version per id.
//!
//! Interior-mutable (T-804, ADR-008 §6 Amendment 4): a background plugin scan can [`upsert`]
//! newly found modules into a `Registry` that's already shared (`Arc`) with a live rack, so the
//! Add-module list picks them up without a restart.

use std::collections::BTreeMap;
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
        Ok(())
    }

    /// Inserts or replaces a factory under its descriptor id, without erroring on a duplicate
    /// (T-804, ADR-008 §6 Amendment 4): a background scan hot-adding a newly found plugin into a
    /// `Registry` a live rack already shares. `true` if this added a new id, `false` if it
    /// replaced an existing one.
    pub fn upsert(&self, factory: Arc<dyn ModuleFactory>) -> bool {
        let id = factory.descriptor().id.clone();
        lock(&self.factories).insert(id, factory).is_none()
    }

    /// The factory for `id`.
    pub fn get(&self, id: &str) -> Option<Arc<dyn ModuleFactory>> {
        lock(&self.factories).get(id).cloned()
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
