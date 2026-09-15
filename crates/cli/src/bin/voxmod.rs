//! `voxmod` (T-805, ADR-006 §3): packs a built module `.clap` into a `.voxmod` package — the
//! manifest template (`voxmod.json` of a module package crate) with `binaries` and `sha256`
//! filled in, the per-platform binaries, and license files. `just voxmod <crate>` runs it after a
//! release build. The package is then opened the way "Install module…" will open it (static
//! validation: layout, manifest, checksums) when it holds a binary for this platform.
//!
//! ```text
//! voxmod --manifest crates/voxmod-gain/voxmod.json \
//!        --binary target/release/libvox_voxmod_gain.so [--binary windows-x86_64=path.dll] \
//!        --license LICENSE-MIT --license LICENSE-APACHE --out target/voxmod
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use vox_plugin_host::voxmod::{self, Manifest, PackageHost, host_platform};

/// Packs a PowerVoice module package (`.voxmod`).
#[derive(Parser)]
#[command(name = "voxmod")]
struct Args {
    /// The manifest template (`voxmod.json`).
    #[arg(long)]
    manifest: PathBuf,
    /// A module binary: `<platform>=<file>`, or just `<file>` for this platform.
    #[arg(long = "binary", required = true)]
    binaries: Vec<String>,
    /// A license file, stored as `licenses/<file name>`.
    #[arg(long = "license")]
    licenses: Vec<PathBuf>,
    /// Output directory (the package is `<id>-<version>.voxmod` in it).
    #[arg(long, default_value = "target/voxmod")]
    out: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let text = std::fs::read_to_string(&args.manifest)
        .with_context(|| format!("reading {}", args.manifest.display()))?;
    let template: Manifest = serde_json::from_str(&text)
        .with_context(|| format!("parsing {}", args.manifest.display()))?;
    let mut binaries = Vec::new();
    for b in &args.binaries {
        let (platform, path) = match b.split_once('=') {
            Some((p, f)) => (p.to_owned(), PathBuf::from(f)),
            None => (host_platform(), PathBuf::from(b)),
        };
        if !path.exists() {
            bail!("{} doesn't exist — build the module first", path.display());
        }
        binaries.push((platform, path));
    }
    let mut extra = Vec::new();
    for l in &args.licenses {
        let name = l
            .file_name()
            .with_context(|| format!("{} has no file name", l.display()))?;
        extra.push((format!("licenses/{}", name.to_string_lossy()), l.clone()));
    }
    std::fs::create_dir_all(&args.out)?;
    let out = args
        .out
        .join(format!("{}-{}.voxmod", template.id, template.version));
    let manifest = voxmod::pack(&template, &binaries, &extra, &out)
        .with_context(|| format!("packing {}", out.display()))?;
    if manifest.binaries.contains_key(&host_platform()) {
        voxmod::open(&out, &PackageHost::default())
            .with_context(|| format!("{} doesn't validate", out.display()))?;
    }
    println!("{}", out.display());
    println!("  {} {} ({})", manifest.id, manifest.version, manifest.name);
    for (platform, path) in &manifest.binaries {
        println!("  {platform}: {path}");
    }
    Ok(())
}
