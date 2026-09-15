//! The JSFX backend's state bytes (ADR-008 Amendment 11 §4): `PVJS`, `u32` version 1, then
//!
//! - the sliders (`u32` count; per slider its `u32` index and `f64` value) — every slider the
//!   header declares, as ysfx saves them;
//! - the `@serialize` bytes (`u32` length, then the bytes; empty without a `@serialize`
//!   section).
//!
//! Little-endian throughout. The proxy wraps these bytes with the plugin identity (ADR-008
//! Amendment 2 §7).

/// Magic.
pub(crate) const MAGIC: [u8; 4] = *b"PVJS";
/// Version.
pub(crate) const VERSION: u32 = 1;

/// The decoded state.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Blob {
    pub(crate) sliders: Vec<(u32, f64)>,
    pub(crate) data: Vec<u8>,
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("truncated JSFX state".into());
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

impl Blob {
    /// The bytes.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16 + self.sliders.len() * 12 + self.data.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.sliders.len() as u32).to_le_bytes());
        for (index, value) in &self.sliders {
            out.extend_from_slice(&index.to_le_bytes());
            out.extend_from_slice(&value.to_le_bytes());
        }
        out.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.data);
        out
    }

    /// Parses [`Self::encode`]'s bytes.
    pub(crate) fn decode(data: &[u8]) -> Result<Self, String> {
        let mut r = Reader(data);
        if r.take(4)? != MAGIC {
            return Err("not a JSFX state".into());
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(format!("JSFX state version {version} is not supported"));
        }
        let mut blob = Self::default();
        for _ in 0..r.u32()? {
            let index = r.u32()?;
            let b = r.take(8)?;
            let mut v = [0u8; 8];
            v.copy_from_slice(b);
            blob.sliders.push((index, f64::from_le_bytes(v)));
        }
        let n = r.u32()? as usize;
        blob.data = r.take(n)?.to_vec();
        if !r.0.is_empty() {
            return Err("trailing bytes after the JSFX state".into());
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
            sliders: vec![(0, -6.5), (3, 1.0), (255, f64::MIN_POSITIVE)],
            data: vec![1, 2, 3, 4, 5],
        };
        let bytes = blob.encode();
        assert_eq!(&bytes[..4], b"PVJS");
        assert_eq!(Blob::decode(&bytes).unwrap(), blob);
        assert!(Blob::decode(&bytes[..bytes.len() - 1]).is_err());
        assert!(Blob::decode(b"PVL2\x01\0\0\0").is_err());
        assert!(Blob::decode(b"PVJS\x02\0\0\0\0\0\0\0\0\0\0\0").is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Blob::decode(&longer).is_err());
        assert_eq!(
            Blob::decode(&Blob::default().encode()).unwrap(),
            Blob::default()
        );
    }
}
