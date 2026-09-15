//! JSFX file conventions shared by the sandbox's JSFX backend and the editor's scanner and
//! installer (T-808, ADR-008 Amendment 11). Plain Rust, no ysfx: the editor never parses a script
//! beyond these header lines.
//!
//! - **Which files are scripts:** `.jsfx` files, and extensionless files whose header (the text
//!   before the first `@section` line) has a `desc:` line — REAPER's own effects are
//!   extensionless. `.jsfx-inc` files are imports, never effects.
//! - **Identity:** a script's id is its path relative to its **effects root** — the nearest
//!   ancestor folder named `Effects` (REAPER's layout, and PowerVoice's own JSFX folder), else the
//!   script's own folder — with `/` separators (ADR-005 §2: `jsfx:<path relative to the effects
//!   root>`). The effects root is also ysfx's import root.
//! - **Imports:** `import <file>` header lines; the installer copies the ones it finds next to the
//!   script (relative references).

use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// The JSFX file extension.
pub const EXTENSION: &str = "jsfx";
/// Import files' extension (never effects themselves).
pub const IMPORT_EXTENSION: &str = "jsfx-inc";
/// The folder name that marks an effects root (REAPER: `<resource path>/Effects`).
pub const EFFECTS_DIR: &str = "Effects";
/// How much of an extensionless file is read to find its `desc:` line.
pub const HEADER_BYTES: usize = 16 * 1024;

/// The header lines of a script: everything before the first `@section` line.
fn header_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(str::trim_start)
        .take_while(|l| !l.starts_with('@'))
}

/// Whether `text` (a script's beginning) declares an effect: a `desc:` line in its header.
pub fn has_desc(text: &str) -> bool {
    header_lines(text).any(|l| l.starts_with("desc:"))
}

/// Whether `path`'s extension is `.jsfx` (any case).
pub fn has_jsfx_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION))
}

/// Whether `path` is a JSFX script: a `.jsfx` file, or an extensionless regular file whose
/// header has a `desc:` line (reads at most [`HEADER_BYTES`]; binary files never match).
pub fn is_script(path: &Path) -> bool {
    if has_jsfx_extension(path) {
        return path.is_file();
    }
    if path.extension().is_some() || !path.is_file() {
        return false;
    }
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = Vec::with_capacity(HEADER_BYTES);
    if file
        .take(HEADER_BYTES as u64)
        .read_to_end(&mut head)
        .is_err()
        || head.contains(&0)
    {
        return false;
    }
    has_desc(&String::from_utf8_lossy(&head))
}

/// The effects root of the script at `path`: its nearest ancestor folder named `Effects` (any
/// case), else its own folder.
pub fn effects_root(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new(""));
    dir.ancestors()
        .find(|a| {
            a.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case(EFFECTS_DIR))
        })
        .unwrap_or(dir)
        .to_path_buf()
}

/// The script's id: its path relative to [`effects_root`], `/`-separated.
pub fn plugin_id(path: &Path) -> String {
    let root = effects_root(path);
    let rel = path.strip_prefix(&root).unwrap_or(path);
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

/// The `import` lines of a script's header, in order (the file names as written).
pub fn imports(text: &str) -> Vec<String> {
    header_lines(text)
        .filter_map(|l| {
            let rest = l.strip_prefix("import")?;
            rest.starts_with(char::is_whitespace)
                .then(|| rest.trim().to_owned())
        })
        .filter(|name| !name.is_empty())
        .collect()
}

/// The script's version: a ReaPack-style `// @version <v>` comment in its header (JSFX itself has
/// no version field).
pub fn version(text: &str) -> Option<String> {
    header_lines(text).find_map(|l| {
        let rest = l
            .strip_prefix("//")?
            .trim_start()
            .strip_prefix("@version")?;
        rest.starts_with(char::is_whitespace)
            .then(|| rest.split_whitespace().next().map(str::to_owned))
            .flatten()
    })
}

/// The files a script at `path` imports **relatively** (found under its own folder), recursively
/// through the imports' own imports, each once: `(path relative to the script's folder, absolute
/// path)`. Imports that resolve elsewhere (the effects root's other folders) aren't included.
pub fn relative_imports(path: &Path) -> Vec<(PathBuf, PathBuf)> {
    let Some(base) = path.parent() else {
        return Vec::new();
    };
    let mut out: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut todo = vec![path.to_path_buf()];
    while let Some(file) = todo.pop() {
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let dir = file.parent().unwrap_or(base).to_path_buf();
        for name in imports(&String::from_utf8_lossy(&bytes)) {
            let candidate = dir.join(&name);
            let Ok(rel) = candidate.strip_prefix(base) else {
                continue;
            };
            let escapes = rel
                .components()
                .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
            if escapes || !candidate.is_file() || out.iter().any(|(_, a)| *a == candidate) {
                continue;
            }
            out.push((rel.to_path_buf(), candidate.clone()));
            todo.push(candidate);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("pv-ipc-jsfx-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }

    #[test]
    fn a_desc_line_before_the_first_section_marks_a_script() {
        assert!(has_desc(
            "// comment\ndesc:Gain\nslider1:0<0,1>x\n@sample\n"
        ));
        assert!(has_desc("  desc: indented"));
        assert!(!has_desc("@init\ndesc:too late\n"));
        assert!(!has_desc("description: no\n"));
        assert_eq!(
            imports(
                "desc:x\nimport lib/a.jsfx-inc\nimport  b.jsfx-inc \nimported:no\n@init\nimport c\n"
            ),
            ["lib/a.jsfx-inc", "b.jsfx-inc"]
        );
        assert_eq!(
            version("desc:x\n// @version 1.2.3 beta\n"),
            Some("1.2.3".into())
        );
        assert_eq!(version("desc:x\n//@version  2\n"), Some("2".into()));
        assert_eq!(
            version("desc:x\n// @versions 2\n@init\n// @version 3\n"),
            None
        );
    }

    #[test]
    fn scripts_ids_and_imports_on_disk() {
        let t = tmp("disk");
        let fx = t.0.join("REAPER").join("Effects").join("utility");
        std::fs::create_dir_all(fx.join("lib")).unwrap();
        let bare = fx.join("volume");
        std::fs::write(&bare, "desc:Volume\nimport lib/a.jsfx-inc\n@sample\n").unwrap();
        std::fs::write(
            fx.join("lib").join("a.jsfx-inc"),
            "import b.jsfx-inc\n@init\n",
        )
        .unwrap();
        std::fs::write(fx.join("lib").join("b.jsfx-inc"), "@init\n").unwrap();
        let named = fx.join("Gain.JSFX");
        std::fs::write(&named, "desc:Gain\n").unwrap();
        let data = fx.join("notes");
        std::fs::write(&data, "just text\n").unwrap();
        let binary = fx.join("blob");
        std::fs::write(&binary, b"desc:\0\x01").unwrap();
        let inc = fx.join("x.jsfx-inc");
        std::fs::write(&inc, "desc:not an effect\n").unwrap();
        assert!(is_script(&bare) && is_script(&named));
        assert!(!is_script(&data) && !is_script(&binary) && !is_script(&inc));
        assert!(!is_script(&fx), "a folder");
        assert_eq!(effects_root(&bare), t.0.join("REAPER").join("Effects"));
        assert_eq!(plugin_id(&bare), "utility/volume");
        assert_eq!(plugin_id(&named), "utility/Gain.JSFX");
        let loose = t.0.join("loose.jsfx");
        std::fs::write(&loose, "desc:Loose\n").unwrap();
        assert_eq!(
            (effects_root(&loose), plugin_id(&loose)),
            (t.0.clone(), "loose.jsfx".into())
        );
        let rel: Vec<PathBuf> = relative_imports(&bare)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        assert_eq!(
            rel,
            [Path::new("lib/a.jsfx-inc"), Path::new("lib/b.jsfx-inc")]
        );
    }
}
