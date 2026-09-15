//! One ysfx effect behind a safe(ish) wrapper (`vox-ysfx-sys`): load and compile a script,
//! read its header, drive it and save/restore it. Not synchronised itself — the instance keeps
//! it behind a mutex, so one thread uses it at a time.

use std::ffi::{CStr, CString, c_char};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use vox_ysfx_sys as sys;

use super::params::RawSlider;

/// What ysfx reports while a script loads and compiles.
struct Log {
    /// Set only while loading and compiling (main thread): ysfx never logs from `@sample`, and
    /// the audio thread must not print anyway.
    capturing: AtomicBool,
    errors: Mutex<Vec<String>>,
}

unsafe extern "C" fn report(userdata: isize, level: sys::ysfx_log_level, message: *const c_char) {
    if userdata == 0 || message.is_null() {
        return;
    }
    // SAFETY: `userdata` is the boxed `Log` of the `Fx` that owns this effect (dropped after
    // `ysfx_free`); `message` is NUL-terminated.
    let (log, message) = unsafe { (&*(userdata as *const Log), CStr::from_ptr(message)) };
    if !log.capturing.load(Ordering::Acquire) {
        return;
    }
    let text = message.to_string_lossy().into_owned();
    if level >= sys::YSFX_LOG_WARNING {
        eprintln!("powervoice-sandbox: [jsfx] {text}");
    }
    if level >= sys::YSFX_LOG_ERROR {
        log.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(text);
    }
}

/// A string ysfx returns (NUL-terminated, owned by the effect).
fn text(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: see above; copied at once.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// `dir` with a final separator: ysfx compares import roots as strings.
fn dir_string(dir: &Path) -> Option<CString> {
    let mut bytes = dir.as_os_str().as_bytes().to_vec();
    if !bytes.ends_with(b"/") {
        bytes.push(b'/');
    }
    CString::new(bytes).ok()
}

/// A saved state: every slider's `(index, value)` and the `@serialize` bytes.
pub(crate) type SavedState = (Vec<(u32, f64)>, Vec<u8>);

/// One compiled script.
pub(crate) struct Fx {
    ptr: *mut sys::ysfx_t,
    log: Box<Log>,
}

// SAFETY: a ysfx effect isn't bound to a thread; every use goes through the instance's mutex.
unsafe impl Send for Fx {}

impl Fx {
    /// Loads and compiles the script at `path` (its imports resolved from its folder, then its
    /// effects root; `@gfx` isn't compiled).
    pub(crate) fn load(path: &Path) -> Result<Self, String> {
        let file = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| format!("{}: unusable path", path.display()))?;
        let root = vox_sandbox_ipc::jsfx::effects_root(path);
        let data = root.parent().map(|p| p.join("Data")).filter(|d| d.is_dir());
        let log = Box::new(Log {
            capturing: AtomicBool::new(true),
            errors: Mutex::new(Vec::new()),
        });
        // SAFETY: ysfx's lifecycle: a configuration, an effect that takes its own reference to
        // it, ours dropped; the strings are copied; the log outlives the effect (see `Drop`).
        let ptr = unsafe {
            let config = sys::ysfx_config_new();
            if config.is_null() {
                return Err("ysfx couldn't allocate a configuration".into());
            }
            sys::ysfx_set_log_reporter(config, report);
            sys::ysfx_set_user_data(config, std::ptr::from_ref::<Log>(&log) as isize);
            sys::ysfx_register_builtin_audio_formats(config);
            if let Some(r) = dir_string(&root) {
                sys::ysfx_set_import_root(config, r.as_ptr());
            }
            if let Some(d) = data.as_deref().and_then(dir_string) {
                sys::ysfx_set_data_root(config, d.as_ptr());
            }
            let fx = sys::ysfx_new(config);
            sys::ysfx_config_free(config);
            fx
        };
        if ptr.is_null() {
            return Err("ysfx couldn't allocate an effect".into());
        }
        let me = Self { ptr, log };
        // SAFETY: a live effect and a NUL-terminated path.
        let ok = unsafe {
            sys::ysfx_load_file(me.ptr, file.as_ptr(), 0)
                && sys::ysfx_compile(me.ptr, sys::YSFX_COMPILE_NO_GFX)
        };
        me.log.capturing.store(false, Ordering::Release);
        if ok { Ok(me) } else { Err(me.failure(path)) }
    }

    fn failure(&self, path: &Path) -> String {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let errors = self
            .log
            .errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match errors.last() {
            Some(e) => format!("{name} can't be used: {e}"),
            None => format!("{name} isn't a usable JSFX script"),
        }
    }

    /// The `desc:` line.
    pub(crate) fn name(&self) -> String {
        // SAFETY: a live, loaded effect.
        text(unsafe { sys::ysfx_get_name(self.ptr) })
    }

    /// The `author:` line.
    pub(crate) fn author(&self) -> String {
        // SAFETY: as above.
        text(unsafe { sys::ysfx_get_author(self.ptr) })
    }

    /// The `tags:` words.
    pub(crate) fn tags(&self) -> Vec<String> {
        // SAFETY: a count query, then at most that many borrowed strings, copied at once.
        unsafe {
            let n = sys::ysfx_get_tags(self.ptr, std::ptr::null_mut(), 0);
            let mut v = vec![std::ptr::null::<c_char>(); n as usize];
            let got = sys::ysfx_get_tags(self.ptr, v.as_mut_ptr(), n);
            v.truncate(got.min(n) as usize);
            v.into_iter().map(text).collect()
        }
    }

    /// Input pins.
    pub(crate) fn num_inputs(&self) -> u32 {
        // SAFETY: a live effect.
        unsafe { sys::ysfx_get_num_inputs(self.ptr) }
    }

    /// Output pins.
    pub(crate) fn num_outputs(&self) -> u32 {
        // SAFETY: a live effect.
        unsafe { sys::ysfx_get_num_outputs(self.ptr) }
    }

    /// Whether the script has section `t` (`YSFX_SECTION_*`).
    pub(crate) fn has_section(&self, t: u32) -> bool {
        // SAFETY: a live effect.
        unsafe { sys::ysfx_has_section(self.ptr, t) }
    }

    /// Every slider the header declares.
    pub(crate) fn sliders(&self) -> Vec<RawSlider> {
        let mut out = Vec::new();
        for index in 0..sys::YSFX_MAX_SLIDERS {
            // SAFETY: a live effect; `index` < the slider count; the curve is ours; enum names
            // are borrowed strings copied at once.
            unsafe {
                if !sys::ysfx_slider_exists(self.ptr, index) {
                    continue;
                }
                let mut c = sys::ysfx_slider_curve_t::default();
                sys::ysfx_slider_get_curve(self.ptr, index, &mut c);
                let mut enum_names = Vec::new();
                if sys::ysfx_slider_is_enum(self.ptr, index) {
                    let n =
                        sys::ysfx_slider_get_enum_names(self.ptr, index, std::ptr::null_mut(), 0);
                    let mut v = vec![std::ptr::null::<c_char>(); n as usize];
                    let got = sys::ysfx_slider_get_enum_names(self.ptr, index, v.as_mut_ptr(), n);
                    v.truncate(got.min(n) as usize);
                    enum_names = v.into_iter().map(text).collect();
                }
                out.push(RawSlider {
                    index,
                    name: text(sys::ysfx_slider_get_name(self.ptr, index)),
                    def: c.def,
                    min: c.min,
                    max: c.max,
                    inc: c.inc,
                    log: c.shape == sys::YSFX_SLIDER_SHAPE_LOG,
                    enum_names,
                    visible: sys::ysfx_slider_is_initially_visible(self.ptr, index),
                });
            }
        }
        out
    }

    /// Slider `index`'s value.
    pub(crate) fn slider_value(&self, index: u32) -> f64 {
        // SAFETY: a live effect (out-of-range indices return 0).
        unsafe { sys::ysfx_slider_get_value(self.ptr, index) }
    }

    /// Sets slider `index`; `@slider` runs before the next processed frames. Real-time safe.
    pub(crate) fn set_slider(&mut self, index: u32, value: f64) {
        // SAFETY: a live effect (out-of-range indices are ignored).
        unsafe { sys::ysfx_slider_set_value(self.ptr, index, value, true) }
    }

    /// Sets the rate and block size and runs `@init` (main thread).
    pub(crate) fn prepare(&mut self, rate: f64, max_block: u32) {
        // SAFETY: a live, compiled effect.
        unsafe {
            sys::ysfx_set_sample_rate(self.ptr, rate);
            sys::ysfx_set_block_size(self.ptr, max_block);
            sys::ysfx_init(self.ptr);
        }
    }

    /// `pdc_delay` (samples; never negative).
    pub(crate) fn pdc_delay(&self) -> f64 {
        // SAFETY: a live effect.
        unsafe { sys::ysfx_get_pdc_delay(self.ptr) }
    }

    /// Runs `@slider` (when a slider changed), `@block` and `@sample` over `frames` frames.
    ///
    /// # Safety
    /// `ins` has one pointer per input pin and `outs` one per output pin, each valid for
    /// `frames` samples; nothing else uses the effect meanwhile.
    pub(crate) unsafe fn process(&mut self, ins: &[*const f32], outs: &[*mut f32], frames: u32) {
        // SAFETY: caller's contract.
        unsafe {
            sys::ysfx_process_float(
                self.ptr,
                ins.as_ptr(),
                outs.as_ptr(),
                ins.len() as u32,
                outs.len() as u32,
                frames,
            );
        }
    }

    /// Every slider's `(index, value)` and the `@serialize` bytes.
    pub(crate) fn save_state(&mut self) -> Result<SavedState, String> {
        // SAFETY: a live effect.
        let st = unsafe { sys::ysfx_save_state(self.ptr) };
        if st.is_null() {
            return Err("the script isn't compiled".into());
        }
        // SAFETY: a state ysfx allocated (`slider_count` sliders, `data_size` bytes), read, then
        // freed once.
        unsafe {
            let s = &*st;
            let sliders = if s.sliders.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(s.sliders, s.slider_count as usize)
                    .iter()
                    .map(|x| (x.index, x.value))
                    .collect()
            };
            let data = if s.data.is_null() || s.data_size == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(s.data, s.data_size).to_vec()
            };
            sys::ysfx_state_free(st);
            Ok((sliders, data))
        }
    }

    /// Restores sliders (the others go back to their defaults) and runs `@serialize` over
    /// `data`.
    pub(crate) fn load_state(&mut self, sliders: &[(u32, f64)], data: &[u8]) -> Result<(), String> {
        let mut sl: Vec<sys::ysfx_state_slider_t> = sliders
            .iter()
            .map(|&(index, value)| sys::ysfx_state_slider_t { index, value })
            .collect();
        let mut bytes = data.to_vec();
        let mut st = sys::ysfx_state_t {
            sliders: sl.as_mut_ptr(),
            slider_count: sl.len() as u32,
            data: bytes.as_mut_ptr(),
            data_size: bytes.len(),
        };
        // SAFETY: a live effect; the state points at our buffers for the call (ysfx copies).
        if unsafe { sys::ysfx_load_state(self.ptr, &mut st) } {
            Ok(())
        } else {
            Err("the script couldn't load the state".into())
        }
    }
}

impl Drop for Fx {
    fn drop(&mut self) {
        // SAFETY: our effect, freed once; the log box drops after this.
        unsafe { sys::ysfx_free(self.ptr) };
    }
}
