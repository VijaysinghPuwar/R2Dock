//! Object key helpers.
//!
//! R2 has no folders. A "folder" is the part of a key before a `/`. These
//! helpers keep that model consistent and treat every key as untrusted.

use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// R2 limits object keys to 1,024 bytes.
pub const MAX_KEY_BYTES: usize = 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeyError {
    #[error("Name is empty.")]
    Empty,
    #[error("Name is too long for R2 (keys are limited to 1,024 bytes).")]
    TooLong,
    #[error("Name contains a control character R2 cannot store.")]
    ControlCharacter,
    #[error("Folder names cannot contain \"/\".")]
    ContainsSlash,
    #[error("A folder path must be empty or end with \"/\".")]
    BadPrefix,
}

/// Validate a full object key.
pub fn validate_key(key: &str) -> Result<(), KeyError> {
    if key.is_empty() {
        return Err(KeyError::Empty);
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(KeyError::TooLong);
    }
    if key.chars().any(is_control) {
        return Err(KeyError::ControlCharacter);
    }
    Ok(())
}

/// Validate a prefix used as a folder location: empty (bucket root) or ending in `/`.
pub fn validate_prefix(prefix: &str) -> Result<(), KeyError> {
    if prefix.is_empty() {
        return Ok(());
    }
    if !prefix.ends_with('/') {
        return Err(KeyError::BadPrefix);
    }
    validate_key(prefix)
}

/// Validate one new name typed by the user (file or folder name, no slashes).
pub fn validate_name(name: &str) -> Result<(), KeyError> {
    if name.trim().is_empty() {
        return Err(KeyError::Empty);
    }
    if name.contains('/') {
        return Err(KeyError::ContainsSlash);
    }
    if name == "." || name == ".." {
        return Err(KeyError::Empty);
    }
    validate_key(name)
}

fn is_control(c: char) -> bool {
    c.is_control()
}

/// Join a folder prefix and a name into a key.
pub fn join_key(prefix: &str, name: &str) -> String {
    let mut key = String::with_capacity(prefix.len() + name.len());
    key.push_str(prefix);
    key.push_str(name);
    key
}

/// The folder prefix that contains `key`. `a/b/c.txt` and `a/b/c/` both give `a/b/`.
pub fn parent_prefix(key: &str) -> &str {
    let trimmed = key.strip_suffix('/').unwrap_or(key);
    match trimmed.rfind('/') {
        Some(i) => &key[..=i],
        None => "",
    }
}

/// The last segment of a key, without a trailing slash.
pub fn base_name(key: &str) -> &str {
    let trimmed = key.strip_suffix('/').unwrap_or(key);
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// Double extensions kept together when building a "Keep both" name.
const COMPOUND_EXTENSIONS: [&str; 5] = [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".sql.gz"];

/// Build the name used by the "Keep both" policy: `cover.webp` becomes `cover (1).webp`.
pub fn keep_both_name(name: &str, n: u32) -> String {
    let lower = name.to_ascii_lowercase();
    let split_at = COMPOUND_EXTENSIONS
        .iter()
        .find(|ext| lower.ends_with(*ext) && lower.len() > ext.len())
        .map(|ext| name.len() - ext.len())
        .or_else(|| match name.rfind('.') {
            // A leading dot is a hidden file, not an extension.
            Some(0) | None => None,
            Some(i) => Some(i),
        });
    match split_at {
        Some(i) => format!("{} ({n}){}", &name[..i], &name[i..]),
        None => format!("{name} ({n})"),
    }
}

/// True for characters that can disguise how a name reads: bidirectional
/// controls, zero-width characters, and other invisible formatting.
pub fn is_deceptive_char(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
            | '\u{061C}'
            | '\u{00AD}'
    )
}

/// Render a name for display with control and deceptive characters shown as
/// visible placeholders such as `⟨U+202E⟩`.
pub fn display_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if is_control(c) || is_deceptive_char(c) {
            out.push_str(&format!("\u{27E8}U+{:04X}\u{27E9}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// Keys with empty segments (`a//b`) are legal in R2 but unusual.
pub fn is_unusual_key(key: &str) -> bool {
    key.contains("//") || key.starts_with('/')
}

/// Build an object key from local relative path components.
/// Components are NFC-normalized so names from macOS (NFD) match other systems.
pub fn key_from_components<'a, I>(prefix: &str, components: I) -> Result<String, KeyError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut key = String::from(prefix);
    let mut first = true;
    for part in components {
        if part.is_empty() || part == "." || part == ".." {
            return Err(KeyError::Empty);
        }
        if !first {
            key.push('/');
        }
        first = false;
        key.extend(part.nfc());
    }
    if first {
        return Err(KeyError::Empty);
    }
    validate_key(&key)?;
    Ok(key)
}

/// A breadcrumb segment: display name and the prefix it navigates to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub name: String,
    pub prefix: String,
}

/// Split a prefix into breadcrumb segments. `a//b/` gives `a`, an empty segment, and `b`.
pub fn breadcrumbs(prefix: &str) -> Vec<Crumb> {
    let mut crumbs = Vec::new();
    let mut end = 0;
    for (i, c) in prefix.char_indices() {
        if c == '/' {
            crumbs.push(Crumb {
                name: prefix[end..i].to_string(),
                prefix: prefix[..=i].to_string(),
            });
            end = i + 1;
        }
    }
    crumbs
}

/// Encode a key for use in a URL path, keeping `/` separators.
pub fn percent_encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for b in key.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parent_and_base() {
        assert_eq!(parent_prefix("a/b/c.txt"), "a/b/");
        assert_eq!(parent_prefix("a/b/c/"), "a/b/");
        assert_eq!(parent_prefix("c.txt"), "");
        assert_eq!(parent_prefix("c/"), "");
        assert_eq!(base_name("a/b/c.txt"), "c.txt");
        assert_eq!(base_name("a/b/c/"), "c");
        assert_eq!(base_name("top"), "top");
    }

    #[test]
    fn keep_both_names() {
        assert_eq!(keep_both_name("cover.webp", 1), "cover (1).webp");
        assert_eq!(keep_both_name("README", 2), "README (2)");
        assert_eq!(keep_both_name(".env", 1), ".env (1)");
        assert_eq!(keep_both_name("db.tar.gz", 1), "db (1).tar.gz");
        assert_eq!(keep_both_name("db-2026.sql.gz", 3), "db-2026 (3).sql.gz");
        assert_eq!(keep_both_name("a.b.c", 1), "a.b (1).c");
    }

    #[test]
    fn display_name_reveals_bidi_override() {
        let name = "invoice\u{202E}fdp.exe";
        let shown = display_name(name);
        assert_eq!(shown, "invoice\u{27E8}U+202E\u{27E9}fdp.exe");
        assert_eq!(display_name("bell\u{7}"), "bell\u{27E8}U+0007\u{27E9}");
        assert_eq!(display_name("plain name.mp3"), "plain name.mp3");
    }

    #[test]
    fn validation() {
        assert_eq!(validate_key(""), Err(KeyError::Empty));
        assert_eq!(validate_key(&"a".repeat(1025)), Err(KeyError::TooLong));
        assert!(validate_key(&"a".repeat(1024)).is_ok());
        assert_eq!(validate_key("a\nb"), Err(KeyError::ControlCharacter));
        assert!(validate_prefix("").is_ok());
        assert!(validate_prefix("a/").is_ok());
        assert_eq!(validate_prefix("a"), Err(KeyError::BadPrefix));
        assert_eq!(validate_name("a/b"), Err(KeyError::ContainsSlash));
        assert_eq!(validate_name("  "), Err(KeyError::Empty));
        assert_eq!(validate_name(".."), Err(KeyError::Empty));
    }

    #[test]
    fn key_from_components_normalizes_nfd() {
        // "é" as e + combining acute (NFD), as macOS may report it.
        let nfd = "caf\u{0065}\u{0301}.txt";
        let key = key_from_components("music/", ["albums", nfd]).unwrap();
        assert_eq!(key, "music/albums/caf\u{00E9}.txt");
        assert!(key_from_components("", ["..", "x"]).is_err());
        assert!(key_from_components("", std::iter::empty()).is_err());
    }

    #[test]
    fn breadcrumbs_keep_empty_segments() {
        let crumbs = breadcrumbs("a//b/");
        assert_eq!(crumbs.len(), 3);
        assert_eq!(crumbs[0].name, "a");
        assert_eq!(crumbs[1].name, "");
        assert_eq!(crumbs[1].prefix, "a//");
        assert_eq!(crumbs[2].prefix, "a//b/");
        assert!(breadcrumbs("").is_empty());
    }

    #[test]
    fn encodes_keys_for_urls() {
        assert_eq!(
            percent_encode_key("Audiobook G/cover.webp"),
            "Audiobook%20G/cover.webp"
        );
        assert_eq!(percent_encode_key("a?b#c"), "a%3Fb%23c");
        assert_eq!(percent_encode_key("é"), "%C3%A9");
    }

    proptest! {
        #[test]
        fn parent_plus_base_rebuilds_file_keys(segs in proptest::collection::vec("[a-z0-9 ._-]{1,8}", 1..5)) {
            let key = segs.join("/");
            let rebuilt = format!("{}{}", parent_prefix(&key), base_name(&key));
            prop_assert_eq!(rebuilt, key);
        }

        #[test]
        fn display_name_never_contains_raw_controls(s in "(?s).{0,40}") {
            let shown = display_name(&s);
            prop_assert!(!shown.chars().any(|c| c.is_control() || is_deceptive_char(c)));
        }
    }
}
