//! Class ids ↔ their FUID strings: 32 upper-case hex digits of the id's four 32-bit words, the
//! SDK's `FUID::toString` order (and `moduleinfo.json`'s `CID`). On Windows a `TUID` is laid
//! out like a COM `GUID` (the first three fields little-endian), elsewhere big-endian — the
//! string is the same on every platform.

use vst3::Steinberg::TUID;

/// The four 32-bit words of `t`.
fn words(t: &TUID) -> [u32; 4] {
    let b: [u8; 16] = t.map(|c| c as u8);
    let be = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    if cfg!(windows) {
        let a = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let hi = u32::from(u16::from_le_bytes([b[4], b[5]]));
        let lo = u32::from(u16::from_le_bytes([b[6], b[7]]));
        [a, (hi << 16) | lo, be(8), be(12)]
    } else {
        [be(0), be(4), be(8), be(12)]
    }
}

/// `t` as its FUID string.
pub(crate) fn to_hex(t: &TUID) -> String {
    words(t).iter().map(|w| format!("{w:08X}")).collect()
}

/// A FUID string (any case) as a class id; `None` unless it's exactly 32 hex digits.
pub(crate) fn from_hex(s: &str) -> Option<TUID> {
    if s.len() != 32 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let w = |i: usize| u32::from_str_radix(&s[i * 8..i * 8 + 8], 16).ok();
    Some(vst3::uid(w(0)?, w(1)?, w(2)?, w(3)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuid_strings_round_trip() {
        let t = vst3::uid(0x5056_5633, 0x5445_5354, 0x4741_494E, 0x0000_0001);
        assert_eq!(to_hex(&t), "50565633544553544741494E00000001");
        assert_eq!(from_hex("50565633544553544741494e00000001"), Some(t));
        assert_eq!(to_hex(&from_hex(&to_hex(&t)).unwrap()), to_hex(&t));
        assert_eq!(from_hex("5056"), None);
        assert_eq!(from_hex("Z0565633544553544741494E00000001"), None);
    }
}
