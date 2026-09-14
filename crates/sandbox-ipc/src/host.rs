//! The host's audio-thread end.

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering, fence};
use std::time::Duration;

use crate::channel::{ChannelConfig, Mapped};
use crate::layout::{ControlBlock, PeerState, WireEvent};
use crate::wakeup::{Deadline, PlatformWakeup, Wakeup};

/// How long [`HostEnd::process`] may wait for output that isn't there yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaitBudget {
    /// A fraction of the current block's period (`frames / sample_rate`). Realtime.
    FractionOfBlock(f32),
    /// A fixed timeout. Offline renders (ADR-008 §2: block until ready, with the hang timeout)
    /// and measurements.
    Fixed(Duration),
}

/// Host-local behaviour (not part of the segment).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HostOptions {
    /// Wait budget per block. Default: 25 % of the block period.
    pub wait: WaitBudget,
    /// Length of the miss/recovery crossfades in samples (≥ 1). Default 96 (2 ms at 48 kHz).
    pub crossfade_samples: u32,
    /// After this many consecutive misses the host stops waiting (budget 0) until a block
    /// arrives on time, so a stalled plugin doesn't eat the audio thread's time. Default 4.
    pub stall_after_misses: u32,
    /// No heartbeat progress for this long (once attached) = hang ([`Monitor`](crate::Monitor)).
    /// Default 250 ms (ADR-008 §5).
    pub hang_timeout: Duration,
}

impl Default for HostOptions {
    fn default() -> Self {
        Self {
            wait: WaitBudget::FractionOfBlock(0.25),
            crossfade_samples: 96,
            stall_after_misses: 4,
            hang_timeout: Duration::from_millis(250),
        }
    }
}

impl HostOptions {
    /// Offline rendering (ADR-008 §2): wait up to `hang_timeout` for every block, never stall.
    /// A [`BlockOutcome::Missed`] then means the render must abort.
    pub fn offline(hang_timeout: Duration) -> Self {
        Self {
            wait: WaitBudget::Fixed(hang_timeout),
            stall_after_misses: u32::MAX,
            hang_timeout,
            ..Self::default()
        }
    }
}

/// What one [`HostEnd::process`] call produced (the worst chunk if it was split).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockOutcome {
    /// The plugin's output (possibly crossfading in from dry after a miss).
    Wet,
    /// The output wasn't there within the budget: dry substituted (crossfaded) and counted.
    Missed,
    /// The channel is bypassed (fault reported, or the plugin stopped): dry, no wake, no wait.
    Bypassed,
}

/// Host-side counters, written by the audio thread, read by the control thread.
#[derive(Debug, Default)]
pub(crate) struct HostStats {
    blocks: AtomicU64,
    wet: AtomicU64,
    missed: AtomicU64,
    bypassed: AtomicU64,
    waited: AtomicU64,
    events_dropped: AtomicU64,
    consecutive_misses: AtomicU32,
    max_consecutive_misses: AtomicU32,
    /// Set by the monitor on a fault: permanent bypass.
    pub(crate) bypass: AtomicBool,
}

impl HostStats {
    pub(crate) fn snapshot(&self) -> HostCounters {
        HostCounters {
            blocks: self.blocks.load(Ordering::Relaxed),
            wet: self.wet.load(Ordering::Relaxed),
            missed: self.missed.load(Ordering::Relaxed),
            bypassed: self.bypassed.load(Ordering::Relaxed),
            waited: self.waited.load(Ordering::Relaxed),
            events_dropped: self.events_dropped.load(Ordering::Relaxed),
            consecutive_misses: self.consecutive_misses.load(Ordering::Relaxed),
            max_consecutive_misses: self.max_consecutive_misses.load(Ordering::Relaxed),
        }
    }
}

/// A snapshot of the host-side counters (chunks of at most `max_block` frames).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostCounters {
    /// Chunks processed.
    pub blocks: u64,
    /// Chunks with wet output.
    pub wet: u64,
    /// Chunks whose output missed the deadline.
    pub missed: u64,
    /// Chunks while bypassed.
    pub bypassed: u64,
    /// Chunks where the host had to wait for the output.
    pub waited: u64,
    /// Host → plugin events dropped (ring full).
    pub events_dropped: u64,
    /// Current run of consecutive misses.
    pub consecutive_misses: u32,
    /// Longest run of consecutive misses.
    pub max_consecutive_misses: u32,
}

/// The miss/recovery crossfade state.
///
/// - **Recovery** (dry → wet, both known): linear equal-gain crossfade over `len` samples.
/// - **Miss** (wet → dry, wet unknown): a splice crossfade from `dry + δ` to `dry`, with
///   `δ = last output − dry at that position`, i.e. the missing wet signal is estimated as the
///   dry signal plus the last observed wet/dry difference, decaying linearly to 0 over `len`.
///   Continuous at the boundary; exactly dry once done.
///
/// Fully wet and fully dry blocks are copied verbatim (bit-exact).
#[derive(Debug)]
struct Fader {
    len: u32,
    inv: f32,
    /// Wet weight = `wet_k / len`.
    wet_k: u32,
    offset: f32,
    offset_rem: u32,
    last_out: f32,
    last_dry: f32,
}

impl Fader {
    fn new(len: u32) -> Self {
        let len = len.max(1);
        Self {
            len,
            inv: 1.0 / len as f32,
            wet_k: len,
            offset: 0.0,
            offset_rem: 0,
            last_out: 0.0,
            last_dry: 0.0,
        }
    }

    #[inline]
    fn next_offset(&mut self) -> f32 {
        if self.offset_rem == 0 {
            0.0
        } else {
            self.offset_rem -= 1;
            self.offset * (self.offset_rem as f32 * self.inv)
        }
    }

    fn mix(&mut self, dry: &[f32], wet: Option<&[f32]>, out: &mut [f32]) {
        let Some(&last_dry) = dry.last() else {
            return;
        };
        match wet {
            Some(wet) if self.wet_k >= self.len && self.offset_rem == 0 => {
                out.copy_from_slice(wet);
            }
            Some(wet) => {
                for ((o, &w), &d) in out.iter_mut().zip(wet).zip(dry) {
                    let d = d + self.next_offset();
                    if self.wet_k < self.len {
                        self.wet_k += 1;
                    }
                    *o = if self.wet_k >= self.len {
                        w
                    } else {
                        let g = self.wet_k as f32 * self.inv;
                        g * w + (1.0 - g) * d
                    };
                }
            }
            None => {
                let delta = self.last_out - self.last_dry;
                self.wet_k = 0;
                if delta.is_finite() && delta != 0.0 {
                    self.offset = delta;
                    self.offset_rem = self.len;
                } else if !delta.is_finite() {
                    self.offset_rem = 0;
                }
                if self.offset_rem == 0 {
                    out.copy_from_slice(dry);
                } else {
                    for (o, &d) in out.iter_mut().zip(dry) {
                        *o = d + self.next_offset();
                    }
                }
            }
        }
        self.last_out = out[out.len() - 1];
        self.last_dry = last_dry;
    }
}

fn wet_ready(cb: &ControlBlock, from: u64, to: u64, published: u64) -> bool {
    let w = cb.out_write_pos.load(Ordering::Acquire);
    let valid = cb.out_valid_from.load(Ordering::Acquire);
    // `w <= published`: output can't be ahead of the input it came from (a garbage cursor is
    // a miss, never an out-of-range read — indices are masked anyway).
    w >= to && valid <= from && w <= published
}

/// The host's audio-thread end of a channel (ADR-008 `ProxyModule` transport).
///
/// RT contract of [`process`](Self::process), [`push_event`](Self::push_event) and
/// [`pop_output_event`](Self::pop_output_event): no allocation, no locks, no I/O or logging, no
/// unbounded loops. `process` makes at most one wake syscall (only if the plugin is parked) and,
/// when output is late, one bounded wait (budget from [`HostOptions::wait`], at most
/// [`MAX_WAIT_ROUNDS`](crate::wakeup::MAX_WAIT_ROUNDS) park rounds) — the documented ADR-002
/// exception (Amendment 2). Drop it off the audio thread (it may unmap the segment).
pub struct HostEnd<W: Wakeup = PlatformWakeup> {
    map: Mapped,
    stats: Arc<HostStats>,
    options: HostOptions,
    sample_rate: f64,
    max_block: usize,
    latency: u64,
    mask: u64,
    event_mask: u64,
    event_capacity: u64,
    pos: u64,
    ev_in_write: u64,
    ev_out_read: u64,
    /// Host-local copy of the input ring: the dry signal never depends on shared memory.
    dry_ring: Box<[f32]>,
    dry_buf: Box<[f32]>,
    wet_buf: Box<[f32]>,
    consecutive_misses: u32,
    fader: Fader,
    _wakeup: PhantomData<fn() -> W>,
}

impl<W: Wakeup> HostEnd<W> {
    pub(crate) fn new(
        map: Mapped,
        config: &ChannelConfig,
        options: HostOptions,
        stats: Arc<HostStats>,
    ) -> Self {
        let cap = map.input().len();
        let events = map.events_in().len() as u64;
        let cb = map.control();
        let pos = cb.in_write_pos.load(Ordering::Relaxed);
        let ev_in_write = cb.ev_in_write.load(Ordering::Relaxed);
        let ev_out_read = cb.ev_out_read.load(Ordering::Relaxed);
        let max_block = config.max_block as usize;
        Self {
            stats,
            options,
            sample_rate: config.sample_rate,
            max_block,
            latency: u64::from(config.latency_samples),
            mask: cap as u64 - 1,
            event_mask: events - 1,
            event_capacity: events,
            pos,
            ev_in_write,
            ev_out_read,
            dry_ring: vec![0.0; cap].into_boxed_slice(),
            dry_buf: vec![0.0; max_block].into_boxed_slice(),
            wet_buf: vec![0.0; max_block].into_boxed_slice(),
            consecutive_misses: 0,
            fader: Fader::new(options.crossfade_samples),
            map,
            _wakeup: PhantomData,
        }
    }

    /// The added latency in samples (`L`): report it through `latency_samples()` (ADR-008 §2).
    pub fn latency_samples(&self) -> u32 {
        self.latency as u32
    }

    /// The next input position.
    pub fn position(&self) -> u64 {
        self.pos
    }

    /// The options.
    pub fn options(&self) -> &HostOptions {
        &self.options
    }

    /// The counters.
    pub fn counters(&self) -> HostCounters {
        self.stats.snapshot()
    }

    /// Whether the channel is permanently bypassed (fault).
    pub fn is_bypassed(&self) -> bool {
        self.stats.bypass.load(Ordering::Acquire)
    }

    /// \[audio thread\] Sends `input` to the plugin and fills `output` with the plugin's output
    /// for the positions `latency_samples` earlier — or with the crossfaded dry signal if it
    /// isn't there within the budget. Any length; split into chunks of at most `max_block`.
    ///
    /// # Panics
    /// If `input` and `output` differ in length (a caller bug).
    pub fn process(&mut self, input: &[f32], output: &mut [f32]) -> BlockOutcome {
        assert_eq!(input.len(), output.len(), "input/output length mismatch");
        let mut outcome = BlockOutcome::Wet;
        for (i, o) in input
            .chunks(self.max_block)
            .zip(output.chunks_mut(self.max_block))
        {
            outcome = outcome.max(self.process_chunk(i, o));
        }
        outcome
    }

    fn budget_ns(&self, frames: usize) -> u64 {
        match self.options.wait {
            WaitBudget::FractionOfBlock(f) => {
                let ns = frames as f64 / self.sample_rate * f64::from(f) * 1e9;
                if ns >= 1.0 { ns as u64 } else { 0 }
            }
            WaitBudget::Fixed(d) => u64::try_from(d.as_nanos()).unwrap_or(u64::MAX),
        }
    }

    fn process_chunk(&mut self, input: &[f32], output: &mut [f32]) -> BlockOutcome {
        let n = input.len();
        let p = self.pos;
        let published = p + n as u64;
        let cb = self.map.control();
        let ring_in = self.map.input();
        // Seqlock writer side: a plugin that observes any of the stores below also observes
        // every earlier publication of `in_write_pos` (it re-checks after an acquire fence).
        fence(Ordering::Release);
        for (i, &x) in input.iter().enumerate() {
            let idx = ((p + i as u64) & self.mask) as usize;
            ring_in[idx].store(x.to_bits(), Ordering::Relaxed);
            self.dry_ring[idx] = x;
        }
        cb.in_write_pos.store(published, Ordering::Release);
        self.pos = published;

        let bypass = self.stats.bypass.load(Ordering::Acquire);
        let peer_done = PeerState::from_u32(cb.state.load(Ordering::Acquire)).is_done();
        let live = !bypass && !peer_done;
        if live {
            cb.to_plugin.ring::<W>();
        }

        let from = p - self.latency;
        let to = from + n as u64;
        let mut ready = live && wet_ready(cb, from, to, published);
        let mut waited = false;
        if live && !ready && self.consecutive_misses < self.options.stall_after_misses {
            let ns = self.budget_ns(n);
            if ns > 0 {
                waited = true;
                ready = cb.to_host.wait_until::<W>(Deadline::after_ns(ns), || {
                    wet_ready(cb, from, to, published)
                });
            }
        }

        for (i, d) in self.dry_buf[..n].iter_mut().enumerate() {
            *d = self.dry_ring[((from + i as u64) & self.mask) as usize];
        }
        let outcome = if ready {
            let ring_out = self.map.output();
            for (i, w) in self.wet_buf[..n].iter_mut().enumerate() {
                let idx = ((from + i as u64) & self.mask) as usize;
                *w = f32::from_bits(ring_out[idx].load(Ordering::Relaxed));
            }
            self.fader
                .mix(&self.dry_buf[..n], Some(&self.wet_buf[..n]), output);
            BlockOutcome::Wet
        } else {
            self.fader.mix(&self.dry_buf[..n], None, output);
            if live {
                BlockOutcome::Missed
            } else {
                BlockOutcome::Bypassed
            }
        };

        let s = &self.stats;
        s.blocks.fetch_add(1, Ordering::Relaxed);
        if waited {
            s.waited.fetch_add(1, Ordering::Relaxed);
        }
        match outcome {
            BlockOutcome::Wet => {
                s.wet.fetch_add(1, Ordering::Relaxed);
                self.consecutive_misses = 0;
            }
            BlockOutcome::Missed => {
                s.missed.fetch_add(1, Ordering::Relaxed);
                self.consecutive_misses = self.consecutive_misses.saturating_add(1);
                s.max_consecutive_misses
                    .fetch_max(self.consecutive_misses, Ordering::Relaxed);
            }
            BlockOutcome::Bypassed => {
                s.bypassed.fetch_add(1, Ordering::Relaxed);
            }
        }
        s.consecutive_misses
            .store(self.consecutive_misses, Ordering::Relaxed);
        outcome
    }

    /// \[audio thread\] Queues an event for the plugin. Push events before the `process` call
    /// whose input contains their position, in non-decreasing position order. `false` (and
    /// counted) if the ring is full.
    pub fn push_event(&mut self, event: WireEvent) -> bool {
        let cb = self.map.control();
        let read = cb.ev_in_read.load(Ordering::Acquire);
        if self.ev_in_write.wrapping_sub(read) >= self.event_capacity {
            self.stats.events_dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        event.store(&self.map.events_in()[(self.ev_in_write & self.event_mask) as usize]);
        self.ev_in_write += 1;
        cb.ev_in_write.store(self.ev_in_write, Ordering::Release);
        true
    }

    /// \[audio thread\] The next plugin → host event, if any.
    pub fn pop_output_event(&mut self) -> Option<WireEvent> {
        let cb = self.map.control();
        let w = cb.ev_out_write.load(Ordering::Acquire);
        let available = w.wrapping_sub(self.ev_out_read);
        if available == 0 {
            return None;
        }
        if available > self.event_capacity {
            // A garbage cursor: resynchronise instead of reading unpublished slots.
            self.ev_out_read = w;
            cb.ev_out_read.store(w, Ordering::Release);
            return None;
        }
        let event =
            WireEvent::load(&self.map.events_out()[(self.ev_out_read & self.event_mask) as usize]);
        self.ev_out_read += 1;
        cb.ev_out_read.store(self.ev_out_read, Ordering::Release);
        Some(event)
    }
}
