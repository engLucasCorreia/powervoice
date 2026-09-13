//! Shared atomic-write helpers (ADR-004 §8): write to a temp file next to the target, `fdatasync`
//! it, rename over the target, then `fsync` the directory. A crash or an error before [`finish`]
//! runs leaves the old file (or its absence) exactly as it was; `path` is only ever touched by the
//! final rename. Shared by every encoder in this crate ([`crate::wav`], [`crate::flac`],
//! [`crate::mp3`]) so they all get the same crash-safety guarantee (SPEC-005 §2.7).

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error::{IoError, Result};

/// `.<file name>.powervoice-tmp-<pid>`, next to `target`.
pub(crate) fn temp_path_for(target: &Path) -> PathBuf {
    let mut name = std::ffi::OsString::from(".");
    name.push(target.file_name().unwrap_or_default());
    name.push(format!(".powervoice-tmp-{}", std::process::id()));
    target.with_file_name(name)
}

pub(crate) fn fsync_path(path: &Path) -> Result<()> {
    OpenOptions::new()
        .write(true)
        .open(path)?
        .sync_all()
        .map_err(IoError::from)
}

pub(crate) fn fsync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        // The file system journals directory changes itself, and a directory can't be opened as
        // a `File` on Windows (ADR-004 §1, `project::fs_util::sync_dir`).
        let _ = dir;
    }
    Ok(())
}

/// Finishes an atomic write: fsync `tmp`, rename over `path`, fsync the parent directory.
pub(crate) fn finish(tmp: &Path, path: &Path) -> Result<()> {
    fsync_path(tmp)?;
    std::fs::rename(tmp, path)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fsync_dir(parent)?;
    }
    Ok(())
}
