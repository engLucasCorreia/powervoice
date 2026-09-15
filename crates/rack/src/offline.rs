//! Offline render (SPEC-012 §2.8, §4.4): the same [`Chain`] code as realtime, with its own
//! instances built from a [`RackModel`], in offline mode, in 4096-frame blocks.
//!
//! The output has the input's length and is **time-aligned**: the total latency L is trimmed by
//! feeding L zeros after the input and dropping the first L output samples. Per-slot bypass
//! flags are honoured; the whole-rack A/B is a listening aid and is not part of the model, so it
//! never affects a render. Missing modules and slot failures are errors (a render is never
//! silently dry).
//!
//! The render runs under the `dsp::fp` FTZ/DAZ guard (ADR-002 §2, SPEC-012 §4.4); the modules are
//! denormal-safe regardless.

use vox_module_api::{
    ActivateConfig, ChannelLayout, DEFAULT_EVENT_CAPACITY, EventListError, ParamEvent, ParamFlags,
    ParamId, ProcessMode, Transport,
};

use crate::{
    Chain, FailReason, OFFLINE_BLOCK, PushEventError, RackError, RackModel, Registry, Resolved,
};

/// Error from an offline render.
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// Slots name modules that are not installed (or whose state is too new).
    #[error("unknown module id(s): {}", .0.join(", "))]
    MissingModules(Vec<String>),
    /// Building the chain failed.
    #[error(transparent)]
    Rack(#[from] RackError),
    /// A slot failed during the render (SPEC-012 §2.9, ADR-008 §5).
    #[error("slot {} ({name}) {}", .index + 1, match .reason {
        FailReason::NonFinite => "produced invalid audio",
        FailReason::ModuleError => "failed",
    })]
    SlotFailed {
        /// Slot index (0-based).
        index: usize,
        /// Module name.
        name: String,
        /// Why.
        reason: FailReason,
    },
    /// An automation event names a missing slot or parameter.
    #[error("automation event for slot {slot}: {reason}")]
    Automation {
        /// Slot index.
        slot: usize,
        /// Why.
        reason: &'static str,
    },
}

/// A parameter change at an absolute input sample position (a job's automation; none in v1
/// besides tests).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationEvent {
    /// Slot index.
    pub slot: usize,
    /// Absolute input sample.
    pub position: u64,
    /// Parameter.
    pub id: ParamId,
    /// Plain value (`clamp_quantize`d by the render).
    pub value: f64,
}

/// Builds and activates an offline chain (`max_block` 4096, offline mode) from `model`.
pub fn build_chain(
    registry: &Registry,
    model: &RackModel,
    sample_rate: f64,
) -> Result<Chain, RenderError> {
    let missing = registry.missing_ids(model);
    if !missing.is_empty() {
        return Err(RenderError::MissingModules(missing));
    }
    let mut chain = Chain::new();
    for s in &model.slots {
        match registry.resolve(s)? {
            Resolved::Module(m) => chain.push(m)?,
            Resolved::Placeholder { message, .. } => {
                return Err(RenderError::MissingModules(vec![format!(
                    "{} ({message})",
                    s.module
                )]));
            }
        }
    }
    chain.activate(&ActivateConfig {
        sample_rate,
        max_block: OFFLINE_BLOCK,
        mode: ProcessMode::Offline,
        layout: ChannelLayout::MONO,
    })?;
    for (i, s) in model.slots.iter().enumerate() {
        if s.bypass {
            chain.set_bypass_now(i, true)?;
        }
    }
    Ok(chain)
}

/// Renders `input` through `model` at `sample_rate` (same length, time-aligned).
pub fn render(
    registry: &Registry,
    model: &RackModel,
    sample_rate: f64,
    input: &[f32],
) -> Result<Vec<f32>, RenderError> {
    render_with_automation(registry, model, sample_rate, input, &[])
}

/// [`render`] with parameter events at absolute input positions.
pub fn render_with_automation(
    registry: &Registry,
    model: &RackModel,
    sample_rate: f64,
    input: &[f32],
    automation: &[AutomationEvent],
) -> Result<Vec<f32>, RenderError> {
    let _fp = vox_dsp::fp::DenormalGuard::new();
    let mut chain = build_chain(registry, model, sample_rate)?;
    let mut events = Vec::with_capacity(automation.len());
    for a in automation {
        let p = chain
            .module(a.slot)
            .ok_or(RenderError::Automation {
                slot: a.slot,
                reason: "no such slot",
            })?
            .params()
            .iter()
            .find(|p| p.id == a.id && !p.flags.contains(ParamFlags::READ_ONLY))
            .ok_or(RenderError::Automation {
                slot: a.slot,
                reason: "no such writable parameter",
            })?;
        events.push(AutomationEvent {
            value: p.clamp_quantize(a.value),
            ..*a
        });
    }
    events.sort_by_key(|e| e.position);

    let latency = chain.latency_samples() as usize;
    let total = input.len() + latency;
    let mut out = vec![0.0f32; total];
    let block = OFFLINE_BLOCK as usize;
    let mut inbuf = vec![0.0f32; block];
    let (mut pos, mut next) = (0usize, 0usize);
    while pos < total {
        let mut n = block.min(total - pos);
        // At most one module event list per call and slot, so nothing is carried over (a
        // carried event would move to the next call's first sample). The block is cut at the
        // first event that would not fit. More than 512 events for *distinct* ids at one single
        // sample (same-id ones coalesce) are delivered by consecutive zero-length flushes at
        // that sample, 512 per list, so they still take effect at their exact position.
        let mut pushed = vec![0usize; chain.len()];
        while let Some(a) = events.get(next)
            && a.position < (pos + n) as u64
        {
            let offset = a.position.saturating_sub(pos as u64) as u32;
            if pushed[a.slot] == DEFAULT_EVENT_CAPACITY {
                n = offset as usize;
                break;
            }
            let ev = ParamEvent {
                offset,
                id: a.id,
                value: a.value,
            };
            match chain.push_event(a.slot, ev) {
                Ok(()) => {
                    next += 1;
                    pushed[a.slot] += 1;
                }
                Err(PushEventError::List(EventListError::Full)) => {
                    // Cut the block at the event (or flush with a zero-length call) so every
                    // event keeps its exact sample position.
                    n = offset as usize;
                    break;
                }
                Err(_) => {
                    return Err(RenderError::Automation {
                        slot: a.slot,
                        reason: "event rejected",
                    });
                }
            }
        }
        let avail = input.len().saturating_sub(pos).min(n);
        // Past the input (flushing the latency) a block can start beyond `input.len()`.
        if avail > 0 {
            inbuf[..avail].copy_from_slice(&input[pos..pos + avail]);
        }
        inbuf[avail..n].fill(0.0);
        chain.process(
            Transport {
                playing: true,
                position_samples: Some(pos as u64),
            },
            &inbuf[..n],
            &mut out[pos..pos + n],
        );
        if let Some((index, reason)) = chain.take_failure() {
            let name = chain.slot_name(index).unwrap_or_default().to_owned();
            chain.deactivate();
            return Err(RenderError::SlotFailed {
                index,
                name,
                reason,
            });
        }
        chain.drain_events(|_| {});
        pos += n;
    }
    chain.deactivate();
    out.drain(..latency);
    out.truncate(input.len());
    Ok(out)
}

// --- T-602: windowed renders (bakes and exports of a range) ----------------------------------
//
// SPEC-012 §2.8 amendment (T-602): a render of `[start, start + len)` inside a longer signal (the
// document) is still length-preserving and time-aligned, and adds two kinds of *context*:
//
// - **Pre-roll.** Real signal before `start` is fed first and its output discarded, so stateful
//   modules (envelopes, adaptive gains, filter memory) reach the state they have in a whole-file
//   render: `min(start, clamp(Σ tails of the non-bypassed slots, PRE_ROLL_MIN_S, PRE_ROLL_MAX_S))`
//   (`Tail::Infinite` counts as the cap). 30 s covers every built-in time constant (≤ 2 s release
//   τ decays below −120 dB in 13.8 τ); the tail sum stretches it for long filter ringing (the EQ's
//   ≈ 35 s worst case, SPEC-015 §4.8) up to 60 s.
// - **Post-roll.** The latency flush feeds the real signal after the range (at most L samples),
//   then zeros — so a look-ahead module sees what really follows the range.
//
// No tail is ever appended: the output has exactly `len` samples, like SPEC-012's whole-file
// render. A whole-signal window (`start = 0`, `len = signal_len`) therefore has no pre-roll and
// zero post-roll, and is bit-identical to [`render`].

/// Smallest pre-roll before a range (seconds, clamped to the signal before it).
pub const PRE_ROLL_MIN_S: f64 = 30.0;
/// Largest pre-roll before a range (seconds).
pub const PRE_ROLL_MAX_S: f64 = 60.0;

/// Where a windowed render sits in its signal (see the section comment above).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderWindow {
    /// Absolute position of the range's first sample.
    pub start: u64,
    /// The range's length, which is the output length.
    pub len: u64,
    /// Signal samples fed before `start` (their output is discarded).
    pub pre_roll: u64,
    /// Signal samples fed after the range during the latency flush (≤ `latency`); zeros follow.
    pub post_roll: u64,
    /// The chain's total latency L, trimmed from the output.
    pub latency: u64,
}

impl RenderWindow {
    /// The first absolute signal sample the render reads (`start − pre_roll`).
    pub fn first(&self) -> u64 {
        self.start - self.pre_roll
    }

    /// Signal samples read, from [`Self::first`] on.
    pub fn read_len(&self) -> u64 {
        self.pre_roll + self.len + self.post_roll
    }

    /// Samples processed by the chain (`pre_roll + len + latency`).
    pub fn total(&self) -> u64 {
        self.pre_roll + self.len + self.latency
    }
}

/// The pre-roll the T-602 policy wants before a range, for an activated `chain` at
/// `sample_rate` (before clamping to the signal that exists before the range).
pub fn pre_roll_samples(chain: &Chain, sample_rate: f64) -> u64 {
    let min = (PRE_ROLL_MIN_S * sample_rate).round() as u64;
    let max = (PRE_ROLL_MAX_S * sample_rate).round() as u64;
    let mut tails = 0u64;
    for i in 0..chain.len() {
        if chain.is_bypassed(i) {
            continue;
        }
        match chain.slot_tail(i) {
            Some(vox_module_api::Tail::Samples(n)) => tails = tails.saturating_add(n),
            Some(vox_module_api::Tail::Infinite) => tails = max,
            None => {}
        }
    }
    tails.clamp(min, max)
}

/// Plans the window for `[start, start + len)` of a signal `signal_len` samples long.
pub fn plan_window(
    chain: &Chain,
    sample_rate: f64,
    start: u64,
    len: u64,
    signal_len: u64,
) -> RenderWindow {
    let latency = u64::from(chain.latency_samples());
    let end = start.saturating_add(len);
    RenderWindow {
        start,
        len,
        pre_roll: start.min(pre_roll_samples(chain, sample_rate)),
        post_roll: signal_len.saturating_sub(end).min(latency),
        latency,
    }
}

/// Error from [`render_range`]: the render itself, or one of the caller's callbacks (reading the
/// signal, taking the output, or the per-block tick that reports progress and cancels).
#[derive(Debug)]
pub enum WindowError<E> {
    /// The rack could not be built, or a slot failed (SPEC-012 §2.9, ADR-008 §5).
    Render(RenderError),
    /// A callback returned an error; the render stopped there.
    Caller(E),
}

impl<E> From<RenderError> for WindowError<E> {
    fn from(e: RenderError) -> Self {
        WindowError::Render(e)
    }
}

/// Renders `[start, start + len)` of a signal `signal_len` samples long through `model`, in
/// 4096-frame offline blocks, with the T-602 pre-roll/post-roll context (see above).
///
/// - `read(pos, buf)` must fill `buf` with the signal from absolute position `pos` (the render
///   reads [`RenderWindow::first`] … `first + read_len` in order, never past `signal_len`);
/// - `write(samples)` receives the output of the range in order, `len` samples in total;
/// - `tick(done, total)` runs after every block (processed samples so far / in total); an error
///   stops the render — how a caller cancels.
///
/// Positions passed to the modules (`Transport::position_samples`) are absolute signal
/// positions. Returns the window used.
#[allow(clippy::too_many_arguments)]
pub fn render_range<E>(
    registry: &Registry,
    model: &RackModel,
    sample_rate: f64,
    start: u64,
    len: u64,
    signal_len: u64,
    mut read: impl FnMut(u64, &mut [f32]) -> Result<(), E>,
    mut write: impl FnMut(&[f32]) -> Result<(), E>,
    mut tick: impl FnMut(u64, u64) -> Result<(), E>,
) -> Result<RenderWindow, WindowError<E>> {
    let _fp = vox_dsp::fp::DenormalGuard::new();
    let mut chain = build_chain(registry, model, sample_rate)?;
    let window = plan_window(&chain, sample_rate, start, len, signal_len);
    let result = run_window(&mut chain, &window, &mut read, &mut write, &mut tick);
    chain.deactivate();
    result.map(|()| window)
}

fn run_window<E>(
    chain: &mut Chain,
    window: &RenderWindow,
    read: &mut impl FnMut(u64, &mut [f32]) -> Result<(), E>,
    write: &mut impl FnMut(&[f32]) -> Result<(), E>,
    tick: &mut impl FnMut(u64, u64) -> Result<(), E>,
) -> Result<(), WindowError<E>> {
    let first = window.first();
    let total = window.total();
    let read_len = window.read_len();
    // Output index `i` (processed-sample count) is input `i − L`: the range's output is
    // `[pre_roll + L, pre_roll + L + len)`.
    let keep_from = window.pre_roll + window.latency;
    let keep_to = keep_from + window.len;
    let block = OFFLINE_BLOCK as usize;
    let mut inbuf = vec![0.0f32; block];
    let mut outbuf = vec![0.0f32; block];
    let mut pos = 0u64;
    while pos < total {
        let n = (total - pos).min(block as u64) as usize;
        let avail = read_len.saturating_sub(pos).min(n as u64) as usize;
        if avail > 0 {
            read(first + pos, &mut inbuf[..avail]).map_err(WindowError::Caller)?;
        }
        inbuf[avail..n].fill(0.0);
        chain.process(
            Transport {
                playing: true,
                position_samples: Some(first + pos),
            },
            &inbuf[..n],
            &mut outbuf[..n],
        );
        if let Some((index, reason)) = chain.take_failure() {
            let name = chain.slot_name(index).unwrap_or_default().to_owned();
            return Err(WindowError::Render(RenderError::SlotFailed {
                index,
                name,
                reason,
            }));
        }
        chain.drain_events(|_| {});
        let block_end = pos + n as u64;
        let from = keep_from.max(pos);
        let to = keep_to.min(block_end);
        if from < to {
            write(&outbuf[(from - pos) as usize..(to - pos) as usize])
                .map_err(WindowError::Caller)?;
        }
        pos = block_end;
        tick(pos, total).map_err(WindowError::Caller)?;
    }
    Ok(())
}
