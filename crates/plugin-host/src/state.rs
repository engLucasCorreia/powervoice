//! The proxy's state blob (T-802): the plugin's own state wrapped with what identifies it, so a
//! sidecar or preset never feeds one plugin's chunk to another.
//!
//! | Bytes | Content |
//! |---|---|
//! | 8 | magic `PVPLUGST` |
//! | 4 | wrapper version (`1`), `u32` little-endian |
//! | 4 | header length `h`, `u32` little-endian |
//! | `h` | header, UTF-8 JSON [`StateHeader`] |
//! | rest | the plugin's state, verbatim |

use serde::{Deserialize, Serialize};

/// Wrapper magic.
pub const STATE_MAGIC: [u8; 8] = *b"PVPLUGST";
/// Wrapper version this build writes and reads.
pub const STATE_WRAPPER_VERSION: u32 = 1;

/// What the wrapped state belongs to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateHeader {
    /// Plugin format (`"test"`, `"clap"`, …).
    pub format: String,
    /// The backend's plugin reference (id, path).
    pub plugin: String,
    /// The module id the rack knows it by (`"<format>:<id>"`).
    pub id: String,
    /// The plugin's version when the state was saved.
    pub version: String,
}

/// Error unwrapping a state blob.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StateBlobError {
    /// Not a wrapped plugin state.
    #[error("not a plugin state (bad magic)")]
    BadMagic,
    /// A wrapper version this build doesn't read.
    #[error("plugin state wrapper version {0} is not supported")]
    Version(u32),
    /// Truncated or malformed.
    #[error("malformed plugin state: {0}")]
    Malformed(String),
}

/// Wraps `data` with `header`.
pub fn wrap(header: &StateHeader, data: &[u8]) -> Vec<u8> {
    let json = serde_json::to_vec(header).unwrap_or_default();
    let mut out = Vec::with_capacity(16 + json.len() + data.len());
    out.extend_from_slice(&STATE_MAGIC);
    out.extend_from_slice(&STATE_WRAPPER_VERSION.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(data);
    out
}

/// Splits a wrapped blob into its header and the plugin's state.
pub fn unwrap(blob: &[u8]) -> Result<(StateHeader, &[u8]), StateBlobError> {
    if blob.len() < 16 || blob[..8] != STATE_MAGIC {
        return Err(StateBlobError::BadMagic);
    }
    let word = |at: usize| u32::from_le_bytes([blob[at], blob[at + 1], blob[at + 2], blob[at + 3]]);
    let version = word(8);
    if version != STATE_WRAPPER_VERSION {
        return Err(StateBlobError::Version(version));
    }
    let h = word(12) as usize;
    let end = 16usize
        .checked_add(h)
        .filter(|&e| e <= blob.len())
        .ok_or_else(|| StateBlobError::Malformed("header length".into()))?;
    let header = serde_json::from_slice(&blob[16..end])
        .map_err(|e| StateBlobError::Malformed(e.to_string()))?;
    Ok((header, &blob[end..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> StateHeader {
        StateHeader {
            format: "test".into(),
            plugin: "gain".into(),
            id: "test:gain".into(),
            version: "1.0.0".into(),
        }
    }

    #[test]
    fn wraps_and_unwraps() {
        let blob = wrap(&header(), b"\x00plugin\xffbytes");
        let (h, data) = unwrap(&blob).unwrap();
        assert_eq!(h, header());
        assert_eq!(data, b"\x00plugin\xffbytes");
        let empty = wrap(&header(), &[]);
        assert_eq!(unwrap(&empty).unwrap().1, b"");
    }

    #[test]
    fn rejects_foreign_and_damaged_blobs() {
        assert_eq!(unwrap(b"{\"gain_db\":1}"), Err(StateBlobError::BadMagic));
        let mut blob = wrap(&header(), b"x");
        blob[8] = 9;
        assert_eq!(unwrap(&blob), Err(StateBlobError::Version(9)));
        let blob = wrap(&header(), b"x");
        assert!(matches!(
            unwrap(&blob[..20]),
            Err(StateBlobError::Malformed(_))
        ));
        let mut blob = wrap(&header(), b"x");
        blob[16] = b'!';
        assert!(matches!(unwrap(&blob), Err(StateBlobError::Malformed(_))));
    }
}
