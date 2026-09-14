//! Edits and the undo/redo stacks of snapshots (ADR-004 §3–§4, Amendment 1).

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::snapshot::{DocSnapshot, Marker, MarkerId, Piece, Source, shift_markers, sort_markers};
use crate::{CHUNK_SAMPLES, ProjectError, Result};

/// How a [`EditOp::Replace`] moves markers (SPEC-008 §4.1 op table). Recorded in the journal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MarkerMapping {
    /// The SPEC-008 §4.2 position mapping ([`shift_markers`]): cut, delete, paste, insert
    /// silence, trim, record.
    #[default]
    Shift,
    /// No marker moves: length-preserving ops (Silence, Normalize). The op must insert exactly as
    /// many samples as it removes.
    Identity,
}

/// A change to the piece table. Ops of one edit apply in order, each to the result of the
/// previous one, and each maps the markers by its [`MarkerMapping`].
#[derive(Clone, Debug, PartialEq)]
pub enum EditOp {
    /// Replace `[at, at + remove_len)` with `pieces` (insert: `remove_len = 0`; delete: no
    /// pieces). This is the journal's piece-table delta (ADR-004 §6).
    Replace {
        at: u64,
        remove_len: u64,
        pieces: Vec<Piece>,
        mapping: MarkerMapping,
    },
}

impl EditOp {
    fn changes_audio(&self) -> bool {
        let EditOp::Replace {
            remove_len, pieces, ..
        } = self;
        *remove_len > 0 || pieces.iter().any(|p| p.len > 0)
    }

    fn at(&self) -> u64 {
        let EditOp::Replace { at, .. } = self;
        *at
    }
}

/// A marker change. Marker ops apply after all [`EditOp`]s, in post-edit document time.
#[derive(Clone, Debug, PartialEq)]
pub enum MarkerOp {
    Add(Marker),
    Remove(MarkerId),
    Rename {
        id: MarkerId,
        name: Arc<str>,
    },
    Move {
        id: MarkerId,
        pos_samples: u64,
        len_samples: u64,
    },
}

/// Placeholder values of an undo label (ADR-004 Amendment 3): `history.normalize` +
/// `{target: "−1"}` renders "Normalize to −1 dB". Journaled verbatim with the edit.
pub type LabelParams = BTreeMap<String, String>;

/// One undoable change: piece-table ops, marker ops, an i18n label key for "Undo ‹label›" (plus
/// its [`LabelParams`]) and the optional opaque attachment (ADR-004 Amendment 1: the engine's
/// serialized pre-bake rack). `project` stores the attachment verbatim and never interprets it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edit {
    pub label_key: String,
    /// ADR-004 Amendment 3: the label's placeholder values.
    pub label_params: LabelParams,
    pub ops: Vec<EditOp>,
    pub marker_ops: Vec<MarkerOp>,
    pub attachment: Option<Vec<u8>>,
}

impl Edit {
    /// An empty edit with a label key (e.g. `history.cut`).
    pub fn new(label_key: impl Into<String>) -> Self {
        Edit {
            label_key: label_key.into(),
            ..Edit::default()
        }
    }

    /// Adds a [`EditOp::Replace`] with the [`MarkerMapping::Shift`] marker mapping.
    pub fn replace(self, at: u64, remove_len: u64, pieces: Vec<Piece>) -> Self {
        self.replace_with(at, remove_len, pieces, MarkerMapping::Shift)
    }

    /// Adds a [`EditOp::Replace`] with an explicit marker mapping.
    pub fn replace_with(
        mut self,
        at: u64,
        remove_len: u64,
        pieces: Vec<Piece>,
        mapping: MarkerMapping,
    ) -> Self {
        self.ops.push(EditOp::Replace {
            at,
            remove_len,
            pieces,
            mapping,
        });
        self
    }

    /// Adds a marker op.
    pub fn marker(mut self, op: MarkerOp) -> Self {
        self.marker_ops.push(op);
        self
    }

    /// Adds one label placeholder value (ADR-004 Amendment 3).
    pub fn with_label_param(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.label_params.insert(name.into(), value.into());
        self
    }

    /// Sets the opaque attachment.
    pub fn with_attachment(mut self, attachment: Vec<u8>) -> Self {
        self.attachment = Some(attachment);
        self
    }

    /// `true` if the edit changes samples (a destructive edit that stops playback).
    pub fn changes_audio(&self) -> bool {
        self.ops.iter().any(EditOp::changes_audio)
    }

    /// The earliest `at` over this edit's `Replace` ops (SPEC-008 §2.3's post-undo/redo cursor
    /// rule); `None` for a marker-only edit (no `Replace` ops).
    pub fn first_at(&self) -> Option<u64> {
        self.ops.iter().map(EditOp::at).min()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    /// On the undo stack: the state before the edit. On the redo stack: the state after it.
    pub(crate) snapshot: Arc<DocSnapshot>,
    pub(crate) seq: u64,
    pub(crate) label_key: Arc<str>,
    pub(crate) label_params: Arc<LabelParams>,
    pub(crate) attachment: Option<Arc<[u8]>>,
    pub(crate) audio: bool,
    pub(crate) first_at: Option<u64>,
}

/// The result of a commit, undo or redo.
#[derive(Clone, Debug)]
pub struct HistoryStep {
    /// The new current snapshot.
    pub snapshot: Arc<DocSnapshot>,
    /// Sequence number of the edit that was committed, undone or redone.
    pub seq: u64,
    /// Its label key.
    pub label_key: Arc<str>,
    /// Its label's placeholder values (ADR-004 Amendment 3).
    pub label_params: Arc<LabelParams>,
    /// Its attachment, verbatim.
    pub attachment: Option<Arc<[u8]>>,
    /// `true` if samples changed: the engine must stop playback first (ADR-004 §3).
    pub audio_changed: bool,
    /// The edit's [`Edit::first_at`] (SPEC-008 §2.3's post-undo/redo cursor rule). Undoing and
    /// redoing the same edit report the same value: the document round-trips to the same shape
    /// at that position either way.
    pub first_at: Option<u64>,
}

pub(crate) struct PreparedEdit {
    snapshot: DocSnapshot,
    seq: u64,
    label_key: Arc<str>,
    label_params: Arc<LabelParams>,
    attachment: Option<Arc<[u8]>>,
    audio: bool,
    first_at: Option<u64>,
    next_marker_id: u64,
    base_rev: u64,
}

impl PreparedEdit {
    pub(crate) fn seq(&self) -> u64 {
        self.seq
    }
}

/// The current snapshot plus unbounded undo/redo stacks of snapshots (ADR-004 §4).
///
/// Every edit gets a fresh sequence number `seq`. The *current seq* is the seq of the edit that
/// produced the current state (0 at the undo floor), so undoing back to a saved state makes the
/// document clean again (SPEC-004 AC-8).
#[derive(Debug)]
pub struct History {
    current: Arc<DocSnapshot>,
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    next_rev: u64,
    next_audio_rev: u64,
    next_seq: u64,
    saved_seq: u64,
    next_marker_id: u64,
}

impl History {
    /// A history whose undo floor is `floor` (imported file or empty recording). The floor gets
    /// `rev = audio_rev = 1`.
    pub fn new(floor: DocSnapshot) -> Self {
        let next_marker_id = floor.markers.iter().map(|m| m.id.0 + 1).max().unwrap_or(1);
        History {
            current: Arc::new(DocSnapshot {
                rev: 1,
                audio_rev: 1,
                ..floor
            }),
            undo: Vec::new(),
            redo: Vec::new(),
            next_rev: 2,
            next_audio_rev: 2,
            next_seq: 1,
            saved_seq: 0,
            next_marker_id,
        }
    }

    /// A fresh history (empty stacks, clean) with `floor` as its undo floor, continuing this
    /// history's `rev`/`audio_rev`/`seq`/marker-id counters so no value is ever reused.
    pub(crate) fn rebased(&self, floor: DocSnapshot) -> History {
        let floor_marker_id = floor.markers.iter().map(|m| m.id.0 + 1).max().unwrap_or(1);
        History {
            current: Arc::new(DocSnapshot {
                rev: self.next_rev,
                audio_rev: self.next_audio_rev,
                ..floor
            }),
            undo: Vec::new(),
            redo: Vec::new(),
            next_rev: self.next_rev + 1,
            next_audio_rev: self.next_audio_rev + 1,
            next_seq: self.next_seq,
            saved_seq: 0,
            next_marker_id: self.next_marker_id.max(floor_marker_id),
        }
    }

    /// The current snapshot.
    pub fn current(&self) -> &Arc<DocSnapshot> {
        &self.current
    }

    /// Number of undoable entries.
    pub fn undo_depth(&self) -> usize {
        self.undo.len()
    }

    /// Number of redoable entries.
    pub fn redo_depth(&self) -> usize {
        self.redo.len()
    }

    /// Label key of the entry Undo would revert.
    pub fn undo_label(&self) -> Option<&str> {
        self.undo.last().map(|e| &*e.label_key)
    }

    /// Label key of the entry Redo would re-apply.
    pub fn redo_label(&self) -> Option<&str> {
        self.redo.last().map(|e| &*e.label_key)
    }

    /// Placeholder values of [`Self::undo_label`] (ADR-004 Amendment 3).
    pub fn undo_label_params(&self) -> Option<&LabelParams> {
        self.undo.last().map(|e| &*e.label_params)
    }

    /// Placeholder values of [`Self::redo_label`].
    pub fn redo_label_params(&self) -> Option<&LabelParams> {
        self.redo.last().map(|e| &*e.label_params)
    }

    /// Labels (key + params) of the undo stack, oldest first.
    pub fn undo_labels(&self) -> Vec<(&str, &LabelParams)> {
        self.undo
            .iter()
            .map(|e| (&*e.label_key, &*e.label_params))
            .collect()
    }

    /// Labels (key + params) of the redo stack, oldest (bottom) first.
    pub fn redo_labels(&self) -> Vec<(&str, &LabelParams)> {
        self.redo
            .iter()
            .map(|e| (&*e.label_key, &*e.label_params))
            .collect()
    }

    /// Seq of the edit that produced the current state (0 at the floor).
    pub fn current_seq(&self) -> u64 {
        self.undo.last().map_or(0, |e| e.seq)
    }

    /// Current seq at the last save (0 = the floor).
    pub fn saved_seq(&self) -> u64 {
        self.saved_seq
    }

    /// `true` while the document differs from the last saved state.
    pub fn is_dirty(&self) -> bool {
        self.current_seq() != self.saved_seq
    }

    /// Records that the current state was saved.
    pub fn mark_saved(&mut self) {
        self.saved_seq = self.current_seq();
    }

    /// A marker id no marker has used yet.
    pub fn allocate_marker_id(&mut self) -> MarkerId {
        let id = MarkerId(self.next_marker_id);
        self.next_marker_id += 1;
        id
    }

    /// Applies an edit in memory (no journal): pushes the current state onto the undo stack and
    /// clears redo.
    pub fn apply(&mut self, edit: &Edit) -> Result<HistoryStep> {
        let prepared = self.prepare(edit)?;
        Ok(self.commit(prepared))
    }

    /// Reverts the top undo entry; `None` at the undo floor.
    pub fn undo(&mut self) -> Option<HistoryStep> {
        let entry = self.undo.pop()?;
        let restamped = self.restamp(&entry.snapshot, entry.audio);
        let after = std::mem::replace(&mut self.current, Arc::new(restamped));
        let step = Self::step(&self.current, &entry);
        self.redo.push(Entry {
            snapshot: after,
            ..entry
        });
        Some(step)
    }

    /// Re-applies the top redo entry; `None` if the redo stack is empty.
    pub fn redo(&mut self) -> Option<HistoryStep> {
        let entry = self.redo.pop()?;
        let restamped = self.restamp(&entry.snapshot, entry.audio);
        let before = std::mem::replace(&mut self.current, Arc::new(restamped));
        let step = Self::step(&self.current, &entry);
        self.undo.push(Entry {
            snapshot: before,
            ..entry
        });
        Some(step)
    }

    pub(crate) fn peek_undo_seq(&self) -> Option<u64> {
        self.undo.last().map(|e| e.seq)
    }

    pub(crate) fn peek_redo_seq(&self) -> Option<u64> {
        self.redo.last().map(|e| e.seq)
    }

    pub(crate) fn undo_entries(&self) -> &[Entry] {
        &self.undo
    }

    pub(crate) fn redo_entries(&self) -> &[Entry] {
        &self.redo
    }

    pub(crate) fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub(crate) fn next_rev(&self) -> u64 {
        self.next_rev
    }

    pub(crate) fn next_audio_rev(&self) -> u64 {
        self.next_audio_rev
    }

    pub(crate) fn next_marker_id(&self) -> u64 {
        self.next_marker_id
    }

    /// A history rebuilt from a checkpoint (recovery, ADR-004 §9): `current` keeps its stamped
    /// `rev`/`audio_rev`; the counters continue exactly where the checkpointed session was.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        current: DocSnapshot,
        undo: Vec<Entry>,
        redo: Vec<Entry>,
        next_rev: u64,
        next_audio_rev: u64,
        next_seq: u64,
        saved_seq: u64,
        next_marker_id: u64,
    ) -> History {
        let floor_marker_id = std::iter::once(&current)
            .chain(undo.iter().chain(&redo).map(|e| &*e.snapshot))
            .flat_map(|s| s.markers.iter())
            .map(|m| m.id.0 + 1)
            .max()
            .unwrap_or(1);
        let max_rev = std::iter::once(&current)
            .chain(undo.iter().chain(&redo).map(|e| &*e.snapshot))
            .map(|s| (s.rev, s.audio_rev))
            .fold((0, 0), |(r, a), (sr, sa)| (r.max(sr), a.max(sa)));
        let max_seq = undo.iter().chain(&redo).map(|e| e.seq).max().unwrap_or(0);
        History {
            current: Arc::new(current),
            undo,
            redo,
            // Older checkpoints may lack the rev counters (`serde(default)` = 0).
            next_rev: next_rev.max(max_rev.0 + 1),
            next_audio_rev: next_audio_rev.max(max_rev.1 + 1),
            next_seq: next_seq.max(max_seq + 1),
            saved_seq,
            next_marker_id: next_marker_id.max(floor_marker_id),
        }
    }

    /// Sets the saved seq (journal `saved` replay).
    pub(crate) fn set_saved_seq(&mut self, seq: u64) {
        self.saved_seq = seq;
    }

    /// Removes the `count` oldest undo entries (SPEC-004 §2.5 OD-1 = A): the oldest remaining
    /// entry's "before" state becomes the new undo floor. The current state and the redo stack
    /// are never touched. Returns how many were removed.
    pub(crate) fn drop_oldest_undo(&mut self, count: usize) -> usize {
        let n = count.min(self.undo.len());
        self.undo.drain(..n);
        n
    }

    fn step(current: &Arc<DocSnapshot>, entry: &Entry) -> HistoryStep {
        HistoryStep {
            snapshot: Arc::clone(current),
            seq: entry.seq,
            label_key: Arc::clone(&entry.label_key),
            label_params: Arc::clone(&entry.label_params),
            attachment: entry.attachment.clone(),
            audio_changed: entry.audio,
            first_at: entry.first_at,
        }
    }

    /// `target` with a fresh `rev`, and a fresh `audio_rev` only if samples change.
    fn restamp(&mut self, target: &DocSnapshot, audio: bool) -> DocSnapshot {
        let rev = self.next_rev;
        self.next_rev += 1;
        let audio_rev = if audio {
            let a = self.next_audio_rev;
            self.next_audio_rev += 1;
            a
        } else {
            self.current.audio_rev
        };
        DocSnapshot {
            rev,
            audio_rev,
            ..target.clone()
        }
    }

    /// Validates `edit` against the current snapshot and builds the resulting snapshot without
    /// changing anything (so the caller can journal it first).
    pub(crate) fn prepare(&self, edit: &Edit) -> Result<PreparedEdit> {
        let cur = &self.current;
        let mut work: Option<DocSnapshot> = None;
        let mut markers: Vec<Marker> = cur.markers.to_vec();
        for op in &edit.ops {
            let EditOp::Replace {
                at,
                remove_len,
                pieces,
                mapping,
            } = op;
            validate_pieces(pieces)?;
            if !op.changes_audio() {
                continue;
            }
            let insert_len: u64 = pieces.iter().map(Piece::len_samples).sum();
            let base: &DocSnapshot = work.as_ref().unwrap_or(cur);
            // T-304 (SPEC-022 §2.5, §2.9): an Overwrite that runs past the end replaces
            // `[at, L)` and grows the document; no marker lies after the replaced range, so
            // keeping every marker in place is still exact.
            let grows_at_end =
                insert_len > *remove_len && at.saturating_add(*remove_len) == base.len_samples;
            if *mapping == MarkerMapping::Identity && insert_len != *remove_len && !grows_at_end {
                return Err(ProjectError::InvalidEdit(format!(
                    "identity marker mapping needs a length-preserving op or growth at the end \
                     (removes {remove_len}, inserts {insert_len})"
                )));
            }
            let new_pieces = base.replaced(*at, *remove_len, pieces)?;
            if *mapping == MarkerMapping::Shift {
                markers = shift_markers(&markers, *at, *remove_len, insert_len);
            }
            work = Some(DocSnapshot::from_normalized(
                cur.sample_rate_hz,
                new_pieces,
                Vec::new(),
                0,
                0,
            ));
        }
        let audio = work.is_some();
        let len = work.as_ref().map_or(cur.len_samples, |w| w.len_samples);
        let mut next_marker_id = self.next_marker_id;
        for op in &edit.marker_ops {
            let find = |markers: &[Marker], id: MarkerId| {
                markers
                    .iter()
                    .position(|m| m.id == id)
                    .ok_or_else(|| ProjectError::InvalidEdit(format!("no marker {}", id.0)))
            };
            match op {
                MarkerOp::Add(marker) => {
                    if markers.iter().any(|m| m.id == marker.id) {
                        return Err(ProjectError::InvalidEdit(format!(
                            "marker {} already exists",
                            marker.id.0
                        )));
                    }
                    check_marker_bounds(marker.pos_samples, marker.len_samples, len)?;
                    next_marker_id = next_marker_id.max(marker.id.0.saturating_add(1));
                    markers.push(marker.clone());
                }
                MarkerOp::Remove(id) => {
                    let i = find(&markers, *id)?;
                    markers.remove(i);
                }
                MarkerOp::Rename { id, name } => {
                    let i = find(&markers, *id)?;
                    markers[i].name = Arc::clone(name);
                }
                MarkerOp::Move {
                    id,
                    pos_samples,
                    len_samples,
                } => {
                    let i = find(&markers, *id)?;
                    check_marker_bounds(*pos_samples, *len_samples, len)?;
                    markers[i].pos_samples = *pos_samples;
                    markers[i].len_samples = *len_samples;
                }
            }
        }
        if !audio && edit.marker_ops.is_empty() {
            return Err(ProjectError::InvalidEdit("the edit changes nothing".into()));
        }
        sort_markers(&mut markers);
        let (pieces, starts, len_samples) = match work {
            Some(w) => (w.pieces, w.starts, w.len_samples),
            // A marker-only edit shares the piece table (ADR-004 §3).
            None => (
                Arc::clone(&cur.pieces),
                Arc::clone(&cur.starts),
                cur.len_samples,
            ),
        };
        let snapshot = DocSnapshot {
            rev: self.next_rev,
            audio_rev: if audio {
                self.next_audio_rev
            } else {
                cur.audio_rev
            },
            sample_rate_hz: cur.sample_rate_hz,
            pieces,
            starts,
            len_samples,
            markers: markers.into(),
        };
        Ok(PreparedEdit {
            snapshot,
            seq: self.next_seq,
            label_key: edit.label_key.as_str().into(),
            label_params: Arc::new(edit.label_params.clone()),
            attachment: edit.attachment.as_deref().map(Arc::from),
            audio,
            first_at: edit.first_at(),
            next_marker_id,
            base_rev: cur.rev,
        })
    }

    /// Commits a [`PreparedEdit`] built from the current state.
    pub(crate) fn commit(&mut self, prepared: PreparedEdit) -> HistoryStep {
        debug_assert_eq!(
            prepared.base_rev, self.current.rev,
            "prepared edit is stale"
        );
        let previous = std::mem::replace(&mut self.current, Arc::new(prepared.snapshot));
        self.undo.push(Entry {
            snapshot: previous,
            seq: prepared.seq,
            label_key: Arc::clone(&prepared.label_key),
            label_params: Arc::clone(&prepared.label_params),
            attachment: prepared.attachment.clone(),
            audio: prepared.audio,
            first_at: prepared.first_at,
        });
        self.redo.clear();
        self.next_rev += 1;
        if prepared.audio {
            self.next_audio_rev += 1;
        }
        self.next_seq += 1;
        self.next_marker_id = prepared.next_marker_id;
        HistoryStep {
            snapshot: Arc::clone(&self.current),
            seq: prepared.seq,
            label_key: prepared.label_key,
            label_params: prepared.label_params,
            attachment: prepared.attachment,
            audio_changed: prepared.audio,
            first_at: prepared.first_at,
        }
    }
}

fn validate_pieces(pieces: &[Piece]) -> Result<()> {
    for piece in pieces {
        let ok = match piece.source {
            Source::Chunk(_) => piece.offset as usize + piece.len as usize <= CHUNK_SAMPLES,
            Source::Silence => piece.offset == 0,
        };
        if !ok {
            return Err(ProjectError::InvalidEdit(format!(
                "invalid piece {piece:?}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn check_marker_bounds(pos: u64, len: u64, doc_len: u64) -> Result<()> {
    match pos.checked_add(len) {
        Some(end) if end <= doc_len => Ok(()),
        _ => Err(ProjectError::InvalidEdit(format!(
            "marker {pos}+{len} lies outside the document (length {doc_len})"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(len: u32) -> Vec<Piece> {
        vec![Piece::chunk(0, 0, len)]
    }

    #[test]
    fn undo_redo_and_redo_truncation() {
        let mut h = History::new(DocSnapshot::empty(48_000));
        let floor_rev = h.current().rev;
        let a = h
            .apply(&Edit::new("history.record").replace(0, 0, audio(100)))
            .unwrap();
        assert!(a.audio_changed);
        assert_eq!(h.current().len_samples, 100);
        let b = h
            .apply(
                &Edit::new("history.marker_add").marker(MarkerOp::Add(Marker::new(
                    MarkerId(1),
                    10,
                    0,
                    "m",
                ))),
            )
            .unwrap();
        assert!(!b.audio_changed);
        assert!(Arc::ptr_eq(&a.snapshot.pieces, &b.snapshot.pieces));
        assert_eq!(a.snapshot.audio_rev, b.snapshot.audio_rev);
        assert_ne!(a.snapshot.rev, b.snapshot.rev);
        assert_eq!((h.undo_depth(), h.redo_depth()), (2, 0));
        assert_eq!(h.undo_label(), Some("history.marker_add"));

        let u = h.undo().unwrap();
        assert!(!u.audio_changed);
        assert_eq!(u.snapshot.audio_rev, b.snapshot.audio_rev);
        assert!(u.snapshot.rev > b.snapshot.rev, "rev is monotonic");
        assert_eq!(h.current().markers.len(), 0);
        let u = h.undo().unwrap();
        assert!(u.audio_changed);
        assert_eq!(h.current().len_samples, 0);
        assert_ne!(h.current().rev, floor_rev);
        assert!(h.undo().is_none());
        assert_eq!(h.redo_label(), Some("history.record"));

        h.redo().unwrap();
        assert_eq!(h.current().len_samples, 100);
        // A new edit clears redo.
        h.apply(&Edit::new("history.cut").replace(0, 50, Vec::new()))
            .unwrap();
        assert_eq!(h.redo_depth(), 0);
        assert!(h.redo().is_none());
    }

    #[test]
    fn dirty_follows_the_saved_state() {
        let mut h = History::new(DocSnapshot::empty(48_000));
        assert!(!h.is_dirty());
        h.apply(&Edit::new("x").replace(0, 0, audio(10))).unwrap();
        assert!(h.is_dirty());
        h.mark_saved();
        assert!(!h.is_dirty());
        h.undo();
        assert!(h.is_dirty());
        h.redo();
        assert!(!h.is_dirty(), "redo back to the saved state is clean");
    }

    #[test]
    fn invalid_edits_change_nothing() {
        let mut h = History::new(DocSnapshot::empty(48_000));
        let rev = h.current().rev;
        assert!(h.apply(&Edit::new("x")).is_err());
        assert!(h.apply(&Edit::new("x").replace(1, 0, audio(10))).is_err());
        assert!(
            h.apply(&Edit::new("x").replace(0, 0, vec![Piece::chunk(0, 65_000, 1000)]))
                .is_err()
        );
        assert!(
            h.apply(&Edit::new("x").marker(MarkerOp::Add(Marker::new(MarkerId(1), 1, 0, "m"))))
                .is_err(),
            "marker past the end"
        );
        assert!(
            h.apply(&Edit::new("x").marker(MarkerOp::Remove(MarkerId(9))))
                .is_err()
        );
        assert_eq!(h.current().rev, rev);
        assert_eq!(h.undo_depth(), 0);

        let mut h = History::new(DocSnapshot::new(48_000, audio(20), Vec::new()));
        assert!(
            h.apply(&Edit::new("x").replace_with(5, 2, audio(10), MarkerMapping::Identity))
                .is_err(),
            "identity mapping on a length-changing op inside the document"
        );
        assert_eq!(h.undo_depth(), 0);
        assert!(
            h.apply(&Edit::new("x").replace_with(15, 5, audio(10), MarkerMapping::Identity))
                .is_ok(),
            "T-304: growth at the end (Overwrite past L) keeps markers in place"
        );
    }

    #[test]
    fn attachment_is_returned_verbatim() {
        let mut h = History::new(DocSnapshot::empty(48_000));
        let blob = vec![0u8, 1, 2, 255];
        let step = h
            .apply(
                &Edit::new("history.bake")
                    .replace(0, 0, audio(10))
                    .with_attachment(blob.clone()),
            )
            .unwrap();
        assert_eq!(step.attachment.as_deref(), Some(&blob[..]));
        assert_eq!(h.undo().unwrap().attachment.as_deref(), Some(&blob[..]));
        assert_eq!(h.redo().unwrap().attachment.as_deref(), Some(&blob[..]));
    }

    #[test]
    fn rebased_floor_continues_the_counters() {
        let mut h = History::new(DocSnapshot::empty(48_000));
        h.apply(&Edit::new("x").replace(0, 0, audio(10))).unwrap();
        let floor = DocSnapshot::new(48_000, audio(20), Vec::new());
        let r = h.rebased(floor);
        assert!(r.current().rev > h.current().rev);
        assert!(r.current().audio_rev > h.current().audio_rev);
        assert_eq!((r.undo_depth(), r.redo_depth()), (0, 0));
        assert!(!r.is_dirty());
        assert_eq!(r.next_seq(), h.next_seq());
    }

    /// SPEC-008 AC-7 (marker mapping) on a 20 s document with points at 1, 3, 5, 10, 20 s and a
    /// region [6 s, 8 s); every row is exact in samples and undo restores the original list.
    #[test]
    fn spec_008_ac7_marker_mapping_table() {
        const S: u64 = 48_000;
        let half = S / 2;
        let silence = |len: u64| Piece::silence_run(len);
        let original: Vec<Marker> = [1, 3, 5, 10, 20]
            .iter()
            .enumerate()
            .map(|(i, &s)| Marker::new(MarkerId(i as u64 + 1), s * S, 0, format!("m{i}")))
            .chain([Marker::new(MarkerId(6), 6 * S, 2 * S, "region")])
            .collect();
        /// (name, edit, point positions of ids 1..=5, region (pos, len) of id 6)
        type Row = (&'static str, Edit, [u64; 5], (u64, u64));
        let rows: Vec<Row> = vec![
            (
                "cut [2,4)",
                Edit::new("c").replace(2 * S, 2 * S, vec![]),
                [S, 2 * S, 3 * S, 8 * S, 18 * S],
                (4 * S, 2 * S),
            ),
            (
                "paste 1.5 s at 5",
                Edit::new("c").replace(5 * S, 0, silence(3 * half)),
                [S, 3 * S, 6 * S + half, 11 * S + half, 21 * S + half],
                (7 * S + half, 2 * S),
            ),
            (
                "paste 1 s over [2,4)",
                Edit::new("c").replace(2 * S, 2 * S, silence(S)),
                [S, 2 * S, 4 * S, 9 * S, 19 * S],
                (5 * S, 2 * S),
            ),
            (
                "insert 1 s at 1",
                Edit::new("c").replace(S, 0, silence(S)),
                [2 * S, 4 * S, 6 * S, 11 * S, 21 * S],
                (7 * S, 2 * S),
            ),
            (
                "insert 1 s at 8 (region end)",
                Edit::new("c").replace(8 * S, 0, silence(S)),
                [S, 3 * S, 5 * S, 11 * S, 21 * S],
                (6 * S, 2 * S),
            ),
            (
                "insert 1 s at 7 (inside region)",
                Edit::new("c").replace(7 * S, 0, silence(S)),
                [S, 3 * S, 5 * S, 11 * S, 21 * S],
                (6 * S, 3 * S),
            ),
            (
                "trim [4,12)",
                Edit::new("c")
                    .replace(12 * S, 8 * S, vec![])
                    .replace(0, 4 * S, vec![]),
                [0, 0, S, 6 * S, 8 * S],
                (2 * S, 2 * S),
            ),
            (
                "delete [5.5,9)",
                Edit::new("c").replace(5 * S + half, 3 * S + half, vec![]),
                [S, 3 * S, 5 * S, 6 * S + half, 16 * S + half],
                (5 * S + half, 0),
            ),
            (
                "silence [2,12)",
                Edit::new("c").replace_with(
                    2 * S,
                    10 * S,
                    silence(10 * S),
                    MarkerMapping::Identity,
                ),
                [S, 3 * S, 5 * S, 10 * S, 20 * S],
                (6 * S, 2 * S),
            ),
        ];
        for (name, edit, points, region) in rows {
            let mut h = History::new(DocSnapshot::new(48_000, silence(20 * S), original.clone()));
            h.apply(&edit).unwrap();
            let cur = h.current();
            let got: Vec<u64> = (1..=5)
                .map(|id| cur.marker(MarkerId(id)).unwrap().pos_samples)
                .collect();
            assert_eq!(got, points, "{name}");
            let r = cur.marker(MarkerId(6)).unwrap();
            assert_eq!((r.pos_samples, r.len_samples), region, "{name}");
            assert_eq!(cur.markers.len(), 6, "{name}: no marker deleted");
            h.undo().unwrap();
            let mut restored = h.current().markers.to_vec();
            let mut expected = original.clone();
            sort_markers(&mut restored);
            sort_markers(&mut expected);
            assert_eq!(restored, expected, "{name}: undo");
        }
    }
}
