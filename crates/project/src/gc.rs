//! Session garbage collection (ADR-004 §9 classification, §10; SPEC-004 §2.8, AC-12).
//!
//! Deletion is crash-safe: the directory is first renamed to `<id>.deleting` (atomic), then
//! removed. Any `*.deleting` directory found at start-up is an interrupted deletion and is
//! finished. If the rename fails (e.g. a Windows scanner holds a handle), the files are removed
//! in place with the journal last, so an interrupted deletion still ends in `close` and is
//! classified clean at the next start.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::fs_util::{dir_size, pid_is_alive, sync_dir, unix_ms};
use crate::journal::{Record, journal_file_name, scan_journal};
use crate::session::{LOCK_FILE_NAME, TAKES_DIR_NAME, read_meta};
use crate::take::{recover_take_file, take_part_path};
use crate::{ProjectError, Result};

/// Suffix of a session directory whose deletion has started.
pub const DELETING_SUFFIX: &str = ".deleting";

/// A session that must be offered for recovery and is never deleted silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoverableSession {
    pub id: String,
    pub dir: PathBuf,
    pub sample_rate_hz: Option<u32>,
    /// The imported file, `None` for an untitled recording.
    pub source_path: Option<PathBuf>,
    /// The path a recovered document is bound to (its last save target, else `source_path`).
    pub path: Option<PathBuf>,
    pub current_seq: u64,
    pub saved_seq: u64,
    pub unsaved_changes: bool,
    /// How many undo/redo steps separate the current state from the saved one ("3 unsaved
    /// changes", SPEC-004 §2.7).
    pub unsaved_count: u64,
    /// The bound file's size or mtime differ from what the session last recorded.
    pub source_changed: bool,
    /// The bound file no longer exists.
    pub source_missing: bool,
    /// Lines after the journal's valid prefix (torn or corrupt).
    pub damaged_lines: usize,
    /// The newest take that was begun but never committed or discarded.
    pub open_take: Option<u32>,
    /// Samples recoverable from uncommitted takes.
    pub open_take_samples: u64,
    /// Committed takes whose WAV holds audio beyond what was committed (store failure mid-take).
    pub truncated_takes: Vec<u32>,
    /// The journal has a torn/corrupt tail or is inconsistent.
    pub journal_damaged: bool,
    pub last_modified: Option<SystemTime>,
    pub size_bytes: u64,
}

/// How start-up treats an unlocked session.
#[derive(Clone, Debug, PartialEq, Eq)]
// Built once per session directory at start-up: the size difference doesn't matter, and boxing
// would change the public shape the engine tests match on.
#[allow(clippy::large_enum_variant)]
pub enum SessionClass {
    /// Ends in `close`, or nothing unsaved and no take with audio: delete it.
    Clean,
    /// Anything else: keep it and offer recovery.
    Recoverable(RecoverableSession),
}

/// What [`collect_garbage`] did.
#[derive(Debug, Default)]
pub struct GcReport {
    /// Directories deleted (clean sessions and interrupted deletions).
    pub deleted: Vec<PathBuf>,
    /// Sessions locked by a running instance: not touched.
    pub locked: Vec<PathBuf>,
    /// Sessions to offer in the recovery dialog.
    pub recoverable: Vec<RecoverableSession>,
    /// Directories that could not be inspected or deleted, with the reason.
    pub errors: Vec<(PathBuf, String)>,
}

/// Start-up pass over `<sessions_dir>`: finishes interrupted deletions, deletes clean sessions,
/// leaves locked ones alone and reports recoverable ones. Never deletes a recoverable session.
pub fn collect_garbage(sessions_dir: &Path) -> GcReport {
    let mut report = GcReport::default();
    let entries = match fs::read_dir(sessions_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return report,
        Err(e) => {
            report
                .errors
                .push((sessions_dir.to_path_buf(), e.to_string()));
            return report;
        }
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect();
    dirs.sort();
    for dir in dirs {
        let is_tombstone = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(DELETING_SUFFIX));
        if is_tombstone {
            match fs::remove_dir_all(&dir) {
                Ok(()) => report.deleted.push(dir),
                Err(e) => report.errors.push((dir, e.to_string())),
            }
            continue;
        }
        match try_lock_session(&dir) {
            Ok(None) => report.locked.push(dir),
            Ok(Some(lock)) => match classify_session(&dir) {
                Ok(SessionClass::Clean) => {
                    drop(lock);
                    match delete_session_dir(&dir) {
                        Ok(()) => report.deleted.push(dir),
                        Err(e) => report.errors.push((dir, e.to_string())),
                    }
                }
                Ok(SessionClass::Recoverable(session)) => report.recoverable.push(session),
                Err(e) => report.errors.push((dir, e.to_string())),
            },
            Err(e) => report.errors.push((dir, e.to_string())),
        }
    }
    report
}

/// Deletes a session the user discarded (recovery dialog, Settings → Clear). Refuses a session
/// locked by a running instance.
pub fn discard_session(dir: &Path) -> Result<()> {
    match try_lock_session(dir)? {
        None => Err(ProjectError::SessionLocked),
        Some(lock) => {
            drop(lock);
            delete_session_dir(dir)
        }
    }
}

/// Tries to take a session's advisory lock: `Ok(None)` if another instance (or another open
/// [`crate::Session`]) holds it.
pub(crate) fn try_lock_session(dir: &Path) -> Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(LOCK_FILE_NAME))
        .map_err(ProjectError::io("opening a session lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(ProjectError::from_io("locking a session", e)),
    }
}

#[derive(Default)]
struct Summary {
    closed: bool,
    undo: Vec<u64>,
    redo: Vec<u64>,
    saved_seq: u64,
    open_takes: BTreeSet<u32>,
    /// T-304: aligned takes whose record window never opened (no `take_window`): a crash in
    /// pre-roll — nothing of them is applicable, so they don't make the session recoverable.
    pending_aligned: BTreeSet<u32>,
    truncated_takes: BTreeSet<u32>,
    inconsistent: bool,
    /// `(path, size, mtime)` of the `open` record's source.
    source: Option<(String, u64, u64)>,
    /// `(path, size?, mtime?)` of the latest `saved` record.
    saved: Option<(String, Option<u64>, Option<u64>)>,
}

fn summarize(records: &[Record]) -> Summary {
    let mut s = Summary::default();
    for record in records {
        s.closed = false;
        match record {
            Record::Edit(e) => {
                s.undo.push(e.seq);
                s.redo.clear();
                if let Some(take) = e.take {
                    s.open_takes.remove(&take);
                    if e.take_wav_samples.is_some() {
                        s.truncated_takes.insert(take);
                    }
                }
            }
            Record::Undo { seq } => match s.undo.pop() {
                Some(top) if top == *seq => s.redo.push(top),
                _ => s.inconsistent = true,
            },
            Record::Redo { seq } => match s.redo.pop() {
                Some(top) if top == *seq => s.undo.push(top),
                _ => s.inconsistent = true,
            },
            Record::TakeBegin { take, aligned, .. } => {
                if *aligned {
                    s.pending_aligned.insert(*take);
                } else {
                    s.open_takes.insert(*take);
                }
            }
            Record::TakeWindow { take, .. } => {
                if s.pending_aligned.remove(take) {
                    s.open_takes.insert(*take);
                }
            }
            Record::TakeDiscard { take } | Record::TakeCancel { take } => {
                s.open_takes.remove(take);
                s.pending_aligned.remove(take);
            }
            Record::Saved {
                seq,
                path,
                file_size_bytes,
                file_mtime_unix_ms,
                ..
            } => {
                s.saved_seq = *seq;
                s.saved = Some((path.clone(), *file_size_bytes, *file_mtime_unix_ms));
            }
            Record::DropUndo { count } => {
                let n = usize::try_from(*count)
                    .unwrap_or(usize::MAX)
                    .min(s.undo.len());
                s.undo.drain(..n);
            }
            Record::Checkpoint(c) => {
                s.undo = c.undo.iter().map(|e| e.seq).collect();
                s.redo = c.redo.iter().map(|e| e.seq).collect();
                s.saved_seq = c.saved_seq;
            }
            Record::Close => s.closed = true,
            Record::Open { source, .. } => {
                s.source = source
                    .as_ref()
                    .map(|src| (src.path.clone(), src.size_bytes, src.mtime_unix_ms));
            }
            Record::Chunks { .. } | Record::State { .. } | Record::TakeMarker { .. } => {}
        }
    }
    s
}

/// Undo/redo steps between the current state and the saved one (0: clean).
fn unsaved_count(undo: &[u64], redo: &[u64], saved: u64) -> u64 {
    let current = undo.last().copied().unwrap_or(0);
    if current == saved {
        return 0;
    }
    let steps = if saved == 0 {
        undo.len()
    } else if let Some(p) = undo.iter().position(|&s| s == saved) {
        undo.len() - 1 - p
    } else if let Some(p) = redo.iter().rposition(|&s| s == saved) {
        redo.len() - p
    } else {
        // The saved state is gone (redo truncated): every edit since the floor is unsaved.
        undo.len().max(1)
    };
    steps as u64
}

/// Classifies an unlocked session directory (ADR-004 §9 step 2). The caller should hold the
/// session's lock.
pub fn classify_session(dir: &Path) -> Result<SessionClass> {
    let meta = read_meta(dir);
    let generation = meta.as_ref().map_or(0, |m| m.generation);
    let journal_path = dir.join(journal_file_name(generation));
    let takes_dir = dir.join(TAKES_DIR_NAME);

    let (summary, damaged, damaged_lines) = if journal_path.exists() {
        let bytes = fs::read(&journal_path).map_err(ProjectError::io("reading the journal"))?;
        let scan = scan_journal(&bytes);
        (
            summarize(&scan.records),
            scan.valid_bytes < bytes.len() as u64,
            scan.damaged_lines,
        )
    } else {
        // Nothing was ever journaled (creation interrupted, or an in-place deletion removed the
        // journal last): only take data could still matter.
        (Summary::default(), false, 0)
    };
    if summary.closed && !damaged {
        return Ok(SessionClass::Clean);
    }
    // A take that was begun and never committed or discarded lives only in its WAV: the session
    // is recoverable as soon as part 0 exists, whatever the parts hold. Samples are summed over
    // the parts that parse; a torn part (crash during rollover, power loss) is skipped instead of
    // zeroing the whole take.
    let (open_take_present, open_take_samples) = if journal_path.exists() {
        summary
            .open_takes
            .iter()
            .map(|&take| lenient_take_samples(&takes_dir, take))
            .fold((false, 0), |(any, sum), (exists, samples)| {
                (any || exists, sum + samples)
            })
    } else {
        stray_takes(&takes_dir)
    };
    let current_seq = summary.undo.last().copied().unwrap_or(0);
    let unsaved = current_seq != summary.saved_seq;
    let journal_damaged = damaged || summary.inconsistent;
    let truncated_takes: Vec<u32> = summary.truncated_takes.iter().copied().collect();
    if !journal_damaged && !unsaved && !open_take_present && truncated_takes.is_empty() {
        return Ok(SessionClass::Clean);
    }
    let unsaved_count = unsaved_count(&summary.undo, &summary.redo, summary.saved_seq);
    let (path, facts) = match (&summary.saved, &summary.source) {
        (Some((path, size, mtime)), _) => (Some(PathBuf::from(path)), size.zip(*mtime)),
        (None, Some((path, size, mtime))) => (Some(PathBuf::from(path)), Some((*size, *mtime))),
        (None, None) => (None, None),
    };
    let file_meta = path.as_ref().and_then(|p| fs::metadata(p).ok());
    let source_missing = path.is_some() && file_meta.is_none();
    let source_changed = match (&file_meta, facts) {
        (Some(meta), Some((size, mtime))) => {
            let now_mtime = meta.modified().map(unix_ms).unwrap_or(0);
            meta.len() != size || now_mtime != mtime
        }
        _ => false,
    };
    Ok(SessionClass::Recoverable(RecoverableSession {
        id: dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        dir: dir.to_path_buf(),
        sample_rate_hz: meta.as_ref().map(|m| m.sample_rate_hz),
        source_path: meta.and_then(|m| m.source_path).map(PathBuf::from),
        path,
        current_seq,
        saved_seq: summary.saved_seq,
        unsaved_changes: unsaved,
        unsaved_count,
        source_changed,
        source_missing,
        damaged_lines,
        open_take: summary.open_takes.iter().next_back().copied(),
        open_take_samples,
        truncated_takes,
        journal_damaged,
        last_modified: fs::metadata(&journal_path).and_then(|m| m.modified()).ok(),
        size_bytes: dir_size(dir),
    }))
}

/// `(part 0 exists, samples summed over the parts that parse)` for take `take`.
fn lenient_take_samples(takes_dir: &Path, take: u32) -> (bool, u64) {
    let mut samples = 0;
    let mut part = 0;
    loop {
        let path = take_part_path(takes_dir, take, part);
        if !path.exists() {
            break;
        }
        if let Ok(recovered) = recover_take_file(&path) {
            samples += recovered.samples;
        }
        part += 1;
    }
    (part > 0, samples)
}

/// `(any take file exists, samples summed over those that parse)` without a journal.
fn stray_takes(takes_dir: &Path) -> (bool, u64) {
    let Ok(entries) = fs::read_dir(takes_dir) else {
        return (false, 0);
    };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "wav"))
        .fold((false, 0), |(_, sum), e| {
            (
                true,
                sum + recover_take_file(&e.path()).map_or(0, |t| t.samples),
            )
        })
}

/// Deletes `.‹name›.powervoice-tmp-‹pid›` leftovers of an interrupted atomic save in `dir` whose
/// pid is no longer running (SPEC-004 §2.6, AC-12): call before saving into `dir`. A temp file
/// of a live process (another instance saving right now) is never touched. Returns how many
/// were removed.
pub fn remove_stale_temp_files(dir: &Path) -> usize {
    const MARKER: &str = ".powervoice-tmp-";
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with('.') {
            continue;
        }
        let Some(pos) = name.rfind(MARKER) else {
            continue;
        };
        let Ok(pid) = name[pos + MARKER.len()..].parse::<i32>() else {
            continue;
        };
        if pid_is_alive(pid) || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Deletes a session directory crash-safely (see the module docs). A missing directory is fine.
pub(crate) fn delete_session_dir(dir: &Path) -> Result<()> {
    const CONTEXT: &str = "deleting a session";
    let Some(name) = dir.file_name() else {
        return Err(ProjectError::InvalidArgument("session path has no name"));
    };
    let mut tomb_name = name.to_os_string();
    tomb_name.push(DELETING_SUFFIX);
    let tomb = dir.with_file_name(tomb_name);
    if tomb.exists() {
        fs::remove_dir_all(&tomb).map_err(ProjectError::io(CONTEXT))?;
    }
    match fs::rename(dir, &tomb) {
        Ok(()) => {
            if let Some(parent) = dir.parent() {
                let _ = sync_dir(parent);
            }
            fs::remove_dir_all(&tomb).map_err(ProjectError::io(CONTEXT))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => delete_in_place(dir).map_err(ProjectError::io(CONTEXT)),
    }
}

/// T-306 (SPEC-018 §2.11): scans `<sessions_dir>` for a session — other than `exclude_session_id`
/// (this instance's own, if any) — whose lock is held (another running instance) and whose
/// `meta.json` source path canonicalizes to `canonical_path`. Only `meta.json` of *locked*
/// sessions is read (SPEC-018 §2.11: "the scan reads only `meta.json` of locked sessions"), so an
/// unlocked (crashed/closed) session never triggers the warning.
pub fn find_already_open(
    sessions_dir: &Path,
    canonical_path: &Path,
    exclude_session_id: Option<&str>,
) -> Option<PathBuf> {
    let entries = fs::read_dir(sessions_dir).ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(DELETING_SUFFIX) || Some(name) == exclude_session_id {
            continue;
        }
        // Locked (another instance holds it) — an unlocked session is never "already open".
        match try_lock_session(&dir) {
            Ok(Some(_lock)) => continue, // uncontended: not open elsewhere; drop releases it
            Ok(None) => {}               // held by another instance: keep checking
            Err(_) => continue,
        }
        let Some(source) = read_meta(&dir).and_then(|m| m.source_path) else {
            continue;
        };
        if crate::canonical_path_for_compare(Path::new(&source)) == canonical_path {
            return Some(dir);
        }
    }
    None
}

fn delete_in_place(dir: &Path) -> io::Result<()> {
    let mut journals = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with("journal.") {
            journals.push(path);
        } else if entry.file_type()?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
    }
    for journal in journals {
        fs::remove_file(journal)?;
    }
    fs::remove_dir(dir)
}
