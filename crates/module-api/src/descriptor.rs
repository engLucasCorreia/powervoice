//! Module identity: version, reference, descriptor, factory (ADR-005 §2).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::module::{Module, ModuleError};
use crate::state::ModuleState;

/// Version of this Module API. Modules record it in [`ModuleDescriptor::api_version`].
pub const MODULE_API_VERSION: u32 = 1;

/// Semantic version. `Display`/`FromStr` use `"MAJOR.MINOR.PATCH"` (no pre-release or build
/// tags in v1). Serialized as that string, e.g. `"1.2.0"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Version {
    /// Incompatible changes.
    pub major: u32,
    /// Additive changes.
    pub minor: u32,
    /// Fixes.
    pub patch: u32,
}

impl Version {
    /// Creates a version.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Lenient parse for adapters (ADR-005 §2): takes a leading `x.y.z`; missing parts are 0;
    /// anything unparsable gives `0.0.0`. Examples: `"1.2"` → 1.2.0, `"2.0 beta"` → 2.0.0,
    /// `"1.2.3.4"` → 1.2.3, `"abc"` → 0.0.0.
    pub fn parse_lenient(s: &str) -> Self {
        let s = s.trim();
        let end = s
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(s.len());
        let mut parts = s[..end].split('.').map(|p| p.parse::<u32>().ok());
        let major = match parts.next().flatten() {
            Some(v) => v,
            None => return Self::default(),
        };
        let minor = parts.next().flatten().unwrap_or(0);
        let patch = parts.next().flatten().unwrap_or(0);
        Self::new(major, minor, patch)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Error parsing a strict `"MAJOR.MINOR.PATCH"` version.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid version `{0}`: expected MAJOR.MINOR.PATCH")]
pub struct ParseVersionError(String);

impl FromStr for Version {
    type Err = ParseVersionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseVersionError(s.to_owned());
        let mut parts = s.split('.');
        let mut next = || -> Result<u32, ParseVersionError> {
            let p = parts.next().ok_or_else(err)?;
            if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
                return Err(err());
            }
            p.parse().map_err(|_| err())
        };
        let v = Self::new(next()?, next()?, next()?);
        if parts.next().is_some() {
            return Err(err());
        }
        Ok(v)
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// `"id@version"`, e.g. `"org.voxedit.gain@1.0.0"`. What presets and the sidecar store.
/// Serialized as that string. Resolution looks up by `id`; the version is informational.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModuleRef {
    /// Module id (see [`ModuleDescriptor::id`]).
    pub id: String,
    /// Version that wrote the reference.
    pub version: Version,
}

impl ModuleRef {
    /// Reference to the module described by `descriptor`.
    pub fn of(descriptor: &ModuleDescriptor) -> Self {
        Self {
            id: descriptor.id.clone(),
            version: descriptor.version,
        }
    }
}

impl fmt::Display for ModuleRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.id, self.version)
    }
}

/// Error parsing a [`ModuleRef`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseModuleRefError {
    /// No `@` separator.
    #[error("module reference `{0}` has no `@version`")]
    MissingVersion(String),
    /// Empty id before `@`.
    #[error("module reference `{0}` has an empty id")]
    EmptyId(String),
    /// Bad version after `@`.
    #[error(transparent)]
    Version(#[from] ParseVersionError),
}

impl FromStr for ModuleRef {
    type Err = ParseModuleRefError;

    /// Splits at the **last** `@`, so ids containing `@` (e.g. some LV2 URIs) still parse.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (id, version) = s
            .rsplit_once('@')
            .ok_or_else(|| ParseModuleRefError::MissingVersion(s.to_owned()))?;
        if id.is_empty() {
            return Err(ParseModuleRefError::EmptyId(s.to_owned()));
        }
        Ok(Self {
            id: id.to_owned(),
            version: version.parse()?,
        })
    }
}

impl Serialize for ModuleRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ModuleRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// UI text: English fallback plus an optional i18n key. Built-ins must set keys; adapters give
/// text only.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalizedText {
    /// English fallback text.
    pub text: String,
    /// i18n key (`ui/src/lib/i18n/`), if any.
    pub key: Option<String>,
}

impl LocalizedText {
    /// Text without an i18n key (adapters).
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: None,
        }
    }

    /// Text with an i18n key (built-ins).
    pub fn keyed(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: Some(key.into()),
        }
    }
}

/// Static description of a module type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleDescriptor {
    /// Built-ins and packaged modules (ADR-006): reverse-DNS, `[a-z0-9.-]+`, e.g.
    /// `"org.voxedit.gain"`. External plugins (adapters): `"<format>:<native id>"` —
    /// `"clap:com.u-he.diva"`, `"vst3:<32 hex cid>"`, `"lv2:<uri>"`,
    /// `"jsfx:<path relative to the effects root>"`, `"vst2:<id>"`. Ids are permanent.
    pub id: String,
    /// Module version. Adapters use [`Version::parse_lenient`].
    pub version: Version,
    /// Display name.
    pub name: LocalizedText,
    /// Vendor name.
    pub vendor: String,
    /// One-line description.
    pub description: LocalizedText,
    /// Homepage.
    pub url: Option<String>,
    /// CLAP feature strings (see [`features`]), e.g. `["audio-effect", "compressor", "mono"]`.
    pub features: Vec<String>,
    /// The [`ModuleState::format_version`] this build writes.
    pub state_format_version: u32,
    /// [`MODULE_API_VERSION`] the module was built against.
    pub api_version: u32,
}

/// CLAP feature strings used in [`ModuleDescriptor::features`].
pub mod features {
    /// Audio effect.
    pub const AUDIO_EFFECT: &str = "audio-effect";
    /// Analyzer.
    pub const ANALYZER: &str = "analyzer";
    /// Equalizer.
    pub const EQUALIZER: &str = "equalizer";
    /// Filter.
    pub const FILTER: &str = "filter";
    /// Compressor.
    pub const COMPRESSOR: &str = "compressor";
    /// Expander.
    pub const EXPANDER: &str = "expander";
    /// Gate.
    pub const GATE: &str = "gate";
    /// Limiter.
    pub const LIMITER: &str = "limiter";
    /// Restoration (noise reduction, de-click, ...).
    pub const RESTORATION: &str = "restoration";
    /// Utility (gain, ...).
    pub const UTILITY: &str = "utility";
    /// Mastering.
    pub const MASTERING: &str = "mastering";
    /// Mono processing.
    pub const MONO: &str = "mono";
}

/// Creates instances of one module type. Registered in the rack's registry
/// (`id → Arc<dyn ModuleFactory>`).
pub trait ModuleFactory: Send + Sync {
    /// The module type's descriptor.
    fn descriptor(&self) -> &ModuleDescriptor;
    /// \[control thread\] A new, inactive instance with default parameter values.
    fn create(&self) -> Result<Box<dyn Module>, ModuleError>;
    /// Read-only factory presets.
    fn presets(&self) -> Vec<ModulePreset> {
        Vec::new()
    }
}

/// A factory preset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModulePreset {
    /// Stable preset key.
    pub key: String,
    /// Display name.
    pub name: LocalizedText,
    /// Module state the preset loads.
    pub state: ModuleState,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_strict_parse_and_display() {
        let v: Version = "1.2.3".parse().unwrap();
        assert_eq!(v, Version::new(1, 2, 3));
        assert_eq!(v.to_string(), "1.2.3");
        for bad in [
            "",
            "1",
            "1.2",
            "1.2.3.4",
            "1.x.3",
            "1.2.-3",
            " 1.2.3",
            "1.2.3-beta",
            "+1.2.3",
        ] {
            assert!(bad.parse::<Version>().is_err(), "{bad:?} should not parse");
        }
    }

    #[test]
    fn version_lenient_parse() {
        assert_eq!(Version::parse_lenient("1.2"), Version::new(1, 2, 0));
        assert_eq!(Version::parse_lenient("2.0 beta"), Version::new(2, 0, 0));
        assert_eq!(Version::parse_lenient("1.2.3.4"), Version::new(1, 2, 3));
        assert_eq!(Version::parse_lenient("abc"), Version::new(0, 0, 0));
        assert_eq!(Version::parse_lenient("7"), Version::new(7, 0, 0));
    }

    #[test]
    fn module_ref_round_trip_and_serde() {
        let r: ModuleRef = "org.voxedit.gain@1.0.0".parse().unwrap();
        assert_eq!(r.id, "org.voxedit.gain");
        assert_eq!(r.version, Version::new(1, 0, 0));
        assert_eq!(r.to_string(), "org.voxedit.gain@1.0.0");
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, "\"org.voxedit.gain@1.0.0\"");
        assert_eq!(serde_json::from_str::<ModuleRef>(&json).unwrap(), r);

        let lv2: ModuleRef = "lv2:urn:a@b@1.2.3".parse().unwrap();
        assert_eq!(lv2.id, "lv2:urn:a@b");
        assert!("org.voxedit.gain".parse::<ModuleRef>().is_err());
        assert!("@1.0.0".parse::<ModuleRef>().is_err());
        assert!("x@1.0".parse::<ModuleRef>().is_err());
    }

    #[test]
    fn version_serde_is_a_string() {
        let json = serde_json::to_string(&Version::new(1, 2, 0)).unwrap();
        assert_eq!(json, "\"1.2.0\"");
        assert!(serde_json::from_str::<Version>("\"1.2\"").is_err());
    }
}
