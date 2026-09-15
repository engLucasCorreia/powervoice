//! Module state, migration and the host-side load pipeline (ADR-005 §10).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::module::Module;
use crate::param::ParamFlags;

/// Persistent state of one module instance (what presets and the sidecar store).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModuleState {
    /// Format of this state; drives migration (the module `version` does not).
    pub format_version: u32,
    /// key → plain value, for every non-READ_ONLY param (HIDDEN included). `BTreeMap` for
    /// deterministic output (stable sidecar diffs). Values finite (−∞ dB = `min`).
    /// External plugins are blob-only: `params` is empty.
    pub params: BTreeMap<String, f64>,
    /// Opaque module data (noise profile, external plugin chunk). JSON: standard base64 string.
    ///
    /// **Size (T-810 audit):** there is no cap on `blob` itself. In practice it is bounded by
    /// whatever carries it: the sandbox control channel's `MAX_FRAME_BYTES` (64 MiB,
    /// `vox_sandbox_ipc::control`, ADR-008 Amendment 2 §2) for a live out-of-process plugin's
    /// save/load round trip, and the sidecar's `SIDECAR_MAX_BYTES` (64 MiB,
    /// `vox_project::sidecar`, SPEC-018 §2.6.6) for what a document can persist — both comfortably
    /// hold blobs an order of magnitude larger than any plugin state seen so far (tested to 10
    /// MiB, see `vox_presets::module_store` and the sandboxed `*_rack` tests). A user or module
    /// preset file (`vox_presets`) has no read-size cap of its own; it inherits the filesystem's.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "blob_base64")]
    pub blob: Option<Vec<u8>>,
}

impl ModuleState {
    /// Empty state (no params, no blob) at `format_version`.
    pub fn new(format_version: u32) -> Self {
        Self {
            format_version,
            params: BTreeMap::new(),
            blob: None,
        }
    }
}

/// Error saving, loading or migrating a state.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    /// The state was written by a newer module build.
    #[error("state format {found} is newer than supported {supported}")]
    TooNew {
        /// Version in the state.
        found: u32,
        /// Version this build writes.
        supported: u32,
    },
    /// The opaque blob could not be decoded.
    #[error("invalid state blob: {0}")]
    InvalidBlob(String),
    /// `migrate_state` failed.
    #[error("state migration failed: {0}")]
    Migration(String),
}

/// Host-side load pipeline (used by rack/project before [`Module::load_state`]):
/// 1. `s.format_version` newer than the module's → [`StateError::TooNew`];
/// 2. older → [`Module::migrate_state`] (which must return the current version);
/// 3. drop unknown and READ_ONLY keys, add missing keys at their default, and
///    `clamp_quantize` every value.
///
/// The blob is passed through unchanged.
pub fn prepare_state(m: &dyn Module, s: ModuleState) -> Result<ModuleState, StateError> {
    let current = m.descriptor().state_format_version;
    let mut s = if s.format_version > current {
        return Err(StateError::TooNew {
            found: s.format_version,
            supported: current,
        });
    } else if s.format_version < current {
        let migrated = m.migrate_state(s)?;
        if migrated.format_version != current {
            return Err(StateError::Migration(format!(
                "migrate_state returned format {} instead of {current}",
                migrated.format_version
            )));
        }
        migrated
    } else {
        s
    };
    let params = m
        .params()
        .iter()
        .filter(|p| !p.flags.contains(ParamFlags::READ_ONLY))
        .map(|p| {
            let v = s.params.get(&p.key).copied().unwrap_or(p.default);
            (p.key.clone(), p.clamp_quantize(v))
        })
        .collect();
    s.params = params;
    Ok(s)
}

/// Hand-written standard base64 (RFC 4648, with padding) for [`ModuleState::blob`].
mod blob_base64 {
    use serde::{Deserialize, Deserializer, Serializer};

    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub(super) fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 0x3f) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn sextet(c: u8) -> Option<u32> {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        Some(u32::from(v))
    }

    pub(super) fn decode(s: &str) -> Result<Vec<u8>, &'static str> {
        let s = s.as_bytes();
        if !s.len().is_multiple_of(4) {
            return Err("length is not a multiple of 4");
        }
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        for (ci, chunk) in s.chunks(4).enumerate() {
            let last = ci + 1 == s.len() / 4;
            let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
            if pad > 2 || (pad > 0 && !last) {
                return Err("misplaced padding");
            }
            let mut n = 0u32;
            for &c in &chunk[..4 - pad] {
                n = (n << 6) | sextet(c).ok_or("invalid character")?;
            }
            n <<= 6 * pad as u32;
            let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
            out.extend_from_slice(&bytes[..3 - pad]);
        }
        Ok(out)
    }

    pub(super) fn serialize<S: Serializer>(
        blob: &Option<Vec<u8>>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        match blob {
            Some(bytes) => s.serialize_str(&encode(bytes)),
            None => s.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Vec<u8>>, D::Error> {
        match Option::<String>::deserialize(d)? {
            Some(s) => decode(&s).map(Some).map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known_vectors() {
        let cases: [(&[u8], &str); 7] = [
            (b"", ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ];
        for (raw, enc) in cases {
            assert_eq!(blob_base64::encode(raw), enc);
            assert_eq!(blob_base64::decode(enc).unwrap(), raw);
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(
            blob_base64::decode(&blob_base64::encode(&all)).unwrap(),
            all
        );
        for bad in ["Zg=", "Z===", "Zg==Zg==", "Z!==", "Zm9v\n"] {
            assert!(blob_base64::decode(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn state_json_shape() {
        let mut s = ModuleState::new(1);
        s.params.insert("gain_db".into(), -6.0);
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"format_version":1,"params":{"gain_db":-6.0}}"#);
        assert_eq!(serde_json::from_str::<ModuleState>(&json).unwrap(), s);

        s.blob = Some(vec![0, 1, 2, 250]);
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""blob":"AAEC+g==""#), "{json}");
        assert_eq!(serde_json::from_str::<ModuleState>(&json).unwrap(), s);
        assert!(
            serde_json::from_str::<ModuleState>(r#"{"format_version":1,"params":{},"blob":"%%"}"#)
                .is_err()
        );
        let null_blob: ModuleState =
            serde_json::from_str(r#"{"format_version":1,"params":{},"blob":null}"#).unwrap();
        assert_eq!(null_blob.blob, None);
    }

    #[test]
    fn state_f64_round_trips_exactly_through_json() {
        let mut s = ModuleState::new(1);
        for (i, v) in [
            0.1,
            -59.999_999_999_999_99,
            1.0 / 3.0,
            20_000.0 * 0.123_456_789_012_345_67,
        ]
        .into_iter()
        .enumerate()
        {
            s.params.insert(format!("p{i}"), v);
        }
        let back: ModuleState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        for (k, v) in &s.params {
            assert_eq!(back.params[k].to_bits(), v.to_bits(), "{k}");
        }
    }
}
