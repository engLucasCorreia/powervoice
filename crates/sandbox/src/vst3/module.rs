//! Loading a `.vst3` module: its binary for this platform, the per-OS entry and exit,
//! `GetPluginFactory`, and the factory's class list.

use std::ffi::{c_char, c_void};
use std::path::Path;

use vox_sandbox_ipc::vst3::{arch_folder, binary_path};
use vst3::Steinberg::{
    FIDString, IPluginFactory, IPluginFactory2, IPluginFactory2Trait, IPluginFactoryTrait,
    PClassInfo, PClassInfo2, PFactoryInfo, TUID, kResultOk,
};
use vst3::{ComPtr, Interface};

type ExitFn = unsafe extern "system" fn() -> bool;
type GetFactoryFn = unsafe extern "system" fn() -> *mut IPluginFactory;

/// One class the factory offers.
#[derive(Clone, Debug)]
pub(crate) struct ClassInfo {
    pub(crate) cid: TUID,
    pub(crate) category: String,
    pub(crate) name: String,
    pub(crate) vendor: String,
    pub(crate) version: String,
    /// `|`-separated (`"Fx|EQ"`); empty without `IPluginFactory2`.
    pub(crate) sub_categories: String,
}

/// A fixed-size C string field.
pub(crate) fn fixed_str(a: &[c_char]) -> String {
    let bytes: Vec<u8> = a
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

/// A loaded, entered VST3 module. On drop: the factory is released, the module exit runs, then
/// the library unloads (every instance holds an `Arc` of it, so it's dropped last).
pub(crate) struct Vst3Module {
    factory: Option<ComPtr<IPluginFactory>>,
    exit: Option<ExitFn>,
    /// `PFactoryInfo` vendor and URL (a class without its own vendor uses the factory's).
    pub(crate) vendor: String,
    pub(crate) url: String,
    lib: Option<libloading::Library>,
    #[cfg(target_os = "macos")]
    _bundle: mac::Bundle,
}

impl Vst3Module {
    /// Loads `path` (a bundle, or a regular file loaded as the binary itself), runs the module
    /// entry and gets the factory.
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let binary = binary_path(path).ok_or_else(|| {
            format!(
                "{} has no VST3 binary for this platform ({})",
                path.display(),
                arch_folder()
            )
        })?;
        // SAFETY: running a foreign library's initialisers is what the sandbox exists for: it
        // happens in this disposable process only (ADR-008 §1), never in the editor.
        let lib = unsafe { libloading::Library::new(&binary) }
            .map_err(|e| format!("couldn't load {}: {e}", path.display()))?;
        // SAFETY: `GetPluginFactory`'s documented signature.
        let get_factory: GetFactoryFn =
            match unsafe { lib.get::<GetFactoryFn>(b"GetPluginFactory\0") } {
                Ok(f) => *f,
                Err(_) => return Err(format!("{} is not a VST3 plugin", path.display())),
            };
        #[cfg(target_os = "macos")]
        let bundle = mac::Bundle::open(path)
            .ok_or_else(|| format!("{} is not a macOS bundle", path.display()))?;
        #[cfg(target_os = "macos")]
        let (lib, exit) = enter(lib, path, bundle.0)?;
        #[cfg(not(target_os = "macos"))]
        let (lib, exit) = enter(lib, path)?;
        let mut me = Self {
            factory: None,
            exit,
            vendor: String::new(),
            url: String::new(),
            lib: Some(lib),
            #[cfg(target_os = "macos")]
            _bundle: bundle,
        };
        // From here on, `Drop` runs the module exit on every error path.
        // SAFETY: the entry succeeded; the factory is a new reference we own.
        let factory = unsafe { ComPtr::from_raw(get_factory()) }
            .ok_or_else(|| format!("{} has no plugin factory", path.display()))?;
        // SAFETY: zeroed POD the factory fills.
        let mut info: PFactoryInfo = unsafe { std::mem::zeroed() };
        // SAFETY: a live factory; `info` is writable.
        if unsafe { factory.getFactoryInfo(&mut info) } == kResultOk {
            me.vendor = fixed_str(&info.vendor);
            me.url = fixed_str(&info.url);
        }
        me.factory = Some(factory);
        Ok(me)
    }

    /// Every class of the factory (`IPluginFactory2`'s richer info when offered).
    pub(crate) fn classes(&self) -> Vec<ClassInfo> {
        let Some(f) = &self.factory else {
            return Vec::new();
        };
        let f2 = f.cast::<IPluginFactory2>();
        let mut out = Vec::new();
        // SAFETY: a live factory, main thread.
        for i in 0..unsafe { f.countClasses() } {
            if let Some(f2) = &f2 {
                // SAFETY: zeroed POD the factory fills.
                let mut c: PClassInfo2 = unsafe { std::mem::zeroed() };
                // SAFETY: `i < countClasses`; `c` is writable.
                if unsafe { f2.getClassInfo2(i, &mut c) } == kResultOk {
                    let vendor = fixed_str(&c.vendor);
                    out.push(ClassInfo {
                        cid: c.cid,
                        category: fixed_str(&c.category),
                        name: fixed_str(&c.name),
                        vendor: if vendor.is_empty() {
                            self.vendor.clone()
                        } else {
                            vendor
                        },
                        version: fixed_str(&c.version),
                        sub_categories: fixed_str(&c.subCategories),
                    });
                    continue;
                }
            }
            // SAFETY: zeroed POD the factory fills.
            let mut c: PClassInfo = unsafe { std::mem::zeroed() };
            // SAFETY: `i < countClasses`; `c` is writable.
            if unsafe { f.getClassInfo(i, &mut c) } == kResultOk {
                out.push(ClassInfo {
                    cid: c.cid,
                    category: fixed_str(&c.category),
                    name: fixed_str(&c.name),
                    vendor: self.vendor.clone(),
                    version: String::new(),
                    sub_categories: String::new(),
                });
            }
        }
        out
    }

    /// A new instance of class `cid` as interface `I` (a reference we own).
    pub(crate) fn create<I: Interface>(&self, cid: &TUID) -> Option<ComPtr<I>> {
        let f = self.factory.as_ref()?;
        let mut obj: *mut c_void = std::ptr::null_mut();
        // SAFETY: a live factory; a 16-byte class id and interface id; `obj` is writable.
        let r = unsafe {
            f.createInstance(
                cid.as_ptr() as FIDString,
                I::IID.as_ptr() as FIDString,
                &mut obj,
            )
        };
        // SAFETY: on success `obj` is an `I` whose reference we now own.
        (r == kResultOk)
            .then(|| unsafe { ComPtr::from_raw(obj.cast::<I>()) })
            .flatten()
    }
}

impl Drop for Vst3Module {
    fn drop(&mut self) {
        // The factory first, then the module exit, then the unload (field order below).
        self.factory = None;
        if let Some(exit) = self.exit {
            // SAFETY: the entry succeeded; exit once, after every object of the module was
            // released (instances hold an `Arc` of this module).
            unsafe { exit() };
        }
        self.lib = None;
    }
}

/// Linux (and other ELF systems): `ModuleEntry(dlopen handle)` / `ModuleExit()`.
#[cfg(all(unix, not(target_os = "macos")))]
fn enter(
    lib: libloading::Library,
    path: &Path,
) -> Result<(libloading::Library, Option<ExitFn>), String> {
    type EntryFn = unsafe extern "system" fn(*mut c_void) -> bool;
    let unix: libloading::os::unix::Library = lib.into();
    let handle = unix.into_raw();
    // SAFETY: the handle just taken out of a live library.
    let lib: libloading::Library =
        unsafe { libloading::os::unix::Library::from_raw(handle) }.into();
    // SAFETY: the SDK's documented signatures.
    let entry = unsafe { lib.get::<EntryFn>(b"ModuleEntry\0") }
        .ok()
        .map(|f| *f);
    // SAFETY: as above.
    let exit = unsafe { lib.get::<ExitFn>(b"ModuleExit\0") }
        .ok()
        .map(|f| *f);
    // SAFETY: the module's entry, once, with its own dlopen handle, before anything else.
    if let Some(entry) = entry
        && !unsafe { entry(handle) }
    {
        return Err(format!("{} failed to initialise", path.display()));
    }
    Ok((lib, exit))
}

/// Windows: the optional `InitDll()` / `ExitDll()`.
#[cfg(windows)]
fn enter(
    lib: libloading::Library,
    path: &Path,
) -> Result<(libloading::Library, Option<ExitFn>), String> {
    // SAFETY: the SDK's documented signatures.
    let init = unsafe { lib.get::<ExitFn>(b"InitDll\0") }.ok().map(|f| *f);
    // SAFETY: as above.
    let exit = unsafe { lib.get::<ExitFn>(b"ExitDll\0") }.ok().map(|f| *f);
    // SAFETY: the DLL's entry, once, before anything else.
    if let Some(init) = init
        && !unsafe { init() }
    {
        return Err(format!("{} failed to initialise", path.display()));
    }
    Ok((lib, exit))
}

/// macOS: `bundleEntry(CFBundleRef)` / `bundleExit()` (older modules: `BundleEntry`/`BundleExit`).
#[cfg(target_os = "macos")]
fn enter(
    lib: libloading::Library,
    path: &Path,
    bundle: *mut c_void,
) -> Result<(libloading::Library, Option<ExitFn>), String> {
    type EntryFn = unsafe extern "system" fn(*mut c_void) -> bool;
    let symbol = |a: &[u8], b: &[u8]| {
        // SAFETY: the SDK's documented signatures.
        unsafe { lib.get::<EntryFn>(a).or_else(|_| lib.get::<EntryFn>(b)) }
            .ok()
            .map(|f| *f)
    };
    let entry = symbol(b"bundleEntry\0", b"BundleEntry\0")
        .ok_or_else(|| format!("{} has no bundleEntry", path.display()))?;
    // SAFETY: as above.
    let exit = unsafe {
        lib.get::<ExitFn>(b"bundleExit\0")
            .or_else(|_| lib.get::<ExitFn>(b"BundleExit\0"))
    }
    .ok()
    .map(|f| *f);
    // SAFETY: the bundle's entry, once, with its CFBundle, before anything else.
    if !unsafe { entry(bundle) } {
        return Err(format!("{} failed to initialise", path.display()));
    }
    Ok((lib, exit))
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;
    use std::path::Path;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            buffer: *const u8,
            len: isize,
            is_directory: u8,
        ) -> *const c_void;
        fn CFBundleCreate(allocator: *const c_void, url: *const c_void) -> *mut c_void;
        fn CFRelease(cf: *const c_void);
    }

    /// A `CFBundleRef` for the module's bundle (what `bundleEntry` takes).
    pub(super) struct Bundle(pub(super) *mut c_void);

    // SAFETY: CoreFoundation objects may be retained/released from any thread; the pointer is
    // only handed to the module's entry.
    unsafe impl Send for Bundle {}
    // SAFETY: see `Send`.
    unsafe impl Sync for Bundle {}

    impl Bundle {
        pub(super) fn open(path: &Path) -> Option<Self> {
            use std::os::unix::ffi::OsStrExt;
            let bytes = path.as_os_str().as_bytes();
            // SAFETY: a valid byte buffer of `len` bytes; the default allocator.
            let url = unsafe {
                CFURLCreateFromFileSystemRepresentation(
                    std::ptr::null(),
                    bytes.as_ptr(),
                    bytes.len() as isize,
                    1,
                )
            };
            if url.is_null() {
                return None;
            }
            // SAFETY: a valid URL; released right after.
            let bundle = unsafe { CFBundleCreate(std::ptr::null(), url) };
            // SAFETY: the URL we created.
            unsafe { CFRelease(url) };
            (!bundle.is_null()).then_some(Self(bundle))
        }
    }

    impl Drop for Bundle {
        fn drop(&mut self) {
            // SAFETY: the bundle we created, released once.
            unsafe { CFRelease(self.0) };
        }
    }
}
