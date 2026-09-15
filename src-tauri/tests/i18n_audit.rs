use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[test]
fn test_backend_i18n_keys_exist() {
    // Get workspace root from CARGO_MANIFEST_DIR (points to src-tauri/Cargo.toml)
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("src-tauri has a parent");

    // Load en.json
    let en_json_path = workspace_root.join("ui/src/lib/i18n/en.json");
    let en_json_content = fs::read_to_string(&en_json_path).expect("Failed to read en.json");
    let en_json: serde_json::Value =
        serde_json::from_str(&en_json_content).expect("Failed to parse en.json");

    // Scan Rust sources for error.* and notice.* keys
    let mut found_keys = HashSet::new();

    scan_rust_dir(&workspace_root.join("src-tauri/src"), &mut found_keys);
    scan_rust_dir(&workspace_root.join("crates"), &mut found_keys);

    // Filter out known patterns (these are intentionally not in en.json)
    let ignored_patterns = [
        "notice.example",               // Test/example key
        "notice.device.lost.{}",        // Template pattern
        "notice.device.not_found.{}",   // Template pattern
        "notice.device.reconnected.{}", // Template pattern
    ];

    let found_keys: Vec<_> = found_keys
        .iter()
        .filter(|k| !ignored_patterns.contains(&k.as_str()))
        .collect();

    let mut missing_keys = Vec::new();

    for key in found_keys {
        if en_json.get(key).is_none() {
            missing_keys.push(key.clone());
        }
    }

    if !missing_keys.is_empty() {
        missing_keys.sort();
        panic!(
            "Backend error/notice keys missing from en.json:\n  {}",
            missing_keys.join("\n  ")
        );
    }
}

fn scan_rust_dir(dir: &Path, keys: &mut HashSet<String>) {
    if !dir.exists() {
        return;
    }

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !path.ends_with("target") {
                scan_rust_dir(&path, keys);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && let Ok(content) = fs::read_to_string(&path)
            {
                extract_i18n_keys(&content, keys);
            }
        }
    }
}

fn extract_i18n_keys(content: &str, keys: &mut HashSet<String>) {
    // Match "error.xxx" and "notice.xxx" strings
    let mut pos = 0;
    while let Some(idx) = content[pos..].find('"') {
        pos += idx + 1;
        let remaining = &content[pos..];

        // Look for error. or notice. prefix
        if remaining.starts_with("error.") || remaining.starts_with("notice.") {
            // Find the closing quote
            if let Some(end_idx) = remaining.find('"') {
                let key = &remaining[..end_idx];
                keys.insert(key.to_string());
                pos += end_idx;
            } else {
                break;
            }
        }
    }
}
