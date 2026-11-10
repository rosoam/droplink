use crate::{Error, Result};
use std::path::Path;

/// Maximum file name length in bytes (common filesystem limit is 255).
const MAX_NAME_BYTES: usize = 200;

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Sanitize a file name received from the network.
///
/// Removes path separators and control characters, rejects empty/dot names, strips trailing dots
/// and spaces, keeps UTF-8 intact and prefixes Windows reserved device names with `_`.
pub fn sanitize_file_name(raw: &str) -> Result<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '\0') && !c.is_control())
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return Err(Error::InvalidFileName(raw.to_string()));
    }

    let mut name = trimmed
        .trim_start_matches('.')
        .trim_end_matches([' ', '.'])
        .to_string();
    if name.is_empty() {
        return Err(Error::InvalidFileName(raw.to_string()));
    }

    let stem = name
        .split('.')
        .next()
        .unwrap_or(&name)
        .trim()
        .to_ascii_uppercase();
    if WINDOWS_RESERVED.contains(&stem.as_str()) {
        name = format!("_{name}");
    }

    // Truncation may expose a trailing space or dot: strip again.
    let name = truncate_utf8(&name, MAX_NAME_BYTES);
    let name = name.trim_end_matches([' ', '.']).to_string();
    if name.is_empty() {
        return Err(Error::InvalidFileName(raw.to_string()));
    }
    Ok(name)
}

/// Candidate names for a received file: `photo.jpg`, `photo (1).jpg`, `photo (2).jpg`, ...
///
/// Pure generator: the atomic `create_new` loop lives in the I/O layer.
pub fn candidate_names(base: &str) -> impl Iterator<Item = String> {
    let base = base.to_string();
    let (stem, ext) = split_name(&base);
    std::iter::once(base).chain((1u32..).map(move |i| format!("{stem} ({i}){ext}")))
}

/// Lexical containment check: `candidate` must stay directly inside `base`.
pub fn is_within(base: &Path, candidate: &Path) -> bool {
    candidate.parent().map(|p| p == base).unwrap_or(false)
}

fn split_name(name: &str) -> (String, String) {
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(name)
        .to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    (stem, ext)
}

fn truncate_utf8(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_plain_names() {
        assert_eq!(sanitize_file_name("photo.jpg").unwrap(), "photo.jpg");
        assert_eq!(sanitize_file_name("café.jpg").unwrap(), "café.jpg");
        assert_eq!(sanitize_file_name("写真.png").unwrap(), "写真.png");
    }

    #[test]
    fn strips_separators() {
        assert_eq!(sanitize_file_name("../../evil").unwrap(), "evil");
        assert_eq!(sanitize_file_name("a/b\\c.txt").unwrap(), "abc.txt");
        assert_eq!(sanitize_file_name("a\0b.txt").unwrap(), "ab.txt");
    }

    #[test]
    fn rejects_empty_and_dots() {
        for bad in ["", "   ", ".", "..", "..\\..", "..."] {
            assert!(sanitize_file_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn prefixes_windows_reserved() {
        assert_eq!(sanitize_file_name("CON").unwrap(), "_CON");
        assert_eq!(sanitize_file_name("nul.txt").unwrap(), "_nul.txt");
        assert_eq!(sanitize_file_name("COM9.tar.gz").unwrap(), "_COM9.tar.gz");
        assert_eq!(sanitize_file_name("console.txt").unwrap(), "console.txt");
    }

    #[test]
    fn strips_trailing_dots_and_spaces() {
        assert_eq!(sanitize_file_name("report. ").unwrap(), "report");
        assert_eq!(sanitize_file_name("file.tar.  ").unwrap(), "file.tar");
    }

    #[test]
    fn truncates_on_char_boundary() {
        let name = "é".repeat(150);
        let out = sanitize_file_name(&name).unwrap();
        assert!(out.len() <= MAX_NAME_BYTES);
        assert!(out.chars().all(|c| c == 'é'));
    }

    #[test]
    fn candidate_names_are_ordered() {
        let names: Vec<String> = candidate_names("photo.jpg").take(4).collect();
        assert_eq!(
            names,
            vec![
                "photo.jpg",
                "photo (1).jpg",
                "photo (2).jpg",
                "photo (3).jpg"
            ]
        );
        let no_ext: Vec<String> = candidate_names("README").take(2).collect();
        assert_eq!(no_ext, vec!["README", "README (1)"]);
        let double: Vec<String> = candidate_names("a.tar.gz").take(2).collect();
        assert_eq!(double, vec!["a.tar.gz", "a.tar (1).gz"]);
    }

    #[test]
    fn within_check() {
        let base = Path::new("/tmp/drop");
        assert!(is_within(base, &base.join("a.txt")));
        assert!(!is_within(base, Path::new("/tmp/other/a.txt")));
        assert!(!is_within(base, &base.join("sub").join("a.txt")));
    }

    mod proptest_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig { cases: 10_000, ..ProptestConfig::default() })]

            #[test]
            fn sanitized_name_is_safe(raw in "\\PC{0,400}") {
                if let Ok(name) = sanitize_file_name(&raw) {
                    prop_assert!(!name.is_empty());
                    prop_assert!(name.len() <= MAX_NAME_BYTES);
                    prop_assert!(!name.contains('/'));
                    prop_assert!(!name.contains('\\'));
                    prop_assert!(!name.contains('\0'));
                    prop_assert!(name != "." && name != "..");
                    prop_assert!(!name.ends_with('.'));
                    prop_assert!(!name.ends_with(' '));
                }
            }
        }
    }
}
