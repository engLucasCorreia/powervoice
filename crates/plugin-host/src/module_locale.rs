//! A `.voxmod` package's `locales/<lang>.json` (ADR-006 §3/§7 step 5, Amendment 3): validated,
//! untrusted data merged into the UI's i18n table under `modules.<id>.*` **only** — a package must
//! never override an app key, so every key not starting with that exact prefix (and every
//! non-string value) makes the whole file rejected. A bad file is never fatal to anything: the
//! caller shows a notice and simply doesn't merge it (`install`/`list` still succeed).

use std::collections::BTreeMap;
use std::path::Path;

/// A locale file over this size is refused outright (untrusted data: same order of magnitude as
/// [`crate::voxmod::MAX_PACKAGE_BYTES`]'s manifest cap, generous for a translation table).
pub const MAX_LOCALE_BYTES: u64 = 1024 * 1024;
/// At most this many keys (a package's own strings, not the whole app's).
pub const MAX_LOCALE_ENTRIES: usize = 4096;
/// Longest key (bytes) accepted.
pub const MAX_KEY_BYTES: usize = 200;
/// Longest value (bytes) accepted — a UI string, not a document.
pub const MAX_VALUE_BYTES: usize = 4096;

/// Why a `locales/<lang>.json` was rejected. Nothing is merged; the file that failed is skipped —
/// callers turn this into a notice, they never fail an install or a listing over it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocaleError {
    /// Over [`MAX_LOCALE_BYTES`].
    #[error("the locale file is larger than {MAX_LOCALE_BYTES} bytes")]
    TooLarge,
    /// Not valid JSON.
    #[error("the locale file isn't valid JSON: {0}")]
    Json(String),
    /// The top level isn't a JSON object of string → string.
    #[error("the locale file must be a flat object of strings")]
    NotAnObject,
    /// More than [`MAX_LOCALE_ENTRIES`] keys.
    #[error("the locale file has more than {MAX_LOCALE_ENTRIES} keys")]
    TooManyEntries,
    /// A key doesn't start with `modules.<id>.` (ADR-006 §3) — the security boundary: a package
    /// may only ever add strings under its own namespace, never touch (let alone override) an app
    /// key.
    #[error("key `{0}` is outside the module's own `modules.<id>.` namespace")]
    KeyOutsideNamespace(String),
    /// A key longer than [`MAX_KEY_BYTES`].
    #[error("key `{0}` is too long")]
    KeyTooLong(String),
    /// A value that isn't a JSON string (untrusted data: strings only).
    #[error("value of `{0}` isn't a string")]
    ValueNotAString(String),
    /// A value longer than [`MAX_VALUE_BYTES`].
    #[error("value of `{0}` is too long")]
    ValueTooLong(String),
}

/// The only prefix a key in `id`'s locale file may start with (ADR-006 §3).
pub fn required_prefix(id: &str) -> String {
    format!("modules.{id}.")
}

/// Validates `bytes` as `id`'s locale file: a flat JSON object, every key namespaced under
/// [`required_prefix`] and every value a string within [`MAX_VALUE_BYTES`]. Any violation rejects
/// the **whole file** (never a partial merge of "the keys that were fine") — see the module docs.
pub fn validate_module_locale(
    id: &str,
    bytes: &[u8],
) -> Result<BTreeMap<String, String>, LocaleError> {
    if bytes.len() as u64 > MAX_LOCALE_BYTES {
        return Err(LocaleError::TooLarge);
    }
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| LocaleError::Json(e.to_string()))?;
    let serde_json::Value::Object(map) = value else {
        return Err(LocaleError::NotAnObject);
    };
    if map.len() > MAX_LOCALE_ENTRIES {
        return Err(LocaleError::TooManyEntries);
    }
    let prefix = required_prefix(id);
    let mut out = BTreeMap::new();
    for (key, value) in map {
        if key.len() > MAX_KEY_BYTES {
            return Err(LocaleError::KeyTooLong(key));
        }
        if !key.starts_with(&prefix) {
            return Err(LocaleError::KeyOutsideNamespace(key));
        }
        let serde_json::Value::String(text) = value else {
            return Err(LocaleError::ValueNotAString(key));
        };
        if text.len() > MAX_VALUE_BYTES {
            return Err(LocaleError::ValueTooLong(key));
        }
        out.insert(key, text);
    }
    Ok(out)
}

/// Reads and validates `<version_dir>/locales/<lang>.json`. `Ok(None)`: the module has no locale
/// file for `lang` (not an error — most languages won't be covered). `Err`: the file exists but
/// failed validation.
pub fn read_module_locale(
    id: &str,
    version_dir: &Path,
    lang: &str,
) -> Result<Option<BTreeMap<String, String>>, LocaleError> {
    let path = version_dir.join("locales").join(format!("{lang}.json"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return Ok(None),
    };
    validate_module_locale(id, &bytes).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "com.acme.deesser";

    #[test]
    fn a_well_formed_locale_file_validates_and_keeps_only_its_own_keys() {
        let json = serde_json::json!({
            "modules.com.acme.deesser.name": "De-esser",
            "modules.com.acme.deesser.presets.warm.name": "Warm",
        })
        .to_string();
        let map = validate_module_locale(ID, json.as_bytes()).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map["modules.com.acme.deesser.name"], "De-esser".to_owned());
    }

    #[test]
    fn a_key_trying_to_override_an_app_key_is_rejected() {
        let json = serde_json::json!({
            "modules.com.acme.deesser.name": "De-esser",
            "app.title": "PowerVoice but evil",
        })
        .to_string();
        assert_eq!(
            validate_module_locale(ID, json.as_bytes()),
            Err(LocaleError::KeyOutsideNamespace("app.title".into()))
        );
    }

    #[test]
    fn a_key_for_a_different_modules_namespace_is_rejected() {
        // Not even another *module's* namespace is allowed — only this package's own id.
        let json =
            serde_json::json!({ "modules.org.powervoice.gain.name": "Not mine" }).to_string();
        assert!(matches!(
            validate_module_locale(ID, json.as_bytes()),
            Err(LocaleError::KeyOutsideNamespace(_))
        ));
        // A prefix look-alike that isn't actually namespaced under this id (no trailing dot) is
        // rejected too: "modules.com.acme.deesser.evil" must not pass for id "com.acme.deesser".
        let json = serde_json::json!({ "modules.com.acme.deesserX.name": "Sneaky" }).to_string();
        assert!(matches!(
            validate_module_locale(ID, json.as_bytes()),
            Err(LocaleError::KeyOutsideNamespace(_))
        ));
    }

    #[test]
    fn a_non_string_value_is_rejected_untrusted_data_is_strings_only() {
        for value in [
            serde_json::json!({ "modules.com.acme.deesser.name": 1 }),
            serde_json::json!({ "modules.com.acme.deesser.name": true }),
            serde_json::json!({ "modules.com.acme.deesser.name": null }),
            serde_json::json!({ "modules.com.acme.deesser.name": ["x"] }),
            serde_json::json!({ "modules.com.acme.deesser.name": { "x": "y" } }),
        ] {
            let bytes = value.to_string();
            assert!(
                matches!(
                    validate_module_locale(ID, bytes.as_bytes()),
                    Err(LocaleError::ValueNotAString(_))
                ),
                "{bytes}"
            );
        }
    }

    #[test]
    fn malformed_json_and_non_object_top_levels_are_rejected() {
        assert!(matches!(
            validate_module_locale(ID, b"{ not json"),
            Err(LocaleError::Json(_))
        ));
        for bytes in [&b"[1,2,3]"[..], b"\"just a string\"", b"42"] {
            assert_eq!(
                validate_module_locale(ID, bytes),
                Err(LocaleError::NotAnObject)
            );
        }
    }

    #[test]
    fn a_file_over_the_size_cap_is_refused_without_being_parsed() {
        let big = vec![b'x'; (MAX_LOCALE_BYTES + 1) as usize];
        assert_eq!(validate_module_locale(ID, &big), Err(LocaleError::TooLarge));
    }

    #[test]
    fn too_many_keys_is_refused() {
        let mut map = serde_json::Map::new();
        for i in 0..=MAX_LOCALE_ENTRIES {
            map.insert(
                format!("modules.com.acme.deesser.k{i}"),
                serde_json::Value::String("v".into()),
            );
        }
        let bytes = serde_json::Value::Object(map).to_string();
        assert_eq!(
            validate_module_locale(ID, bytes.as_bytes()),
            Err(LocaleError::TooManyEntries)
        );
    }

    #[test]
    fn an_overlong_key_or_value_is_refused() {
        let long_key = format!("modules.com.acme.deesser.{}", "k".repeat(MAX_KEY_BYTES));
        let json = serde_json::json!({ long_key.clone(): "v" }).to_string();
        assert!(matches!(
            validate_module_locale(ID, json.as_bytes()),
            Err(LocaleError::KeyTooLong(_))
        ));
        let long_value = "v".repeat(MAX_VALUE_BYTES + 1);
        let json = serde_json::json!({ "modules.com.acme.deesser.name": long_value }).to_string();
        assert!(matches!(
            validate_module_locale(ID, json.as_bytes()),
            Err(LocaleError::ValueTooLong(_))
        ));
    }

    #[test]
    fn reading_a_missing_file_is_ok_none_not_an_error() {
        let dir = crate::install::tests::TempDir::new("locale-missing");
        assert_eq!(read_module_locale(ID, &dir.0, "en"), Ok(None));
    }

    #[test]
    fn reading_reads_the_requested_language_only() {
        let dir = crate::install::tests::TempDir::new("locale-read");
        let locales = dir.0.join("locales");
        std::fs::create_dir_all(&locales).unwrap();
        std::fs::write(
            locales.join("en.json"),
            serde_json::json!({ "modules.com.acme.deesser.name": "De-esser" }).to_string(),
        )
        .unwrap();
        assert_eq!(read_module_locale(ID, &dir.0, "fr"), Ok(None));
        let map = read_module_locale(ID, &dir.0, "en").unwrap().unwrap();
        assert_eq!(map["modules.com.acme.deesser.name"], "De-esser".to_owned());
    }

    #[test]
    fn reading_an_invalid_file_returns_the_error_instead_of_panicking() {
        let dir = crate::install::tests::TempDir::new("locale-invalid");
        let locales = dir.0.join("locales");
        std::fs::create_dir_all(&locales).unwrap();
        std::fs::write(locales.join("en.json"), b"{ not json").unwrap();
        assert!(matches!(
            read_module_locale(ID, &dir.0, "en"),
            Err(LocaleError::Json(_))
        ));
    }
}
