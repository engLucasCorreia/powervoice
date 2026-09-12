//! Minimal binary framing for the spike, following ADR-003 §2's conventions: little-endian
//! fields, an 8-byte common prefix (`magic[4]`, `version:u16`, `header_len:u16`), payload starting
//! at `header_len`. `SPWV`/`SPST` are throwaway spike-only shapes (the real `VXPK`/`VXST` layouts
//! land with T-203/T-204); `telemetry_frame` is the exception — it reproduces the *real* `VXTM`
//! layout verbatim because the ADR-003 follow-up asks for the cost of "a 72-byte binary telemetry
//! frame", i.e. this exact frame shape.

/// `SPWV` — spike waveform peaks. Header (16 bytes): common prefix + `count:u32` + `spp:u32`.
/// Payload: `count * (min:f32, max:f32)`, matching `VXPK`'s bucket layout without the rest of its
/// request/audio_rev bookkeeping (there is no document here).
pub fn waveform_frame(min_max: &[(f32, f32)], spp: u32) -> Vec<u8> {
    let count = u32::try_from(min_max.len()).expect("bucket count fits u32");
    let mut buf = Vec::with_capacity(16 + min_max.len() * 8);
    buf.extend_from_slice(b"SPWV");
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&spp.to_le_bytes());
    for &(min, max) in min_max {
        buf.extend_from_slice(&min.to_le_bytes());
        buf.extend_from_slice(&max.to_le_bytes());
    }
    buf
}

/// `SPST` — spike spectrogram texture. Header (24 bytes): common prefix + `width:u32` +
/// `height:u32` + 8 reserved bytes. Payload: `width * height` u8 magnitudes, row-major (bin 0 /
/// low frequency first in each row, oldest time first).
pub fn spectrogram_frame(pixels: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24 + pixels.len());
    buf.extend_from_slice(b"SPST");
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&24u16.to_le_bytes());
    buf.extend_from_slice(&width.to_le_bytes());
    buf.extend_from_slice(&height.to_le_bytes());
    buf.extend_from_slice(&[0u8; 8]);
    buf.extend_from_slice(pixels);
    buf
}

/// One `VXTM` telemetry frame (ADR-003): header-only, 72 bytes, no payload yet. All fields beyond
/// `seq` are synthetic placeholders — this command exists only to measure the fixed per-frame
/// Channel cost at 30 Hz vs 60 Hz, not to model real playback state.
pub fn telemetry_frame(seq: u32) -> [u8; 72] {
    let mut buf = [0u8; 72];
    buf[0..4].copy_from_slice(b"VXTM");
    buf[4..6].copy_from_slice(&1u16.to_le_bytes());
    buf[6..8].copy_from_slice(&72u16.to_le_bytes());
    buf[8..12].copy_from_slice(&seq.to_le_bytes());
    // flags = 0 (bit0 PLAYING set, for realism)
    buf[12..16].copy_from_slice(&1u32.to_le_bytes());
    let playhead_sample = u64::from(seq) * 1_600; // ~33ms of audio per frame at 48kHz, arbitrary
    buf[16..24].copy_from_slice(&playhead_sample.to_le_bytes());
    let playhead_time_ns = u64::from(seq) * 33_333_333;
    buf[24..32].copy_from_slice(&playhead_time_ns.to_le_bytes());
    buf[32..40].copy_from_slice(&48_000.0f64.to_le_bytes()); // rate
    buf[40..44].copy_from_slice(&(-6.0f32).to_le_bytes()); // out_peak_dbfs
    buf[44..48].copy_from_slice(&(-18.0f32).to_le_bytes()); // out_rms_dbfs
    buf[48..52].copy_from_slice(&(-6.0f32).to_le_bytes()); // in_peak_dbfs
    buf[52..56].copy_from_slice(&(-18.0f32).to_le_bytes()); // in_rms_dbfs
    buf[56..64].copy_from_slice(&1u64.to_le_bytes()); // audio_rev
    // dropped_rt_events, reserved = 0
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_frame_header_len_matches_layout() {
        let frame = waveform_frame(&[(-1.0, 1.0), (-0.5, 0.5)], 256);
        assert_eq!(&frame[0..4], b"SPWV");
        assert_eq!(u16::from_le_bytes([frame[4], frame[5]]), 1);
        assert_eq!(u16::from_le_bytes([frame[6], frame[7]]), 16);
        assert_eq!(frame.len(), 16 + 2 * 8);
    }

    #[test]
    fn spectrogram_frame_header_len_matches_layout() {
        let pixels = vec![0u8; 4];
        let frame = spectrogram_frame(&pixels, 2, 2);
        assert_eq!(&frame[0..4], b"SPST");
        assert_eq!(u16::from_le_bytes([frame[6], frame[7]]), 24);
        assert_eq!(frame.len(), 24 + 4);
    }

    #[test]
    fn telemetry_frame_is_72_bytes_with_vxtm_header() {
        let frame = telemetry_frame(7);
        assert_eq!(frame.len(), 72);
        assert_eq!(&frame[0..4], b"VXTM");
        assert_eq!(u16::from_le_bytes([frame[6], frame[7]]), 72);
        assert_eq!(
            u32::from_le_bytes([frame[8], frame[9], frame[10], frame[11]]),
            7
        );
    }
}
