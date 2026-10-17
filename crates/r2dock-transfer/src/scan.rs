//! Scanning local selections for upload (PLAN.md 9.6, 12.3).
//!
//! Symbolic links and special files are skipped and reported. Folder
//! structure is kept, including empty folders (as folder markers).

use std::path::{Path, PathBuf};

use r2dock_core::keys::{display_name, key_from_components, validate_key};
use r2dock_core::redact::redact_path;
use serde::Serialize;
use ts_rs::TS;
use walkdir::WalkDir;

/// Refuse selections larger than this many files in one go.
pub const MAX_FILES: usize = 200_000;

const HIDDEN_OS_FILES: [&str; 4] = [".DS_Store", "Thumbs.db", "desktop.ini", "._.DS_Store"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanFile {
    pub local: PathBuf,
    /// Path components below the selection's parent folder.
    pub components: Vec<String>,
    pub size: u64,
    pub identity: Option<crate::source::FileIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SkippedItem {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scan {
    pub files: Vec<ScanFile>,
    /// Empty folders, as components, to create as markers.
    pub empty_dirs: Vec<Vec<String>>,
    pub folders: u32,
    pub bytes: u64,
    pub skipped: Vec<SkippedItem>,
    /// Names of the top-level folders selected (for group labels).
    pub top_folders: Vec<String>,
    pub too_many: bool,
}

fn components_of(base: &Path, path: &Path) -> Option<Vec<String>> {
    let rel = path.strip_prefix(base).ok()?;
    rel.components()
        .map(|c| c.as_os_str().to_str().map(str::to_string))
        .collect()
}

fn unstorable(name: &str) -> bool {
    name.chars().any(char::is_control) || validate_key(name).is_err()
}

/// Scan files and folders. Blocking: call from `spawn_blocking`.
pub fn scan(paths: &[PathBuf], include_hidden_os_files: bool, home: Option<&Path>) -> Scan {
    let mut out = Scan::default();
    let skip = |out: &mut Scan, path: &Path, reason: &str| {
        out.skipped.push(SkippedItem {
            path: display_name(&redact_path(path, home)),
            reason: reason.to_string(),
        });
    };
    for root in paths {
        let Some(base) = root.parent() else { continue };
        let meta = match std::fs::symlink_metadata(root) {
            Ok(m) => m,
            Err(_) => {
                skip(&mut out, root, "Could not read this item.");
                continue;
            }
        };
        if meta.file_type().is_symlink() {
            skip(&mut out, root, "Shortcut (symbolic link)");
            continue;
        }
        if meta.is_dir()
            && let Some(name) = root.file_name().and_then(|n| n.to_str())
        {
            out.top_folders.push(name.to_string());
        }
        let walker = WalkDir::new(root).follow_links(false).sort_by_file_name();
        let mut dir_children: Vec<(PathBuf, usize)> = Vec::new();
        for entry in walker {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    let p = e.path().map_or_else(|| root.clone(), Path::to_path_buf);
                    skip(&mut out, &p, "Could not read this item.");
                    continue;
                }
            };
            let path = entry.path();
            // Count children for empty-folder detection.
            if let Some(parent) = path.parent()
                && let Some(slot) = dir_children.iter_mut().rev().find(|(d, _)| d == parent)
            {
                slot.1 += 1;
            }
            let ft = entry.file_type();
            let name = entry.file_name().to_string_lossy();
            if !include_hidden_os_files && HIDDEN_OS_FILES.contains(&name.as_ref()) {
                continue;
            }
            let Some(components) = components_of(base, path) else {
                skip(
                    &mut out,
                    path,
                    "Name is not valid text, so R2 cannot store it.",
                );
                continue;
            };
            if components.iter().any(|c| unstorable(c)) {
                skip(&mut out, path, "Name contains characters R2 cannot store.");
                continue;
            }
            if ft.is_symlink() {
                skip(&mut out, path, "Shortcut (symbolic link)");
            } else if ft.is_dir() {
                out.folders += 1;
                dir_children.push((path.to_path_buf(), 0));
            } else if ft.is_file() {
                if out.files.len() >= MAX_FILES {
                    out.too_many = true;
                    break;
                }
                let meta = entry.metadata().ok();
                let size = meta.as_ref().map_or(0, std::fs::Metadata::len);
                out.bytes += size;
                out.files.push(ScanFile {
                    local: path.to_path_buf(),
                    components,
                    size,
                    identity: meta.as_ref().and_then(crate::source::FileIdentity::of),
                });
            } else {
                skip(&mut out, path, "Special file (not a regular file)");
            }
        }
        for (dir, children) in dir_children {
            if children == 0
                && let Some(c) = components_of(base, &dir)
            {
                out.empty_dirs.push(c);
            }
        }
        if out.too_many {
            break;
        }
    }
    out
}

/// Build the destination key for a scanned file. `None` when the key would be invalid.
pub fn key_for(prefix: &str, components: &[String]) -> Option<String> {
    key_from_components(prefix, components.iter().map(String::as_str)).ok()
}

/// Folder marker key for an empty folder.
pub fn marker_for(prefix: &str, components: &[String]) -> Option<String> {
    let mut key = key_for(prefix, components)?;
    key.push('/');
    validate_key(&key).ok()?;
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_folders_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("album");
        std::fs::create_dir_all(root.join("disc 1")).unwrap();
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("disc 1").join("01.mp3"), b"abc").unwrap();
        std::fs::write(root.join("cover.jpg"), b"12345").unwrap();
        std::fs::write(root.join(".DS_Store"), b"junk").unwrap();
        let single = dir.path().join("single.txt");
        std::fs::write(&single, b"x").unwrap();

        let s = scan(&[root.clone(), single], false, None);
        assert_eq!(s.files.len(), 3);
        assert_eq!(s.bytes, 9);
        assert_eq!(s.folders, 3);
        assert_eq!(s.top_folders, vec!["album".to_string()]);
        assert_eq!(
            s.empty_dirs,
            vec![vec!["album".to_string(), "empty".to_string()]]
        );
        let keys: Vec<String> = s
            .files
            .iter()
            .map(|f| key_for("up/", &f.components).unwrap())
            .collect();
        assert!(keys.contains(&"up/album/disc 1/01.mp3".to_string()));
        assert!(keys.contains(&"up/single.txt".to_string()));
        assert_eq!(
            marker_for("up/", &s.empty_dirs[0]).unwrap(),
            "up/album/empty/"
        );

        let with_hidden = scan(&[root], true, None);
        assert_eq!(with_hidden.files.len(), 3);
    }

    #[cfg(unix)]
    #[test]
    fn skips_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("src");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("real.txt"), b"x").unwrap();
        std::os::unix::fs::symlink("/etc/hosts", root.join("link.txt")).unwrap();
        std::os::unix::fs::symlink("/etc", root.join("linkdir")).unwrap();
        let s = scan(&[root], false, None);
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.skipped.len(), 2);
        assert!(s.skipped.iter().all(|i| i.reason.contains("symbolic link")));
    }

    #[cfg(unix)]
    #[test]
    fn skips_control_character_names() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("bad\u{7}.txt"), b"x").unwrap();
        let s = scan(&[dir.path().join("bad\u{7}.txt")], false, None);
        assert!(s.files.is_empty());
        assert_eq!(s.skipped.len(), 1);
        assert!(s.skipped[0].path.contains("U+0007"));
    }
}
