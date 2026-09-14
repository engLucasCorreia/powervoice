//! Record operations (T-304, SPEC-022): record at the cursor (Insert / Overwrite) and punch-in
//! over a selection with pre-roll and post-roll, aligned after latency compensation.
//!
//! This module holds the pure parts, testable without an engine:
//! - [`resolve_record`]: what Record does (SPEC-022 §2.2's resolution table) → a [`RecordPlan`];
//! - [`RunSpec`]: what the reader plays during an operation (§2.7, §4.4: pre-roll with silence
//!   padding before 0, muting of the record range, the 5 ms listening fades, post-roll, the stop
//!   fade), as a function of virtual playback positions;
//! - [`take_index_at`]: the capture-writer's take index for an app-clock capture time (§4.3),
//!   read from the input block whose capture span contains it.
//!
//! The phase machine (pre-roll → recording → post-roll → finished/cancelled) runs on the control
//! thread (`control.rs`); the RT callbacks are unchanged apart from the calibration sweep.

use std::collections::VecDeque;

use crate::record::RecordError;

/// SPEC-022 §3 `listen_fade_ms`: playback fade at the mute boundaries (listening only).
pub const LISTEN_FADE_MS: f64 = 5.0;
/// SPEC-022 §3 `preroll_s` / `postroll_s`: upper bound.
pub const MAX_ROLL_S: f64 = 20.0;
/// SPEC-022 §3 `punch_xfade_ms`: upper bound.
pub const MAX_XFADE_MS: f64 = 50.0;
/// SPEC-022 §3 `record_offset_ms`: the offset is clamped to ±500 ms.
pub const MAX_OFFSET_MS: f64 = 500.0;

/// What one Record press does (SPEC-022 §2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordOpKind {
    /// Row 1: the document is empty — a new recording (SPEC-002 §2.2).
    New,
    /// Record at the cursor / selection start, inserting (§2.4).
    Insert,
    /// Record at the cursor / selection start, overwriting (§2.5).
    Overwrite,
    /// Punch-in over the selection (§2.6).
    Punch,
}

/// SPEC-022 §3 `record_mode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorRecordMode {
    /// Factory default: never destroys audio.
    #[default]
    Insert,
    Overwrite,
}

/// The SPEC-022 §3 preferences a Record press resolves with (app preferences, T-104 settings).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecordPrefs {
    pub mode: CursorRecordMode,
    pub punch_on_selection: bool,
    pub preroll_s: f64,
    pub postroll_s: f64,
    pub preroll_at_cursor: bool,
    pub hear_original: bool,
    pub xfade_ms: f64,
    /// The residual recording offset δ for the current device setup (§2.13), in ms (positive:
    /// recorded audio would land late and is moved earlier).
    pub offset_ms: f64,
}

impl Default for RecordPrefs {
    /// SPEC-022 §2.3 defaults.
    fn default() -> Self {
        Self {
            mode: CursorRecordMode::Insert,
            punch_on_selection: true,
            preroll_s: 5.0,
            postroll_s: 1.0,
            preroll_at_cursor: false,
            hear_original: false,
            xfade_ms: 10.0,
            offset_ms: 0.0,
        }
    }
}

/// A resolved Record press: everything the engine and the session need to run it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordPlan {
    pub kind: RecordOpKind,
    /// The record point `at` (the punch start `S`), document samples.
    pub at_samples: u64,
    /// The punch end `E` (`None`: cursor recordings end with Stop).
    pub end_samples: Option<u64>,
    /// Pre-roll length (0: free start or no pre-roll).
    pub preroll_samples: u64,
    /// Post-roll length (punch only).
    pub postroll_samples: u64,
    /// The window opens at an aligned start (§2.1): playback precedes it.
    pub aligned: bool,
    /// δ in ns, applied only to aligned starts (§2.13 "Where the offset applies").
    pub offset_ns: i64,
    /// Hear original during the window (Punch and Overwrite only, §2.3).
    pub hear_original: bool,
    /// Punch/Overwrite crossfade `X` in document samples (§4.2).
    pub xfade_samples: u64,
    pub doc_len_samples: u64,
    pub doc_rate_hz: u32,
}

impl RecordPlan {
    /// A new recording into an empty document (row 1).
    pub fn new_recording(doc_rate_hz: u32) -> Self {
        Self {
            kind: RecordOpKind::New,
            at_samples: 0,
            end_samples: None,
            preroll_samples: 0,
            postroll_samples: 0,
            aligned: false,
            offset_ns: 0,
            hear_original: false,
            xfade_samples: 0,
            doc_len_samples: 0,
            doc_rate_hz,
        }
    }
}

fn seconds_to_samples(s: f64, rate_hz: u32, max_s: f64) -> u64 {
    let s = if s.is_finite() {
        s.clamp(0.0, max_s)
    } else {
        0.0
    };
    (s * f64::from(rate_hz)).round() as u64
}

/// §3 `record_offset_ms` → ns, clamped to ±500 ms.
pub fn offset_ms_to_ns(ms: f64) -> i64 {
    let ms = if ms.is_finite() {
        ms.clamp(-MAX_OFFSET_MS, MAX_OFFSET_MS)
    } else {
        0.0
    };
    (ms * 1e6).round() as i64
}

/// SPEC-022 §2.2: resolves one Record press. `selection` is `[S, E)` (empty or `None`: no
/// selection), `cursor` the playhead while stopped (the heard position when Record stopped a
/// playing transport). A punch without an output device is refused
/// ([`RecordError::PunchNeedsOutput`]); cursor recordings without one fall back to a free start.
pub fn resolve_record(
    doc_len_samples: u64,
    doc_rate_hz: u32,
    selection: Option<(u64, u64)>,
    cursor: u64,
    has_output: bool,
    prefs: &RecordPrefs,
) -> Result<RecordPlan, RecordError> {
    if doc_len_samples == 0 {
        return Ok(RecordPlan::new_recording(doc_rate_hz));
    }
    let xfade_samples =
        (prefs.xfade_ms.clamp(0.0, MAX_XFADE_MS) * f64::from(doc_rate_hz) / 1000.0).round() as u64;
    let selection = selection
        .map(|(a, b)| (a.min(b).min(doc_len_samples), a.max(b).min(doc_len_samples)))
        .filter(|(a, b)| a < b);
    let mode_kind = match prefs.mode {
        CursorRecordMode::Insert => RecordOpKind::Insert,
        CursorRecordMode::Overwrite => RecordOpKind::Overwrite,
    };
    let (kind, at, end) = match selection {
        Some((s, e)) if prefs.punch_on_selection => {
            if !has_output {
                return Err(RecordError::PunchNeedsOutput);
            }
            (RecordOpKind::Punch, s, Some(e))
        }
        Some((s, _)) => (mode_kind, s, None),
        None => (mode_kind, cursor.min(doc_len_samples), None),
    };
    let aligned = has_output && (kind == RecordOpKind::Punch || prefs.preroll_at_cursor);
    Ok(RecordPlan {
        kind,
        at_samples: at,
        end_samples: end,
        preroll_samples: if aligned {
            seconds_to_samples(prefs.preroll_s, doc_rate_hz, MAX_ROLL_S)
        } else {
            0
        },
        postroll_samples: if kind == RecordOpKind::Punch {
            seconds_to_samples(prefs.postroll_s, doc_rate_hz, MAX_ROLL_S)
        } else {
            0
        },
        aligned,
        offset_ns: if aligned {
            offset_ms_to_ns(prefs.offset_ms)
        } else {
            0
        },
        hear_original: prefs.hear_original && aligned && kind != RecordOpKind::Insert,
        xfade_samples,
        doc_len_samples,
        doc_rate_hz,
    })
}

/// What the reader plays during an aligned operation (SPEC-022 §2.7, §4.4), over **virtual**
/// positions `v = q + offset` so the pre-roll can start before document position 0 (`q < 0` is
/// digital silence, so the pre-roll always lasts its full length).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RunSpec {
    /// `v − q`.
    pub(crate) offset: u64,
    /// The record point `at`: the fade-out ends here and muting starts.
    pub(crate) at: u64,
    /// Muting ends at the punch end `E` (a fade-in starts there); `None`: muted to the end.
    pub(crate) mute_end: Option<u64>,
    /// Document length `L` (positions ≥ `L` are always silence).
    pub(crate) doc_len: u64,
    /// Play `A[q]` in the muted range while `q < L`, with no listening fades.
    pub(crate) hear_original: bool,
    /// The run ends at this document position (a stop fade before it); `None`: at Stop.
    pub(crate) end: Option<u64>,
    /// Listening fade length in document samples.
    pub(crate) fade: u64,
}

impl RunSpec {
    /// The virtual start position for a pre-roll of `pre` samples before `at`.
    pub(crate) fn start_v(&self, pre: u64) -> u64 {
        (self.at + self.offset).saturating_sub(pre)
    }

    /// The virtual end position (`None`: open-ended).
    pub(crate) fn end_v(&self) -> Option<u64> {
        self.end.map(|e| e + self.offset)
    }

    /// Gain of document position `q` (0 outside the audible parts), per §4.4:
    /// - fade-out `(at − q)/F` over `[at − F, at)`;
    /// - fade-in `(q − E + 1)/F` over `[E, E + F)`;
    /// - stop fade `(end − q)/F` over `[end − F, end)`;
    /// - muted range `[at, E)` silent unless Hear original is on (then no listening fades).
    pub(crate) fn gain(&self, q: i128) -> f32 {
        if q < 0 || q >= i128::from(self.doc_len) {
            return 0.0;
        }
        let (at, f) = (i128::from(self.at), i128::from(self.fade.max(1)));
        let muted = q >= at && self.mute_end.is_none_or(|e| q < i128::from(e));
        let mut g = 1.0f32;
        if !self.hear_original {
            if muted {
                return 0.0;
            }
            if q < at && q >= at - f {
                g *= (at - q) as f32 / f as f32;
            }
            if let Some(e) = self.mute_end.map(i128::from)
                && q >= e
                && q < e + f
            {
                g *= (q - e + 1) as f32 / f as f32;
            }
        }
        if let Some(end) = self.end.map(i128::from)
            && q < end
            && q >= end - f
        {
            g *= (end - q) as f32 / f as f32;
        }
        g
    }

    /// Fills `out` with what plays at virtual positions `[v0, v0 + out.len())`. `read(q, buf)`
    /// copies `A[q, q + buf.len())` (only ever called inside `[0, L)`).
    pub(crate) fn render(&self, v0: u64, out: &mut [f32], mut read: impl FnMut(u64, &mut [f32])) {
        out.fill(0.0);
        let q0 = i128::from(v0) - i128::from(self.offset);
        let n = out.len() as i128;
        let lo = q0.max(0);
        let hi = (q0 + n).min(i128::from(self.doc_len));
        if lo < hi {
            let (a, b) = ((lo - q0) as usize, (hi - q0) as usize);
            read(lo as u64, &mut out[a..b]);
        }
        for (i, x) in out.iter_mut().enumerate() {
            // A gain of exactly 1.0 leaves the sample bit-identical (IEEE multiplication).
            *x *= self.gain(q0 + i as i128);
        }
    }
}

/// One input block as the control thread saw it (from `InputEvent::Block`): the take position at
/// its end (device frames, dropout fills included) and the app-clock capture time of its first
/// frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CaptureBlock {
    pub(crate) end_take: u64,
    pub(crate) frames: u32,
    pub(crate) start_ns: u64,
}

/// SPEC-022 §4.3: the take index (device frames) of the sample captured at app time `t_ns`,
/// `k_b + round((t − c_b)·r/1e9)` from the block `b` whose capture span contains `t` (or the
/// earliest one when `t` precedes them all). `None` while no block reaches `t` yet. The result
/// may be negative (before the take's first sample).
pub(crate) fn take_index_at(
    blocks: &VecDeque<CaptureBlock>,
    t_ns: i128,
    rate_hz: u32,
) -> Option<i128> {
    let rate = i128::from(rate_hz.max(1));
    let span_end =
        |b: &CaptureBlock| i128::from(b.start_ns) + i128::from(b.frames) * 1_000_000_000 / rate;
    let last = blocks.back()?;
    if t_ns >= span_end(last) {
        return None;
    }
    let block = blocks.iter().find(|b| t_ns < span_end(b)).unwrap_or(last);
    let k_b = i128::from(block.end_take) - i128::from(block.frames);
    let dt = t_ns - i128::from(block.start_ns);
    // Round half away from zero.
    let num = dt * rate;
    let k = if num >= 0 {
        (num + 500_000_000) / 1_000_000_000
    } else {
        -((-num + 500_000_000) / 1_000_000_000)
    };
    Some(k_b + k)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 48_000;

    fn prefs() -> RecordPrefs {
        RecordPrefs::default()
    }

    /// AC-1: every combination of document empty/non-empty, selection null/empty/non-empty,
    /// punch-on-selection on/off, mode Insert/Overwrite and output present/absent resolves per
    /// the §2.2 table; a punch without an output is refused.
    #[test]
    fn resolution_follows_the_table() {
        for len in [0u64, 960_000] {
            for sel in [None, Some((300_000, 300_000)), Some((240_000, 384_000))] {
                for punch in [true, false] {
                    for mode in [CursorRecordMode::Insert, CursorRecordMode::Overwrite] {
                        for out in [true, false] {
                            let p = RecordPrefs {
                                punch_on_selection: punch,
                                mode,
                                ..prefs()
                            };
                            let got = resolve_record(len, R, sel, 100_000, out, &p);
                            let nonempty = sel.is_some_and(|(a, b)| a < b);
                            let mode_kind = match mode {
                                CursorRecordMode::Insert => RecordOpKind::Insert,
                                CursorRecordMode::Overwrite => RecordOpKind::Overwrite,
                            };
                            if len == 0 {
                                assert_eq!(got.unwrap().kind, RecordOpKind::New);
                            } else if nonempty && punch {
                                if out {
                                    let plan = got.unwrap();
                                    assert_eq!(plan.kind, RecordOpKind::Punch);
                                    assert_eq!(
                                        (plan.at_samples, plan.end_samples),
                                        (240_000, Some(384_000))
                                    );
                                    assert!(plan.aligned);
                                    assert_eq!(plan.preroll_samples, 240_000);
                                    assert_eq!(plan.postroll_samples, 48_000);
                                    assert_eq!(plan.xfade_samples, 480);
                                } else {
                                    assert_eq!(got, Err(RecordError::PunchNeedsOutput));
                                }
                            } else if nonempty {
                                let plan = got.unwrap();
                                assert_eq!(plan.kind, mode_kind);
                                assert_eq!(plan.at_samples, 240_000, "rule 3: at S");
                                assert!(!plan.aligned, "pre-roll at the cursor is off");
                                assert_eq!(plan.end_samples, None);
                            } else {
                                let plan = got.unwrap();
                                assert_eq!(plan.kind, mode_kind);
                                assert_eq!(plan.at_samples, 100_000, "rule 4: at c");
                            }
                        }
                    }
                }
            }
        }
    }

    /// §2.3/§2.13: pre-roll at the cursor aligns cursor recordings (only with an output);
    /// "Hear original" never applies to Insert; δ applies only to aligned starts and is clamped
    /// to ±500 ms; rolls clamp to 0–20 s.
    #[test]
    fn preroll_at_cursor_offset_and_clamps() {
        let p = RecordPrefs {
            preroll_at_cursor: true,
            hear_original: true,
            offset_ms: 3.0,
            ..prefs()
        };
        let plan = resolve_record(960_000, R, None, 5, true, &p).unwrap();
        assert!(
            plan.aligned && !plan.hear_original,
            "Insert never hears the original"
        );
        assert_eq!(plan.offset_ns, 3_000_000);
        assert_eq!(plan.postroll_samples, 0, "post-roll is punch-only");
        let plan = resolve_record(960_000, R, None, 5, false, &p).unwrap();
        assert!(!plan.aligned, "no output: free start");
        assert_eq!((plan.preroll_samples, plan.offset_ns), (0, 0));
        let ow = RecordPrefs {
            mode: CursorRecordMode::Overwrite,
            ..p
        };
        assert!(
            resolve_record(960_000, R, None, 5, true, &ow)
                .unwrap()
                .hear_original
        );
        assert_eq!(offset_ms_to_ns(600.0), 500_000_000);
        assert_eq!(offset_ms_to_ns(-600.0), -500_000_000);
        let long = RecordPrefs {
            preroll_s: 99.0,
            ..prefs()
        };
        let plan = resolve_record(960_000, R, Some((1, 2)), 0, true, &long).unwrap();
        assert_eq!(plan.preroll_samples, 20 * 48_000);
    }

    fn doc(q: u64) -> f32 {
        (q % 1000) as f32 / 1000.0 + 0.001
    }

    fn render(spec: &RunSpec, v0: u64, n: usize) -> Vec<f32> {
        let mut out = vec![9.0; n];
        spec.render(v0, &mut out, |q, buf| {
            for (i, x) in buf.iter_mut().enumerate() {
                *x = doc(q + i as u64);
            }
        });
        out
    }

    /// AC-8 (reader part): pre-roll `A[P₀, S)` with a linear 5 ms fade-out ending at `S`, the
    /// window silent, post-roll `A[E, E + post)` with a fade-in at `E` and the stop fade at its
    /// end; silence padding before 0; Hear original plays the window unfaded.
    #[test]
    fn run_mutes_the_window_and_fades_only_what_is_heard() {
        let (s, e, f) = (240_000u64, 384_000u64, 240u64);
        let spec = RunSpec {
            offset: 0,
            at: s,
            mute_end: Some(e),
            doc_len: 960_000,
            hear_original: false,
            end: Some(e + 48_000),
            fade: f,
        };
        let v0 = spec.start_v(96_000);
        assert_eq!(v0, 144_000);
        let total = (spec.end_v().unwrap() - v0) as usize;
        let out = render(&spec, v0, total);
        for (i, &x) in out.iter().enumerate() {
            let q = v0 + i as u64;
            let want = if q < s - f {
                doc(q)
            } else if q < s {
                doc(q) * ((s - q) as f32 / f as f32)
            } else if q < e {
                0.0
            } else if q < e + f {
                doc(q) * ((q - e + 1) as f32 / f as f32)
            } else if q < e + 48_000 - f {
                doc(q)
            } else {
                doc(q) * ((e + 48_000 - q) as f32 / f as f32)
            };
            assert!((x - want).abs() <= 1e-6, "q {q}: {x} vs {want}");
        }

        // S = 48 000 with a 2 s pre-roll: 48 000 frames of padding, then A[0, …).
        let near = RunSpec {
            offset: 48_000,
            at: 48_000,
            ..spec
        };
        let v0 = near.start_v(96_000);
        assert_eq!(v0, 0);
        let out = render(&near, v0, 60_000);
        assert!(
            out[..48_000]
                .iter()
                .all(|&x| x.to_bits() == 0.0f32.to_bits()),
            "padding"
        );
        assert_eq!(out[48_000].to_bits(), doc(0).to_bits());

        let hear = RunSpec {
            hear_original: true,
            ..spec
        };
        let out = render(&hear, s - 10, 20 + (e - s) as usize);
        for (i, &x) in out.iter().enumerate() {
            assert_eq!(
                x.to_bits(),
                doc(s - 10 + i as u64).to_bits(),
                "unfaded original"
            );
        }

        // Cursor recordings: muted from `at` to the end; past `L` always silence.
        let cursor = RunSpec {
            mute_end: None,
            end: None,
            hear_original: true,
            doc_len: 1_000,
            at: 500,
            ..spec
        };
        let out = render(&cursor, 900, 200);
        assert!(out[100..].iter().all(|&x| x == 0.0), "q ≥ L is silence");
    }

    /// §4.3: the take index of a capture time comes from the block containing it; negative before
    /// the take; `None` until a block reaches it.
    #[test]
    fn take_index_comes_from_the_containing_block() {
        let ms = 1_000_000u64;
        let blocks: VecDeque<CaptureBlock> = [
            CaptureBlock {
                end_take: 480,
                frames: 480,
                start_ns: 100 * ms,
            },
            CaptureBlock {
                end_take: 1_440,
                frames: 480,
                start_ns: 120 * ms, // a 10 ms dropout filled with 480 frames before it
            },
        ]
        .into_iter()
        .collect();
        assert_eq!(take_index_at(&blocks, i128::from(105 * ms), R), Some(240));
        assert_eq!(take_index_at(&blocks, i128::from(121 * ms), R), Some(1_008));
        assert_eq!(take_index_at(&blocks, i128::from(99 * ms), R), Some(-48));
        assert_eq!(take_index_at(&blocks, i128::from(130 * ms), R), None);
    }
}
