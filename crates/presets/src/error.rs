//! Errors from preset storage (T-406).

/// An error saving, loading, renaming, deleting or listing a preset.
#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    /// The given name can't be turned into a safe file name (empty, or only `.`/`..`).
    #[error("`{0}` is not a valid preset name")]
    InvalidName(String),
    /// No preset with that name exists.
    #[error("no preset named `{0}`")]
    NotFound(String),
    /// A preset with that name already exists (`save`/`rename` don't overwrite silently).
    #[error("a preset named `{0}` already exists")]
    AlreadyExists(String),
    /// The stored file isn't valid JSON, or isn't the expected shape.
    #[error("preset `{name}` is corrupt: {message}")]
    Corrupt {
        /// Preset name.
        name: String,
        /// What went wrong.
        message: String,
    },
    /// The stored file's `format_version` is newer than this build supports.
    #[error("preset `{name}` was saved by a newer version of PowerVoice")]
    TooNew {
        /// Preset name.
        name: String,
    },
    /// Filesystem I/O failed.
    #[error("preset storage: {0}")]
    Io(#[from] std::io::Error),
}
