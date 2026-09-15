//! The sandbox side of a channel.

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{Ordering, fence};
use std::time::Duration;

use crate::channel::{ChannelConfig, ChannelError, Mapped, read_header};
use crate::layout::{CMD_SHUTDOWN, ControlBlock, PeerState, RING_BLOCKS, WireEvent};
use crate::shm::SharedRegion;
use crate::wakeup::{Deadline, PlatformWakeup, Wakeup};

/// Upper bound of chunks one [`PluginEnd::service`] call processes (it keeps catching up with
/// input that arrives meanwhile, but never loops unboundedly).
pub const MAX_CHUNKS_PER_SERVICE: u32 = 2 * RING_BLOCKS;

/// Queues `event` on the plugin → host ring at `*write` (`false`, dropped, when full).
fn push_out(
    cb: &ControlBlock,
    ring: &[crate::layout::EventSlot],
    write: &mut u64,
    mask: u64,
    capacity: u64,
    event: WireEvent,
) -> bool {
    let read = cb.ev_out_read.load(Ordering::Acquire);
    if write.wrapping_sub(read) >= capacity {
        return false;
    }
    event.store(&ring[(*write & mask) as usize]);
    *write += 1;
    cb.ev_out_write.store(*write, Ordering::Release);
    true
}

/// One chunk to process: `input.len() == output.len() ≤ max_block` frames at stream position
/// `pos`.
pub struct Chunk<'a> {
    /// Stream position of `input[0]`.
    pub pos: u64,
    /// Input samples.
    pub input: &'a [f32],
    /// Output samples to fill (initially the previous chunk's contents).
    pub output: &'a mut [f32],
    /// Events with positions before the chunk's end, in order (late ones included; see
    /// [`Chunk::offset`]).
    pub events: &'a [WireEvent],
}

impl Chunk<'_> {
    /// The sample offset of `event` within this chunk (late events clamp to 0).
    pub fn offset(&self, event: &WireEvent) -> usize {
        let off = usize::try_from(event.pos.saturating_sub(self.pos)).unwrap_or(usize::MAX);
        off.min(self.input.len().saturating_sub(1))
    }
}

/// Result of one [`PluginEnd::service`] call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Serviced {
    /// No input within the timeout (the heartbeat was still bumped).
    Idle,
    /// Chunks processed (0 after a pure resynchronisation).
    Processed {
        /// Chunks.
        chunks: u32,
        /// Frames.
        frames: u64,
    },
    /// The host requested shutdown: call [`PluginEnd::stop`] and exit.
    Shutdown,
}

/// The sandbox's end: attach to a segment, then call [`service`](Self::service) in a loop on
/// the sandbox audio thread (with a timeout well below the host's hang timeout, e.g. 20 ms, so
/// the heartbeat keeps moving while idle).
pub struct PluginEnd<W: Wakeup = PlatformWakeup> {
    map: Mapped,
    config: ChannelConfig,
    mask: u64,
    /// The plugin may lag the host by at most `C − max_block` samples (overrun beyond).
    window: u64,
    event_mask: u64,
    event_capacity: u64,
    read_pos: u64,
    ev_in_read: u64,
    ev_out_write: u64,
    in_buf: Box<[f32]>,
    out_buf: Box<[f32]>,
    events: Vec<WireEvent>,
    _wakeup: PhantomData<fn() -> W>,
}

fn resync(cb: &ControlBlock, to: u64) {
    // `out_valid_from` first: a host that sees the new `out_write_pos` sees it too.
    cb.out_valid_from.store(to, Ordering::Release);
    cb.out_write_pos.store(to, Ordering::Release);
    cb.in_read_pos.store(to, Ordering::Release);
    cb.overruns.fetch_add(1, Ordering::Relaxed);
}

impl<W: Wakeup> PluginEnd<W> {
    /// Validates the segment's header, marks the plugin [`PeerState::Running`] and returns the
    /// end. Allocates its scratch buffers here (never in `service`).
    pub fn attach(region: SharedRegion) -> Result<Self, ChannelError> {
        let (config, layout) = read_header(&region)?;
        let map = Mapped::new(Arc::new(region), layout)?;
        // Touch every page of the rings now rather than on the first audio chunk.
        for w in map.input().iter().chain(map.output()).step_by(1024) {
            w.load(Ordering::Relaxed);
        }
        let cb = map.control();
        let read_pos = cb.in_read_pos.load(Ordering::Acquire);
        let ev_in_read = cb.ev_in_read.load(Ordering::Acquire);
        let ev_out_write = cb.ev_out_write.load(Ordering::Acquire);
        cb.plugin_pid.store(std::process::id(), Ordering::Relaxed);
        cb.heartbeat.fetch_add(1, Ordering::Release);
        cb.state.store(PeerState::Running as u32, Ordering::Release);
        cb.to_host.ring::<W>();
        let cap = u64::from(layout.ring_capacity);
        let events = u64::from(layout.event_capacity);
        let max_block = config.max_block as usize;
        Ok(Self {
            config,
            mask: cap - 1,
            window: cap - u64::from(config.max_block),
            event_mask: events - 1,
            event_capacity: events,
            read_pos,
            ev_in_read,
            ev_out_write,
            in_buf: vec![0.0; max_block].into_boxed_slice(),
            out_buf: vec![0.0; max_block].into_boxed_slice(),
            events: Vec::with_capacity(layout.event_capacity as usize),
            map,
            _wakeup: PhantomData,
        })
    }

    /// The channel configuration from the header.
    pub fn config(&self) -> ChannelConfig {
        self.config
    }

    /// The host's process id (from the header).
    pub fn host_pid(&self) -> u32 {
        self.map.control().host_pid.load(Ordering::Relaxed)
    }

    /// Input overruns so far (resynchronisations).
    pub fn overruns(&self) -> u64 {
        self.map.control().overruns.load(Ordering::Relaxed)
    }

    /// Waits up to `timeout` for input (if none is pending), then processes every published
    /// chunk (at most [`MAX_CHUNKS_PER_SERVICE`]) through `process`, publishing each chunk's
    /// output and ringing the host. Bumps the heartbeat on every call, wake-up and chunk.
    /// Allocation-free. A plugin with output events uses [`Self::service_with_events`].
    pub fn service<F: FnMut(Chunk<'_>)>(&mut self, timeout: Duration, mut process: F) -> Serviced {
        let mut none = Vec::new();
        self.service_with_events(timeout, &mut none, |chunk, _| process(chunk))
    }

    /// [`Self::service`] for a plugin that reports events (H-36): `process` also gets
    /// `out_events` (cleared before each chunk; never grown beyond its capacity — no
    /// allocation), and each chunk's events are queued on the plugin → host event ring
    /// **before** that chunk's output is published. So the host sees a chunk's events no later
    /// than its audio, however many chunks one call processes — in an offline render the host
    /// keeps publishing input, and one call can span the whole stream. Events that don't fit in
    /// the ring are dropped (like [`Self::push_output_event`]'s `false`).
    pub fn service_with_events<F: FnMut(Chunk<'_>, &mut Vec<WireEvent>)>(
        &mut self,
        timeout: Duration,
        out_events: &mut Vec<WireEvent>,
        mut process: F,
    ) -> Serviced {
        let cb = self.map.control();
        cb.heartbeat.fetch_add(1, Ordering::Release);
        let shutdown = || cb.host_command.load(Ordering::Acquire) == CMD_SHUTDOWN;
        if shutdown() {
            return Serviced::Shutdown;
        }
        let mut w = cb.in_write_pos.load(Ordering::Acquire);
        if w == self.read_pos {
            let read_pos = self.read_pos;
            let woke = cb.to_plugin.wait_until::<W>(Deadline::after(timeout), || {
                cb.in_write_pos.load(Ordering::Acquire) != read_pos || shutdown()
            });
            cb.heartbeat.fetch_add(1, Ordering::Release);
            if shutdown() {
                return Serviced::Shutdown;
            }
            if !woke {
                return Serviced::Idle;
            }
            w = cb.in_write_pos.load(Ordering::Acquire);
        }

        let ring_in = self.map.input();
        let ring_out = self.map.output();
        let ev_ring = self.map.events_in();
        let max_block = u64::from(self.config.max_block);
        let (mut chunks, mut frames) = (0u32, 0u64);
        for _ in 0..MAX_CHUNKS_PER_SERVICE {
            if w == self.read_pos {
                break;
            }
            // Too far behind (or a cursor went backwards): skip to the present.
            if w.wrapping_sub(self.read_pos) > self.window {
                resync(cb, w);
                self.read_pos = w;
                continue;
            }
            let start = self.read_pos;
            let end = w.min(start + max_block);
            let n = (end - start) as usize;
            for (i, x) in self.in_buf[..n].iter_mut().enumerate() {
                let idx = ((start + i as u64) & self.mask) as usize;
                *x = f32::from_bits(ring_in[idx].load(Ordering::Relaxed));
            }
            // Seqlock reader side: if the host overwrote any slot we just read, the re-read
            // cursor shows it (the host writes at most `max_block` beyond what it published).
            fence(Ordering::Acquire);
            let w2 = cb.in_write_pos.load(Ordering::Relaxed);
            if w2.wrapping_sub(start) > self.window {
                resync(cb, w2);
                self.read_pos = w2;
                w = w2;
                continue;
            }

            self.events.clear();
            let ev_w = cb.ev_in_write.load(Ordering::Acquire);
            if ev_w.wrapping_sub(self.ev_in_read) > self.event_capacity {
                self.ev_in_read = ev_w;
            }
            while self.ev_in_read != ev_w && self.events.len() < self.events.capacity() {
                let ev = WireEvent::load(&ev_ring[(self.ev_in_read & self.event_mask) as usize]);
                if ev.pos >= end {
                    break;
                }
                self.events.push(ev);
                self.ev_in_read += 1;
            }
            cb.ev_in_read.store(self.ev_in_read, Ordering::Release);

            cb.in_call.store(1, Ordering::Release);
            out_events.clear();
            process(
                Chunk {
                    pos: start,
                    input: &self.in_buf[..n],
                    output: &mut self.out_buf[..n],
                    events: &self.events,
                },
                out_events,
            );
            cb.in_call.store(0, Ordering::Release);
            // The chunk's events first, then its output (the host pops events after it sees
            // the output).
            for e in out_events.drain(..) {
                push_out(
                    cb,
                    self.map.events_out(),
                    &mut self.ev_out_write,
                    self.event_mask,
                    self.event_capacity,
                    e,
                );
            }

            for (i, &y) in self.out_buf[..n].iter().enumerate() {
                let idx = ((start + i as u64) & self.mask) as usize;
                ring_out[idx].store(y.to_bits(), Ordering::Relaxed);
            }
            cb.out_write_pos.store(end, Ordering::Release);
            cb.in_read_pos.store(end, Ordering::Release);
            cb.chunks.fetch_add(1, Ordering::Relaxed);
            cb.heartbeat.fetch_add(1, Ordering::Release);
            cb.to_host.ring::<W>();
            self.read_pos = end;
            chunks += 1;
            frames += n as u64;
            w = cb.in_write_pos.load(Ordering::Acquire);
        }
        Serviced::Processed { chunks, frames }
    }

    /// Queues a plugin → host event (e.g. a parameter the plugin's GUI changed). `false` if the
    /// ring is full.
    pub fn push_output_event(&mut self, event: WireEvent) -> bool {
        push_out(
            self.map.control(),
            self.map.events_out(),
            &mut self.ev_out_write,
            self.event_mask,
            self.event_capacity,
            event,
        )
    }

    /// Writes telemetry cell `index` (ignored if out of range).
    pub fn set_telemetry(&self, index: usize, value: f32) {
        if let Some(cell) = self.map.control().telemetry.get(index) {
            cell.store(value.to_bits(), Ordering::Relaxed);
        }
    }

    /// Marks the plugin cleanly [`PeerState::Stopped`] (before exiting).
    pub fn stop(&self) {
        self.set_state(PeerState::Stopped);
    }

    /// Marks the plugin [`PeerState::Failed`] (a fatal error it could report).
    pub fn fail(&self) {
        self.set_state(PeerState::Failed);
    }

    fn set_state(&self, state: PeerState) {
        let cb = self.map.control();
        cb.state.store(state as u32, Ordering::Release);
        cb.to_host.ring::<W>();
    }
}
