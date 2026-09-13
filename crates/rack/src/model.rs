//! The rack as plain data (ADR-001 §5): an ordered list of slots in the sidecar's slot schema
//! (ADR-005 §10):
//!
//! ```json
//! { "module": "org.powervoice.gain@1.0.0", "bypass": false,
//!   "state": { "format_version": 1, "params": { "gain_db": -6.0 } } }
//! ```
//!
//! `state` is kept as raw JSON and unknown keys are preserved, so a slot whose module is not
//! installed (a placeholder) is written back verbatim.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use vox_module_api::{ModuleRef, ModuleState, ParseModuleRefError};

/// One slot of a [`RackModel`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SlotModel {
    /// `"id@version"` of the module that wrote the slot (kept as text so an unparsable or
    /// unknown reference round-trips verbatim).
    pub module: String,
    /// Host bypass flag (belongs to the slot, not to the module state).
    #[serde(default)]
    pub bypass: bool,
    /// The module's [`ModuleState`], as raw JSON.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub state: Value,
    /// Any other keys, preserved.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl SlotModel {
    /// A slot for `module` with `state`.
    pub fn new(module: &ModuleRef, bypass: bool, state: &ModuleState) -> Self {
        Self {
            module: module.to_string(),
            bypass,
            state: serde_json::to_value(state).unwrap_or(Value::Null),
            extra: Map::new(),
        }
    }

    /// The parsed module reference.
    pub fn module_ref(&self) -> Result<ModuleRef, ParseModuleRefError> {
        self.module.parse()
    }
}

/// An ordered list of slots (processing order).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RackModel {
    /// Slots, top to bottom.
    pub slots: Vec<SlotModel>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RackJson {
    Object(RackModel),
    Slots(Vec<SlotModel>),
}

impl RackModel {
    /// Parses a rack file: either `{ "slots": [ … ] }` or a bare array of slots (a rack preset).
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        Ok(match serde_json::from_str(s)? {
            RackJson::Object(m) => m,
            RackJson::Slots(slots) => Self { slots },
        })
    }

    /// `{ "slots": [ … ] }`, pretty-printed.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}
