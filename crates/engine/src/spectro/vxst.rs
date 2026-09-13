//! `VXST` — one spectrogram tile (ADR-003 §2 + Amendment 1), streamed on a spectral view's
//! channel, one message per tile.
//!
//! ```text
//! Off  Type     Field
//! 0    [u8;4]   "VXST"
//! 4    u16      version = 1
//! 6    u16      header_len = 64
//! 8    u32      request_id
//! 12   u32      flags: bit0 LAST (final tile of the request), bit1 PREVIEW (replaced later)
//! 16   u64      audio_rev
//! 24   u64      first_frame_center_sample (frame i is centred at this + i·hop)
//! 32   u32      hop_samples
//! 36   u32      fft_size
//! 40   u32      frames (≤ 256)
//! 44   u32      bins (= fft_size/2 + 1)
//! 48   f32      q_floor_db = −150
//! 52   f32      q_ceil_db = +6
//! 56   u32      tile_index
//! 60   u32      window (0 = Hann)
//! 64   u8[]     frames × bins codes, frame-major, bin 0 = DC
//! ```

/// `VXST`'s magic, version and header length (ADR-003 §2).
pub const VXST_MAGIC: [u8; 4] = *b"VXST";
pub const VXST_VERSION: u16 = 1;
pub const VXST_HEADER_LEN: u16 = 64;
/// ADR-003 `window` field: periodic Hann.
pub const WINDOW_HANN: u32 = 0;

/// `VXST` flag bits.
pub mod vxst_flags {
    /// The final tile of its request (after refinement).
    pub const LAST: u32 = 1 << 0;
    /// A fast preview; a refined tile of the same request and `tile_index` replaces it
    /// (ADR-003 Amendment 1, SPEC-007 §4.4).
    pub const PREVIEW: u32 = 1 << 1;
}

/// Everything in a `VXST` frame but the code payload.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VxstHeader {
    pub request_id: u32,
    pub flags: u32,
    pub audio_rev: u64,
    pub first_frame_center_sample: u64,
    pub hop_samples: u32,
    pub fft_size: u32,
    pub frames: u32,
    pub bins: u32,
    pub q_floor_db: f32,
    pub q_ceil_db: f32,
    pub tile_index: u32,
    pub window: u32,
}

/// Encodes one `VXST` frame: the 64-byte header, then `payload` (`frames × bins` codes).
pub fn encode_vxst(header: &VxstHeader, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(usize::from(VXST_HEADER_LEN) + payload.len());
    out.extend_from_slice(&VXST_MAGIC);
    out.extend_from_slice(&VXST_VERSION.to_le_bytes());
    out.extend_from_slice(&VXST_HEADER_LEN.to_le_bytes());
    out.extend_from_slice(&header.request_id.to_le_bytes());
    out.extend_from_slice(&header.flags.to_le_bytes());
    out.extend_from_slice(&header.audio_rev.to_le_bytes());
    out.extend_from_slice(&header.first_frame_center_sample.to_le_bytes());
    out.extend_from_slice(&header.hop_samples.to_le_bytes());
    out.extend_from_slice(&header.fft_size.to_le_bytes());
    out.extend_from_slice(&header.frames.to_le_bytes());
    out.extend_from_slice(&header.bins.to_le_bytes());
    out.extend_from_slice(&header.q_floor_db.to_le_bytes());
    out.extend_from_slice(&header.q_ceil_db.to_le_bytes());
    out.extend_from_slice(&header.tile_index.to_le_bytes());
    out.extend_from_slice(&header.window.to_le_bytes());
    debug_assert_eq!(out.len(), usize::from(VXST_HEADER_LEN));
    out.extend_from_slice(payload);
    out
}

/// Decodes a `VXST` frame into its header and `frames × bins` payload; `None` if `bytes` isn't
/// one (wrong magic/version, short header or payload). Accepts a longer `header_len` (ADR-003
/// §2: fields may be appended).
pub fn decode_vxst(bytes: &[u8]) -> Option<(VxstHeader, &[u8])> {
    let u16_at = |o: usize| Some(u16::from_le_bytes(bytes.get(o..o + 2)?.try_into().ok()?));
    let u32_at = |o: usize| Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?));
    let u64_at = |o: usize| Some(u64::from_le_bytes(bytes.get(o..o + 8)?.try_into().ok()?));
    let f32_at = |o: usize| Some(f32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?));
    if bytes.get(0..4)? != VXST_MAGIC || u16_at(4)? != VXST_VERSION {
        return None;
    }
    let header_len = usize::from(u16_at(6)?);
    if header_len < usize::from(VXST_HEADER_LEN) {
        return None;
    }
    let header = VxstHeader {
        request_id: u32_at(8)?,
        flags: u32_at(12)?,
        audio_rev: u64_at(16)?,
        first_frame_center_sample: u64_at(24)?,
        hop_samples: u32_at(32)?,
        fft_size: u32_at(36)?,
        frames: u32_at(40)?,
        bins: u32_at(44)?,
        q_floor_db: f32_at(48)?,
        q_ceil_db: f32_at(52)?,
        tile_index: u32_at(56)?,
        window: u32_at(60)?,
    };
    let payload_len = (header.frames as usize).checked_mul(header.bins as usize)?;
    let payload = bytes.get(header_len..header_len.checked_add(payload_len)?)?;
    Some((header, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> VxstHeader {
        VxstHeader {
            request_id: 0x0102_0304,
            flags: vxst_flags::LAST | vxst_flags::PREVIEW,
            audio_rev: 0x1112_1314_1516_1718,
            first_frame_center_sample: 0x2122_2324_2526_2728,
            hop_samples: 512,
            fft_size: 2048,
            frames: 2,
            bins: 1025,
            q_floor_db: -150.0,
            q_ceil_db: 6.0,
            tile_index: 9,
            window: WINDOW_HANN,
        }
    }

    #[test]
    fn header_layout_matches_adr_003() {
        let h = header();
        let payload: Vec<u8> = (0..2 * 1025).map(|i| (i % 251) as u8).collect();
        let b = encode_vxst(&h, &payload);
        assert_eq!(b.len(), 64 + 2 * 1025);
        assert_eq!(&b[0..4], b"VXST");
        assert_eq!(u16::from_le_bytes([b[4], b[5]]), 1);
        assert_eq!(u16::from_le_bytes([b[6], b[7]]), 64);
        assert_eq!(
            u32::from_le_bytes(b[8..12].try_into().unwrap()),
            0x0102_0304
        );
        assert_eq!(u32::from_le_bytes(b[12..16].try_into().unwrap()), 0b11);
        assert_eq!(
            u64::from_le_bytes(b[16..24].try_into().unwrap()),
            0x1112_1314_1516_1718
        );
        assert_eq!(
            u64::from_le_bytes(b[24..32].try_into().unwrap()),
            0x2122_2324_2526_2728
        );
        assert_eq!(u32::from_le_bytes(b[32..36].try_into().unwrap()), 512);
        assert_eq!(u32::from_le_bytes(b[36..40].try_into().unwrap()), 2048);
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(b[44..48].try_into().unwrap()), 1025);
        assert_eq!(&b[48..52], &(-150.0f32).to_le_bytes());
        assert_eq!(&b[52..56], &6.0f32.to_le_bytes());
        assert_eq!(u32::from_le_bytes(b[56..60].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(b[60..64].try_into().unwrap()), 0);
        assert_eq!(&b[64..], &payload[..]);
    }

    #[test]
    fn round_trip_and_rejections() {
        let h = header();
        let payload: Vec<u8> = (0..2 * 1025).map(|i| (i * 7 % 256) as u8).collect();
        let bytes = encode_vxst(&h, &payload);
        let (back, body) = decode_vxst(&bytes).unwrap();
        assert_eq!(back, h);
        assert_eq!(body, &payload[..]);

        assert!(decode_vxst(&bytes[..63]).is_none(), "short header");
        assert!(
            decode_vxst(&bytes[..bytes.len() - 1]).is_none(),
            "short payload"
        );
        let mut wrong = bytes.clone();
        wrong[0] = b'Q';
        assert!(decode_vxst(&wrong).is_none(), "wrong magic");
        let mut v2 = bytes.clone();
        v2[4] = 2;
        assert!(decode_vxst(&v2).is_none(), "unknown version");

        // A longer header_len (appended fields) is skipped over.
        let mut longer = bytes[..64].to_vec();
        longer[6] = 72;
        longer.extend_from_slice(&[0xAA; 8]);
        longer.extend_from_slice(&payload);
        let (back, body) = decode_vxst(&longer).unwrap();
        assert_eq!(back.tile_index, 9);
        assert_eq!(body, &payload[..]);
    }
}
