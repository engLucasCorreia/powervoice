//! Builds the vendored ysfx library (`third_party/ysfx/`, T-808) into one static archive,
//! `libysfx.a`: the EEL2 compiler and WDL's FFT (C, plus the x86-64 JIT's GAS stubs) and ysfx
//! itself (C++17), without its `@gfx` support (`YSFX_NO_GFX`: no LICE, SWELL, stb or GUI
//! libraries). Mirrors upstream's `cmake.wdl.txt` / `cmake.ysfx.txt` for the `ysfx` target.
//!
//! Only on unix x86_64 / aarch64; on any other target this does nothing and the crate is empty
//! (the sandbox answers "JSFX isn't supported on this platform").

use std::env;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"))
        .join("../../third_party/ysfx");
    println!("cargo::rerun-if-changed={}", root.display());
    let family = env::var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if !family.split(',').any(|f| f == "unix") || !matches!(arch.as_str(), "x86_64" | "aarch64") {
        return;
    }
    let wdl = root.join("thirdparty/WDL/source");
    let eel2 = wdl.join("WDL/eel2");
    let sources = root.join("sources");

    let configure = |b: &mut cc::Build| {
        b.include(root.join("include"))
            .include(&sources)
            .include(&wdl)
            .include(root.join("thirdparty/dr_libs"))
            .define("WDL_FFT_REALSIZE", "8")
            .define("WDL_LINEPARSE_ATOF", "ysfx_wdl_atof")
            .define("NSEEL_ATOF", "ysfx_wdl_atof")
            .define("YSFX_NO_GFX", None)
            .define("_FILE_OFFSET_BITS", "64")
            // WDL wants `char` signed (aarch64 Linux defaults to unsigned).
            .flag("-fsigned-char")
            // The JIT and the parser are hot paths of the sandbox even in debug builds.
            .opt_level(2)
            // Vendored, unmodified code: its warnings are upstream's business.
            .warnings(false)
            .cargo_warnings(false)
            .flag_if_supported("-w");
    };

    let mut c = cc::Build::new();
    configure(&mut c);
    for f in [
        "nseel-caltab.c",
        "nseel-cfunc.c",
        "nseel-compiler.c",
        "nseel-eval.c",
        "nseel-lextab.c",
        "nseel-ram.c",
        "nseel-yylex.c",
    ] {
        c.file(eel2.join(f));
    }
    c.file(wdl.join("WDL/fft.c"));
    if arch == "x86_64" {
        // aarch64's JIT stubs are inline assembly inside nseel-cfunc.c.
        c.file(sources.join("eel2-gas/sources/asm-nseel-x64-sse.S"));
    }
    let objects = c.compile_intermediates();

    let mut cpp = cc::Build::new();
    configure(&mut cpp);
    cpp.cpp(true).std("c++17");
    for f in [
        "ysfx.cpp",
        "ysfx_config.cpp",
        "ysfx_midi.cpp",
        "ysfx_reader.cpp",
        "ysfx_parse.cpp",
        "ysfx_parse_menu.cpp",
        "ysfx_preset.cpp",
        "ysfx_audio_wav.cpp",
        "ysfx_audio_flac.cpp",
        "ysfx_utils.cpp",
        "ysfx_utils_fts.cpp",
        "ysfx_api_eel.cpp",
        "ysfx_gmem.cpp",
        "ysfx_api_reaper.cpp",
        "ysfx_api_file.cpp",
        "ysfx_api_gfx.cpp",
        "ysfx_eel_utils.cpp",
        "ysfx_preprocess.cpp",
    ] {
        cpp.file(sources.join(f));
    }
    // One archive for both halves: they reference each other (`ysfx_wdl_atof`, the EEL2 API).
    cpp.objects(objects).compile("ysfx");
}
