//! Segment creation, header validation and the typed views over a mapping.

use std::io;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use crate::host::{HostEnd, HostOptions, HostStats};
use crate::layout::{
    CMD_RUN, CONTROL_SIZE, ControlBlock, EventSlot, LAYOUT_VERSION, Layout, LayoutError, MAGIC,
    PeerState,
};
use crate::monitor::Monitor;
use crate::shm::SharedRegion;
use crate::wakeup::{PlatformWakeup, Wakeup};

/// What a segment is sized and initialised for (written into its header).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelConfig {
    /// Sample rate in Hz.
    pub sample_rate: f64,
    /// Largest chunk either side moves at once (ADR-008 `B`; the rack's `max_block` for the
    /// sandboxed slot). Host calls with more frames are split.
    pub max_block: u32,
    /// Output delay `L` in samples, `0..=max_block`. `max_block` = the ADR-008 pipelined
    /// realtime mode (the plugin has a whole period); `0` = synchronous round trip inside the
    /// callback (the plugin must answer within the host's wait budget).
    pub latency_samples: u32,
    /// Events per direction (a power of two).
    pub event_capacity: u32,
}

impl ChannelConfig {
    /// Default event ring capacity (ADR-002 per-slot event capacity).
    pub const DEFAULT_EVENT_CAPACITY: u32 = 512;

    /// ADR-008 realtime mode: output delayed by exactly `max_block` samples.
    pub fn pipelined(sample_rate: f64, max_block: u32) -> Self {
        Self {
            sample_rate,
            max_block,
            latency_samples: max_block,
            event_capacity: Self::DEFAULT_EVENT_CAPACITY,
        }
    }

    /// Zero added latency: the host waits (bounded) for this block's own output.
    pub fn synchronous(sample_rate: f64, max_block: u32) -> Self {
        Self {
            latency_samples: 0,
            ..Self::pipelined(sample_rate, max_block)
        }
    }

    /// Validates and computes the segment layout.
    pub fn layout(&self) -> Result<Layout, ChannelError> {
        if !(self.sample_rate.is_finite() && self.sample_rate > 0.0) {
            return Err(ChannelError::Config(
                "sample_rate must be finite and positive",
            ));
        }
        if self.latency_samples > self.max_block {
            return Err(ChannelError::Config("latency_samples must be <= max_block"));
        }
        Ok(Layout::new(self.max_block, self.event_capacity)?)
    }
}

/// Error creating, opening or attaching to a segment.
#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    /// The OS refused the shared-memory operation.
    #[error("shared memory: {0}")]
    Io(#[from] io::Error),
    /// Invalid sizes.
    #[error("invalid layout: {0}")]
    Layout(#[from] LayoutError),
    /// Invalid [`ChannelConfig`].
    #[error("invalid channel config: {0}")]
    Config(&'static str),
    /// Not a PowerVoice sandbox segment.
    #[error("not a PowerVoice sandbox segment (bad magic)")]
    BadMagic,
    /// A layout this build doesn't speak.
    #[error("unsupported segment layout version {found} (expected {LAYOUT_VERSION})")]
    Version {
        /// The segment's version.
        found: u32,
    },
    /// The mapping is smaller than its header claims.
    #[error("segment is {len} bytes, the layout needs {needed}")]
    TooSmall {
        /// Mapping size.
        len: usize,
        /// Required size.
        needed: usize,
    },
    /// An inconsistent header field.
    #[error("inconsistent segment header: {0}")]
    Header(&'static str),
}

fn control_of(region: &SharedRegion) -> &ControlBlock {
    assert!(
        region.len() >= CONTROL_SIZE,
        "segment smaller than its control block"
    );
    // SAFETY: the mapping is page-aligned (≥ 64, ControlBlock's alignment), at least
    // CONTROL_SIZE bytes (asserted), and lives as long as `region`. Every ControlBlock field is
    // an atomic (valid for any bit pattern, interior-mutable), so sharing it with another
    // process that writes concurrently is sound.
    unsafe { &*region.as_ptr().cast::<ControlBlock>() }
}

/// Typed views over a validated mapping (cheap to clone: one `Arc`).
#[derive(Clone)]
pub(crate) struct Mapped {
    region: Arc<SharedRegion>,
    layout: Layout,
}

impl Mapped {
    pub(crate) fn new(region: Arc<SharedRegion>, layout: Layout) -> Result<Self, ChannelError> {
        if region.len() < layout.total_size {
            return Err(ChannelError::TooSmall {
                len: region.len(),
                needed: layout.total_size,
            });
        }
        Ok(Self { region, layout })
    }

    pub(crate) fn region(&self) -> &SharedRegion {
        &self.region
    }

    pub(crate) fn control(&self) -> &ControlBlock {
        control_of(&self.region)
    }

    fn words(&self, offset: usize, count: usize) -> &[AtomicU32] {
        // SAFETY: `Mapped::new` checked that the layout fits in the mapping; `offset` is a
        // layout offset (a multiple of 64 from a page-aligned base) and `count` u32 words fit
        // before the next array. AtomicU32 has u32's layout and is valid for any bits.
        unsafe {
            std::slice::from_raw_parts(self.region.as_ptr().add(offset).cast::<AtomicU32>(), count)
        }
    }

    fn slots(&self, offset: usize) -> &[EventSlot] {
        // SAFETY: as `words`; EventSlot is 32 bytes of atomics (align 8), `event_capacity`
        // slots fit at this layout offset.
        unsafe {
            std::slice::from_raw_parts(
                self.region.as_ptr().add(offset).cast::<EventSlot>(),
                self.layout.event_capacity as usize,
            )
        }
    }

    pub(crate) fn input(&self) -> &[AtomicU32] {
        self.words(self.layout.input_offset, self.layout.ring_capacity as usize)
    }

    pub(crate) fn output(&self) -> &[AtomicU32] {
        self.words(
            self.layout.output_offset,
            self.layout.ring_capacity as usize,
        )
    }

    pub(crate) fn events_in(&self) -> &[EventSlot] {
        self.slots(self.layout.events_in_offset)
    }

    pub(crate) fn events_out(&self) -> &[EventSlot] {
        self.slots(self.layout.events_out_offset)
    }
}

/// Reads and validates a segment header (the plugin side never trusts sizes blindly).
pub(crate) fn read_header(region: &SharedRegion) -> Result<(ChannelConfig, Layout), ChannelError> {
    if region.len() < CONTROL_SIZE {
        return Err(ChannelError::TooSmall {
            len: region.len(),
            needed: CONTROL_SIZE,
        });
    }
    let cb = control_of(region);
    if cb.magic.load(Ordering::Acquire) != MAGIC {
        return Err(ChannelError::BadMagic);
    }
    let found = cb.version.load(Ordering::Relaxed);
    if found != LAYOUT_VERSION {
        return Err(ChannelError::Version { found });
    }
    if cb.control_size.load(Ordering::Relaxed) as usize != CONTROL_SIZE {
        return Err(ChannelError::Header("control_size"));
    }
    let config = ChannelConfig {
        sample_rate: f64::from_bits(cb.sample_rate_bits.load(Ordering::Relaxed)),
        max_block: cb.max_block.load(Ordering::Relaxed),
        latency_samples: cb.latency_samples.load(Ordering::Relaxed),
        event_capacity: cb.event_capacity.load(Ordering::Relaxed),
    };
    let layout = config.layout()?;
    if cb.ring_capacity.load(Ordering::Relaxed) != layout.ring_capacity {
        return Err(ChannelError::Header("ring_capacity"));
    }
    if cb.total_size.load(Ordering::Relaxed) != layout.total_size as u64 {
        return Err(ChannelError::Header("total_size"));
    }
    if cb.start_pos.load(Ordering::Relaxed) != u64::from(config.latency_samples) {
        return Err(ChannelError::Header("start_pos"));
    }
    if region.len() < layout.total_size {
        return Err(ChannelError::TooSmall {
            len: region.len(),
            needed: layout.total_size,
        });
    }
    Ok((config, layout))
}

/// A freshly initialised segment, before it is split into [`HostEnd`] + [`Monitor`].
///
/// ```ignore
/// let chan = Channel::create(ChannelConfig::pipelined(48_000.0, 256))?;
/// let handle = chan.share_with(&mut cmd);            // pass `handle` to the sandbox
/// let child = cmd.arg("--shm").arg(handle).spawn()?;
/// let (host, monitor) = chan.into_ends(HostOptions::default());
/// // audio thread: host.process(input, output); control thread: monitor.poll(..)
/// ```
pub struct Channel {
    mapped: Mapped,
    config: ChannelConfig,
}

impl Channel {
    /// Creates a segment with the platform's default backend and initialises it.
    pub fn create(config: ChannelConfig) -> Result<Self, ChannelError> {
        let layout = config.layout()?;
        Self::create_in(SharedRegion::create(layout.total_size)?, config)
    }

    /// Initialises `region` (at least [`Layout::total_size`] bytes, zero-filled) for `config`.
    pub fn create_in(region: SharedRegion, config: ChannelConfig) -> Result<Self, ChannelError> {
        let layout = config.layout()?;
        let mapped = Mapped::new(Arc::new(region), layout)?;
        let cb = mapped.control();
        // Zero the rings: also touches every page here, off the audio thread.
        for w in mapped.input().iter().chain(mapped.output()) {
            w.store(0, Ordering::Relaxed);
        }
        for s in mapped.events_in().iter().chain(mapped.events_out()) {
            s.pos.store(0, Ordering::Relaxed);
        }
        let start = u64::from(config.latency_samples);
        cb.version.store(LAYOUT_VERSION, Ordering::Relaxed);
        cb.control_size
            .store(CONTROL_SIZE as u32, Ordering::Relaxed);
        cb.total_size
            .store(layout.total_size as u64, Ordering::Relaxed);
        cb.sample_rate_bits
            .store(config.sample_rate.to_bits(), Ordering::Relaxed);
        cb.max_block.store(config.max_block, Ordering::Relaxed);
        cb.ring_capacity
            .store(layout.ring_capacity, Ordering::Relaxed);
        cb.event_capacity
            .store(layout.event_capacity, Ordering::Relaxed);
        cb.latency_samples
            .store(config.latency_samples, Ordering::Relaxed);
        cb.start_pos.store(start, Ordering::Relaxed);
        cb.host_pid.store(std::process::id(), Ordering::Relaxed);
        cb.flags.store(0, Ordering::Relaxed);
        cb.in_write_pos.store(start, Ordering::Relaxed);
        cb.ev_in_write.store(0, Ordering::Relaxed);
        cb.ev_out_read.store(0, Ordering::Relaxed);
        cb.host_command.store(CMD_RUN, Ordering::Relaxed);
        cb.in_read_pos.store(start, Ordering::Relaxed);
        // Output positions [0, L) are the latency fill: valid zeros.
        cb.out_write_pos.store(start, Ordering::Relaxed);
        cb.out_valid_from.store(0, Ordering::Relaxed);
        cb.ev_in_read.store(0, Ordering::Relaxed);
        cb.ev_out_write.store(0, Ordering::Relaxed);
        cb.overruns.store(0, Ordering::Relaxed);
        cb.chunks.store(0, Ordering::Relaxed);
        cb.state.store(PeerState::Created as u32, Ordering::Relaxed);
        cb.plugin_pid.store(0, Ordering::Relaxed);
        cb.heartbeat.store(0, Ordering::Relaxed);
        cb.in_call.store(0, Ordering::Relaxed);
        // Last, with Release: a peer that sees the magic sees a complete header.
        cb.magic.store(MAGIC, Ordering::Release);
        Ok(Self { mapped, config })
    }

    /// The segment.
    pub fn region(&self) -> &SharedRegion {
        self.mapped.region()
    }

    /// Prepares `cmd` so its child can open the segment; returns the handle to pass it
    /// ([`SharedRegion::share_with`]).
    pub fn share_with(&self, cmd: &mut Command) -> String {
        self.mapped.region().share_with(cmd)
    }

    /// The configuration.
    pub fn config(&self) -> ChannelConfig {
        self.config
    }

    /// The layout.
    pub fn layout(&self) -> Layout {
        self.mapped.layout
    }

    /// The control block (diagnostics and tests).
    pub fn control(&self) -> &ControlBlock {
        self.mapped.control()
    }

    /// Splits into the audio-thread end and the control-thread monitor, with the platform's
    /// wakeup primitive.
    pub fn into_ends(self, options: HostOptions) -> (HostEnd, Monitor) {
        self.into_ends_with::<PlatformWakeup>(options)
    }

    /// As [`into_ends`](Self::into_ends) with an explicit wakeup primitive.
    pub fn into_ends_with<W: Wakeup>(self, options: HostOptions) -> (HostEnd<W>, Monitor) {
        let stats = Arc::new(HostStats::default());
        let host = HostEnd::new(
            self.mapped.clone(),
            &self.config,
            options,
            Arc::clone(&stats),
        );
        let monitor = Monitor::new(self.mapped, stats, options.hang_timeout, Instant::now());
        (host, monitor)
    }
}
