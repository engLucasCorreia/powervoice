//! Preset name sanitization (T-406): turns a user-typed display name into a safe single path
//! segment, so a saved preset's file name never escapes its presets directory.

use crate::error::PresetError;

/// Longest sanitized name kept (characters, not bytes) — generous for a display name, short
/// enough to never brush against filesystem path-length limits.
pub const MAX_NAME_LEN: usize = 120;

/// Turns `raw` into a name safe to use as one path component: trims whitespace, drops control
/// characters, replaces path separators and other filesystem-hostile characters (Windows
/// reserves `\ / : * ? " < > |` too, so those are replaced even on Unix, for portability) with
/// `_`, caps the length, and — since the result is also used as (or as part of) a bare file name,
/// never a `.`-prefixed one — replaces every leading `.` with `_` so the sanitized name can never
/// be mistaken for a hidden file (in particular, one of `atomic::write_atomic`'s own `.`-prefixed
/// temp files, which [`crate::atomic::list_json_names`] skips). Rejects (rather than mangles
/// further) a name that is empty after cleaning, or that cleans down to exactly `.` or `..` — the
/// two path components that would otherwise let a "preset name" escape the presets directory when
/// used directly as a directory name (see `sanitize_module_id`).
pub fn sanitize_preset_name(raw: &str) -> Result<String, PresetError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(PresetError::InvalidName(raw.to_owned()));
    }
    let cleaned: String = trimmed
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return Err(PresetError::InvalidName(raw.to_owned()));
    }
    // Replace only the *leading* run of dots (e.g. "..x" -> "__x", "..." -> "___"): every
    // character is replaced one-for-one, so the result stays exactly as long as `cleaned` and can
    // never come out empty. `.`/`..` themselves were already rejected above, so this never turns
    // a rejected name into an accepted one; it only stops longer dot-led names ("...", "..evil",
    // a sanitized "../../evil" that lost its slashes) from starting with `.`.
    let leading_dots = cleaned.chars().take_while(|&c| c == '.').count();
    let cleaned = if leading_dots > 0 {
        "_".repeat(leading_dots) + &cleaned[leading_dots..]
    } else {
        cleaned.to_owned()
    };
    let cleaned = if cleaned.chars().count() > MAX_NAME_LEN {
        cleaned.chars().take(MAX_NAME_LEN).collect::<String>()
    } else {
        cleaned
    };
    // The truncation above, or trimming, could in principle leave a name that once again cleans
    // down to nothing/`.`/`..`, or that starts with `.` again; re-check rather than assume.
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." || cleaned.starts_with('.') {
        return Err(PresetError::InvalidName(raw.to_owned()));
    }
    Ok(cleaned)
}

/// Same sanitization for a module id used as a directory name. Built-in ids
/// (`org.powervoice.*`, ADR-005 §2) never need it, but an external plugin id can legitimately
/// contain `/` (a `jsfx:<path>` id) or `:` (`vst3:<cid>`, `clap:<id>`) — those are replaced the
/// same way `sanitize_preset_name` replaces them in a display name, so every module gets its own
/// directory without ever escaping the modules root.
pub fn sanitize_module_id(id: &str) -> Result<String, PresetError> {
    sanitize_preset_name(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_ordinary_names_unchanged() {
        for name in [
            "Podcast voice",
            "Gentle cleanup",
            "Audiobook (ACX)",
            "v2 - final",
        ] {
            assert_eq!(sanitize_preset_name(name).unwrap(), name);
        }
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(sanitize_preset_name("  Podcast  ").unwrap(), "Podcast");
    }

    #[test]
    fn rejects_empty_or_whitespace_only_names() {
        for name in ["", "   ", "\t\n"] {
            assert!(matches!(
                sanitize_preset_name(name),
                Err(PresetError::InvalidName(_))
            ));
        }
    }

    #[test]
    fn rejects_dot_and_dot_dot() {
        for name in [".", "..", "  .  ", "  ..  "] {
            assert!(
                matches!(sanitize_preset_name(name), Err(PresetError::InvalidName(_))),
                "{name:?} should be rejected"
            );
        }
    }

    #[test]
    fn strips_path_separators_so_traversal_cannot_escape_the_presets_dir() {
        for traversal in ["../../etc/passwd", "..\\..\\windows\\system32", "a/../../b"] {
            let cleaned = sanitize_preset_name(traversal).unwrap();
            assert!(!cleaned.contains('/'), "{cleaned:?}");
            assert!(!cleaned.contains('\\'), "{cleaned:?}");
            // A single path component built from `cleaned` can only ever resolve inside its
            // parent directory: `Path::join` on a string with no separators never adds a `..`
            // or `.` component of its own.
            let joined = std::path::Path::new("/tmp/presets").join(&cleaned);
            assert_eq!(joined.parent(), Some(std::path::Path::new("/tmp/presets")));
        }
    }

    /// H-33: a traversal-like name such as `"../x"` used to sanitize down to `".._x"` — a leading
    /// dot that `atomic::list_json_names` then hid as if it were a crash-leftover temp file, so
    /// the preset saved but never appeared in a listing. No sanitized name may start with `.`.
    #[test]
    fn traversal_like_names_never_sanitize_to_a_leading_dot() {
        for traversal in [
            "../x",
            "..\\x",
            "../../etc/passwd",
            "..\\..\\windows\\system32",
            "a/../../b",
            "./hidden",
        ] {
            let cleaned = sanitize_preset_name(traversal).unwrap();
            assert!(
                !cleaned.starts_with('.'),
                "{traversal:?} sanitized to {cleaned:?}, which starts with '.'"
            );
        }
    }

    /// A name made of nothing but dots is either exactly `.`/`..` (still rejected — those are the
    /// two path components that escape a directory when used as a bare directory name, see
    /// `sanitize_module_id`) or, for three or more dots, accepted but must not stay dot-only: it
    /// would otherwise both start with `.` and be indistinguishable from an all-underscore-of-
    /// dots collision waiting to happen.
    #[test]
    fn names_made_only_of_dots() {
        assert!(matches!(
            sanitize_preset_name("."),
            Err(PresetError::InvalidName(_))
        ));
        assert!(matches!(
            sanitize_preset_name(".."),
            Err(PresetError::InvalidName(_))
        ));
        for dots in ["...", "....", "....................."] {
            let cleaned = sanitize_preset_name(dots).unwrap();
            assert!(!cleaned.is_empty());
            assert!(!cleaned.starts_with('.'), "{dots:?} -> {cleaned:?}");
            assert_eq!(
                cleaned.len(),
                dots.len(),
                "an all-dot name should keep its length, just lose the leading dot"
            );
        }
    }

    #[test]
    fn strips_windows_reserved_characters_even_on_unix() {
        let cleaned = sanitize_preset_name("weird:name*with?chars\"<>|").unwrap();
        for bad in [':', '*', '?', '"', '<', '>', '|'] {
            assert!(!cleaned.contains(bad), "{cleaned:?} still has {bad:?}");
        }
    }

    #[test]
    fn drops_control_characters() {
        let cleaned = sanitize_preset_name("a\u{0}b\u{7}c").unwrap();
        assert_eq!(cleaned, "abc");
    }

    #[test]
    fn caps_the_length() {
        let long = "x".repeat(500);
        let cleaned = sanitize_preset_name(&long).unwrap();
        assert_eq!(cleaned.chars().count(), MAX_NAME_LEN);
    }

    #[test]
    fn module_id_with_slashes_sanitizes_to_a_single_path_component() {
        // A hypothetical jsfx module id (ADR-005 §2): "jsfx:<path relative to the effects root>".
        let cleaned = sanitize_module_id("jsfx:vocal/de-esser.jsfx").unwrap();
        assert!(!cleaned.contains('/'));
        assert!(!cleaned.contains(':'));
    }
}
