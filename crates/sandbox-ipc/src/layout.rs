//! The versioned shared-memory layout (ADR-008 Amendment 1 §1).
//!
//! One segment per plugin instance:
//!
//! | Offset | Size | Content |
//! |---|---|---|
//! | 0 | 512 | [`ControlBlock`]: header, cursors, doorbells, liveness, telemetry (8 cache lines) |
//! | 512 | `C × 4` | input sample ring (host → plugin), `f32` bits |
//! | `512 + 4C` | `C × 4` | output sample ring (plugin → host), `f32` bits |
//! | `512 + 8C` | `E × 32` | event ring host → plugin ([`EventSlot`]) |
//! | `512 + 8C + 32E` | `E × 32` | event ring plugin → host |
//!
//! `C` = [`Layout::ring_capacity`] (`next_pow2(8 × max_block)`, at least 64), `E` =
//! [`Layout::event_capacity`] (a power of two). The total is rounded up to 4096 bytes.
//!
//! The rings are indexed by **absolute stream position** (`u64` samples, masked by `C − 1`),
//! not by block: the host's latency is exactly `latency_samples` for any callback size.
//!
//! Native byte order (both processes run on the same machine). Every word is an atomic, so a
//! misbehaving peer can at worst produce wrong numbers, never undefined behaviour in the other
//! process. A layout change bumps [`LAYOUT_VERSION`]; `tests/layout.rs` freezes the offsets.

use std::mem::{offset_of, size_of};
use std::sync::atomic::{AtomicU32, AtomicU64};

use crate::wakeup::Doorbell;

/// `b"PVSBXIPC"` read as a little-endian `u64`.
pub const MAGIC: u64 = u64::from_le_bytes(*b"PVSBXIPC");
/// Bumped on any change to the layout or to the meaning of a field.
pub const LAYOUT_VERSION: u32 = 1;
/// Size of the [`ControlBlock`] at offset 0.
pub const CONTROL_SIZE: usize = 512;
/// Size of one [`EventSlot`].
pub const EVENT_SLOT_SIZE: usize = 32;
/// Plugin-written telemetry cells (`f32` bits), polled by the control side.
pub const TELEMETRY_CELLS: usize = 16;
/// Ring capacity in blocks: `C = next_pow2(RING_BLOCKS × max_block)`.
pub const RING_BLOCKS: u32 = 8;
/// Smallest sample ring.
pub const MIN_RING_CAPACITY: u32 = 64;
/// Largest `max_block` a segment accepts.
pub const MAX_BLOCK_LIMIT: u32 = 8192;
/// Largest event ring (per direction).
pub const MAX_EVENT_CAPACITY: u32 = 65_536;
/// Segment sizes are rounded up to this.
pub const REGION_ALIGN: usize = 4096;

/// `host_command`: keep running.
pub const CMD_RUN: u32 = 0;
/// `host_command`: the plugin should stop serving and mark itself [`PeerState::Stopped`].
pub const CMD_SHUTDOWN: u32 = 1;

/// The plugin side's lifecycle word (`ControlBlock::state`, written by the plugin).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
pub enum PeerState {
    /// Initialised by the host; no plugin attached yet.
    Created = 0,
    /// A plugin attached and serves blocks.
    Running = 1,
    /// The plugin stopped cleanly.
    Stopped = 2,
    /// The plugin reported a fatal error.
    Failed = 3,
}

impl PeerState {
    /// Decodes a state word; unknown values read as [`PeerState::Failed`].
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Created,
            1 => Self::Running,
            2 => Self::Stopped,
            _ => Self::Failed,
        }
    }

    /// Stopped or failed: the peer will not produce output any more.
    pub fn is_done(self) -> bool {
        matches!(self, Self::Stopped | Self::Failed)
    }
}

/// The control block at offset 0 (8 cache lines, 512 bytes). Field groups sit on their own
/// cache lines by writer, so the two processes never false-share a line they both write.
///
/// Ownership: line 0 is written by the host at creation and read-only afterwards; line 1 is
/// host-written; lines 2, 5 and 6 are plugin-written; each doorbell is rung by one side and
/// waited on by the other.
#[repr(C, align(64))]
pub struct ControlBlock {
    // --- line 0: header (host, at creation) -------------------------------------------------
    /// [`MAGIC`].
    pub magic: AtomicU64,
    /// [`LAYOUT_VERSION`].
    pub version: AtomicU32,
    /// [`CONTROL_SIZE`].
    pub control_size: AtomicU32,
    /// Segment size in bytes ([`Layout::total_size`]).
    pub total_size: AtomicU64,
    /// Sample rate, `f64` bits.
    pub sample_rate_bits: AtomicU64,
    /// Largest chunk either side moves at once (the ADR-008 `B`).
    pub max_block: AtomicU32,
    /// `C`, samples, a power of two.
    pub ring_capacity: AtomicU32,
    /// `E`, events per direction, a power of two.
    pub event_capacity: AtomicU32,
    /// The host's output delay `L` in samples (`0 ≤ L ≤ max_block`).
    pub latency_samples: AtomicU32,
    /// First input position (= `latency_samples`, so output positions start at 0).
    pub start_pos: AtomicU64,
    /// The host's process id.
    pub host_pid: AtomicU32,
    /// Reserved, 0.
    pub flags: AtomicU32,
    // --- line 1: host-written cursors ---------------------------------------------------------
    /// Input published up to this position (exclusive).
    pub in_write_pos: AtomicU64,
    /// Host → plugin events published (count).
    pub ev_in_write: AtomicU64,
    /// Plugin → host events consumed (count).
    pub ev_out_read: AtomicU64,
    /// [`CMD_RUN`] or [`CMD_SHUTDOWN`].
    pub host_command: AtomicU32,
    reserved1: [AtomicU32; 9],
    // --- line 2: plugin-written cursors -------------------------------------------------------
    /// Input consumed up to this position.
    pub in_read_pos: AtomicU64,
    /// Output published up to this position (exclusive).
    pub out_write_pos: AtomicU64,
    /// Output below this position is invalid (set when the plugin resynchronises after an
    /// input overrun).
    pub out_valid_from: AtomicU64,
    /// Host → plugin events consumed (count).
    pub ev_in_read: AtomicU64,
    /// Plugin → host events published (count).
    pub ev_out_write: AtomicU64,
    /// Input overruns (the plugin fell more than `C − max_block` behind and skipped ahead).
    pub overruns: AtomicU64,
    /// Chunks processed.
    pub chunks: AtomicU64,
    reserved2: AtomicU64,
    // --- lines 3, 4: doorbells ----------------------------------------------------------------
    /// Rung by the host after publishing input (or a command); the plugin waits on it.
    pub to_plugin: Doorbell,
    /// Rung by the plugin after publishing output; the host waits on it (bounded).
    pub to_host: Doorbell,
    // --- line 5: liveness (plugin-written) ----------------------------------------------------
    /// [`PeerState`] as `u32`.
    pub state: AtomicU32,
    /// The plugin process id (0 until attached).
    pub plugin_pid: AtomicU32,
    /// Bumped on every processed chunk and every idle wake-up; a stale value means a hang.
    pub heartbeat: AtomicU64,
    /// 1 while the plugin's processing callback runs ("currently in plugin call").
    pub in_call: AtomicU32,
    reserved5: [AtomicU32; 11],
    // --- line 6: telemetry (plugin-written) ---------------------------------------------------
    /// `f32` bits, meaning defined by the plugin adapter (ADR-006 §5 telemetry cache).
    pub telemetry: [AtomicU32; TELEMETRY_CELLS],
    // --- line 7: reserved ---------------------------------------------------------------------
    reserved7: [AtomicU64; 8],
}

const _: () = assert!(size_of::<ControlBlock>() == CONTROL_SIZE);
const _: () = assert!(offset_of!(ControlBlock, in_write_pos) == 64);
const _: () = assert!(offset_of!(ControlBlock, in_read_pos) == 128);
const _: () = assert!(offset_of!(ControlBlock, to_plugin) == 192);
const _: () = assert!(offset_of!(ControlBlock, to_host) == 256);
const _: () = assert!(offset_of!(ControlBlock, state) == 320);
const _: () = assert!(offset_of!(ControlBlock, telemetry) == 384);
const _: () = assert!(offset_of!(ControlBlock, reserved7) == 448);

/// One event (32 bytes). Written by the producer before it publishes the ring's write count.
#[repr(C)]
pub struct EventSlot {
    /// Absolute stream position (same axis as the sample rings).
    pub pos: AtomicU64,
    /// Parameter id (ADR-005 `ParamId`).
    pub id: AtomicU32,
    /// [`EventKind`] as `u32`.
    pub kind: AtomicU32,
    /// Value, `f64` bits.
    pub value_bits: AtomicU64,
    reserved: AtomicU64,
}

const _: () = assert!(size_of::<EventSlot>() == EVENT_SLOT_SIZE);

/// What an event means. Unknown kinds are passed through (forward compatible).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EventKind(pub u32);

impl EventKind {
    /// A plain parameter value (ADR-005 `ParamEvent`).
    pub const PARAM_VALUE: Self = Self(1);
    /// The user started dragging a parameter (plugin → host).
    pub const GESTURE_BEGIN: Self = Self(2);
    /// The user released a parameter (plugin → host).
    pub const GESTURE_END: Self = Self(3);
    /// Host → plugin (T-802): the module API's `reset()` (seek / loop wrap / transport start) —
    /// clear delay lines and envelopes before processing the chunk. `id` and `value` are
    /// unused. [`PluginEnd::service`](crate::PluginEnd::service) ends a chunk at `pos`, so the
    /// reset always belongs to the chunk that *starts* there (H-55): a backend applies it at
    /// the chunk's first sample and the render stays independent of how far ahead the host
    /// published.
    pub const RESET: Self = Self(4);
    /// Plugin → host (T-803): the plugin asked to be restarted (CLAP `request_restart`, e.g.
    /// its latency changed). The proxy raises `HostRequest::Restart` (ADR-008 §2); `pos`, `id`
    /// and `value` are unused.
    pub const RESTART_REQUEST: Self = Self(5);
}

/// An event as carried by the rings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WireEvent {
    /// Absolute stream position.
    pub pos: u64,
    /// Kind.
    pub kind: EventKind,
    /// Parameter id.
    pub id: u32,
    /// Plain value.
    pub value: f64,
}

impl WireEvent {
    /// A parameter value event.
    pub fn param(pos: u64, id: u32, value: f64) -> Self {
        Self {
            pos,
            kind: EventKind::PARAM_VALUE,
            id,
            value,
        }
    }

    pub(crate) fn store(&self, slot: &EventSlot) {
        use std::sync::atomic::Ordering::Relaxed;
        slot.pos.store(self.pos, Relaxed);
        slot.id.store(self.id, Relaxed);
        slot.kind.store(self.kind.0, Relaxed);
        slot.value_bits.store(self.value.to_bits(), Relaxed);
    }

    pub(crate) fn load(slot: &EventSlot) -> Self {
        use std::sync::atomic::Ordering::Relaxed;
        Self {
            pos: slot.pos.load(Relaxed),
            kind: EventKind(slot.kind.load(Relaxed)),
            id: slot.id.load(Relaxed),
            value: f64::from_bits(slot.value_bits.load(Relaxed)),
        }
    }
}

/// Error computing a [`Layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// `max_block` is 0 or above [`MAX_BLOCK_LIMIT`].
    #[error("max_block must be 1..={MAX_BLOCK_LIMIT}, got {0}")]
    MaxBlock(u32),
    /// `event_capacity` is not a power of two in `2..=`[`MAX_EVENT_CAPACITY`].
    #[error("event_capacity must be a power of two in 2..={MAX_EVENT_CAPACITY}, got {0}")]
    EventCapacity(u32),
}

/// Byte offsets and sizes of one segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    /// The ADR-008 `B`.
    pub max_block: u32,
    /// `C`, samples.
    pub ring_capacity: u32,
    /// `E`, events per direction.
    pub event_capacity: u32,
    /// Input sample ring.
    pub input_offset: usize,
    /// Output sample ring.
    pub output_offset: usize,
    /// Host → plugin event ring.
    pub events_in_offset: usize,
    /// Plugin → host event ring.
    pub events_out_offset: usize,
    /// Segment size (a multiple of [`REGION_ALIGN`]).
    pub total_size: usize,
}

impl Layout {
    /// The layout for `max_block` and `event_capacity`.
    pub fn new(max_block: u32, event_capacity: u32) -> Result<Self, LayoutError> {
        if max_block == 0 || max_block > MAX_BLOCK_LIMIT {
            return Err(LayoutError::MaxBlock(max_block));
        }
        if !(2..=MAX_EVENT_CAPACITY).contains(&event_capacity) || !event_capacity.is_power_of_two()
        {
            return Err(LayoutError::EventCapacity(event_capacity));
        }
        let ring_capacity = (max_block * RING_BLOCKS)
            .next_power_of_two()
            .max(MIN_RING_CAPACITY);
        let ring_bytes = ring_capacity as usize * size_of::<u32>();
        let event_bytes = event_capacity as usize * EVENT_SLOT_SIZE;
        let input_offset = CONTROL_SIZE;
        let output_offset = input_offset + ring_bytes;
        let events_in_offset = output_offset + ring_bytes;
        let events_out_offset = events_in_offset + event_bytes;
        let end = events_out_offset + event_bytes;
        Ok(Self {
            max_block,
            ring_capacity,
            event_capacity,
            input_offset,
            output_offset,
            events_in_offset,
            events_out_offset,
            total_size: end.next_multiple_of(REGION_ALIGN),
        })
    }
}
