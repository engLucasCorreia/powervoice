//! Error type of `vox-project`.

use std::io;

use crate::store::ChunkId;

/// Result alias used throughout the crate.
pub type Result<T, E = ProjectError> = std::result::Result<T, E>;

/// Everything that can go wrong in the document model.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProjectError {
    /// Audio edits, undo and redo are refused while a take is open (SPEC-004 §2.3, AC-15).
    #[error("not available while recording")]
    NotWhileRecording,
    /// The volume is full (or a size/quota limit was hit). With preallocated segments this
    /// surfaces when a segment is created, never as `SIGBUS` on a later write (ADR-004 §2).
    #[error("disk full while {context}: {source}")]
    DiskFull {
        context: &'static str,
        #[source]
        source: io::Error,
    },
    /// Any other I/O failure.
    #[error("I/O error while {context}: {source}")]
    Io {
        context: &'static str,
        #[source]
        source: io::Error,
    },
    /// A committed chunk no longer matches its CRC32.
    #[error("chunk {chunk} failed its CRC32 check")]
    ChecksumMismatch { chunk: ChunkId },
    /// A piece references a chunk id the store doesn't have.
    #[error("chunk {0} is not in the store")]
    UnknownChunk(ChunkId),
    /// A piece references samples beyond the end of its chunk.
    #[error("chunk {chunk}: range {offset}+{len} is out of bounds")]
    OutOfBounds {
        chunk: ChunkId,
        offset: u32,
        len: u32,
    },
    /// The edit is malformed (range outside the document, duplicate marker id, …).
    #[error("invalid edit: {0}")]
    InvalidEdit(String),
    /// An argument outside its documented range.
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    /// A cancellable operation was cancelled.
    #[error("operation cancelled")]
    Cancelled,
    /// The session directory is locked by another running instance.
    #[error("session is in use by another instance")]
    SessionLocked,
    /// `close` was called while a reader or writer still shares the store.
    #[error("the session store is still in use by a reader or writer")]
    StoreInUse,
    /// A take command named a take that isn't open.
    #[error("take {0} is not open")]
    NoSuchTake(u32),
    /// `begin_take` was called while a take is already open.
    #[error("a take is already open")]
    TakeAlreadyOpen,
    /// A take WAV can't be parsed.
    #[error("invalid take file: {0}")]
    InvalidTakeFile(&'static str),
    /// A take's audio is only in its WAV (the chunk store failed before its first chunk). The
    /// take stays open, so crash recovery can apply it from the WAV.
    #[error("take {take}: its {wav_samples} samples are only in the take file")]
    TakeNotInStore { take: u32, wav_samples: u64 },
    /// JSON (de)serialization of a journal record or `meta.json` failed.
    #[error("serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Reading or writing a WAV file failed ([`vox_io`], S1-02 `import_wav`/`save_snapshot_wav`).
    #[error("WAV I/O failed: {0}")]
    Wav(#[from] vox_io::IoError),
    /// A normalize scope contains a non-finite sample (SPEC-010 §2.7): defensive, since the store
    /// should never hold one. `error.normalize_non_finite`.
    #[error("the scope contains a non-finite sample")]
    NonFiniteSample,
    /// The BS.1770 loudness scan failed (S4-01, LUFS normalize): only reachable with a bad
    /// channel/rate configuration, which never happens here (mono, the document's own rate).
    #[error("loudness measurement failed: {0}")]
    Loudness(#[from] vox_dsp::loudness::LoudnessError),
}

impl ProjectError {
    /// Wraps an I/O error, classifying "no space"/"too large"/"quota" as [`ProjectError::DiskFull`].
    pub fn from_io(context: &'static str, source: io::Error) -> Self {
        if is_disk_full(&source) {
            ProjectError::DiskFull { context, source }
        } else {
            ProjectError::Io { context, source }
        }
    }

    /// `map_err` helper: `.map_err(ProjectError::io("reading the journal"))`.
    pub(crate) fn io(context: &'static str) -> impl FnOnce(io::Error) -> ProjectError {
        move |source| ProjectError::from_io(context, source)
    }

    /// The i18n key the IPC layer reports for this error (`IpcError.key`, ADR-003).
    pub fn i18n_key(&self) -> &'static str {
        match self {
            ProjectError::NotWhileRecording => "error.not_while_recording",
            ProjectError::DiskFull { .. } => "error.disk_full",
            ProjectError::Io { .. } => "error.io",
            ProjectError::ChecksumMismatch { .. } => "error.store_corrupt",
            ProjectError::Cancelled => "error.cancelled",
            ProjectError::SessionLocked => "error.session_locked",
            ProjectError::InvalidEdit(_)
            | ProjectError::InvalidArgument(_)
            | ProjectError::OutOfBounds { .. }
            | ProjectError::UnknownChunk(_) => "error.invalid_edit",
            ProjectError::TakeNotInStore { .. } => "error.take_not_in_store",
            ProjectError::NonFiniteSample => "error.normalize_non_finite",
            ProjectError::StoreInUse
            | ProjectError::NoSuchTake(_)
            | ProjectError::TakeAlreadyOpen
            | ProjectError::InvalidTakeFile(_)
            | ProjectError::Loudness(_)
            | ProjectError::Json(_) => "error.internal",
            // T-202: SPEC-005 §2.5's specific open/import error keys, when the wrapped `IoError`
            // says which one applies; every other `IoError` (write/encode failures, plain I/O)
            // keeps the general `error.io` key.
            ProjectError::Wav(io_err) => match io_err {
                vox_io::IoError::UnrecognizedFormat(_) => "error.open.unsupported_format",
                vox_io::IoError::UnsupportedCodec(_) => "error.open.unsupported_codec",
                vox_io::IoError::NoAudioTrack => "error.open.no_audio_track",
                vox_io::IoError::RateOutOfRange(_) => "error.open.rate_out_of_range",
                vox_io::IoError::TooManyChannels(_) => "error.open.too_many_channels",
                vox_io::IoError::ChainedStreamChanged => "error.open.unsupported_format",
                vox_io::IoError::TooDamaged(_, _) => "error.open.damaged",
                vox_io::IoError::Io(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    "error.open.not_found"
                }
                vox_io::IoError::Io(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    "error.open.permission"
                }
                _ => "error.io",
            },
        }
    }

    /// `true` for [`ProjectError::DiskFull`].
    pub fn is_disk_full(&self) -> bool {
        matches!(self, ProjectError::DiskFull { .. })
    }
}

fn is_disk_full(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::StorageFull | io::ErrorKind::QuotaExceeded | io::ErrorKind::FileTooLarge
    )
}
