//! `VXPK` — the peaks/raw-samples binary response (ADR-003 §2, response to `peaks_get`).
//!
//! ```text
//! Off  Type     Field
//! 0    [u8;4]   "VXPK"
//! 4    u16      version = 1
//! 6    u16      header_len = 48
//! 8    u32      request_id
//! 12   u32      flags: bit0 RAW (payload = f32 samples, spp = 1); bit1 PARTIAL
//! 16   u64      audio_rev
//! 24   u64      start_sample
//! 32   u32      samples_per_bucket (spp)
//! 36   u32      count (buckets, or samples if RAW)
//! 40   u32      sample_rate_hz
//! 44   u32      reserved = 0
//! 48   f32[]    RAW: `count` samples. Otherwise `count x (min, max)`.
//! ```
//!
//! `src-tauri` (S1-03) owns the actual `ipc::Response`/`peaks_get` wiring; this crate only
//! provides the byte encoder, since `peaks_query::peaks`'s buckets are the payload.

/// `VXPK`'s magic, version and fixed header length (ADR-003 §2).
pub const VXPK_MAGIC: [u8; 4] = *b"VXPK";
pub const VXPK_VERSION: u16 = 1;
pub const VXPK_HEADER_LEN: u16 = 48;

const FLAG_RAW: u32 = 1 << 0;
const FLAG_PARTIAL: u32 = 1 << 1;

/// Everything in a `VXPK` frame but the bucket/sample payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VxpkHeader {
    pub request_id: u32,
    pub audio_rev: u64,
    pub start_sample: u64,
    /// Samples per bucket; `1` means the payload is `RAW` samples (see [`encode_vxpk`]).
    pub samples_per_bucket: u32,
    pub sample_rate_hz: u32,
    /// Some buckets are not yet computed (`(NaN, NaN)`, ADR-003). Set by `peaks_get`'s live
    /// counterparts — `record_peaks_get` (H-07, always `true`: a take is always still growing)
    /// and `import_peaks_get` (H-71: `true` whenever the requested range reaches past what the
    /// import has committed so far) — never by `peaks_get` itself, which only ever reads a
    /// finished document.
    pub partial: bool,
}

/// Encodes one `VXPK` frame. `buckets.len()` becomes `count`. In `RAW` mode
/// (`header.samples_per_bucket == 1`, [`crate::peaks_query::peaks`]'s convention for raw samples)
/// only each pair's first element is written, since `min == max` for a single sample.
pub fn encode_vxpk(header: &VxpkHeader, buckets: &[(f32, f32)]) -> Vec<u8> {
    let raw = header.samples_per_bucket == 1;
    let mut flags = 0u32;
    if raw {
        flags |= FLAG_RAW;
    }
    if header.partial {
        flags |= FLAG_PARTIAL;
    }
    let count = u32::try_from(buckets.len()).unwrap_or(u32::MAX);
    let payload_bytes = buckets.len() * if raw { 4 } else { 8 };

    let mut out = Vec::with_capacity(VXPK_HEADER_LEN as usize + payload_bytes);
    out.extend_from_slice(&VXPK_MAGIC);
    out.extend_from_slice(&VXPK_VERSION.to_le_bytes());
    out.extend_from_slice(&VXPK_HEADER_LEN.to_le_bytes());
    out.extend_from_slice(&header.request_id.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&header.audio_rev.to_le_bytes());
    out.extend_from_slice(&header.start_sample.to_le_bytes());
    out.extend_from_slice(&header.samples_per_bucket.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&header.sample_rate_hz.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved
    debug_assert_eq!(out.len(), VXPK_HEADER_LEN as usize);

    for &(mn, mx) in buckets {
        out.extend_from_slice(&mn.to_le_bytes());
        if !raw {
            out.extend_from_slice(&mx.to_le_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_is_48_bytes_with_the_documented_layout() {
        let header = VxpkHeader {
            request_id: 0x0102_0304,
            audio_rev: 0x1112_1314_1516_1718,
            start_sample: 0x2122_2324_2526_2728,
            samples_per_bucket: 256,
            sample_rate_hz: 48_000,
            partial: false,
        };
        let bytes = encode_vxpk(&header, &[(-1.0, 1.0), (-0.5, 0.25)]);

        assert_eq!(&bytes[0..4], b"VXPK");
        assert_eq!(u16::from_le_bytes(bytes[4..6].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 48);
        assert_eq!(
            u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
            0x0102_0304
        );
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0); // no RAW, no PARTIAL
        assert_eq!(
            u64::from_le_bytes(bytes[16..24].try_into().unwrap()),
            0x1112_1314_1516_1718
        );
        assert_eq!(
            u64::from_le_bytes(bytes[24..32].try_into().unwrap()),
            0x2122_2324_2526_2728
        );
        assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 256);
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_le_bytes(bytes[40..44].try_into().unwrap()),
            48_000
        );
        assert_eq!(u32::from_le_bytes(bytes[44..48].try_into().unwrap()), 0);
        assert_eq!(bytes.len(), 48 + 2 * 8);
    }

    /// Golden bytes: pins the exact wire encoding of one small, fixed `VXPK` frame. A change here
    /// must be a deliberate, reviewed format change (it breaks the TS decoder in ADR-003).
    #[test]
    fn golden_bytes_for_a_fixed_bucket_frame() {
        let header = VxpkHeader {
            request_id: 7,
            audio_rev: 42,
            start_sample: 1000,
            samples_per_bucket: 64,
            sample_rate_hz: 48_000,
            partial: false,
        };
        let bytes = encode_vxpk(&header, &[(-0.5, 0.5)]);
        #[rustfmt::skip]
        let expected: Vec<u8> = vec![
            b'V', b'X', b'P', b'K',
            1, 0,             // version
            48, 0,            // header_len
            7, 0, 0, 0,       // request_id
            0, 0, 0, 0,       // flags
            42, 0, 0, 0, 0, 0, 0, 0,             // audio_rev
            0xE8, 0x03, 0, 0, 0, 0, 0, 0,        // start_sample = 1000
            64, 0, 0, 0,      // spp
            1, 0, 0, 0,       // count
            0x80, 0xBB, 0, 0, // sample_rate_hz = 48000
            0, 0, 0, 0,       // reserved
        ];
        let mut expected = expected;
        expected.extend_from_slice(&(-0.5f32).to_le_bytes());
        expected.extend_from_slice(&(0.5f32).to_le_bytes());
        assert_eq!(bytes, expected);
    }

    #[test]
    fn golden_bytes_for_a_raw_frame() {
        let header = VxpkHeader {
            request_id: 1,
            audio_rev: 0,
            start_sample: 0,
            samples_per_bucket: 1,
            sample_rate_hz: 48_000,
            partial: false,
        };
        let bytes = encode_vxpk(&header, &[(0.25, 0.25), (-0.75, -0.75)]);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 1); // RAW flag set
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 2); // count = 2 samples
        assert_eq!(bytes.len(), 48 + 2 * 4, "RAW payload is one f32 per sample");
        assert_eq!(&bytes[48..52], &0.25f32.to_le_bytes());
        assert_eq!(&bytes[52..56], &(-0.75f32).to_le_bytes());
    }

    #[test]
    fn partial_flag_is_set_when_requested() {
        let header = VxpkHeader {
            request_id: 0,
            audio_rev: 0,
            start_sample: 0,
            samples_per_bucket: 64,
            sample_rate_hz: 48_000,
            partial: true,
        };
        let bytes = encode_vxpk(&header, &[]);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0b10);
        assert_eq!(bytes.len(), 48, "no buckets: header only");
    }
}
