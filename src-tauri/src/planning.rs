//! Turning local selections into upload jobs and keys into download jobs.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use r2dock_core::keys::{base_name, parent_prefix};
use r2dock_core::path_guard::{Platform, key_to_relative_path};
use r2dock_core::policy::ConflictPolicy;
use r2dock_r2::bulk::ObjectRef;
use r2dock_r2::{R2Client, R2Error};
use r2dock_transfer::scan::{Scan, SkippedItem, key_for, marker_for};
use tokio_util::sync::CancellationToken;

/// Above this many files, conflicts are found by listing instead of one HEAD per file.
const HEAD_LIMIT: usize = 100;
/// Do not list more than this many objects to look for conflicts.
const CONFLICT_LIST_CAP: usize = 200_000;

pub struct UploadItems {
    pub files: Vec<crate::state::UploadFile>,
    pub markers: Vec<String>,
    pub skipped: Vec<SkippedItem>,
}

/// Build destination keys under `prefix`. Names R2 cannot store are skipped.
pub fn upload_items(scan: Scan, prefix: &str, home: Option<&Path>) -> UploadItems {
    let mut skipped = scan.skipped;
    let mut files = Vec::with_capacity(scan.files.len());
    for f in scan.files {
        match key_for(prefix, &f.components) {
            Some(key) => files.push(crate::state::UploadFile {
                local: f.local,
                key,
                size: f.size,
                identity: f.identity,
            }),
            None => skipped.push(SkippedItem {
                path: r2dock_core::redact::redact_path(&f.local, home),
                reason: "Name is too long for R2 (keys are limited to 1,024 bytes).".to_string(),
            }),
        }
    }
    let markers = scan
        .empty_dirs
        .iter()
        .filter_map(|c| marker_for(prefix, c))
        .collect();
    UploadItems {
        files,
        markers,
        skipped,
    }
}

/// Count destination keys that already exist.
pub async fn count_conflicts(
    client: &R2Client,
    bucket: &str,
    prefix: &str,
    keys: &[String],
) -> Result<u32, R2Error> {
    if keys.len() <= HEAD_LIMIT {
        let futures: Vec<_> = keys
            .iter()
            .map(|k| Box::pin(client.exists(bucket, k)))
            .collect();
        let found = futures_util::stream::iter(futures)
            .buffer_unordered(8)
            .collect::<Vec<_>>()
            .await;
        let mut n = 0;
        for r in found {
            if r? {
                n += 1;
            }
        }
        return Ok(n);
    }
    // List each top-level folder of the selection under the destination.
    let wanted: HashSet<&str> = keys.iter().map(String::as_str).collect();
    let tops: BTreeSet<String> = keys
        .iter()
        .map(|k| {
            let rest = &k[prefix.len().min(k.len())..];
            match rest.find('/') {
                Some(i) => format!("{prefix}{}", &rest[..=i]),
                None => k.clone(),
            }
        })
        .collect();
    let cancel = CancellationToken::new();
    let mut conflicts = 0u32;
    let mut seen = 0usize;
    for top in tops {
        if !top.ends_with('/') {
            if client.exists(bucket, &top).await? {
                conflicts += 1;
            }
            continue;
        }
        client
            .for_each_object(bucket, &top, &cancel, |o| {
                seen += 1;
                if wanted.contains(o.key.as_str()) {
                    conflicts += 1;
                }
                seen < CONFLICT_LIST_CAP
            })
            .await?;
    }
    Ok(conflicts)
}

/// Default conflict policy: the user's setting, or per DESIGN.md "Skip
/// existing" for folder uploads and "Ask" for individual files.
pub fn default_policy(setting: ConflictPolicy, has_folders: bool) -> ConflictPolicy {
    match setting {
        ConflictPolicy::Ask if has_folders => ConflictPolicy::Skip,
        other => other,
    }
}

/// Where each downloaded object goes, relative to the destination folder.
/// Selected files land directly in it; selected folders keep their structure.
/// Folder markers produce directories rather than files.
pub struct DownloadItem {
    pub key: String,
    pub size: u64,
    pub rel_path: PathBuf,
}

pub fn download_items(
    objects: &[ObjectRef],
    keys: &[String],
    prefixes: &[String],
) -> (Vec<DownloadItem>, Vec<PathBuf>) {
    let mut items = Vec::new();
    let mut dirs = Vec::new();
    let rel_for = |rel_key: &str| {
        key_to_relative_path(rel_key, Platform::current())
            .unwrap_or_else(|_| PathBuf::from(rel_key))
    };
    for obj in objects {
        let rel_key = if keys.contains(&obj.key) {
            base_name(&obj.key).to_string()
        } else if let Some(p) = prefixes.iter().find(|p| obj.key.starts_with(p.as_str())) {
            let base = parent_prefix(p);
            obj.key[base.len()..].to_string()
        } else {
            continue;
        };
        if obj.key.ends_with('/') {
            if let Ok(dir) = key_to_relative_path(&rel_key, Platform::current()) {
                dirs.push(dir);
            }
            continue;
        }
        items.push(DownloadItem {
            key: obj.key.clone(),
            size: obj.size,
            rel_path: rel_for(&rel_key),
        });
    }
    (items, dirs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(key: &str) -> ObjectRef {
        ObjectRef {
            key: key.into(),
            size: 1,
        }
    }

    #[test]
    fn download_paths_keep_folder_structure() {
        let objects = vec![
            o("a/b/cover.webp"),
            o("a/b/ch/"),
            o("a/b/ch/1.mp3"),
            o("a/b/ch/sub/2.mp3"),
        ];
        let (items, dirs) =
            download_items(&objects, &["a/b/cover.webp".into()], &["a/b/ch/".into()]);
        let rels: Vec<_> = items.iter().map(|i| i.rel_path.clone()).collect();
        assert!(rels.contains(&PathBuf::from("cover.webp")));
        assert!(rels.contains(&PathBuf::from("ch").join("1.mp3")));
        assert!(rels.contains(&PathBuf::from("ch").join("sub").join("2.mp3")));
        assert_eq!(dirs, vec![PathBuf::from("ch")]);
    }

    #[test]
    fn traversal_keys_keep_raw_path_so_the_job_fails_safely() {
        let objects = vec![o("p/../../x")];
        let (items, _) = download_items(&objects, &[], &["p/".into()]);
        assert_eq!(items.len(), 1);
        assert!(
            items[0]
                .rel_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        );
    }

    #[test]
    fn default_policies() {
        assert_eq!(
            default_policy(ConflictPolicy::Ask, true),
            ConflictPolicy::Skip
        );
        assert_eq!(
            default_policy(ConflictPolicy::Ask, false),
            ConflictPolicy::Ask
        );
        assert_eq!(
            default_policy(ConflictPolicy::Replace, true),
            ConflictPolicy::Replace
        );
    }

    #[test]
    fn upload_keys_and_markers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("album");
        std::fs::create_dir_all(root.join("empty")).unwrap();
        std::fs::write(root.join("a.mp3"), b"x").unwrap();
        let scan = r2dock_transfer::scan::scan(&[root], false, None);
        let items = upload_items(scan, "music/", None);
        assert_eq!(items.files[0].key, "music/album/a.mp3");
        assert_eq!(items.markers, vec!["music/album/empty/".to_string()]);
    }
}
