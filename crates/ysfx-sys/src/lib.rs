//! Hand-written bindings for the subset of the **ysfx** C API (`third_party/ysfx/include/ysfx.h`,
//! JoepVanlier fork, Apache-2.0) that `powervoice-sandbox`'s JSFX backend uses (T-808,
//! ADR-008 Amendment 11). The library is compiled from the vendored sources by `build.rs`.
//!
//! Present only on unix x86_64 / aarch64 (the targets `build.rs` compiles for); elsewhere the
//! crate is empty.
#![cfg(all(unix, any(target_arch = "x86_64", target_arch = "aarch64")))]
#![allow(non_camel_case_types)]

use std::ffi::c_char;
use std::marker::{PhantomData, PhantomPinned};

/// `ysfx_real`: every script value is a `double`.
pub type ysfx_real = f64;

/// `ysfx_max_sliders`.
pub const YSFX_MAX_SLIDERS: u32 = 256;
/// `ysfx_max_channels`.
pub const YSFX_MAX_CHANNELS: u32 = 64;

/// `ysfx_log_level`.
pub type ysfx_log_level = u32;
/// `ysfx_log_info`.
pub const YSFX_LOG_INFO: ysfx_log_level = 0;
/// `ysfx_log_warning`.
pub const YSFX_LOG_WARNING: ysfx_log_level = 1;
/// `ysfx_log_error`.
pub const YSFX_LOG_ERROR: ysfx_log_level = 2;

/// `ysfx_section_type_t`.
pub const YSFX_SECTION_INIT: u32 = 1;
/// `ysfx_section_slider`.
pub const YSFX_SECTION_SLIDER: u32 = 2;
/// `ysfx_section_block`.
pub const YSFX_SECTION_BLOCK: u32 = 3;
/// `ysfx_section_sample`.
pub const YSFX_SECTION_SAMPLE: u32 = 4;
/// `ysfx_section_gfx`.
pub const YSFX_SECTION_GFX: u32 = 5;
/// `ysfx_section_serialize`.
pub const YSFX_SECTION_SERIALIZE: u32 = 6;

/// `ysfx_compile_no_gfx`: skip compiling the `@gfx` section.
pub const YSFX_COMPILE_NO_GFX: u32 = 1 << 1;

/// Slider shapes (`ysfx_slider_curve_t::shape`).
pub const YSFX_SLIDER_SHAPE_LINEAR: u8 = 0;
/// Logarithmic slider (`:log` modifier).
pub const YSFX_SLIDER_SHAPE_LOG: u8 = 1;
/// Square-law slider (`:sqr` modifier).
pub const YSFX_SLIDER_SHAPE_SQR: u8 = 2;

macro_rules! opaque {
    ($($name:ident),*) => {$(
        /// An opaque ysfx object.
        #[repr(C)]
        pub struct $name {
            _data: [u8; 0],
            _marker: PhantomData<(*mut u8, PhantomPinned)>,
        }
    )*};
}

opaque!(ysfx_config_t, ysfx_t);

/// `ysfx_log_reporter_t`.
pub type ysfx_log_reporter_t =
    unsafe extern "C" fn(userdata: isize, level: ysfx_log_level, message: *const c_char);

/// `ysfx_slider_curve_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ysfx_slider_curve_t {
    /// Default value.
    pub def: ysfx_real,
    /// Minimum.
    pub min: ysfx_real,
    /// Maximum.
    pub max: ysfx_real,
    /// Increment (0: continuous).
    pub inc: ysfx_real,
    /// `YSFX_SLIDER_SHAPE_*`.
    pub shape: u8,
    /// The shape's modifier (log: the centre value; sqr: the exponent).
    pub modifier: ysfx_real,
}

/// `ysfx_state_slider_t`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ysfx_state_slider_t {
    /// Slider index (0-based).
    pub index: u32,
    /// Its value.
    pub value: ysfx_real,
}

/// `ysfx_state_t`.
#[repr(C)]
#[derive(Debug)]
pub struct ysfx_state_t {
    /// Slider values.
    pub sliders: *mut ysfx_state_slider_t,
    /// Their count.
    pub slider_count: u32,
    /// The `@serialize` bytes.
    pub data: *mut u8,
    /// Their size.
    pub data_size: usize,
}

// The library is `libysfx.a` (build.rs) and needs the C++ runtime, which `cc` links.
#[link(name = "ysfx", kind = "static")]
unsafe extern "C" {
    pub fn ysfx_config_new() -> *mut ysfx_config_t;
    pub fn ysfx_config_free(config: *mut ysfx_config_t);
    pub fn ysfx_set_import_root(config: *mut ysfx_config_t, root: *const c_char);
    pub fn ysfx_set_data_root(config: *mut ysfx_config_t, root: *const c_char);
    pub fn ysfx_guess_file_roots(config: *mut ysfx_config_t, sourcepath: *const c_char);
    pub fn ysfx_register_builtin_audio_formats(config: *mut ysfx_config_t);
    pub fn ysfx_set_log_reporter(config: *mut ysfx_config_t, reporter: ysfx_log_reporter_t);
    pub fn ysfx_set_user_data(config: *mut ysfx_config_t, userdata: isize);

    pub fn ysfx_new(config: *mut ysfx_config_t) -> *mut ysfx_t;
    pub fn ysfx_free(fx: *mut ysfx_t);
    pub fn ysfx_load_file(fx: *mut ysfx_t, filepath: *const c_char, loadopts: u32) -> bool;
    pub fn ysfx_compile(fx: *mut ysfx_t, compileopts: u32) -> bool;
    pub fn ysfx_is_compiled(fx: *mut ysfx_t) -> bool;

    pub fn ysfx_get_name(fx: *mut ysfx_t) -> *const c_char;
    pub fn ysfx_get_author(fx: *mut ysfx_t) -> *const c_char;
    pub fn ysfx_get_tags(fx: *mut ysfx_t, dest: *mut *const c_char, destsize: u32) -> u32;
    pub fn ysfx_get_num_inputs(fx: *mut ysfx_t) -> u32;
    pub fn ysfx_get_num_outputs(fx: *mut ysfx_t) -> u32;
    pub fn ysfx_has_section(fx: *mut ysfx_t, type_: u32) -> bool;

    pub fn ysfx_slider_exists(fx: *mut ysfx_t, index: u32) -> bool;
    pub fn ysfx_slider_get_name(fx: *mut ysfx_t, index: u32) -> *const c_char;
    pub fn ysfx_slider_get_curve(
        fx: *mut ysfx_t,
        index: u32,
        curve: *mut ysfx_slider_curve_t,
    ) -> bool;
    pub fn ysfx_slider_is_enum(fx: *mut ysfx_t, index: u32) -> bool;
    pub fn ysfx_slider_get_enum_names(
        fx: *mut ysfx_t,
        index: u32,
        dest: *mut *const c_char,
        destsize: u32,
    ) -> u32;
    pub fn ysfx_slider_is_path(fx: *mut ysfx_t, index: u32) -> bool;
    pub fn ysfx_slider_is_initially_visible(fx: *mut ysfx_t, index: u32) -> bool;
    pub fn ysfx_slider_get_value(fx: *mut ysfx_t, index: u32) -> ysfx_real;
    pub fn ysfx_slider_set_value(fx: *mut ysfx_t, index: u32, value: ysfx_real, notify: bool);

    pub fn ysfx_set_block_size(fx: *mut ysfx_t, blocksize: u32);
    pub fn ysfx_set_sample_rate(fx: *mut ysfx_t, samplerate: ysfx_real);
    pub fn ysfx_init(fx: *mut ysfx_t);
    pub fn ysfx_get_pdc_delay(fx: *mut ysfx_t) -> ysfx_real;

    pub fn ysfx_process_float(
        fx: *mut ysfx_t,
        ins: *const *const f32,
        outs: *const *mut f32,
        num_ins: u32,
        num_outs: u32,
        num_frames: u32,
    );

    pub fn ysfx_save_state(fx: *mut ysfx_t) -> *mut ysfx_state_t;
    pub fn ysfx_load_state(fx: *mut ysfx_t, state: *mut ysfx_state_t) -> bool;
    pub fn ysfx_state_free(state: *mut ysfx_state_t);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn layouts_match_the_header() {
        // `double def, min, max, inc; uint8_t shape; double modifier` — the C layout.
        assert_eq!(std::mem::size_of::<ysfx_slider_curve_t>(), 48);
        assert_eq!(std::mem::offset_of!(ysfx_slider_curve_t, shape), 32);
        assert_eq!(std::mem::offset_of!(ysfx_slider_curve_t, modifier), 40);
        assert_eq!(std::mem::size_of::<ysfx_state_slider_t>(), 16);
        assert_eq!(std::mem::offset_of!(ysfx_state_t, data), 16);
    }

    #[test]
    fn compiles_and_runs_a_script() {
        let dir = std::env::temp_dir().join(format!("pv-ysfx-sys-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("half.jsfx");
        std::fs::write(
            &file,
            "desc:Half\nslider1:g=0.5<0,1,0.01>Gain\nin_pin:in\nout_pin:out\n@init\next_nodenorm = 1;\n\
             @sample\nspl0 *= slider1;\n",
        )
        .unwrap();
        let path = CString::new(file.to_str().unwrap()).unwrap();
        // SAFETY: the documented ysfx lifecycle, single-threaded; every object is freed once.
        unsafe {
            let config = ysfx_config_new();
            let fx = ysfx_new(config);
            ysfx_config_free(config);
            assert!(ysfx_load_file(fx, path.as_ptr(), 0));
            assert!(ysfx_compile(fx, YSFX_COMPILE_NO_GFX));
            assert_eq!(ysfx_get_num_inputs(fx), 1);
            assert!(ysfx_slider_exists(fx, 0) && !ysfx_slider_exists(fx, 1));
            ysfx_set_sample_rate(fx, 48_000.0);
            ysfx_set_block_size(fx, 4);
            ysfx_init(fx);
            let input = [0.25f32, -1.0, 0.5, 2.0];
            let mut out = [0.0f32; 4];
            let ins = [input.as_ptr()];
            let outs = [out.as_mut_ptr()];
            ysfx_process_float(fx, ins.as_ptr(), outs.as_ptr(), 1, 1, 4);
            assert_eq!(
                out.map(f32::to_bits),
                [0.125f32, -0.5, 0.25, 1.0].map(f32::to_bits)
            );
            ysfx_free(fx);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
