//! Free disk space (A-010, SPEC-002 §2.5 recording disk floor/remaining time, SPEC-004 §2.5
//! housekeeping): behind a small provider trait so the engine's periodic disk-space check and the
//! record panel's remaining-time display can inject fake values in tests instead of touching the
//! real file system.
//!
//! - Unix: `libc::statvfs` (already a `cfg(unix)` dependency of this crate, ADR-004 §2's
//!   preallocation code).
//! - Windows: `GetDiskFreeSpaceExW` (`windows-sys`, already in the dependency tree via
//!   tauri/cpal — A-010 allows a direct `cfg(windows)` edge instead of a new third-party crate).

use std::io;
use std::path::Path;

/// Free space on the volume holding a path.
pub trait FreeSpaceProvider: std::fmt::Debug + Send + Sync {
    /// Bytes free on the volume containing `path` (an existing file or directory).
    fn free_bytes(&self, path: &Path) -> io::Result<u64>;
}

/// The real provider.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemFreeSpace;

impl FreeSpaceProvider for SystemFreeSpace {
    fn free_bytes(&self, path: &Path) -> io::Result<u64> {
        imp::free_bytes(path)
    }
}

/// A provider whose value the caller sets directly (tests: the engine's disk-floor/remaining-time
/// checks, the record panel's soft-confirm threshold) — cheaply cloneable, so the same fake can be
/// shared with the code under test and updated (e.g. simulating the disk filling up) from the
/// test itself.
#[derive(Debug, Clone)]
pub struct FixedFreeSpace(std::sync::Arc<std::sync::atomic::AtomicU64>);

impl FixedFreeSpace {
    /// A provider that always reports `bytes` free, until [`Self::set`] changes it.
    pub fn new(bytes: u64) -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
            bytes,
        )))
    }

    /// Changes the reported free space.
    pub fn set(&self, bytes: u64) {
        self.0.store(bytes, std::sync::atomic::Ordering::Release);
    }
}

impl FreeSpaceProvider for FixedFreeSpace {
    fn free_bytes(&self, _path: &Path) -> io::Result<u64> {
        Ok(self.0.load(std::sync::atomic::Ordering::Acquire))
    }
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    pub(super) fn free_bytes(path: &Path) -> io::Result<u64> {
        let c_path = CString::new(path.as_os_str().as_bytes())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut stat = MaybeUninit::<libc::statvfs>::uninit();
        // SAFETY: `c_path` is a valid NUL-terminated C string for the call's duration, and `stat`
        // is a valid, writable `statvfs` the kernel fully initializes on success (checked below).
        let ret = unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) };
        if ret != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `statvfs` returned 0 (success), so `stat` is fully initialized.
        let stat = unsafe { stat.assume_init() };
        // Blocks available to an unprivileged process (`f_bavail`, not the possibly-larger
        // `f_bfree`) times the fragment size — widened through `u128` first so the multiplication
        // never overflows regardless of the platform's native field widths, then narrowed back
        // (saturating: no real volume is anywhere near `u64::MAX` bytes).
        let free = u128::from(stat.f_bavail) * u128::from(stat.f_frsize);
        Ok(u64::try_from(free).unwrap_or(u64::MAX))
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    pub(super) fn free_bytes(path: &Path) -> io::Result<u64> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut free_bytes_available: u64 = 0;
        // SAFETY: `wide` is a valid, NUL-terminated UTF-16 string for the call's duration, and
        // `free_bytes_available` is a valid, writable `u64` the API fills in on success; the
        // other two (unused) out-parameters are null, which the API accepts.
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_bytes_available,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(free_bytes_available)
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::io;
    use std::path::Path;

    pub(super) fn free_bytes(_path: &Path) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no free-space query on this platform",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_free_space_reports_something_plausible_for_the_temp_dir() {
        let free = SystemFreeSpace.free_bytes(&std::env::temp_dir()).unwrap();
        // Not a real bound, just a sanity check: it didn't error, and it isn't a bogus 0 on a
        // volume that (in CI and dev) always has some room.
        assert!(free > 0, "reported {free} bytes free");
    }

    #[test]
    fn system_free_space_errors_on_a_path_that_does_not_exist() {
        let missing = std::env::temp_dir().join("vox-project-disk-test-does-not-exist");
        assert!(SystemFreeSpace.free_bytes(&missing).is_err());
    }

    #[test]
    fn fixed_free_space_reports_and_updates_the_set_value() {
        let disk = FixedFreeSpace::new(1_000);
        assert_eq!(disk.free_bytes(Path::new("/anything")).unwrap(), 1_000);
        disk.set(0);
        assert_eq!(disk.free_bytes(Path::new("/anything")).unwrap(), 0);
    }
}
