//! Shared scripted-session helpers for the T-301 recovery and crash tests.

use std::path::{Path, PathBuf};

use vox_project::{
    Edit, EditTarget, LabelParams, Marker, MarkerOp, NormalizeResult, Piece, Session,
    SessionConfig, StoreOptions, edit, normalize_peak, validate_range,
};

use super::{Fnv, MIB, RATE, read_all, write_audio};

pub fn options() -> StoreOptions {
    StoreOptions::with_memory_budget(256 * MIB)
}

pub fn new_session(sessions: &Path) -> Session {
    Session::create(
        sessions,
        SessionConfig {
            sample_rate_hz: RATE,
            source: None,
            store: options(),
        },
    )
    .unwrap()
}

pub fn set_floor(session: &mut Session, samples: &[f32]) {
    let audio = write_audio(session.store(), samples);
    session.set_floor(&audio, Vec::new()).unwrap();
}

pub fn hash_of(samples: &[f32]) -> u64 {
    let mut fnv = Fnv::new();
    fnv.update(samples);
    fnv.finish()
}

/// Everything AC-9 compares: the audio hash, the marker list, undo/redo depths and entry labels
/// (with their params), and the modified flag — as one line.
pub fn fingerprint(s: &Session) -> String {
    let snap = s.current();
    let hash = hash_of(&read_all(s.store(), &snap));
    let markers: Vec<String> = snap
        .markers
        .iter()
        .map(|m| format!("{}@{}+{}:{}", m.id.0, m.pos_samples, m.len_samples, m.name))
        .collect();
    let h = s.history();
    let labels = |v: Vec<(&str, &LabelParams)>| {
        v.iter()
            .map(|(k, p)| format!("{k}{p:?}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "hash={hash:016x} len={} markers=[{}] undo={} [{}] redo={} [{}] dirty={}",
        snap.len_samples,
        markers.join(";"),
        h.undo_depth(),
        labels(h.undo_labels()),
        h.redo_depth(),
        labels(h.redo_labels()),
        s.is_dirty()
    )
}

/// xorshift64: deterministic, dependency-free test randomness.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
}

pub fn random_range(rng: &mut Rng, len: u64) -> Option<vox_project::Range> {
    if len < 2 {
        return None;
    }
    let a = rng.below(len - 1);
    let b = a + 1 + rng.below((len - a).min(96_000));
    validate_range(a, b.min(len), len).ok()
}

/// Performs one random document command that succeeds (cut, paste, delete, silence, insert
/// silence, normalize, marker add/rename/move/delete, undo, redo) and returns its name.
pub fn random_op(s: &mut Session, rng: &mut Rng, clip: &mut Vec<Piece>) -> &'static str {
    loop {
        let len = s.current().len_samples;
        let done: Option<&'static str> = match rng.below(11) {
            0 => random_range(rng, len).and_then(|r| {
                let (e, c) = edit::cut(&s.current(), r).unwrap();
                s.commit_edit(e).ok().map(|_| {
                    *clip = c;
                    "cut"
                })
            }),
            1 if !clip.is_empty() => {
                let at = rng.below(len + 1);
                s.commit_edit(edit::paste(clip, EditTarget::Cursor(at)))
                    .ok()
                    .map(|_| "paste")
            }
            2 => random_range(rng, len)
                .and_then(|r| s.commit_edit(edit::delete(r)).ok().map(|_| "delete")),
            3 => random_range(rng, len)
                .and_then(|r| s.commit_edit(edit::silence(r)).ok().map(|_| "silence")),
            4 => {
                let at = rng.below(len + 1);
                let n = 1 + rng.below(48_000);
                s.commit_edit(Edit::new("history.insert_silence").replace(
                    at,
                    0,
                    Piece::silence_run(n),
                ))
                .ok()
                .map(|_| "insert_silence")
            }
            5 => random_range(rng, len).and_then(|r| {
                let target = -1.0 - rng.below(4) as f64 * 0.5;
                match normalize_peak(s, r, target).unwrap() {
                    NormalizeResult::Applied(_) => Some("normalize"),
                    _ => None,
                }
            }),
            6 => {
                let id = s.new_marker_id();
                let pos = rng.below(len + 1);
                let edit = Edit::new("history.marker_add").marker(MarkerOp::Add(Marker::new(
                    id,
                    pos,
                    0,
                    format!("m{}", id.0),
                )));
                s.commit_edit(edit).ok().map(|_| "marker_add")
            }
            7 => {
                let markers = s.current().markers.clone();
                if markers.is_empty() {
                    None
                } else {
                    let m = &markers[rng.below(markers.len() as u64) as usize];
                    let op = match rng.below(3) {
                        0 => MarkerOp::Rename {
                            id: m.id,
                            name: format!("r{}", rng.below(1000)).into(),
                        },
                        1 => MarkerOp::Move {
                            id: m.id,
                            pos_samples: rng.below(len + 1),
                            len_samples: 0,
                        },
                        _ => MarkerOp::Remove(m.id),
                    };
                    s.commit_edit(Edit::new("history.marker_edit").marker(op))
                        .ok()
                        .map(|_| "marker_edit")
                }
            }
            8 | 9 => s.undo().unwrap().map(|_| "undo"),
            _ => s.redo().unwrap().map(|_| "redo"),
        };
        if let Some(name) = done {
            return name;
        }
    }
}

pub fn only_session(dir: &Path) -> PathBuf {
    let sessions: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    sessions.into_iter().next().unwrap()
}
