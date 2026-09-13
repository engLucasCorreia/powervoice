//! Segment preallocation (ADR-004 §2): reserve the blocks of a new segment up front so that a full
//! disk is reported when the segment is created, not as `SIGBUS` when a mapped write faults later.
//!
//! - Linux/Android: `posix_fallocate`. If the file system can't preallocate (`EOPNOTSUPP`, e.g.
//!   ZFS or musl without emulation) the file is only extended (sparse) and the result says
//!   [`Reservation::Unreserved`], so the engine can warn.
//! - macOS/iOS: `fcntl(F_PREALLOCATE)` followed by `ftruncate` (`File::set_len`), because
//!   `F_PREALLOCATE` allocates blocks without changing the file size.
//! - Windows: `File::set_len`, i.e. `SetFileInformationByHandle(FileEndOfFileInfo)` (the
//!   `SetEndOfFile` equivalent). For a non-sparse NTFS file this allocates the clusters and
//!   fails with `ERROR_DISK_FULL`, so it counts as reserved.
//! - Other targets: `File::set_len` only, reported as [`Reservation::Unreserved`].
//!
//! **Copy-on-write file systems** (btrfs, ZFS, APFS with clones/snapshots): a reservation does
//! not fully guarantee that a later write into the range finds space, because CoW may allocate
//! new blocks for it (e.g. after a snapshot shares the preallocated extents). There a full disk
//! can still surface as `SIGBUS` on a mapped write; ADR-004 accepts this risk for local session
//! directories, and the engine's disk-space floor (SPEC-002 §2.5, SPEC-004 §2.5) keeps free space
//! above zero in practice.

use std::fs::File;
use std::io;

/// Whether a segment's disk space was actually reserved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reservation {
    /// Blocks are allocated: disk-full surfaced here, not later.
    Reserved,
    /// The file was only extended (sparse): a later mapped write could still hit a full disk.
    #[cfg_attr(
        any(target_os = "macos", target_os = "ios", windows),
        allow(dead_code, reason = "these targets always reserve")
    )]
    Unreserved,
}

/// Reserves disk space for `[offset, offset + len)` of `file` and extends it to cover the range.
pub(crate) fn preallocate(file: &File, offset: u64, len: u64) -> io::Result<Reservation> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| io::Error::from(io::ErrorKind::FileTooLarge))?;
    imp::preallocate(file, offset, len, end)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;

    use super::Reservation;

    pub(super) fn preallocate(
        file: &File,
        offset: u64,
        len: u64,
        end: u64,
    ) -> io::Result<Reservation> {
        let too_large = || io::Error::from(io::ErrorKind::FileTooLarge);
        let off = libc::off_t::try_from(offset).map_err(|_| too_large())?;
        let size = libc::off_t::try_from(len).map_err(|_| too_large())?;
        loop {
            // SAFETY: `file` owns a valid open descriptor for the duration of the call, and
            // `posix_fallocate` takes only integers (no pointers into our memory).
            let ret = unsafe { libc::posix_fallocate(file.as_raw_fd(), off, size) };
            match ret {
                0 => return Ok(Reservation::Reserved),
                libc::EINTR => continue,
                // The file system can't preallocate: extend the file and say so.
                libc::EOPNOTSUPP => {
                    super::extend(file, end)?;
                    return Ok(Reservation::Unreserved);
                }
                err => return Err(io::Error::from_raw_os_error(err)),
            }
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::fd::AsRawFd;

    use super::Reservation;

    pub(super) fn preallocate(
        file: &File,
        _offset: u64,
        _len: u64,
        end: u64,
    ) -> io::Result<Reservation> {
        let current = file.metadata()?.len();
        if end > current {
            let length = libc::off_t::try_from(end - current)
                .map_err(|_| io::Error::from(io::ErrorKind::FileTooLarge))?;
            // `F_PEOFPOSMODE` allocates `fst_length` bytes past the physical end of the file,
            // which equals its logical end because every earlier segment was preallocated.
            let mut store = libc::fstore_t {
                fst_flags: libc::F_ALLOCATECONTIG | libc::F_ALLOCATEALL,
                fst_posmode: libc::F_PEOFPOSMODE,
                fst_offset: 0,
                fst_length: length,
                fst_bytesalloc: 0,
            };
            // SAFETY: valid descriptor; `store` is a live, properly initialized `fstore_t` that
            // `fcntl(F_PREALLOCATE)` reads and updates in place during the call.
            let mut ret = unsafe {
                libc::fcntl(
                    file.as_raw_fd(),
                    libc::F_PREALLOCATE,
                    &mut store as *mut libc::fstore_t,
                )
            };
            if ret == -1 {
                // No contiguous run available: accept a fragmented allocation.
                store.fst_flags = libc::F_ALLOCATEALL;
                // SAFETY: as above.
                ret = unsafe {
                    libc::fcntl(
                        file.as_raw_fd(),
                        libc::F_PREALLOCATE,
                        &mut store as *mut libc::fstore_t,
                    )
                };
            }
            if ret == -1 {
                return Err(io::Error::last_os_error());
            }
        }
        super::extend(file, end)?;
        Ok(Reservation::Reserved)
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io;

    use super::Reservation;

    pub(super) fn preallocate(
        file: &File,
        _offset: u64,
        _len: u64,
        end: u64,
    ) -> io::Result<Reservation> {
        super::extend(file, end)?;
        Ok(Reservation::Reserved)
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows
)))]
mod imp {
    use std::fs::File;
    use std::io;

    use super::Reservation;

    pub(super) fn preallocate(
        file: &File,
        _offset: u64,
        _len: u64,
        end: u64,
    ) -> io::Result<Reservation> {
        super::extend(file, end)?;
        Ok(Reservation::Unreserved)
    }
}

/// Extends `file` to `end` bytes if it is shorter.
fn extend(file: &File, end: u64) -> io::Result<()> {
    if file.metadata()?.len() < end {
        file.set_len(end)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> (std::path::PathBuf, File) {
        let path = std::env::temp_dir().join(format!(
            "vox-project-prealloc-{}-{name}",
            std::process::id()
        ));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        (path, file)
    }

    #[test]
    fn preallocation_extends_the_file() {
        let (path, file) = temp_file("extends");
        let first = preallocate(&file, 0, 1 << 20).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 1 << 20);
        preallocate(&file, 1 << 20, 1 << 20).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 2 << 20);
        // tmpfs, ext4, xfs and btrfs all support `fallocate`.
        #[cfg(target_os = "linux")]
        assert_eq!(first, Reservation::Reserved);
        let _ = first;
        drop(file);
        std::fs::remove_file(path).unwrap();
    }

    /// A real (not injected) allocation failure surfaces as an error, not a crash: 1 EiB exceeds
    /// every file system's free space or maximum file size.
    #[test]
    fn impossible_preallocation_is_an_error() {
        let (path, file) = temp_file("impossible");
        let result = preallocate(&file, 0, 1 << 60);
        assert!(
            result.is_err(),
            "1 EiB preallocation unexpectedly succeeded"
        );
        drop(file);
        std::fs::remove_file(path).unwrap();
    }
}
