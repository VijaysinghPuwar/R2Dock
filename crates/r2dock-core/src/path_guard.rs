//! Safe mapping from untrusted object keys to local paths.
//!
//! Rules (PLAN.md 9.6): drop empty and `.` segments, reject `..`, replace
//! characters the platform cannot store, rename reserved Windows names,
//! strip invisible formatting characters, stay inside the destination root,
//! and never write through a symbolic link.

use std::io;
use std::path::{Component, Path, PathBuf};

use thiserror::Error;

use crate::keys::is_deceptive_char;

/// Most file systems limit one name to 255 bytes.
pub const MAX_COMPONENT_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Unix,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathGuardError {
    #[error("The name contains \"..\" and cannot be saved safely.")]
    Traversal,
    #[error("The name is empty.")]
    Empty,
    #[error("A part of the name is too long for this system.")]
    ComponentTooLong,
    #[error("The destination is outside the chosen folder.")]
    OutsideRoot,
    #[error("The destination contains a shortcut (symbolic link), so nothing was written there.")]
    SymlinkInPath,
    #[error("A file is in the way of a folder with the same name.")]
    NotADirectory,
    #[error("Could not prepare the destination folder.")]
    Io(io::ErrorKind),
}

impl From<io::Error> for PathGuardError {
    fn from(err: io::Error) -> Self {
        Self::Io(err.kind())
    }
}

const WINDOWS_INVALID: [char; 9] = ['<', '>', ':', '"', '|', '?', '*', '\\', '/'];

fn is_windows_reserved(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    match upper.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => {
            // COM0-COM9 and LPT0-LPT9, plus the superscript digits Windows
            // also treats as device numbers.
            let Some(rest) = upper
                .strip_prefix("COM")
                .or_else(|| upper.strip_prefix("LPT"))
            else {
                return false;
            };
            let mut chars = rest.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('0'..='9' | '¹' | '²' | '³'), None)
            )
        }
    }
}

/// Shorten a file name to at most `max` bytes, keeping a short extension and
/// cutting only at character boundaries.
pub fn truncate_name(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let ext = match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 && name.len() - i < max => &name[i..],
        _ => "",
    };
    let stem = &name[..name.len() - ext.len()];
    let mut cut = max - ext.len();
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", &stem[..cut])
}

/// Like [`key_to_relative_path`], but never fails on a single bad segment:
/// empty or dot-only segments become `_` and long names are shortened.
/// Used for downloads, where an odd key should still land as a usable file.
pub fn key_to_relative_path_lossy(rel_key: &str, platform: Platform) -> PathBuf {
    let mut path = PathBuf::new();
    for segment in rel_key.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        let safe = match sanitize_component(segment, platform) {
            Ok(s) => s,
            Err(PathGuardError::ComponentTooLong) => {
                sanitize_component(&truncate_name(segment, MAX_COMPONENT_BYTES), platform)
                    .unwrap_or_else(|_| "_".to_string())
            }
            Err(_) => "_".to_string(),
        };
        path.push(safe);
    }
    if path.as_os_str().is_empty() {
        path.push("_");
    }
    path
}

/// Make one path segment safe for the target platform.
pub fn sanitize_component(segment: &str, platform: Platform) -> Result<String, PathGuardError> {
    if segment == ".." {
        return Err(PathGuardError::Traversal);
    }
    let mut out: String = segment
        .chars()
        .filter(|c| !is_deceptive_char(*c))
        .map(|c| {
            if c.is_control() || (platform == Platform::Windows && WINDOWS_INVALID.contains(&c)) {
                '_'
            } else {
                c
            }
        })
        .collect();

    if platform == Platform::Windows {
        let trimmed_len = out.trim_end_matches(['.', ' ']).len();
        out.truncate(trimmed_len);
        let stem = out.split('.').next().unwrap_or("");
        if is_windows_reserved(stem.trim_end()) {
            out.insert(0, '_');
        }
    }

    if out.is_empty() || out == "." {
        return Err(PathGuardError::Empty);
    }
    if out == ".." {
        return Err(PathGuardError::Traversal);
    }
    if out.len() > MAX_COMPONENT_BYTES {
        return Err(PathGuardError::ComponentTooLong);
    }
    Ok(out)
}

/// Convert the part of a key below the download prefix into a relative path.
pub fn key_to_relative_path(rel_key: &str, platform: Platform) -> Result<PathBuf, PathGuardError> {
    let mut path = PathBuf::new();
    for segment in rel_key.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        path.push(sanitize_component(segment, platform)?);
    }
    if path.as_os_str().is_empty() {
        return Err(PathGuardError::Empty);
    }
    Ok(path)
}

/// Create (or check) every directory of `rel_dir` under `root`, refusing
/// symlinks and anything that would leave the root. Returns the absolute directory.
pub fn ensure_dir_under_root(root: &Path, rel_dir: &Path) -> Result<PathBuf, PathGuardError> {
    let root = root.canonicalize()?;
    if !root.is_dir() {
        return Err(PathGuardError::NotADirectory);
    }
    let mut current = root.clone();
    for component in rel_dir.components() {
        let Component::Normal(name) = component else {
            return Err(PathGuardError::Traversal);
        };
        current.push(name);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => return Err(PathGuardError::SymlinkInPath),
            Ok(meta) if !meta.is_dir() => return Err(PathGuardError::NotADirectory),
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                match std::fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                        let meta = std::fs::symlink_metadata(&current)?;
                        if meta.file_type().is_symlink() {
                            return Err(PathGuardError::SymlinkInPath);
                        }
                        if !meta.is_dir() {
                            return Err(PathGuardError::NotADirectory);
                        }
                    }
                    Err(err) => return Err(err.into()),
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
    if !current.starts_with(&root) {
        return Err(PathGuardError::OutsideRoot);
    }
    Ok(current)
}

/// Resolve the final file path for a relative path under `root`, creating
/// parent folders safely. The file itself is not created.
pub fn resolve_file_under_root(root: &Path, rel_file: &Path) -> Result<PathBuf, PathGuardError> {
    let file_name = rel_file.file_name().ok_or(PathGuardError::Empty)?;
    let parent = rel_file.parent().unwrap_or_else(|| Path::new(""));
    let dir = ensure_dir_under_root(root, parent)?;
    let target = dir.join(file_name);
    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if meta.file_type().is_symlink() {
            return Err(PathGuardError::SymlinkInPath);
        }
        if meta.is_dir() {
            return Err(PathGuardError::NotADirectory);
        }
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rejects_traversal() {
        for key in ["../etc/passwd", "a/../../b", "..", "a/.."] {
            assert_eq!(
                key_to_relative_path(key, Platform::Unix),
                Err(PathGuardError::Traversal),
                "{key}"
            );
            assert_eq!(
                key_to_relative_path(key, Platform::Windows),
                Err(PathGuardError::Traversal),
                "{key}"
            );
        }
    }

    #[test]
    fn absolute_looking_keys_stay_relative() {
        let p = key_to_relative_path("/etc/passwd", Platform::Unix).unwrap();
        assert_eq!(p, PathBuf::from("etc").join("passwd"));
        let w = key_to_relative_path("C:/Windows/system.ini", Platform::Windows).unwrap();
        assert!(w.is_relative());
        assert!(w.to_string_lossy().starts_with("C_"));
        let b = key_to_relative_path("..\\..\\x", Platform::Windows).unwrap();
        assert!(!b.to_string_lossy().contains('\\') || b.components().count() == 1);
        assert_eq!(b, PathBuf::from(".._.._x"));
    }

    #[test]
    fn drops_empty_and_dot_segments() {
        let p = key_to_relative_path("a//./b/c.txt", Platform::Unix).unwrap();
        assert_eq!(p, PathBuf::from("a").join("b").join("c.txt"));
        assert_eq!(
            key_to_relative_path("//./", Platform::Unix),
            Err(PathGuardError::Empty)
        );
    }

    #[test]
    fn windows_reserved_and_invalid() {
        let s = |x| sanitize_component(x, Platform::Windows).unwrap();
        assert_eq!(s("CON"), "_CON");
        assert_eq!(s("con.txt"), "_con.txt");
        assert_eq!(s("LPT9.log"), "_LPT9.log");
        assert_eq!(s("COM10"), "COM10");
        assert_eq!(s("a<b>c?.txt"), "a_b_c_.txt");
        assert_eq!(s("trailing. . "), "trailing");
        assert_eq!(
            sanitize_component("...", Platform::Windows),
            Err(PathGuardError::Empty)
        );
        assert_eq!(sanitize_component("CON", Platform::Unix).unwrap(), "CON");
    }

    #[test]
    fn strips_bidi_and_replaces_controls() {
        let s = sanitize_component("report\u{202E}fdp.exe", Platform::Unix).unwrap();
        assert_eq!(s, "reportfdp.exe");
        assert_eq!(
            sanitize_component("a\u{7}b", Platform::Unix).unwrap(),
            "a_b"
        );
    }

    #[test]
    fn long_component_rejected() {
        let long = "a".repeat(256);
        assert_eq!(
            sanitize_component(&long, Platform::Unix),
            Err(PathGuardError::ComponentTooLong)
        );
    }

    #[test]
    fn creates_dirs_inside_root() {
        let dir = tempfile::tempdir().unwrap();
        let rel = key_to_relative_path("x/y/z.txt", Platform::current()).unwrap();
        let target = resolve_file_under_root(dir.path(), &rel).unwrap();
        assert!(target.starts_with(dir.path().canonicalize().unwrap()));
        assert!(target.parent().unwrap().is_dir());
    }

    #[test]
    fn file_in_the_way_of_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x"), b"file").unwrap();
        let rel = PathBuf::from("x").join("y.txt");
        assert_eq!(
            resolve_file_under_root(dir.path(), &rel),
            Err(PathGuardError::NotADirectory)
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        let rel = PathBuf::from("link").join("evil.txt");
        assert_eq!(
            resolve_file_under_root(dir.path(), &rel),
            Err(PathGuardError::SymlinkInPath)
        );
        assert!(!outside.path().join("evil.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_target_file() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path().join("t"), dir.path().join("f.txt")).unwrap();
        assert_eq!(
            resolve_file_under_root(dir.path(), Path::new("f.txt")),
            Err(PathGuardError::SymlinkInPath)
        );
    }

    #[test]
    fn windows_device_names_with_zero_and_superscripts() {
        for name in ["COM0", "lpt0.txt", "COM¹", "LPT³.log", "com9"] {
            let out = sanitize_component(name, Platform::Windows).unwrap();
            assert!(out.starts_with('_'), "{name} -> {out}");
        }
        for name in ["COM10", "COMx", "LPT", "computer.txt"] {
            assert_eq!(sanitize_component(name, Platform::Windows).unwrap(), name);
        }
    }

    #[test]
    fn truncates_names_keeping_extension() {
        let long = format!("{}.mp3", "a".repeat(300));
        let t = truncate_name(&long, 255);
        assert_eq!(t.len(), 255);
        assert!(t.ends_with(".mp3"));
        // Multi-byte characters are never split.
        let wide = "é".repeat(200);
        let t = truncate_name(&wide, 101);
        assert!(t.len() <= 101 && t.chars().all(|c| c == 'é'));
        assert_eq!(truncate_name("short.txt", 255), "short.txt");
    }

    #[test]
    fn lossy_paths_never_use_the_raw_key() {
        let p = key_to_relative_path_lossy("photos/. ./x.jpg", Platform::Windows);
        assert_eq!(p, PathBuf::from("photos").join("_").join("x.jpg"));
        let p = key_to_relative_path_lossy("a/../b", Platform::Windows);
        assert_eq!(p, PathBuf::from("a").join("_").join("b"));
        let long = "n".repeat(400);
        let p = key_to_relative_path_lossy(&long, Platform::Windows);
        assert_eq!(p.as_os_str().len(), MAX_COMPONENT_BYTES);
        assert_eq!(
            key_to_relative_path_lossy("///", Platform::Unix),
            PathBuf::from("_")
        );
    }

    proptest! {
        #[test]
        fn lossy_never_escapes_root(key in "(?s).{0,80}") {
            let p = key_to_relative_path_lossy(&key, Platform::current());
            prop_assert!(p.is_relative());
            for c in p.components() {
                prop_assert!(matches!(c, Component::Normal(_)), "bad component in {:?}", p);
            }
        }

        #[test]
        fn never_escapes_root(key in "(?s).{0,60}") {
            // Paths are interpreted by the host, so only the host platform's rules apply.
            {
                if let Ok(p) = key_to_relative_path(&key, Platform::current()) {
                    prop_assert!(p.is_relative());
                    for c in p.components() {
                        prop_assert!(matches!(c, Component::Normal(_)), "bad component in {:?}", p);
                    }
                }
            }
        }
    }
}
