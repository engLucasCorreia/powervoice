//! **`.voxmod` module packages** (ADR-006 §3 and §7, T-805): a zip archive holding a
//! `manifest.json`, per-platform `.clap` binaries, their SHA-256 checksums, and optional
//! `licenses/`, `presets/` and `locales/`:
//!
//! ```text
//! manifest.json
//! bin/linux-x86_64/<id>.clap
//! bin/windows-x86_64/<id>.clap
//! bin/macos-universal/<id>.clap/…     (bundle directory)
//! presets/*.vopreset.json
//! locales/<lang>.json
//! licenses/…
//! ```
//!
//! [`open`] validates a package **statically — no code from it ever runs**:
//! - size (≤ [`MAX_PACKAGE_BYTES`], compressed and uncompressed) and zip integrity;
//! - every entry name is relative and clean: no absolute path, drive letter, `\`, `.`/`..`
//!   component, control character, duplicate (case-insensitively), symlink or encrypted entry;
//! - the manifest's schema, `manifest_version`, id format, strict version, `module_api` ≤ the
//!   host's and `min_host_version` ≤ the host's; the id must not be a built-in's;
//! - a binary exists for this platform ([`host_platform`]), every binary file has a checksum,
//!   and every checksummed file exists and matches.
//!
//! [`Package::extract`] then writes only this platform's binary (flattened out of
//! `bin/<platform>/`), the manifest and the data folders, re-checking every checksum on the way
//! (the file could have changed since it was validated). [`pack`] builds packages
//! (`just voxmod`, tests).

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use vox_module_api::{MODULE_API_VERSION, Version, is_valid_module_id};

use crate::sha256::{Sha256, hex, is_sha256_hex, sha256_hex};

/// The package file extension.
pub const EXTENSION: &str = "voxmod";
/// The manifest's name in the archive.
pub const MANIFEST_NAME: &str = "manifest.json";
/// The `manifest_version` this build reads and writes.
pub const MANIFEST_VERSION: u32 = 1;
/// ADR-006 §7: packages over 256 MB are refused (compressed, and uncompressed in total).
pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 4096;
const MAX_NAME_BYTES: usize = 1024;
/// Folders extracted next to the binary (besides the manifest).
const DATA_DIRS: [&str; 3] = ["licenses/", "presets/", "locales/"];

/// `manifest.json` (ADR-006 §3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// [`MANIFEST_VERSION`].
    pub manifest_version: u32,
    /// Module id (reverse-DNS; also the CLAP plugin id).
    pub id: String,
    /// Strict `MAJOR.MINOR.PATCH`; equals the plugin's descriptor version.
    pub version: String,
    /// The Module API version the module was built against (≤ the host's).
    pub module_api: u32,
    /// Oldest PowerVoice version that can run it (`MAJOR.MINOR.PATCH`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_host_version: Option<String>,
    /// Display name.
    pub name: String,
    /// Vendor.
    #[serde(default)]
    pub vendor: String,
    /// License (SPDX expression).
    #[serde(default)]
    pub license: String,
    /// Homepage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Platform (`linux-x86_64`, …) → the `.clap` inside the archive.
    #[serde(default)]
    pub binaries: BTreeMap<String, String>,
    /// Archive path → lowercase hex SHA-256.
    #[serde(default)]
    pub sha256: BTreeMap<String, String>,
}

/// This platform's key in [`Manifest::binaries`]: `linux-x86_64`, `windows-x86_64`,
/// `linux-aarch64`, …; `macos-universal` on every Mac (ADR-006 §3).
pub fn host_platform() -> String {
    if cfg!(target_os = "macos") {
        return "macos-universal".into();
    }
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// What the host checks a package against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageHost {
    /// The Module API version this host implements.
    pub module_api: u32,
    /// This PowerVoice's version.
    pub host_version: Version,
    /// [`host_platform`].
    pub platform: String,
    /// Built-in module ids: a package can never claim one (ADR-006 §4, §7).
    pub reserved_ids: Vec<String>,
}

impl Default for PackageHost {
    fn default() -> Self {
        Self {
            module_api: MODULE_API_VERSION,
            host_version: env!("CARGO_PKG_VERSION").parse().unwrap_or_default(),
            platform: host_platform(),
            reserved_ids: Vec::new(),
        }
    }
}

/// Why a package was refused. Nothing from it has run, and nothing was written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VoxmodError {
    /// Over [`MAX_PACKAGE_BYTES`] (the file, or what it expands to).
    #[error("the package is larger than 256 MB")]
    TooLarge,
    /// Not a readable zip archive (damaged, truncated, unsupported compression).
    #[error("the package isn't a valid zip archive: {0}")]
    NotAZip(String),
    /// An entry name that could escape the install folder: absolute, a drive letter, `\`, a `.`
    /// or `..` component, a control character, or too long.
    #[error("unsafe path in the package: {0}")]
    UnsafePath(String),
    /// A symbolic link entry.
    #[error("the package contains a symbolic link: {0}")]
    Symlink(String),
    /// An encrypted entry.
    #[error("the package contains an encrypted file: {0}")]
    Encrypted(String),
    /// Two entries with the same name (ignoring case).
    #[error("the package contains {0} twice")]
    DuplicateEntry(String),
    /// No `manifest.json`.
    #[error("the package has no manifest.json")]
    MissingManifest,
    /// The manifest isn't valid (JSON, schema or a field).
    #[error("invalid manifest: {0}")]
    BadManifest(String),
    /// The id isn't a reverse-DNS module id.
    #[error("`{0}` isn't a valid module id")]
    InvalidId(String),
    /// The id is a built-in module's.
    #[error("`{0}` is the id of a built-in module")]
    BuiltinId(String),
    /// Built against a newer Module API.
    #[error("the module needs Module API {required}; this PowerVoice implements {supported}")]
    ModuleApiTooNew {
        /// The manifest's `module_api`.
        required: u32,
        /// The host's.
        supported: u32,
    },
    /// Needs a newer PowerVoice.
    #[error("the module needs PowerVoice {required} or newer (this is {host})")]
    HostTooOld {
        /// The manifest's `min_host_version`.
        required: String,
        /// This PowerVoice's version.
        host: String,
    },
    /// No binary for this platform.
    #[error("the package has no binary for {platform} (it has: {})", .available.join(", "))]
    WrongPlatform {
        /// This platform.
        platform: String,
        /// The platforms it does have.
        available: Vec<String>,
    },
    /// A file the manifest names isn't in the archive.
    #[error("the package lacks {0}")]
    MissingFile(String),
    /// A file doesn't match its checksum.
    #[error("{0} doesn't match its checksum")]
    ChecksumMismatch(String),
    /// Reading or writing failed.
    #[error("{0}")]
    Io(String),
}

fn io(e: impl std::fmt::Display) -> VoxmodError {
    VoxmodError::Io(e.to_string())
}

fn zip_error(e: zip::result::ZipError) -> VoxmodError {
    match e {
        zip::result::ZipError::Io(e) => VoxmodError::Io(e.to_string()),
        other => VoxmodError::NotAZip(other.to_string()),
    }
}

/// Checks one archive entry name (`/`-separated; a trailing `/` marks a directory).
fn check_name(name: &str) -> Result<(), VoxmodError> {
    let refuse = || Err(VoxmodError::UnsafePath(name.to_owned()));
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name.starts_with('/')
        || name
            .chars()
            .any(|c| c.is_control() || c == '\\' || c == ':')
    {
        return refuse();
    }
    let body = name.strip_suffix('/').unwrap_or(name);
    if body
        .split('/')
        .any(|c| c.is_empty() || c == "." || c == "..")
    {
        return refuse();
    }
    Ok(())
}

fn is_dir_name(name: &str) -> bool {
    name.ends_with('/')
}

/// Reads entry `name` fully (at most `size` bytes, as declared by the central directory),
/// hashing it; `sink` receives the bytes.
fn read_entry(
    zip: &mut zip::ZipArchive<File>,
    name: &str,
    size: u64,
    mut sink: impl FnMut(&[u8]) -> Result<(), VoxmodError>,
) -> Result<String, VoxmodError> {
    let entry = zip.by_name(name).map_err(zip_error)?;
    // One byte more than declared: a lying header shows up as a size mismatch.
    let mut reader = entry.take(size.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            if e.kind() == std::io::ErrorKind::InvalidData {
                VoxmodError::NotAZip(format!("{name}: {e}"))
            } else {
                io(e)
            }
        })?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > size {
            return Err(VoxmodError::NotAZip(format!(
                "{name} is larger than declared"
            )));
        }
        hasher.update(&buf[..n]);
        sink(&buf[..n])?;
    }
    if total != size {
        return Err(VoxmodError::NotAZip(format!(
            "{name} is shorter than declared"
        )));
    }
    Ok(hex(&hasher.finalize()))
}

/// A validated package, ready to [`extract`](Self::extract).
#[derive(Clone, Debug)]
pub struct Package {
    /// The package file.
    pub path: PathBuf,
    /// Its manifest.
    pub manifest: Manifest,
    /// The manifest's version, parsed.
    pub version: Version,
    /// This platform's binary in the archive (a file, or a macOS bundle directory).
    binary: String,
    /// Archive entries to extract, with their declared sizes.
    files: BTreeMap<String, u64>,
}

/// Opens and validates the package at `path` for `host` (see the module docs). Runs no code
/// from the package and writes nothing.
pub fn open(path: &Path, host: &PackageHost) -> Result<Package, VoxmodError> {
    let meta = std::fs::metadata(path).map_err(io)?;
    if meta.len() > MAX_PACKAGE_BYTES {
        return Err(VoxmodError::TooLarge);
    }
    let file = File::open(path).map_err(io)?;
    let mut zip = zip::ZipArchive::new(file).map_err(zip_error)?;
    if zip.len() > MAX_ENTRIES {
        return Err(VoxmodError::NotAZip(format!(
            "more than {MAX_ENTRIES} entries"
        )));
    }

    // Every entry: its name, kind and declared size (no decompression yet).
    let mut files: BTreeMap<String, u64> = BTreeMap::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).map_err(zip_error)?;
        let name = entry.name().to_owned();
        check_name(&name)?;
        if entry.is_symlink() {
            return Err(VoxmodError::Symlink(name));
        }
        if entry.encrypted() {
            return Err(VoxmodError::Encrypted(name));
        }
        let key = name.trim_end_matches('/').to_lowercase();
        if !seen.insert(key) {
            return Err(VoxmodError::DuplicateEntry(name));
        }
        if !is_dir_name(&name) {
            total = total.saturating_add(entry.size());
            if total > MAX_PACKAGE_BYTES {
                return Err(VoxmodError::TooLarge);
            }
            files.insert(name, entry.size());
        }
    }

    // The manifest.
    let manifest_size = *files
        .get(MANIFEST_NAME)
        .ok_or(VoxmodError::MissingManifest)?;
    if manifest_size > MAX_MANIFEST_BYTES {
        return Err(VoxmodError::BadManifest(
            "manifest.json is too large".into(),
        ));
    }
    let mut bytes = Vec::new();
    read_entry(&mut zip, MANIFEST_NAME, manifest_size, |b| {
        bytes.extend_from_slice(b);
        Ok(())
    })?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| VoxmodError::BadManifest(e.to_string()))?;
    let version = validate_manifest(&manifest, host)?;

    // This platform's binary: a `.clap` file, or a bundle directory whose files all have
    // checksums.
    let Some(binary) = manifest.binaries.get(&host.platform).cloned() else {
        return Err(VoxmodError::WrongPlatform {
            platform: host.platform.clone(),
            available: manifest.binaries.keys().cloned().collect(),
        });
    };
    let bundle_prefix = format!("{binary}/");
    let binary_files: Vec<String> = if files.contains_key(&binary) {
        vec![binary.clone()]
    } else {
        files
            .keys()
            .filter(|n| n.starts_with(&bundle_prefix))
            .cloned()
            .collect()
    };
    if binary_files.is_empty() {
        return Err(VoxmodError::MissingFile(binary));
    }
    if let Some(unlisted) = binary_files
        .iter()
        .find(|f| !manifest.sha256.contains_key(*f))
    {
        return Err(VoxmodError::BadManifest(format!(
            "{unlisted} has no checksum"
        )));
    }

    // Every checksum.
    for (name, want) in &manifest.sha256 {
        let size = *files
            .get(name)
            .ok_or_else(|| VoxmodError::MissingFile(name.clone()))?;
        let got = read_entry(&mut zip, name, size, |_| Ok(()))?;
        if &got != want {
            return Err(VoxmodError::ChecksumMismatch(name.clone()));
        }
    }

    let extract: BTreeMap<String, u64> = files
        .into_iter()
        .filter(|(n, _)| {
            n == MANIFEST_NAME
                || binary_files.contains(n)
                || DATA_DIRS.iter().any(|d| n.starts_with(d))
        })
        .collect();
    Ok(Package {
        path: path.to_path_buf(),
        manifest,
        version,
        binary,
        files: extract,
    })
}

/// Checks the manifest's fields; returns its parsed version.
fn validate_manifest(m: &Manifest, host: &PackageHost) -> Result<Version, VoxmodError> {
    let bad = |what: String| Err(VoxmodError::BadManifest(what));
    if m.manifest_version != MANIFEST_VERSION {
        return bad(format!(
            "manifest_version {} isn't supported",
            m.manifest_version
        ));
    }
    if !is_valid_module_id(&m.id) {
        return Err(VoxmodError::InvalidId(m.id.clone()));
    }
    if host.reserved_ids.iter().any(|r| r == &m.id) {
        return Err(VoxmodError::BuiltinId(m.id.clone()));
    }
    let Ok(version) = m.version.parse::<Version>() else {
        return bad(format!("version `{}` isn't MAJOR.MINOR.PATCH", m.version));
    };
    if m.module_api == 0 {
        return bad("module_api must be at least 1".into());
    }
    if m.module_api > host.module_api {
        return Err(VoxmodError::ModuleApiTooNew {
            required: m.module_api,
            supported: host.module_api,
        });
    }
    if let Some(min) = &m.min_host_version {
        let Ok(min_v) = min.parse::<Version>() else {
            return bad(format!("min_host_version `{min}` isn't MAJOR.MINOR.PATCH"));
        };
        if min_v > host.host_version {
            return Err(VoxmodError::HostTooOld {
                required: min.clone(),
                host: host.host_version.to_string(),
            });
        }
    }
    if m.name.trim().is_empty() {
        return bad("name is empty".into());
    }
    if m.binaries.is_empty() {
        return bad("binaries is empty".into());
    }
    for (platform, path) in &m.binaries {
        check_name(path)?;
        if platform.is_empty() || !path.starts_with("bin/") || !path.ends_with(".clap") {
            return bad(format!(
                "binary `{path}` for `{platform}` must be a bin/…/*.clap path"
            ));
        }
    }
    for (path, sum) in &m.sha256 {
        check_name(path)?;
        if !is_sha256_hex(sum) {
            return bad(format!(
                "the checksum of {path} isn't lowercase hex SHA-256"
            ));
        }
    }
    Ok(version)
}

impl Package {
    /// The name the `.clap` extracts under (`<id>.clap` for packages built by [`pack`]).
    pub fn binary_name(&self) -> &str {
        self.binary.rsplit('/').next().unwrap_or(&self.binary)
    }

    /// Where an archive entry goes below the install directory: this platform's binary is
    /// flattened out of `bin/<platform>/`; everything else keeps its path.
    fn relative_target(&self, name: &str) -> String {
        let parent = self
            .binary
            .rsplit_once('/')
            .map_or(String::new(), |(p, _)| format!("{p}/"));
        match name.strip_prefix(&parent) {
            Some(rest) if name == self.binary || name.starts_with(&format!("{}/", self.binary)) => {
                rest.to_owned()
            }
            _ => name.to_owned(),
        }
    }

    /// Extracts the binary for this platform, the manifest and the data folders into `dest`
    /// (created if needed; must not already hold them), syncing every file. Every checksum is
    /// checked again while writing. Returns the `.clap` path inside `dest`.
    pub fn extract(&self, dest: &Path) -> Result<PathBuf, VoxmodError> {
        std::fs::create_dir_all(dest).map_err(io)?;
        let file = File::open(&self.path).map_err(io)?;
        let mut zip = zip::ZipArchive::new(file).map_err(zip_error)?;
        let mut dirs: BTreeSet<PathBuf> = BTreeSet::new();
        for (name, &size) in &self.files {
            let rel = self.relative_target(name);
            check_name(&rel)?;
            let target = rel.split('/').fold(dest.to_path_buf(), |p, c| p.join(c));
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
                dirs.insert(parent.to_path_buf());
            }
            let mut out = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(io)?;
            let got = read_entry(&mut zip, name, size, |b| out.write_all(b).map_err(io))?;
            if let Some(want) = self.manifest.sha256.get(name)
                && &got != want
            {
                return Err(VoxmodError::ChecksumMismatch(name.clone()));
            }
            out.sync_all().map_err(io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if name == &self.binary || name.starts_with(&self.binary) {
                    0o755
                } else {
                    0o644
                };
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode))
                    .map_err(io)?;
            }
        }
        #[cfg(unix)]
        for d in dirs.iter().chain(std::iter::once(&dest.to_path_buf())) {
            File::open(d).and_then(|f| f.sync_all()).map_err(io)?;
        }
        Ok(dest.join(self.binary_name()))
    }
}

/// Every file below `dir` (relative, `/`-separated), sorted.
fn walk_files(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), VoxmodError> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(io)?
        .collect::<Result<_, _>>()
        .map_err(io)?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for e in entries {
        let name = format!("{prefix}{}", e.file_name().to_string_lossy());
        if e.file_type().map_err(io)?.is_dir() {
            walk_files(&e.path(), &format!("{name}/"), out)?;
        } else {
            out.push((name, e.path()));
        }
    }
    Ok(())
}

/// Builds a package at `out` from `template` (its `binaries` and `sha256` are filled in):
/// `binaries` are `(platform, .clap file or macOS bundle directory)`, stored as
/// `bin/<platform>/<id>.clap`; `extra` are `(archive path, file)` — licenses, presets, locales.
/// Writes a temporary file next to `out`, then renames it. Returns the final manifest.
pub fn pack(
    template: &Manifest,
    binaries: &[(String, PathBuf)],
    extra: &[(String, PathBuf)],
    out: &Path,
) -> Result<Manifest, VoxmodError> {
    let mut manifest = template.clone();
    manifest.manifest_version = MANIFEST_VERSION;
    manifest.binaries.clear();
    manifest.sha256.clear();
    let mut entries: Vec<(String, PathBuf, bool)> = Vec::new();
    for (platform, path) in binaries {
        let name = format!("bin/{platform}/{}.clap", manifest.id);
        check_name(&name)?;
        if path.is_dir() {
            let mut files = Vec::new();
            walk_files(path, &format!("{name}/"), &mut files)?;
            entries.extend(files.into_iter().map(|(n, p)| (n, p, true)));
        } else {
            entries.push((name.clone(), path.clone(), true));
        }
        manifest.binaries.insert(platform.clone(), name);
    }
    for (name, path) in extra {
        check_name(name)?;
        entries.push((name.clone(), path.clone(), false));
    }
    let mut contents = Vec::with_capacity(entries.len());
    for (name, path, executable) in entries {
        let bytes = std::fs::read(&path).map_err(io)?;
        manifest.sha256.insert(name.clone(), sha256_hex(&bytes));
        contents.push((name, bytes, executable));
    }
    let json = serde_json::to_vec_pretty(&manifest).map_err(io)?;

    let tmp = out.with_extension("voxmod.partial");
    let result = (|| -> Result<(), VoxmodError> {
        let file = File::create(&tmp).map_err(io)?;
        let mut zip = zip::ZipWriter::new(file);
        let options = |mode: u32| {
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(mode)
        };
        zip.start_file(MANIFEST_NAME, options(0o644))
            .map_err(zip_error)?;
        zip.write_all(&json).map_err(io)?;
        for (name, bytes, executable) in &contents {
            zip.start_file(
                name.as_str(),
                options(if *executable { 0o755 } else { 0o644 }),
            )
            .map_err(zip_error)?;
            zip.write_all(bytes).map_err(io)?;
        }
        zip.finish().map_err(zip_error)?.sync_all().map_err(io)?;
        std::fs::rename(&tmp, out).map_err(io)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map(|()| manifest)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::install::tests::TempDir;

    /// A change applied to a test manifest.
    type ManifestEdit = Box<dyn Fn(&mut Manifest)>;
    /// Whether a refusal is the expected one.
    pub(crate) type ExpectErr = fn(&VoxmodError) -> bool;

    pub(crate) fn manifest(id: &str) -> Manifest {
        Manifest {
            manifest_version: MANIFEST_VERSION,
            id: id.into(),
            version: "1.2.0".into(),
            module_api: 1,
            min_host_version: Some("0.1.0".into()),
            name: "De-esser".into(),
            vendor: "Acme".into(),
            license: "MIT".into(),
            url: None,
            binaries: BTreeMap::new(),
            sha256: BTreeMap::new(),
        }
    }

    /// A valid package for this platform with `binary` as the `.clap` and a license file.
    pub(crate) fn valid_package(dir: &Path, id: &str, binary: &[u8]) -> PathBuf {
        package_with(dir, &manifest(id), binary)
    }

    /// [`valid_package`] from manifest `m` (id, version, …).
    pub(crate) fn package_with(dir: &Path, m: &Manifest, binary: &[u8]) -> PathBuf {
        let bin = dir.join("plugin.clap");
        std::fs::write(&bin, binary).unwrap();
        let license = dir.join("LICENSE");
        std::fs::write(&license, b"MIT license text").unwrap();
        let out = dir.join(format!("{}-{}.voxmod", m.id, m.version));
        pack(
            m,
            &[(host_platform(), bin)],
            &[("licenses/LICENSE".into(), license)],
            &out,
        )
        .unwrap();
        out
    }

    /// A raw archive of `(name, bytes)` entries, stored (tests craft hostile ones).
    pub(crate) fn raw_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }

    /// A manifest for `raw_zip` packages: `binary` for this platform, checksums of `files`.
    fn manifest_json(id: &str, binary: &str, files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut m = manifest(id);
        m.binaries.insert(host_platform(), binary.into());
        for (name, bytes) in files {
            m.sha256.insert((*name).into(), sha256_hex(bytes));
        }
        serde_json::to_vec(&m).unwrap()
    }

    fn host() -> PackageHost {
        PackageHost {
            reserved_ids: vec!["org.powervoice.gain".into()],
            ..PackageHost::default()
        }
    }

    #[test]
    fn a_packed_package_opens_and_extracts_the_binary_manifest_and_licenses() {
        let dir = TempDir::new("voxmod-pack");
        let path = valid_package(&dir.0, "com.acme.deesser", b"\x7fELF binary");
        let pkg = open(&path, &host()).unwrap();
        assert_eq!(pkg.manifest.id, "com.acme.deesser");
        assert_eq!(pkg.version, Version::new(1, 2, 0));
        assert_eq!(pkg.binary_name(), "com.acme.deesser.clap");
        let bin = format!("bin/{}/com.acme.deesser.clap", host_platform());
        assert_eq!(pkg.manifest.binaries[&host_platform()], bin);
        assert_eq!(pkg.manifest.sha256[&bin], sha256_hex(b"\x7fELF binary"));

        let dest = dir.0.join("installed");
        let clap = pkg.extract(&dest).unwrap();
        assert_eq!(clap, dest.join("com.acme.deesser.clap"));
        assert_eq!(std::fs::read(&clap).unwrap(), b"\x7fELF binary");
        assert_eq!(
            std::fs::read(dest.join("licenses").join("LICENSE")).unwrap(),
            b"MIT license text"
        );
        let m: Manifest =
            serde_json::from_slice(&std::fs::read(dest.join(MANIFEST_NAME)).unwrap()).unwrap();
        assert_eq!(m, pkg.manifest);
        assert!(!dest.join("bin").exists(), "the binary is flattened");
    }

    #[test]
    fn other_platforms_binaries_are_not_extracted() {
        let dir = TempDir::new("voxmod-platforms");
        let (here, there) = (dir.0.join("here.clap"), dir.0.join("there.clap"));
        std::fs::write(&here, b"native").unwrap();
        std::fs::write(&there, b"foreign").unwrap();
        let path = dir.0.join("multi.voxmod");
        pack(
            &manifest("com.acme.multi"),
            &[(host_platform(), here), ("plan9-mips".into(), there)],
            &[],
            &path,
        )
        .unwrap();
        let dest = dir.0.join("out");
        let clap = open(&path, &host()).unwrap().extract(&dest).unwrap();
        assert_eq!(std::fs::read(clap).unwrap(), b"native");
        let names: Vec<_> = std::fs::read_dir(&dest)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}: the binary and the manifest");
    }

    #[test]
    fn bad_manifests_are_refused() {
        let dir = TempDir::new("voxmod-manifest");
        let bin: &[u8] = b"binary";
        let bin_name = format!("bin/{}/x.clap", host_platform());
        let cases: Vec<(&str, ManifestEdit, ExpectErr)> = vec![
            ("version", Box::new(|m| m.manifest_version = 2), |e| {
                matches!(e, VoxmodError::BadManifest(_))
            }),
            ("id", Box::new(|m| m.id = "Not An Id".into()), |e| {
                matches!(e, VoxmodError::InvalidId(_))
            }),
            (
                "builtin",
                Box::new(|m| m.id = "org.powervoice.gain".into()),
                |e| matches!(e, VoxmodError::BuiltinId(_)),
            ),
            ("semver", Box::new(|m| m.version = "1.2".into()), |e| {
                matches!(e, VoxmodError::BadManifest(_))
            }),
            (
                "api",
                Box::new(|m| m.module_api = MODULE_API_VERSION + 1),
                |e| matches!(e, VoxmodError::ModuleApiTooNew { .. }),
            ),
            (
                "host",
                Box::new(|m| m.min_host_version = Some("99.0.0".into())),
                |e| matches!(e, VoxmodError::HostTooOld { .. }),
            ),
            ("name", Box::new(|m| m.name = " ".into()), |e| {
                matches!(e, VoxmodError::BadManifest(_))
            }),
            (
                "hex",
                Box::new(|m| {
                    let k = m.sha256.keys().next().unwrap().clone();
                    m.sha256.insert(k, "ABC".into());
                }),
                |e| matches!(e, VoxmodError::BadManifest(_)),
            ),
            (
                "not-clap",
                Box::new(|m| {
                    m.binaries.insert(host_platform(), "bin/x.so".into());
                }),
                |e| matches!(e, VoxmodError::BadManifest(_)),
            ),
        ];
        for (label, edit, expected) in cases {
            let mut m = manifest("com.acme.x");
            m.binaries.insert(host_platform(), bin_name.clone());
            m.sha256.insert(bin_name.clone(), sha256_hex(bin));
            edit(&mut m);
            let path = dir.0.join(format!("{label}.voxmod"));
            let json = serde_json::to_vec(&m).unwrap();
            raw_zip(&path, &[(MANIFEST_NAME, &json), (&bin_name, bin)]);
            let err = open(&path, &host()).unwrap_err();
            assert!(expected(&err), "{label}: {err:?}");
        }
        // Not JSON, and not the schema.
        for (label, json) in [("garbage", &b"{not json"[..]), ("schema", b"{\"id\":1}")] {
            let path = dir.0.join(format!("{label}.voxmod"));
            raw_zip(&path, &[(MANIFEST_NAME, json)]);
            assert!(matches!(
                open(&path, &host()),
                Err(VoxmodError::BadManifest(_))
            ));
        }
        // No manifest at all.
        let path = dir.0.join("none.voxmod");
        raw_zip(&path, &[(bin_name.as_str(), bin)]);
        assert_eq!(
            open(&path, &host()).unwrap_err(),
            VoxmodError::MissingManifest
        );
    }

    #[test]
    fn checksum_mismatches_and_missing_files_are_refused() {
        let dir = TempDir::new("voxmod-checksum");
        let bin_name = format!("bin/{}/x.clap", host_platform());
        // The manifest's checksum is of other bytes.
        let path = dir.0.join("tampered.voxmod");
        let json = manifest_json("com.acme.x", &bin_name, &[(&bin_name, b"original")]);
        raw_zip(&path, &[(MANIFEST_NAME, &json), (&bin_name, b"tampered")]);
        assert_eq!(
            open(&path, &host()).unwrap_err(),
            VoxmodError::ChecksumMismatch(bin_name.clone())
        );
        // A checksummed file that isn't there.
        let path = dir.0.join("missing.voxmod");
        let json = manifest_json(
            "com.acme.x",
            &bin_name,
            &[(&bin_name, b"bin"), ("licenses/LICENSE", b"text")],
        );
        raw_zip(&path, &[(MANIFEST_NAME, &json), (&bin_name, b"bin")]);
        assert_eq!(
            open(&path, &host()).unwrap_err(),
            VoxmodError::MissingFile("licenses/LICENSE".into())
        );
        // A binary without a checksum.
        let path = dir.0.join("unlisted.voxmod");
        let json = manifest_json("com.acme.x", &bin_name, &[]);
        raw_zip(&path, &[(MANIFEST_NAME, &json), (&bin_name, b"bin")]);
        assert!(matches!(
            open(&path, &host()),
            Err(VoxmodError::BadManifest(_))
        ));
    }

    #[test]
    fn a_package_without_a_binary_for_this_platform_is_refused() {
        let dir = TempDir::new("voxmod-platform");
        let path = dir.0.join("foreign.voxmod");
        let mut m = manifest("com.acme.x");
        m.binaries
            .insert("plan9-mips".into(), "bin/plan9-mips/x.clap".into());
        m.sha256
            .insert("bin/plan9-mips/x.clap".into(), sha256_hex(b"bin"));
        let json = serde_json::to_vec(&m).unwrap();
        raw_zip(
            &path,
            &[(MANIFEST_NAME, &json), ("bin/plan9-mips/x.clap", b"bin")],
        );
        assert_eq!(
            open(&path, &host()).unwrap_err(),
            VoxmodError::WrongPlatform {
                platform: host_platform(),
                available: vec!["plan9-mips".into()]
            }
        );
    }

    #[test]
    fn unsafe_paths_symlinks_and_duplicates_are_refused() {
        let dir = TempDir::new("voxmod-paths");
        for name in [
            "../evil.clap",
            "bin/../../evil",
            "/etc/passwd",
            "C:/Windows/evil.dll",
            "bin\\..\\evil",
            "bin//x",
            "./x",
            "bin/\u{1}x",
        ] {
            let path = dir.0.join("hostile.voxmod");
            raw_zip(&path, &[(name, b"x")]);
            assert_eq!(
                open(&path, &host()).unwrap_err(),
                VoxmodError::UnsafePath(name.into()),
                "{name}"
            );
        }
        // A symlink entry.
        let path = dir.0.join("symlink.voxmod");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        zip.add_symlink(
            "licenses/link",
            "/etc/passwd",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        zip.finish().unwrap();
        assert_eq!(
            open(&path, &host()).unwrap_err(),
            VoxmodError::Symlink("licenses/link".into())
        );
        // The same name twice, differing only in case.
        let path = dir.0.join("dup.voxmod");
        raw_zip(&path, &[("licenses/A", b"1"), ("licenses/a", b"2")]);
        assert!(matches!(
            open(&path, &host()),
            Err(VoxmodError::DuplicateEntry(_))
        ));
    }

    #[test]
    fn damaged_and_oversized_files_are_refused() {
        let dir = TempDir::new("voxmod-damaged");
        let path = dir.0.join("garbage.voxmod");
        std::fs::write(&path, b"PK\x03\x04 this is not a zip").unwrap();
        assert!(matches!(open(&path, &host()), Err(VoxmodError::NotAZip(_))));
        // A valid package cut short.
        let good = valid_package(&dir.0, "com.acme.x", &[7u8; 10_000]);
        let bytes = std::fs::read(&good).unwrap();
        let cut = dir.0.join("cut.voxmod");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        assert!(matches!(open(&cut, &host()), Err(VoxmodError::NotAZip(_))));
        // Over 256 MB on disk (a sparse file: nothing is read).
        let big = dir.0.join("big.voxmod");
        File::create(&big)
            .unwrap()
            .set_len(MAX_PACKAGE_BYTES + 1)
            .unwrap();
        assert_eq!(open(&big, &host()).unwrap_err(), VoxmodError::TooLarge);
    }

    #[test]
    fn host_platforms_follow_the_adr_names() {
        let p = host_platform();
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(p, "linux-x86_64");
        #[cfg(all(windows, target_arch = "x86_64"))]
        assert_eq!(p, "windows-x86_64");
        #[cfg(target_os = "macos")]
        assert_eq!(p, "macos-universal");
        assert!(!p.is_empty());
    }
}
