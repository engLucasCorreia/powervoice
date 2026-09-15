//! The LV2 backend's state bytes (ADR-008 Amendment 8 §4): `PVL2`, `u32` version 1, then
//!
//! - the control input values by **port symbol** (`u32` count; per port a `u32`-length-prefixed
//!   UTF-8 symbol and an `f32`) — LV2 hosts save port values themselves, and symbols (unlike
//!   indices) are stable across plugin versions;
//! - the `state:interface` properties (`u32` count; per property the key URI and the type URI,
//!   each `u32`-length-prefixed, the `u32` flags and the `u32`-length-prefixed value bytes) —
//!   URIs, not URIDs, since URIDs only mean something inside one instance.
//!
//! Little-endian throughout. The proxy wraps these bytes with the plugin identity (ADR-008
//! Amendment 2 §7).

/// Magic.
pub(crate) const MAGIC: [u8; 4] = *b"PVL2";
/// Version.
pub(crate) const VERSION: u32 = 1;

/// One `state:interface` property.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Property {
    pub(crate) key: String,
    pub(crate) type_: String,
    pub(crate) flags: u32,
    pub(crate) value: Vec<u8>,
}

/// The decoded state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Blob {
    pub(crate) controls: Vec<(String, f32)>,
    pub(crate) properties: Vec<Property>,
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("truncated LV2 state".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        self.take(n)
    }

    fn string(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| "bad LV2 state text".to_owned())
    }
}

impl Blob {
    /// The bytes.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.controls.len() as u32).to_le_bytes());
        for (symbol, value) in &self.controls {
            put_bytes(&mut out, symbol.as_bytes());
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&(self.properties.len() as u32).to_le_bytes());
        for p in &self.properties {
            put_bytes(&mut out, p.key.as_bytes());
            put_bytes(&mut out, p.type_.as_bytes());
            out.extend_from_slice(&p.flags.to_le_bytes());
            put_bytes(&mut out, &p.value);
        }
        out
    }

    /// Parses [`Self::encode`]'s bytes.
    pub(crate) fn decode(data: &[u8]) -> Result<Self, String> {
        let mut r = Reader(data);
        if r.take(4)? != MAGIC {
            return Err("not an LV2 state".into());
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(format!("LV2 state version {version} is not supported"));
        }
        let mut blob = Self::default();
        for _ in 0..r.u32()? {
            let symbol = r.string()?;
            let b = r.take(4)?;
            blob.controls
                .push((symbol, f32::from_le_bytes([b[0], b[1], b[2], b[3]])));
        }
        for _ in 0..r.u32()? {
            let key = r.string()?;
            let type_ = r.string()?;
            let flags = r.u32()?;
            let value = r.bytes()?.to_vec();
            blob.properties.push(Property {
                key,
                type_,
                flags,
                value,
            });
        }
        if !r.0.is_empty() {
            return Err("trailing bytes after the LV2 state".into());
        }
        Ok(blob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_damage() {
        let blob = Blob {
            controls: vec![("gain".into(), -6.5), ("mode".into(), 2.0)],
            properties: vec![Property {
                key: "urn:x#k".into(),
                type_: "http://lv2plug.in/ns/ext/atom#Int".into(),
                flags: 3,
                value: vec![1, 0, 0, 0],
            }],
        };
        let bytes = blob.encode();
        assert_eq!(&bytes[..4], b"PVL2");
        assert_eq!(Blob::decode(&bytes).unwrap(), blob);
        assert!(Blob::decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(Blob::decode(b"PVV3\x01\0\0\0").is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Blob::decode(&longer).is_err());
        assert_eq!(
            Blob::decode(&Blob::default().encode()).unwrap(),
            Blob::default()
        );
    }
}
