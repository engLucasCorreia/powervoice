//! Telemetry frames (ADR-003 §2 `VXTM`): playhead anchor + output meter, built by the control
//! thread at the telemetry rate (60 Hz, ADR-009). The engine hands [`TelemetryFrame`]s to a sink;
//! the app encodes them with [`TelemetryFrame::encode`] and sends them over a Tauri `Channel`.

/// Encoded `VXTM` frame length (header only, version 1).
pub const VXTM_LEN: usize = 72;

/// `VXTM` flag bits (ADR-003 §2).
pub mod vxtm_flags {
    /// Playback is running.
    pub const PLAYING: u32 = 1 << 0;
    /// Recording.
    pub const RECORDING: u32 = 1 << 1;
    /// Monitoring.
    pub const MONITORING: u32 = 1 << 2;
    /// Loop playback.
    pub const LOOPING: u32 = 1 << 3;
    /// An underrun/xrun since the previous frame.
    pub const XRUN: u32 = 1 << 4;
    /// Output peak ≥ 0 dBFS since the previous frame.
    pub const OUT_CLIP: u32 = 1 << 5;
    /// Input peak ≥ 0 dBFS since the previous frame.
    pub const IN_CLIP: u32 = 1 << 6;
}

/// One telemetry frame. Meter values are max-hold (peak) / energy average (RMS) since the
/// previous frame; `-inf` = digital silence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TelemetryFrame {
    /// Frame counter.
    pub seq: u32,
    /// [`vxtm_flags`] bits.
    pub flags: u32,
    /// Heard position (document samples) at `playhead_time_ns`.
    pub playhead_sample: u64,
    /// App-clock time of `playhead_sample`.
    pub playhead_time_ns: u64,
    /// Document samples per second while playing, 0 when stopped.
    pub rate: f64,
    /// Output peak since the previous frame (post-rack).
    pub out_peak_dbfs: f32,
    /// Output RMS over the audio since the previous frame.
    pub out_rms_dbfs: f32,
    /// Input peak (no input yet: `-inf`).
    pub in_peak_dbfs: f32,
    /// Input RMS (no input yet: `-inf`).
    pub in_rms_dbfs: f32,
    /// `audio_rev` of the playing document (0: none).
    pub audio_rev: u64,
    /// RT events dropped so far (full rings).
    pub dropped_rt_events: u32,
}

impl TelemetryFrame {
    /// Little-endian `VXTM` v1 encoding (ADR-003 §2).
    pub fn encode(&self) -> [u8; VXTM_LEN] {
        let mut b = [0u8; VXTM_LEN];
        b[0..4].copy_from_slice(b"VXTM");
        b[4..6].copy_from_slice(&1u16.to_le_bytes());
        b[6..8].copy_from_slice(&(VXTM_LEN as u16).to_le_bytes());
        b[8..12].copy_from_slice(&self.seq.to_le_bytes());
        b[12..16].copy_from_slice(&self.flags.to_le_bytes());
        b[16..24].copy_from_slice(&self.playhead_sample.to_le_bytes());
        b[24..32].copy_from_slice(&self.playhead_time_ns.to_le_bytes());
        b[32..40].copy_from_slice(&self.rate.to_le_bytes());
        b[40..44].copy_from_slice(&self.out_peak_dbfs.to_le_bytes());
        b[44..48].copy_from_slice(&self.out_rms_dbfs.to_le_bytes());
        b[48..52].copy_from_slice(&self.in_peak_dbfs.to_le_bytes());
        b[52..56].copy_from_slice(&self.in_rms_dbfs.to_le_bytes());
        b[56..64].copy_from_slice(&self.audio_rev.to_le_bytes());
        b[64..68].copy_from_slice(&self.dropped_rt_events.to_le_bytes());
        b
    }
}

/// Receives telemetry frames on the control thread (must not block for long).
pub type TelemetrySink = Box<dyn FnMut(&TelemetryFrame) + Send>;

/// Linear amplitude → dBFS (`-inf` for 0).
pub fn to_dbfs(x: f64) -> f32 {
    if x > 0.0 {
        (20.0 * x.log10()) as f32
    } else {
        f32::NEG_INFINITY
    }
}

/// Output meter aggregation between two frames.
#[derive(Debug, Default)]
pub(crate) struct Meter {
    peak: f32,
    sum_sq: f64,
    frames: u64,
}

impl Meter {
    pub(crate) fn add(&mut self, peak: f32, sum_sq: f64, frames: u32) {
        self.peak = self.peak.max(peak);
        self.sum_sq += sum_sq;
        self.frames += u64::from(frames);
    }

    /// `(peak, peak dBFS, rms dBFS)` since the last call.
    pub(crate) fn take(&mut self) -> (f32, f32, f32) {
        let peak = self.peak;
        let rms = if self.frames > 0 {
            (self.sum_sq / self.frames as f64).sqrt()
        } else {
            0.0
        };
        *self = Self::default();
        (peak, to_dbfs(f64::from(peak)), to_dbfs(rms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vxtm_layout() {
        let f = TelemetryFrame {
            seq: 7,
            flags: vxtm_flags::PLAYING | vxtm_flags::XRUN,
            playhead_sample: 48_000,
            playhead_time_ns: 1_000_000_000,
            rate: 44_100.0,
            out_peak_dbfs: -6.0,
            out_rms_dbfs: f32::NEG_INFINITY,
            in_peak_dbfs: f32::NEG_INFINITY,
            in_rms_dbfs: f32::NEG_INFINITY,
            audio_rev: 3,
            dropped_rt_events: 2,
        };
        let b = f.encode();
        assert_eq!(&b[0..4], b"VXTM");
        assert_eq!(u16::from_le_bytes([b[6], b[7]]), 72);
        assert_eq!(u32::from_le_bytes(b[12..16].try_into().unwrap()), 0b1_0001);
        assert_eq!(u64::from_le_bytes(b[16..24].try_into().unwrap()), 48_000);
        assert_eq!(
            f64::from_le_bytes(b[32..40].try_into().unwrap()).to_bits(),
            44_100.0f64.to_bits()
        );
        assert!(f32::from_le_bytes(b[44..48].try_into().unwrap()).is_infinite());
        assert_eq!(u64::from_le_bytes(b[56..64].try_into().unwrap()), 3);
        assert_eq!(&b[68..72], &[0, 0, 0, 0]);
    }

    #[test]
    fn meter_peak_and_rms() {
        let mut m = Meter::default();
        m.add(0.5, 0.25 * 100.0, 100);
        m.add(0.25, 0.0, 100);
        let (peak, peak_db, rms_db) = m.take();
        assert_eq!(peak.to_bits(), 0.5f32.to_bits());
        assert!((peak_db + 6.0206).abs() < 1e-3);
        assert!((rms_db - to_dbfs(0.125f64.sqrt())).abs() < 1e-4);
        assert!(m.take().1.is_infinite(), "reset after take");
    }
}
