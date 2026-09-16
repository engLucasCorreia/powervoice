//! Cut, copy, paste, delete, trim, silence (SPEC-008): builds the [`Edit`]s these operations
//! commit through [`crate::session::Session::commit_edit`], plus the range validation and the
//! post-op selection/cursor rule (SPEC-008 §2.2, §2.3). Pure and session-free, so its exactness
//! (SPEC-008 AC-1..AC-10) is testable without a session, engine or IPC layer — sequencing
//! (stopping playback, swapping the engine's document, the clipboard) is `DocumentService`'s job
//! (S2-01).

use crate::history::{Edit, MarkerMapping};
use crate::snapshot::{DocSnapshot, Piece};

/// Undo label keys (SPEC-008 §2.9). Copy adds no undo entry.
pub const LABEL_CUT: &str = "history.cut";
pub const LABEL_PASTE: &str = "history.paste";
pub const LABEL_DELETE: &str = "history.delete";
pub const LABEL_TRIM: &str = "history.trim";
pub const LABEL_SILENCE: &str = "history.silence";
pub const LABEL_INSERT_SILENCE: &str = "history.insert_silence";

/// SPEC-008 §2.5: Insert Silence's duration range, "1 sample to 3 600 s".
pub const INSERT_SILENCE_MAX_SECONDS: u64 = 3_600;

/// A non-empty, in-bounds `[start, end)` document-sample selection (SPEC-006 §2.2), validated by
/// [`validate_range`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: u64,
    pub end: u64,
}

impl Range {
    /// Length in samples (`end - start`, always `> 0`).
    pub fn len_samples(&self) -> u64 {
        self.end - self.start
    }
}

/// Why a candidate `[start, end)` can't be used as a selection (SPEC-008 §2.2, §4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeError {
    /// `start >= end`: SPEC-008 §2.2 "no selection" (an empty selection counts as the cursor at
    /// `start`) — `error.no_selection`.
    Empty,
    /// `end` lies past the document's length — `error.invalid_range`.
    OutOfBounds,
}

/// Validates a selection against the document length (SPEC-008 §2.2, §4.3's `error.no_selection`
/// / `error.invalid_range`).
pub fn validate_range(start: u64, end: u64, len_samples: u64) -> Result<Range, RangeError> {
    if start >= end {
        return Err(RangeError::Empty);
    }
    if end > len_samples {
        return Err(RangeError::OutOfBounds);
    }
    Ok(Range { start, end })
}

/// Where Paste inserts or replaces (SPEC-008 §2.1's "at the cursor" / "over a selection" rows).
#[derive(Clone, Copy, Debug)]
pub enum Target {
    /// Insert at the cursor (`remove_len = 0`).
    Cursor(u64),
    /// Replace an existing, non-empty selection.
    Range(Range),
}

/// The selection and playhead an op leaves behind (SPEC-008 §2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostEdit {
    pub selection: Option<(u64, u64)>,
    pub playhead: u64,
}

/// Cut `range`: the edit plus the clipboard pieces (`A[S,E)`, SPEC-008 §2.1 table, §2.6).
pub fn cut(snapshot: &DocSnapshot, range: Range) -> crate::Result<(Edit, Vec<Piece>)> {
    let clip = snapshot.slice(range.start, range.len_samples())?;
    let edit = Edit::new(LABEL_CUT).replace(range.start, range.len_samples(), Vec::new());
    Ok((edit, clip))
}

/// Copy `range`: just the clipboard pieces — not an edit (SPEC-008 §2.1 table, AC-2).
pub fn copy(snapshot: &DocSnapshot, range: Range) -> crate::Result<Vec<Piece>> {
    snapshot.slice(range.start, range.len_samples())
}

/// Delete `range` (SPEC-008 §2.1 table: `A[0,S) ‖ A[E,L)`).
pub fn delete(range: Range) -> Edit {
    Edit::new(LABEL_DELETE).replace(range.start, range.len_samples(), Vec::new())
}

/// Trim to `range` (Audition: Crop); `None` when `range` already spans the whole document
/// (SPEC-008 §2.1: a no-op — `changed = false`, no undo entry, playback not stopped).
pub fn trim(range: Range, len_samples: u64) -> Option<Edit> {
    if range.start == 0 && range.end == len_samples {
        return None;
    }
    Some(
        Edit::new(LABEL_TRIM)
            .replace(range.end, len_samples - range.end, Vec::new())
            .replace(0, range.start, Vec::new()),
    )
}

/// Silences `range`: exact `+0.0` samples, length-preserving (SPEC-008 §2.1/§2.4: no marker
/// moves, [`MarkerMapping::Identity`]).
pub fn silence(range: Range) -> Edit {
    Edit::new(LABEL_SILENCE).replace_with(
        range.start,
        range.len_samples(),
        Piece::silence_run(range.len_samples()),
        MarkerMapping::Identity,
    )
}

/// Pastes `clip` at `target`: inserts at a cursor, or replaces a selection (SPEC-008 §2.1 table).
pub fn paste(clip: &[Piece], target: Target) -> Edit {
    let (at, remove_len) = match target {
        Target::Cursor(c) => (c, 0),
        Target::Range(r) => (r.start, r.len_samples()),
    };
    Edit::new(LABEL_PASTE).replace(at, remove_len, clip.to_vec())
}

/// Total sample length of clipboard pieces (the `n` of SPEC-008 §2.1).
pub fn pieces_len_samples(pieces: &[Piece]) -> u64 {
    pieces.iter().map(Piece::len_samples).sum()
}

/// Why a candidate Insert Silence duration can't be used (SPEC-008 §2.5, §4.3's
/// `error.insert_silence_duration`): `0` samples, or more than [`INSERT_SILENCE_MAX_SECONDS`] at
/// the document's rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InsertSilenceLenError;

/// Validates an Insert Silence duration already converted to samples (the dialog does the
/// parsing/rounding, SPEC-008 §2.5) against the document's sample rate.
pub fn validate_insert_silence_len(
    len_samples: u64,
    sample_rate_hz: u32,
) -> Result<u64, InsertSilenceLenError> {
    let max = INSERT_SILENCE_MAX_SECONDS.saturating_mul(u64::from(sample_rate_hz));
    if len_samples == 0 || len_samples > max {
        return Err(InsertSilenceLenError);
    }
    Ok(len_samples)
}

/// Inserts `len_samples` of silence at `target`'s position: the selection start if one exists,
/// otherwise the cursor (SPEC-008 §2.1/§2.5) — inserting never removes anything, so a `Range`
/// target's end is ignored.
pub fn insert_silence(target: Target, len_samples: u64) -> Edit {
    let at = match target {
        Target::Cursor(c) => c,
        Target::Range(r) => r.start,
    };
    Edit::new(LABEL_INSERT_SILENCE).replace(at, 0, Piece::silence_run(len_samples))
}

// --- Post-edit selection/cursor (SPEC-008 §2.3) -------------------------------------------------

/// Cut and Delete: no selection, playhead at the removed range's start.
pub fn post_cut_or_delete(range: Range) -> PostEdit {
    PostEdit {
        selection: None,
        playhead: range.start,
    }
}

/// Paste: the pasted audio becomes the selection, playhead at its start.
pub fn post_paste(target: Target, inserted_len_samples: u64) -> PostEdit {
    let at = match target {
        Target::Cursor(c) => c,
        Target::Range(r) => r.start,
    };
    PostEdit {
        selection: Some((at, at + inserted_len_samples)),
        playhead: at,
    }
}

/// Insert silence: the inserted range becomes the selection, playhead at its start (mirrors
/// [`post_paste`] — SPEC-008 §2.3).
pub fn post_insert_silence(target: Target, len_samples: u64) -> PostEdit {
    post_paste(target, len_samples)
}

/// Trim: no selection, playhead at 0 (the start of what was kept).
pub fn post_trim() -> PostEdit {
    PostEdit {
        selection: None,
        playhead: 0,
    }
}

/// Silence: the selection is unchanged, playhead unchanged (both stay `[S, E)` / `S`, since the
/// caller already has a playhead inside or at the edge of the untouched selection).
pub fn post_silence(range: Range) -> PostEdit {
    PostEdit {
        selection: Some((range.start, range.end)),
        playhead: range.start,
    }
}

/// After undo or redo of one of these ops (SPEC-008 §2.3): the selection is cleared and the
/// playhead goes to the edit's first affected position (`min(at)` over its `Replace` ops,
/// [`crate::history::HistoryStep::first_at`]), clamped to the (post-undo/redo) document length.
/// `None` (a marker-only edit) uses 0.
pub fn post_undo_redo(first_at: Option<u64>, len_samples: u64) -> PostEdit {
    PostEdit {
        selection: None,
        playhead: first_at.unwrap_or(0).min(len_samples),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::DocSnapshot;

    fn doc(len: u32) -> DocSnapshot {
        DocSnapshot::new(48_000, vec![Piece::chunk(0, 0, len)], Vec::new())
    }

    #[test]
    fn range_validation() {
        assert_eq!(validate_range(10, 10, 100), Err(RangeError::Empty));
        assert_eq!(validate_range(10, 5, 100), Err(RangeError::Empty));
        assert_eq!(validate_range(0, 101, 100), Err(RangeError::OutOfBounds));
        assert_eq!(
            validate_range(0, 100, 100),
            Ok(Range { start: 0, end: 100 })
        );
    }

    #[test]
    fn cut_builds_the_replace_and_the_clipboard() {
        let d = doc(100);
        let range = validate_range(10, 40, 100).unwrap();
        let (edit, clip) = cut(&d, range).unwrap();
        assert_eq!(edit.label_key, LABEL_CUT);
        assert_eq!(edit.ops.len(), 1);
        assert_eq!(clip, vec![Piece::chunk(0, 10, 30)]);
        assert_eq!(post_cut_or_delete(range).playhead, 10);
        assert_eq!(post_cut_or_delete(range).selection, None);
    }

    #[test]
    fn trim_is_two_replaces_and_a_no_op_over_the_whole_document() {
        let range = validate_range(10, 40, 100).unwrap();
        let edit = trim(range, 100).unwrap();
        assert_eq!(edit.ops.len(), 2);
        assert_eq!(post_trim().playhead, 0);
        assert_eq!(post_trim().selection, None);

        let whole = validate_range(0, 100, 100).unwrap();
        assert!(trim(whole, 100).is_none(), "trim of [0, L) is a no-op");
    }

    #[test]
    fn silence_keeps_the_selection_and_uses_identity_mapping() {
        let range = validate_range(10, 40, 100).unwrap();
        let edit = silence(range);
        let crate::history::EditOp::Replace {
            pieces, mapping, ..
        } = &edit.ops[0];
        assert_eq!(pieces_len_samples(pieces), 30);
        assert_eq!(*mapping, MarkerMapping::Identity);
        assert_eq!(post_silence(range).selection, Some((10, 40)));
    }

    #[test]
    fn paste_at_cursor_inserts_and_over_a_selection_replaces() {
        let clip = vec![Piece::chunk(1, 0, 20)];
        let inserted = pieces_len_samples(&clip);
        assert_eq!(inserted, 20);

        let at_cursor = paste(&clip, Target::Cursor(5));
        let crate::history::EditOp::Replace { at, remove_len, .. } = &at_cursor.ops[0];
        assert_eq!((*at, *remove_len), (5, 0));
        assert_eq!(
            post_paste(Target::Cursor(5), inserted).selection,
            Some((5, 25))
        );

        let range = validate_range(10, 40, 100).unwrap();
        let over_selection = paste(&clip, Target::Range(range));
        let crate::history::EditOp::Replace { at, remove_len, .. } = &over_selection.ops[0];
        assert_eq!((*at, *remove_len), (10, 30));
        assert_eq!(
            post_paste(Target::Range(range), inserted).selection,
            Some((10, 30))
        );
    }

    #[test]
    fn insert_silence_len_validation() {
        assert_eq!(
            validate_insert_silence_len(0, 48_000),
            Err(InsertSilenceLenError)
        );
        assert_eq!(validate_insert_silence_len(1, 48_000), Ok(1));
        assert_eq!(
            validate_insert_silence_len(3_600 * 48_000, 48_000),
            Ok(3_600 * 48_000)
        );
        assert_eq!(
            validate_insert_silence_len(3_600 * 48_000 + 1, 48_000),
            Err(InsertSilenceLenError)
        );
    }

    #[test]
    fn insert_silence_at_cursor_and_over_a_selection_start() {
        let m = 480;
        let at_cursor = insert_silence(Target::Cursor(5), m);
        assert_eq!(at_cursor.label_key, LABEL_INSERT_SILENCE);
        let crate::history::EditOp::Replace { at, remove_len, .. } = &at_cursor.ops[0];
        assert_eq!((*at, *remove_len), (5, 0), "insert never removes anything");
        assert_eq!(
            post_insert_silence(Target::Cursor(5), m).selection,
            Some((5, 5 + m))
        );

        // Inserting silence with a selection: SPEC-008 §2.1 "inserts at the selection start and
        // removes nothing" — the range's end is ignored.
        let range = validate_range(10, 40, 100).unwrap();
        let over_selection = insert_silence(Target::Range(range), m);
        let crate::history::EditOp::Replace { at, remove_len, .. } = &over_selection.ops[0];
        assert_eq!((*at, *remove_len), (10, 0));
        assert_eq!(
            post_insert_silence(Target::Range(range), m).selection,
            Some((10, 10 + m))
        );
    }

    #[test]
    fn post_undo_redo_clamps_and_defaults_to_zero() {
        assert_eq!(
            post_undo_redo(Some(500), 100),
            PostEdit {
                selection: None,
                playhead: 100
            }
        );
        assert_eq!(
            post_undo_redo(None, 100),
            PostEdit {
                selection: None,
                playhead: 0
            }
        );
    }
}
