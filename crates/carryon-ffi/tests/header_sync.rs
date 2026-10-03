//! Drift guard: every `#[no_mangle]` symbol in src/ must be declared in
//! include/carryon.h. Hand-written header + this test replace cbindgen (offline).

use std::fs;
use std::path::Path;

#[test]
fn every_exported_symbol_is_in_header() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let header = fs::read_to_string(root.join("include/carryon.h")).expect("read carryon.h");

    let mut symbols = Vec::new();
    for entry in fs::read_dir(root.join("src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = src.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if line.trim_start().starts_with("#[no_mangle]") {
                // The symbol name is on a following `pub ... fn <name>` line.
                for follow in lines.iter().skip(i + 1).take(4) {
                    if let Some(name) = extract_fn_name(follow) {
                        // The test-only panic entry is feature-gated; not in the header.
                        if name != "carryon_debug_panic" {
                            symbols.push(name);
                        }
                        break;
                    }
                }
            }
        }
    }
    symbols.sort();
    symbols.dedup();
    assert!(!symbols.is_empty(), "no exported symbols found");

    let missing: Vec<_> = symbols
        .iter()
        .filter(|s| !header.contains(s.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "symbols missing from include/carryon.h: {missing:?}"
    );
}

fn extract_fn_name(line: &str) -> Option<String> {
    let idx = line.find("fn ")?;
    let rest = &line[idx + 3..];
    let end = rest.find(['(', '<', ' ']).unwrap_or(rest.len());
    let name = rest[..end].trim();
    if name.starts_with("carryon_") {
        Some(name.to_string())
    } else {
        None
    }
}
