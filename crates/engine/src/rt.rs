//! Types that cross the real-time boundary (ADR-002 §2–§5). All `Copy`, carried by `rtrb` rings
//! that are allocated when the output stream is built.

use std::sync::atomic::AtomicU32;

use crate::monitor::MonitorTap;

/// Samples per playback packet (ADR-002 §5).
pub(crate) const PACKET_FRAMES: usize = 256;
/// Playback ring capacity in packets (ADR-002 §2).
pub(crate) const PLAYBACK_RING_PACKETS: usize = 64;
/// Control → audio command ring capacity (ADR-002 §2).
pub(crate) const AUDIO_CMD_CAPACITY: usize = 1024;
/// Audio → control event ring capacity (ADR-002 §2).
pub(crate) const RT_EVENT_CAPACITY: usize = 1024;
/// Commands the output callback drains per call at most (ADR-002 §3).
pub(crate) const MAX_AUDIO_CMDS_PER_CALLBACK: usize = 256;
/// Fade on start/stop/seek (SPEC-003 §2.1, ADR-002 §5).
pub(crate) const FADE_MS: f64 = 5.0;
/// Current-epoch audio buffered before playback starts audibly (ADR-002 §5).
pub(crate) const PREBUFFER_MS: f64 = 20.0;
/// Reader read-ahead (ADR-002 §1).
pub(crate) const READ_AHEAD_MS: u64 = 200;

/// [`Packet::flags`] bits.
pub(crate) mod packet_flags {
    /// The document ends here; the packet carries no samples.
    pub(crate) const END: u16 = 1 << 0;
}

/// One playback-ring element (ADR-002 §5): `len` device-rate samples whose first sample is
/// document position `doc_pos`, tagged with the transport epoch that requested them.
#[derive(Clone, Copy)]
pub(crate) struct Packet {
    pub(crate) epoch: u32,
    pub(crate) flags: u16,
    pub(crate) len: u16,
    pub(crate) doc_pos: u64,
    pub(crate) samples: [f32; PACKET_FRAMES],
}

impl Packet {
    pub(crate) fn new(epoch: u32) -> Self {
        Self {
            epoch,
            flags: 0,
            len: 0,
            doc_pos: 0,
            samples: [0.0; PACKET_FRAMES],
        }
    }
}

/// Control → output callback (ADR-002 §3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AudioCmd {
    /// Start playing `epoch` from `pos` once the prebuffer is queued; `reset` the rack first
    /// (not on a resume from pause, SPEC-003 §4).
    Play { epoch: u32, pos: u64, reset: bool },
    /// Fade out, drop the old epoch, reset the rack, fade in at `pos`.
    Seek { epoch: u32, pos: u64 },
    /// Fade out and go idle (reports [`RtEvent::Stopped`]).
    Stop,
    /// Monitoring (T-107, SPEC-002 §2.7, ADR-002 §6): the tap (≤ 10 ms fades), the linked input
    /// rate (0: unlinked), the servo target F* in input frames, and whether the input is ending
    /// (disarm/close: the fade-out then fits what the ring still holds).
    Monitor {
        tap: MonitorTap,
        in_rate_hz: u32,
        target_frames: u32,
        ending: bool,
    },
    /// T-304 (SPEC-022 §2.14, §4.7): start (`true`) or abort the calibration run — the
    /// preallocated sweep, 5 repetitions at 1.6 s spacing, mixed in after the rack.
    Calibrate { start: bool },
}

/// Output callback → control (ADR-002 §4 step 5, §7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RtEvent {
    /// One per callback. `heard_pos`: heard document position of the first frame (`None` when
    /// not playing); `heard_time_ns`: app-clock time it is heard (cpal `playback` instant);
    /// `latency_ns`: `playback − callback` (T-107: the monitoring latency readout).
    Block {
        epoch: u32,
        heard_pos: Option<u64>,
        heard_time_ns: u64,
        latency_ns: u32,
        frames: u32,
        peak: f32,
        sum_sq: f64,
    },
    /// A stop fade finished; `pos` = the next document position that would have played.
    Stopped { epoch: u32, pos: u64 },
    /// The document end was played.
    Ended { epoch: u32, pos: u64 },
    /// T-304 (SPEC-022 §4.7): calibration repetition `rep` starts being heard at app time
    /// `heard_time_ns` (reported like `WindowHeard`).
    CalibRep { rep: u32, heard_time_ns: u64 },
    /// T-304: the calibration run's last repetition was played (or it was aborted).
    CalibDone,
}

/// Counters the callback bumps (atomics only).
#[derive(Debug, Default)]
pub(crate) struct RtCounters {
    pub(crate) underruns: AtomicU32,
    pub(crate) dropped_events: AtomicU32,
    /// T-107: monitor-ring underruns (fade out, re-prime; SPEC-002 §2.7 monitor dropouts).
    pub(crate) mon_underruns: AtomicU32,
    /// T-107: monitor-ring overruns (drop back to F* with a crossfade, ADR-002 §6).
    pub(crate) mon_overruns: AtomicU32,
}
