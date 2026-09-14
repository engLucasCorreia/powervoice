//! Shared-memory segments (ADR-008 §3): a thin OS layer, no framework.
//!
//! | Platform | [`SharedRegion::create`] | Handle string | Child side |
//! |---|---|---|---|
//! | Linux | `memfd_create` + `ftruncate` + seals (shrink/grow/seal) | `memfd:<fd>` | fd inherited (only by the child passed to [`SharedRegion::share_with`]) |
//! | other unix (macOS) | `shm_open` with a random name, `O_EXCL`, mode 0600 | `posix:<name>` | `shm_open` by name; the creator unlinks after the handshake ([`SharedRegion::unlink`]) or on drop |
//! | Windows | pagefile-backed `CreateFileMappingW`, random `Local\` name | `win:<name>:<len>` | `OpenFileMappingW` by name |
//!
//! The POSIX named path also builds (and is tested) on Linux via [`SharedRegion::create_named`].
//! Mappings are read/write and shared; the memory is only ever accessed through atomics.

use std::io;
use std::process::Command;
use std::ptr::NonNull;

/// A mapped shared-memory segment. Unmapped (and, for a named segment the creator still owns,
/// unlinked) on drop — which is a syscall: drop it off the audio thread (return ring).
pub struct SharedRegion {
    ptr: NonNull<u8>,
    len: usize,
    os: imp::Handle,
}

// SAFETY: the region is plain shared memory; this crate only accesses it through atomics, and
// the OS handle is only used by `&self` methods that are thread-safe syscalls (or by `Drop`).
unsafe impl Send for SharedRegion {}
// SAFETY: see `Send`: all access through `&self` is atomic or a thread-safe syscall.
unsafe impl Sync for SharedRegion {}

impl std::fmt::Debug for SharedRegion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedRegion")
            .field("len", &self.len)
            .field("handle", &self.handle())
            .finish()
    }
}

impl SharedRegion {
    /// Creates a zero-filled segment of `len` bytes with the platform's default backend.
    pub fn create(len: usize) -> io::Result<Self> {
        let (ptr, os) = imp::create(len)?;
        Ok(Self { ptr, len, os })
    }

    /// Creates a POSIX named segment (`shm_open`; the macOS backend, also usable on Linux).
    #[cfg(unix)]
    pub fn create_named(len: usize) -> io::Result<Self> {
        let (ptr, os) = imp::create_named(len)?;
        Ok(Self { ptr, len, os })
    }

    /// Opens a segment from its [`handle`](Self::handle) string (the plugin side).
    pub fn open(handle: &str) -> io::Result<Self> {
        let (ptr, len, os) = imp::open(handle)?;
        Ok(Self { ptr, len, os })
    }

    /// Maps the same segment a second time in this process (an in-process peer; tests).
    pub fn duplicate(&self) -> io::Result<Self> {
        let (ptr, len, os) = imp::duplicate(&self.os, self.len)?;
        Ok(Self { ptr, len, os })
    }

    /// The string a peer passes to [`open`](Self::open).
    pub fn handle(&self) -> String {
        imp::handle(&self.os, self.len)
    }

    /// Prepares `cmd` so its child can open this segment and returns the handle string to pass
    /// to it. Linux: clears `FD_CLOEXEC` on the memfd **in that child only** (`pre_exec`), so
    /// concurrently spawned processes don't inherit it. Named backends: nothing to do.
    pub fn share_with(&self, cmd: &mut Command) -> String {
        imp::share_with(&self.os, cmd);
        self.handle()
    }

    /// Removes a named segment's name now (both sides have it open). No-op for memfd.
    pub fn unlink(&self) {
        imp::unlink(&self.os);
    }

    /// Mapping length in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Always false (segments are never empty).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Base address (page-aligned).
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }
}

impl Drop for SharedRegion {
    fn drop(&mut self) {
        imp::release(self.ptr, self.len, &self.os);
    }
}

/// 64 random bits (std's per-process random SipHash keys; no extra dependency).
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    h.write_u64(crate::wakeup::now_ns());
    h.finish()
}

#[cfg(unix)]
mod imp {
    use std::ffi::CString;
    use std::io;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::process::Command;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub(super) enum Handle {
        #[cfg(target_os = "linux")]
        Memfd(OwnedFd),
        Named {
            name: CString,
            /// The creator unlinks the name on drop unless already unlinked.
            owner: bool,
            unlinked: AtomicBool,
        },
    }

    fn check(ret: libc::c_int) -> io::Result<libc::c_int> {
        if ret == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret)
        }
    }

    fn map(fd: RawFd, len: usize) -> io::Result<NonNull<u8>> {
        #[cfg(target_os = "linux")]
        let flags = libc::MAP_SHARED | libc::MAP_POPULATE;
        #[cfg(not(target_os = "linux"))]
        let flags = libc::MAP_SHARED;
        // SAFETY: a fresh mapping (no address hint) of `len` bytes of a valid fd; the result is
        // checked against MAP_FAILED.
        let p = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                flags,
                fd,
                0,
            )
        };
        if p == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        NonNull::new(p.cast::<u8>()).ok_or_else(|| io::Error::other("mmap returned null"))
    }

    fn fd_len(fd: RawFd) -> io::Result<usize> {
        // SAFETY: `st` is a valid, writable stat buffer; fstat on any fd value is safe.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: see above.
        check(unsafe { libc::fstat(fd, &mut st) })?;
        usize::try_from(st.st_size).map_err(|_| io::Error::other("negative segment size"))
    }

    fn truncate(fd: RawFd, len: usize) -> io::Result<()> {
        let len = libc::off_t::try_from(len).map_err(|_| io::Error::other("segment too large"))?;
        // SAFETY: plain syscall on an fd we own.
        check(unsafe { libc::ftruncate(fd, len) })?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(super) fn create(len: usize) -> io::Result<(NonNull<u8>, Handle)> {
        // SAFETY: the name is a valid NUL-terminated string; flags are valid.
        let raw = check(unsafe {
            libc::memfd_create(
                c"powervoice-sandbox".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        })?;
        // SAFETY: `raw` is a fresh fd we own.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        truncate(fd.as_raw_fd(), len)?;
        // SAFETY: plain fcntl on an fd we own. The seals freeze the size, so a peer can't
        // shrink the segment under the other side's mapping (SIGBUS).
        check(unsafe {
            libc::fcntl(
                fd.as_raw_fd(),
                libc::F_ADD_SEALS,
                libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL,
            )
        })?;
        let ptr = map(fd.as_raw_fd(), len)?;
        Ok((ptr, Handle::Memfd(fd)))
    }

    #[cfg(not(target_os = "linux"))]
    pub(super) fn create(len: usize) -> io::Result<(NonNull<u8>, Handle)> {
        create_named(len)
    }

    fn shm_open_raw(name: &CString, oflag: libc::c_int) -> libc::c_int {
        #[cfg(target_vendor = "apple")]
        {
            // SAFETY: valid NUL-terminated name; Apple's shm_open is variadic (mode promoted).
            unsafe { libc::shm_open(name.as_ptr(), oflag, 0o600 as libc::c_uint) }
        }
        #[cfg(not(target_vendor = "apple"))]
        {
            // SAFETY: valid NUL-terminated name, valid flags and mode.
            unsafe { libc::shm_open(name.as_ptr(), oflag, 0o600) }
        }
    }

    pub(super) fn create_named(len: usize) -> io::Result<(NonNull<u8>, Handle)> {
        for _ in 0..8 {
            // 21 characters: within macOS's 31-character PSHMNAMLEN.
            let name = CString::new(format!("/pvs.{:016x}", super::random_u64()))
                .map_err(io::Error::other)?;
            let raw = shm_open_raw(&name, libc::O_CREAT | libc::O_EXCL | libc::O_RDWR);
            if raw == -1 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EEXIST) {
                    continue;
                }
                return Err(err);
            }
            // SAFETY: `raw` is a fresh fd we own.
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            let handle = Handle::Named {
                name,
                owner: true,
                unlinked: AtomicBool::new(false),
            };
            // On error `handle` drops without unlinking: unlink explicitly.
            let mapped = truncate(fd.as_raw_fd(), len).and_then(|()| map(fd.as_raw_fd(), len));
            return match mapped {
                Ok(ptr) => Ok((ptr, handle)),
                Err(e) => {
                    unlink(&handle);
                    Err(e)
                }
            };
        }
        Err(io::Error::other("no free shared-memory name"))
    }

    pub(super) fn open(handle: &str) -> io::Result<(NonNull<u8>, usize, Handle)> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "bad segment handle");
        if let Some(fd) = handle.strip_prefix("memfd:") {
            #[cfg(target_os = "linux")]
            {
                let raw: RawFd = fd.parse().map_err(|_| invalid())?;
                if raw < 3 {
                    return Err(invalid());
                }
                let len = fd_len(raw)?;
                // SAFETY: plain fcntl; re-enables close-on-exec on the inherited fd so it
                // doesn't leak into this process's own children.
                check(unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) })?;
                // SAFETY: the fd was inherited for us (`share_with`) and nothing else owns it.
                let fd = unsafe { OwnedFd::from_raw_fd(raw) };
                let ptr = map(fd.as_raw_fd(), len)?;
                return Ok((ptr, len, Handle::Memfd(fd)));
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = fd;
                return Err(invalid());
            }
        }
        let name = handle.strip_prefix("posix:").ok_or_else(invalid)?;
        let name = CString::new(name).map_err(|_| invalid())?;
        open_named(name, false)
    }

    fn open_named(name: CString, owner: bool) -> io::Result<(NonNull<u8>, usize, Handle)> {
        let raw = check(shm_open_raw(&name, libc::O_RDWR))?;
        // SAFETY: `raw` is a fresh fd we own.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let len = fd_len(fd.as_raw_fd())?;
        let ptr = map(fd.as_raw_fd(), len)?;
        Ok((
            ptr,
            len,
            Handle::Named {
                name,
                owner,
                unlinked: AtomicBool::new(false),
            },
        ))
    }

    pub(super) fn duplicate(h: &Handle, len: usize) -> io::Result<(NonNull<u8>, usize, Handle)> {
        match h {
            #[cfg(target_os = "linux")]
            Handle::Memfd(fd) => {
                let fd = fd.try_clone()?;
                let ptr = map(fd.as_raw_fd(), len)?;
                Ok((ptr, len, Handle::Memfd(fd)))
            }
            Handle::Named { name, .. } => open_named(name.clone(), false),
        }
    }

    pub(super) fn handle(h: &Handle, _len: usize) -> String {
        match h {
            #[cfg(target_os = "linux")]
            Handle::Memfd(fd) => format!("memfd:{}", fd.as_raw_fd()),
            Handle::Named { name, .. } => format!("posix:{}", name.to_string_lossy()),
        }
    }

    pub(super) fn share_with(h: &Handle, cmd: &mut Command) {
        match h {
            #[cfg(target_os = "linux")]
            Handle::Memfd(fd) => {
                use std::os::unix::process::CommandExt;
                let raw = fd.as_raw_fd();
                // SAFETY: the closure runs in the forked child before exec and only calls
                // fcntl, which is async-signal-safe; it allocates nothing.
                unsafe {
                    cmd.pre_exec(move || {
                        if libc::fcntl(raw, libc::F_SETFD, 0) == -1 {
                            return Err(io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
            }
            Handle::Named { .. } => {
                let _ = cmd;
            }
        }
    }

    pub(super) fn unlink(h: &Handle) {
        #[cfg(target_os = "linux")]
        if let Handle::Memfd(_) = h {
            return;
        }
        if let Handle::Named { name, unlinked, .. } = h
            && !unlinked.swap(true, Ordering::AcqRel)
        {
            // SAFETY: valid NUL-terminated name; failure (already unlinked) is harmless.
            unsafe { libc::shm_unlink(name.as_ptr()) };
        }
    }

    pub(super) fn release(ptr: NonNull<u8>, len: usize, h: &Handle) {
        // SAFETY: `ptr`/`len` are exactly the mapping created for this region; after drop
        // nothing references it (every view borrows the region).
        unsafe { libc::munmap(ptr.as_ptr().cast(), len) };
        if let Handle::Named { owner: true, .. } = h {
            unlink(h);
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::process::Command;
    use std::ptr::NonNull;

    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::System::Memory::{
        CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
        OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
    };

    pub(super) struct Handle {
        mapping: HANDLE,
        name: String,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn view(mapping: HANDLE, len: usize) -> io::Result<NonNull<u8>> {
        // SAFETY: `mapping` is a valid file-mapping handle we own.
        let v = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, len) };
        NonNull::new(v.Value.cast::<u8>()).ok_or_else(io::Error::last_os_error)
    }

    pub(super) fn create(len: usize) -> io::Result<(NonNull<u8>, Handle)> {
        let size = len as u64;
        for _ in 0..8 {
            let name = format!(
                "Local\\powervoice-sbx-{:016x}{:016x}",
                super::random_u64(),
                super::random_u64()
            );
            let w = wide(&name);
            // SAFETY: pagefile-backed mapping (INVALID_HANDLE_VALUE), default security, valid
            // NUL-terminated wide name.
            let mapping = unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    std::ptr::null(),
                    PAGE_READWRITE,
                    (size >> 32) as u32,
                    size as u32,
                    w.as_ptr(),
                )
            };
            if mapping.is_null() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: plain thread-local error read right after the call.
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                // SAFETY: a handle we own.
                unsafe { CloseHandle(mapping) };
                continue;
            }
            return match view(mapping, len) {
                Ok(ptr) => Ok((ptr, Handle { mapping, name })),
                Err(e) => {
                    // SAFETY: a handle we own.
                    unsafe { CloseHandle(mapping) };
                    Err(e)
                }
            };
        }
        Err(io::Error::other("no free shared-memory name"))
    }

    fn open_named(name: &str, len: usize) -> io::Result<(NonNull<u8>, usize, Handle)> {
        let w = wide(name);
        // SAFETY: valid NUL-terminated wide name.
        let mapping = unsafe { OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, w.as_ptr()) };
        if mapping.is_null() {
            return Err(io::Error::last_os_error());
        }
        match view(mapping, len) {
            Ok(ptr) => Ok((
                ptr,
                len,
                Handle {
                    mapping,
                    name: name.to_owned(),
                },
            )),
            Err(e) => {
                // SAFETY: a handle we own.
                unsafe { CloseHandle(mapping) };
                Err(e)
            }
        }
    }

    pub(super) fn open(handle: &str) -> io::Result<(NonNull<u8>, usize, Handle)> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "bad segment handle");
        let rest = handle.strip_prefix("win:").ok_or_else(invalid)?;
        let (name, len) = rest.rsplit_once(':').ok_or_else(invalid)?;
        let len: usize = len.parse().map_err(|_| invalid())?;
        open_named(name, len)
    }

    pub(super) fn duplicate(h: &Handle, len: usize) -> io::Result<(NonNull<u8>, usize, Handle)> {
        open_named(&h.name, len)
    }

    pub(super) fn handle(h: &Handle, len: usize) -> String {
        format!("win:{}:{len}", h.name)
    }

    pub(super) fn share_with(_h: &Handle, _cmd: &mut Command) {}

    pub(super) fn unlink(_h: &Handle) {}

    pub(super) fn release(ptr: NonNull<u8>, _len: usize, h: &Handle) {
        // SAFETY: `ptr` is the view created for this region and `mapping` a handle we own;
        // nothing references the view after drop.
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: ptr.as_ptr().cast(),
            });
            CloseHandle(h.mapping);
        }
    }
}
