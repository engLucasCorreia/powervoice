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
        inbuf[..avail].copy_from_slice(&input[pos..pos + avail]);
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
