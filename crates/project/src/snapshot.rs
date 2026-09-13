//! Snapshots, the piece table and markers (ADR-004 §3).

use std::sync::Arc;

use crate::store::ChunkId;
use crate::{ProjectError, Result};

/// Where a piece's samples come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A committed chunk in the store.
    Chunk(ChunkId),
    /// Digital silence; no storage at all.
    Silence,
}

/// A run of `len` samples: `[offset, offset + len)` of a chunk, or `len` samples of silence
/// (`offset` is always 0 for silence).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Piece {
    pub source: Source,
    pub offset: u32,
    pub len: u32,
}

impl Piece {
    /// Samples `[offset, offset + len)` of chunk `id`.
    pub fn chunk(id: ChunkId, offset: u32, len: u32) -> Piece {
        Piece {
            source: Source::Chunk(id),
            offset,
            len,
        }
    }

    /// `len` samples of silence.
    pub fn silence(len: u32) -> Piece {
        Piece {
            source: Source::Silence,
            offset: 0,
            len,
        }
    }

    /// Silence of any length, split into pieces of at most `u32::MAX` samples.
    pub fn silence_run(len_samples: u64) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut rest = len_samples;
        while rest > 0 {
            let n = rest.min(u64::from(u32::MAX)) as u32;
            pieces.push(Piece::silence(n));
            rest -= u64::from(n);
        }
        pieces
    }

    /// Length in samples.
    pub fn len_samples(&self) -> u64 {
        u64::from(self.len)
    }

    fn sub(&self, from: u32, len: u32) -> Piece {
        match self.source {
            Source::Chunk(_) => Piece {
                offset: self.offset + from,
                len,
                ..*self
            },
            Source::Silence => Piece::silence(len),
        }
    }

    fn merged(&self, next: &Piece) -> Option<Piece> {
        match (self.source, next.source) {
            (Source::Chunk(a), Source::Chunk(b))
                if a == b && self.offset.checked_add(self.len) == Some(next.offset) =>
            {
                Some(Piece {
                    len: self.len.checked_add(next.len)?,
                    ..*self
                })
            }
            (Source::Silence, Source::Silence) => {
                Some(Piece::silence(self.len.checked_add(next.len)?))
            }
            _ => None,
        }
    }
}

/// Appends `piece`, dropping empty pieces and merging it into the previous piece when both are
/// adjacent ranges of the same chunk or both silence. Keeps piece tables canonical.
pub(crate) fn push_piece(out: &mut Vec<Piece>, piece: Piece) {
    if piece.len == 0 {
        return;
    }
    if let Some(last) = out.last_mut()
        && let Some(merged) = last.merged(&piece)
    {
        *last = merged;
        return;
    }
    out.push(piece);
}

/// Stable identity of a marker across edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MarkerId(pub u64);

/// A point (`len_samples == 0`) or range marker in document time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub id: MarkerId,
    pub pos_samples: u64,
    pub len_samples: u64,
    pub name: Arc<str>,
}

impl Marker {
    /// A marker at `pos_samples` spanning `len_samples`.
    pub fn new(
        id: MarkerId,
        pos_samples: u64,
        len_samples: u64,
        name: impl Into<Arc<str>>,
    ) -> Marker {
        Marker {
            id,
            pos_samples,
            len_samples,
            name: name.into(),
        }
    }

    /// Exclusive end position.
    pub fn end_samples(&self) -> u64 {
        self.pos_samples.saturating_add(self.len_samples)
    }
}

/// An immutable revision of the document (ADR-004 §3), shared as `Arc<DocSnapshot>`.
///
/// `rev` and `audio_rev` are monotonic counters assigned by the history: `rev` takes a new value
/// on every change (edit, undo, redo), `audio_rev` only when the samples change. Compare them for
/// inequality, not order, when checking staleness.
#[derive(Clone, Debug)]
pub struct DocSnapshot {
    pub rev: u64,
    pub audio_rev: u64,
    pub sample_rate_hz: u32,
    pub pieces: Arc<[Piece]>,
    /// `starts[i]` is the document position of `pieces[i]` (prefix sums → O(log n) lookup).
    pub starts: Arc<[u64]>,
    pub len_samples: u64,
    /// Sorted by `(pos_samples, id)`.
    pub markers: Arc<[Marker]>,
}

impl DocSnapshot {
    /// The empty document (the undo floor of a new recording).
    pub fn empty(sample_rate_hz: u32) -> Self {
        Self::new(sample_rate_hz, Vec::new(), Vec::new())
    }

    /// A snapshot with revisions 0 from any piece list (normalized: empty pieces dropped, adjacent
    /// ranges merged) and markers (sorted).
    pub fn new(sample_rate_hz: u32, pieces: Vec<Piece>, markers: Vec<Marker>) -> Self {
        let mut normalized = Vec::with_capacity(pieces.len());
        for piece in pieces {
            push_piece(&mut normalized, piece);
        }
        Self::from_normalized(sample_rate_hz, normalized, markers, 0, 0)
    }

    pub(crate) fn from_normalized(
        sample_rate_hz: u32,
        pieces: Vec<Piece>,
        mut markers: Vec<Marker>,
        rev: u64,
        audio_rev: u64,
    ) -> Self {
        let mut starts = Vec::with_capacity(pieces.len());
        let mut acc = 0u64;
        for piece in &pieces {
            starts.push(acc);
            acc += u64::from(piece.len);
        }
        sort_markers(&mut markers);
        DocSnapshot {
            rev,
            audio_rev,
            sample_rate_hz,
            pieces: pieces.into(),
            starts: starts.into(),
            len_samples: acc,
            markers: markers.into(),
        }
    }

    /// The piece containing `pos` and the offset of `pos` inside it; `None` at or past the end.
    pub fn locate(&self, pos: u64) -> Option<(usize, u64)> {
        if pos >= self.len_samples {
            return None;
        }
        let i = self.starts.partition_point(|&s| s <= pos).checked_sub(1)?;
        Some((i, pos - self.starts[i]))
    }

    /// The pieces covering `[at, at + len)` (a clipboard copy).
    pub fn slice(&self, at: u64, len: u64) -> Result<Vec<Piece>> {
        let end = self.checked_end(at, len)?;
        let mut out = Vec::new();
        self.push_range(&mut out, at, end);
        Ok(out)
    }

    /// The piece table after replacing `[at, at + remove_len)` with `insert`.
    pub fn replaced(&self, at: u64, remove_len: u64, insert: &[Piece]) -> Result<Vec<Piece>> {
        let end = self.checked_end(at, remove_len)?;
        let mut out = Vec::with_capacity(self.pieces.len() + insert.len() + 2);
        self.push_range(&mut out, 0, at);
        for &piece in insert {
            push_piece(&mut out, piece);
        }
        self.push_range(&mut out, end, self.len_samples);
        Ok(out)
    }

    fn checked_end(&self, at: u64, len: u64) -> Result<u64> {
        at.checked_add(len)
            .filter(|&end| end <= self.len_samples)
            .ok_or_else(|| {
                ProjectError::InvalidEdit(format!(
                    "range {at}+{len} exceeds the document length {}",
                    self.len_samples
                ))
            })
    }

    fn push_range(&self, out: &mut Vec<Piece>, at: u64, end: u64) {
        let Some((mut i, mut off)) = self.locate(at) else {
            return;
        };
        let mut remaining = end.saturating_sub(at);
        while remaining > 0 && i < self.pieces.len() {
            let piece = self.pieces[i];
            let take = (u64::from(piece.len) - off).min(remaining);
            push_piece(out, piece.sub(off as u32, take as u32));
            remaining -= take;
            i += 1;
            off = 0;
        }
    }

    /// The marker with `id`.
    pub fn marker(&self, id: MarkerId) -> Option<&Marker> {
        self.markers.iter().find(|m| m.id == id)
    }

    /// `true` when both snapshots have the same samples (same piece table).
    pub fn same_audio(&self, other: &DocSnapshot) -> bool {
        Arc::ptr_eq(&self.pieces, &other.pieces) || *self.pieces == *other.pieces
    }
}

/// Sorts markers by `(pos_samples, id)`; several markers may share a position.
pub(crate) fn sort_markers(markers: &mut [Marker]) {
    markers.sort_by_key(|m| (m.pos_samples, m.id));
}

/// Marker positions after `Replace { at = a, remove_len = r, insert_len = n }`, exactly as
/// SPEC-008 §4.2 (which confirms ADR-004 §3 and SPEC-004 §2.2):
///
/// ```text
/// map_start(p):  p < a → p;  r = 0 and p ≥ a → p + n;  p < a + r → a;  otherwise p − r + n
/// map_end(e):    e ≤ a → e;  r = 0 and e > a → e + n;  e ≤ a + r → a;  otherwise e − r + n
/// new_len = map_end(pos + len) − map_start(pos)      (region markers only; 0 → point marker)
/// ```
///
/// Audio edits never delete markers. Length-preserving ops (Silence, Normalize) don't use this
/// mapping at all: their ops carry [`crate::history::MarkerMapping::Identity`].
pub fn shift_markers(markers: &[Marker], at: u64, remove_len: u64, insert_len: u64) -> Vec<Marker> {
    let (a, r, n) = (at, remove_len, insert_len);
    let removed_end = a.saturating_add(r);
    let map_start = |p: u64| -> u64 {
        if p < a {
            p
        } else if r == 0 {
            p.saturating_add(n)
        } else if p < removed_end {
            a
        } else {
            p - r + n
        }
    };
    let map_end = |e: u64| -> u64 {
        if e <= a {
            e
        } else if r == 0 {
            e.saturating_add(n)
        } else if e <= removed_end {
            a
        } else {
            e - r + n
        }
    };
    let mut out: Vec<Marker> = markers
        .iter()
        .map(|m| {
            let pos = map_start(m.pos_samples);
            let len = if m.len_samples == 0 {
                0
            } else {
                map_end(m.end_samples()).saturating_sub(pos)
            };
            Marker {
                pos_samples: pos,
                len_samples: len,
                ..m.clone()
            }
        })
        .collect();
    sort_markers(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(pieces: Vec<Piece>) -> DocSnapshot {
        DocSnapshot::new(48_000, pieces, Vec::new())
    }

    #[test]
    fn locate_uses_prefix_sums() {
        let d = doc(vec![
            Piece::chunk(0, 0, 10),
            Piece::silence(5),
            Piece::chunk(1, 3, 7),
        ]);
        assert_eq!(d.len_samples, 22);
        assert_eq!(d.locate(0), Some((0, 0)));
        assert_eq!(d.locate(9), Some((0, 9)));
        assert_eq!(d.locate(10), Some((1, 0)));
        assert_eq!(d.locate(15), Some((2, 0)));
        assert_eq!(d.locate(21), Some((2, 6)));
        assert_eq!(d.locate(22), None);
    }

    #[test]
    fn normalization_merges_and_drops() {
        let d = doc(vec![
            Piece::chunk(0, 0, 10),
            Piece::chunk(0, 10, 5),
            Piece::silence(0),
            Piece::silence(3),
            Piece::silence(4),
            Piece::chunk(0, 20, 5),
        ]);
        assert_eq!(
            &*d.pieces,
            &[
                Piece::chunk(0, 0, 15),
                Piece::silence(7),
                Piece::chunk(0, 20, 5)
            ]
        );
    }

    #[test]
    fn replace_splits_pieces_and_cut_paste_round_trips() {
        let d = doc(vec![Piece::chunk(0, 0, 100), Piece::chunk(1, 0, 100)]);
        let clip = d.slice(50, 100).unwrap();
        assert_eq!(clip, vec![Piece::chunk(0, 50, 50), Piece::chunk(1, 0, 50)]);
        let cut = doc(d.replaced(50, 100, &[]).unwrap());
        assert_eq!(cut.len_samples, 100);
        assert_eq!(
            &*cut.pieces,
            &[Piece::chunk(0, 0, 50), Piece::chunk(1, 50, 50)]
        );
        let pasted = doc(cut.replaced(50, 0, &clip).unwrap());
        assert_eq!(
            &*pasted.pieces, &*d.pieces,
            "merging restores the original table"
        );
        assert!(d.replaced(150, 51, &[]).is_err());
        assert!(d.slice(u64::MAX, 2).is_err());
    }

    #[test]
    fn marker_shift_rules() {
        const S: u64 = 48_000;
        let m = |id, pos| Marker::new(MarkerId(id), pos, 0, "m");
        let markers = vec![m(1, S), m(2, 3 * S), m(3, 10 * S)];
        // SPEC-004 AC-7: cut [2 s, 4 s) → 1 s, 2 s, 8 s.
        let cut = shift_markers(&markers, 2 * S, 2 * S, 0);
        let pos: Vec<u64> = cut.iter().map(|m| m.pos_samples).collect();
        assert_eq!(pos, vec![S, 2 * S, 8 * S]);
        // Insert 1 s of silence at 1 s: the marker at 1 s moves to 2 s.
        let ins = shift_markers(&markers, S, 0, S);
        assert_eq!(ins[0].pos_samples, 2 * S);
        assert_eq!(ins[1].pos_samples, 4 * S);
        // A replacement sends markers inside [S, E) to S, the start of the new content, even
        // when the lengths match (length-preserving ops use the Identity mapping instead).
        let replaced = shift_markers(&markers, 0, 10 * S, 10 * S);
        let pos: Vec<u64> = replaced.iter().map(|m| m.pos_samples).collect();
        assert_eq!(pos, vec![0, 0, 10 * S]);
    }

    #[test]
    fn region_marker_shift() {
        let r = Marker::new(MarkerId(1), 100, 100, "r");
        let one = |a, rem, n| {
            let m = &shift_markers(std::slice::from_ref(&r), a, rem, n)[0];
            (m.pos_samples, m.len_samples)
        };
        assert_eq!(one(150, 0, 10), (100, 110), "insertion inside grows it");
        assert_eq!(one(200, 0, 10), (100, 100), "insertion at its end doesn't");
        assert_eq!(
            one(100, 0, 10),
            (110, 100),
            "insertion at its start moves it"
        );
        assert_eq!(one(50, 200, 0), (50, 0), "covered by a deletion → point");
        assert_eq!(one(150, 100, 0), (100, 50), "partial deletion shrinks it");
        assert_eq!(one(50, 100, 0), (50, 50), "deletion of its head");
    }
}
