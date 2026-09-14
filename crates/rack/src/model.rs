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

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use vox_module_api::{ModuleRef, ModuleState, ParseModuleRefError};

/// The well-formed slot shape (ADR-005 §10, SPEC-018 §2.6.4): used for the happy-path (de)
/// serialization of [`SlotModel`]. A JSON array element that doesn't fit this shape (no `module`
/// string, or not even an object) is kept as [`SlotModel::raw`] instead (SPEC-018 §2.6.4:
/// "a slot object that is not even a well-formed slot… becomes an 'Unreadable module' placeholder
/// and is also written back verbatim").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct WellFormedSlot {
    pub module: String,
    #[serde(default)]
    pub bypass: bool,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub state: Value,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// One slot of a [`RackModel`].
#[derive(Clone, Debug, PartialEq)]
pub struct SlotModel {
    /// `"id@version"` of the module that wrote the slot (kept as text so an unparsable or
    /// unknown reference round-trips verbatim). Empty when [`Self::raw`] is set — the module
    /// couldn't even be read out of the slot's JSON.
    pub module: String,
    /// Host bypass flag (belongs to the slot, not to the module state).
    pub bypass: bool,
    /// The module's [`ModuleState`], as raw JSON.
    pub state: Value,
    /// Any other keys, preserved.
    pub extra: Map<String, Value>,
    /// Set when this slot's source JSON was not a well-formed slot object (SPEC-018 §2.6.4): the
    /// exact value read, serialized back verbatim instead of being reconstructed from
    /// `module`/`bypass`/`state`/`extra` (which are left at their defaults). `Registry::resolve`
    /// turns this into an "Unreadable module" placeholder — never a deserialization error, so one
    /// bad slot never invalidates the rest of the rack or the sidecar.
    pub raw: Option<Value>,
}

impl SlotModel {
    /// A slot for `module` with `state`.
    pub fn new(module: &ModuleRef, bypass: bool, state: &ModuleState) -> Self {
        Self {
            module: module.to_string(),
            bypass,
            state: serde_json::to_value(state).unwrap_or(Value::Null),
            extra: Map::new(),
            raw: None,
        }
    }

    /// The parsed module reference.
    pub fn module_ref(&self) -> Result<ModuleRef, ParseModuleRefError> {
        self.module.parse()
    }

    /// Whether this slot's JSON wasn't even a well-formed slot object (SPEC-018 §2.6.4) — an
    /// "Unreadable module" placeholder, kept and written back verbatim.
    pub fn is_malformed(&self) -> bool {
        self.raw.is_some()
    }
}

impl Serialize for SlotModel {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.raw {
            Some(raw) => raw.serialize(serializer),
            None => WellFormedSlot {
                module: self.module.clone(),
                bypass: self.bypass,
                state: self.state.clone(),
                extra: self.extra.clone(),
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for SlotModel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Ok(
            match serde_json::from_value::<WellFormedSlot>(value.clone()) {
                Ok(w) => SlotModel {
                    module: w.module,
                    bypass: w.bypass,
                    state: w.state,
                    extra: w.extra,
                    raw: None,
                },
                Err(_) => SlotModel {
                    module: String::new(),
                    bypass: false,
                    state: Value::Null,
                    extra: Map::new(),
                    raw: Some(value),
                },
            },
        )
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
