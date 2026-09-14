//! Loading a `.clap` file: dlopen, `clap_entry`, `init`/`deinit`, the plugin factory.

use std::ffi::CString;
use std::path::{Path, PathBuf};

use vox_clap_abi::{
    CLAP_ENTRY_SYMBOL, CLAP_PLUGIN_FACTORY_ID, clap_plugin_entry, clap_plugin_factory,
    clap_version_is_compatible,
};

/// A loaded, initialised `.clap` file. `deinit` runs and the library unloads on drop (after
/// every plugin of it was destroyed: owners drop it last).
pub(crate) struct ClapLibrary {
    entry: *const clap_plugin_entry,
    _lib: libloading::Library,
}

// SAFETY: `entry` points into the loaded library, which lives as long as `self`; CLAP declares
// `get_factory` thread-safe and the factory's calls are made from the main thread only.
unsafe impl Send for ClapLibrary {}
// SAFETY: see `Send`.
unsafe impl Sync for ClapLibrary {}

/// The file to dlopen for a `.clap` path: the file itself (Linux, Windows) or the binary inside
/// a macOS bundle directory (`X.clap/Contents/MacOS/X`).
fn binary_path(path: &Path) -> PathBuf {
    if path.is_dir()
        && let Some(stem) = path.file_stem()
    {
        return path.join("Contents").join("MacOS").join(stem);
    }
    path.to_path_buf()
}

fn c_path(path: &Path) -> Result<CString, String> {
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.to_string_lossy().into_owned().into_bytes();
    CString::new(bytes).map_err(|_| format!("bad plugin path {}", path.display()))
}

impl ClapLibrary {
    /// Loads `path` and calls `clap_entry.init(path)`.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let binary = binary_path(path);
        // SAFETY: running a foreign library's initialisers is what the sandbox exists for: it
        // happens in this disposable process only (ADR-008 §1), never in the editor.
        let lib = unsafe { libloading::Library::new(&binary) }
            .map_err(|e| format!("couldn't load {}: {e}", path.display()))?;
        // SAFETY: `clap_entry` is a data symbol of type `clap_plugin_entry`; asking for
        // `*const clap_plugin_entry` yields its address.
        let entry: *const clap_plugin_entry =
            match unsafe { lib.get::<*const clap_plugin_entry>(CLAP_ENTRY_SYMBOL) } {
                Ok(sym) => *sym,
                Err(_) => return Err(format!("{} is not a CLAP plugin", path.display())),
            };
        if entry.is_null() {
            return Err(format!("{} is not a CLAP plugin", path.display()));
        }
        // SAFETY: non-null `clap_entry` of the loaded library.
        let (version, init) = unsafe { ((*entry).clap_version, (*entry).init) };
        if !clap_version_is_compatible(version) {
            return Err(format!(
                "{} uses CLAP {}.{}.{}, which is not supported",
                path.display(),
                version.major,
                version.minor,
                version.revision
            ));
        }
        let c_path = c_path(path)?;
        let init = init.ok_or_else(|| format!("{} has no clap_entry.init", path.display()))?;
        // SAFETY: CLAP's entry contract: `init` once, with the plugin path, before anything else.
        if !unsafe { init(c_path.as_ptr()) } {
            return Err(format!("{} failed to initialise", path.display()));
        }
        Ok(Self { entry, _lib: lib })
    }

    /// The plugin factory, if the file has one.
    pub(crate) fn plugin_factory(&self) -> Option<&clap_plugin_factory> {
        // SAFETY: `entry` is valid while the library is loaded.
        let get = unsafe { (*self.entry).get_factory }?;
        // SAFETY: `get_factory` with a valid factory id; the result is null or a factory that
        // lives as long as the library.
        let f = unsafe { get(CLAP_PLUGIN_FACTORY_ID.as_ptr()) } as *const clap_plugin_factory;
        // SAFETY: non-null factories stay valid until `deinit`.
        (!f.is_null()).then(|| unsafe { &*f })
    }
}

impl Drop for ClapLibrary {
    fn drop(&mut self) {
        // SAFETY: `init` succeeded; `deinit` once, after every plugin was destroyed.
        unsafe {
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
        }
    }
}
