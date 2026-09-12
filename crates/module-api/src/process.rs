//! Lifecycle configuration and the per-block process context (ADR-005 §5).

use serde::{Deserialize, Serialize};

use crate::event::{OutputEvents, ParamEvent};

/// Realtime (playback, monitoring) or offline (export, bake, CLI). v1 built-ins produce the same
/// output in both modes; the mode exists for adapters and future quality modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProcessMode {
    /// Driven by the audio callback.
    Realtime,
    /// Driven by an offline render.
    Offline,
}

/// Input/output channel counts of a module.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelLayout {
    /// Input channels.
    pub inputs: u16,
    /// Output channels.
    pub outputs: u16,
}

impl ChannelLayout {
    /// 1 in, 1 out (the only layout the v1 rack negotiates).
    pub const MONO: Self = Self {
        inputs: 1,
        outputs: 1,
    };
    /// 2 in, 2 out (driven through the rack's dual-mono shim).
    pub const STEREO: Self = Self {
        inputs: 2,
        outputs: 2,
    };
    /// 1 in, 2 out (the left output is taken).
    pub const MONO_TO_STEREO: Self = Self {
        inputs: 1,
        outputs: 2,
    };
}

/// Arguments of [`Module::activate`](crate::Module::activate).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActivateConfig {
    /// Sample rate in Hz.
    pub sample_rate: f64,
    /// `>= 1`. The host never passes more frames per `process()` call.
    pub max_block: u32,
    /// Realtime or offline.
    pub mode: ProcessMode,
    /// One of `supported_layouts()`. Always MONO in v1, possibly through the host shim.
    pub layout: ChannelLayout,
}

/// How long output continues after the input becomes silent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tail {
    /// Output decays to exactly zero or below −120 dBFS within n samples after the input
    /// becomes silent. `Samples(0)` = no tail.
    Samples(u64),
    /// Host caps the render (policy in the bake/export spec).
    Infinite,
}

/// Transport state for one block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Transport {
    /// True while playing or recording; false for monitoring-only processing.
    pub playing: bool,
    /// Document position of the block's first input sample, when the input is document audio.
    pub position_samples: Option<u64>,
}

/// Something a module asks the host to do (from `process`, RT-safe).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostRequest {
    /// Latency or tail must change, or an activate-time resource must be rebuilt
    /// (e.g. a new FFT size). The host performs a replacement.
    Restart,
}

impl HostRequest {
    const fn bit(self) -> u32 {
        match self {
            HostRequest::Restart => 1 << 0,
        }
    }
}

/// Everything a module sees for one block besides the audio slices.
#[derive(Debug)]
pub struct ProcessContext<'a> {
    /// `0 <= frames <= max_block`. Every input/output slice has exactly this length.
    pub frames: u32,
    /// Samples processed since activate (CLAP steady_time). Not reset by `reset()`.
    pub steady_time: u64,
    /// Transport state at the block's first sample.
    pub transport: Transport,
    /// Time-sorted parameter events for this block.
    pub events: &'a [ParamEvent],
    /// Parameter reports back to the host.
    pub out_events: &'a mut OutputEvents,
    requests: u32,
}

impl<'a> ProcessContext<'a> {
    /// Builds a context. RT-safe.
    pub fn new(
        frames: u32,
        steady_time: u64,
        transport: Transport,
        events: &'a [ParamEvent],
        out_events: &'a mut OutputEvents,
    ) -> Self {
        Self {
            frames,
            steady_time,
            transport,
            events,
            out_events,
            requests: 0,
        }
    }

    /// RT-safe (sets a bit). The host forwards it to the control thread via the RT event ring.
    pub fn request(&mut self, r: HostRequest) {
        self.requests |= r.bit();
    }

    /// True if the module requested `r` during this block.
    pub fn requested(&self, r: HostRequest) -> bool {
        self.requests & r.bit() != 0
    }
}

/// Result of one `process()` call. Non-exhaustive: room for CLAP-style statuses (e.g. SLEEP,
/// TAIL) without a breaking change.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProcessStatus {
    /// Output is valid.
    Continue,
    /// Output could not be produced correctly (adapters only: sandbox died, IPC deadline).
    /// The module must still have written finite samples (silence or latency-matched dry).
    /// The host bypasses the slot (crossfade), marks it failed and notifies the user.
    /// Built-in modules never return Error.
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_bits() {
        let mut out = OutputEvents::with_capacity(0);
        let mut ctx = ProcessContext::new(0, 0, Transport::default(), &[], &mut out);
        assert!(!ctx.requested(HostRequest::Restart));
        ctx.request(HostRequest::Restart);
        assert!(ctx.requested(HostRequest::Restart));
    }
}
