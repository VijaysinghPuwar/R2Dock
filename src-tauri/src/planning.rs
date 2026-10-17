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

/// Most key changes listed in the review dialog.
const KEY_CHANGES_SHOWN: usize = 50;

/// Decide compression for every file of an upload and sum it up for the
/// review dialog. Reads file headers only. Blocking.
/// `ffmpeg` is the ready FFmpeg, if any; without it videos are not
/// estimated. Must run on a blocking thread inside the Tokio runtime.
pub fn compression_estimate(
    files: &mut [crate::state::UploadFile],
    prefs: &r2dock_core::compression::CompressionPrefs,
    ffmpeg: Option<&r2dock_media::ffmpeg::Ffmpeg>,
) -> Option<crate::dto::CompressionEstimate> {
    use r2dock_media::classify::ContentOverride;
    use r2dock_media::estimate::{FileDecision, decide_file, decide_video};
    let settings = &prefs.audio;
    let tools_ready = ffmpeg.is_some();
    let runtime = tokio::runtime::Handle::try_current().ok();
    use std::collections::HashMap;

    let mut est = crate::dto::CompressionEstimate {
        files: 0,
        bytes_before: 0,
        bytes_after: 0,
        unchanged: 0,
        video_files: 0,
        key_changes: Vec::new(),
        key_change_count: 0,
        collisions: Vec::new(),
        tools_ready,
    };
    let mut audio = 0u32;
    for f in files.iter_mut() {
        let video = match (ffmpeg, &runtime) {
            (Some(ff), Some(rt)) => {
                rt.block_on(decide_video(ff, &f.local, &f.key, settings, &prefs.video))
            }
            _ => None,
        };
        let decision =
            video.unwrap_or_else(|| decide_file(&f.local, &f.key, settings, ContentOverride::Auto));
        match &decision {
            FileDecision::Compress {
                estimated_size,
                new_key,
                video,
            } => {
                audio += 1;
                est.files += 1;
                if *video {
                    est.video_files += 1;
                }
                est.bytes_before += f.size;
                est.bytes_after += (*estimated_size).min(f.size);
                if *new_key != f.key {
                    est.key_change_count += 1;
                    if est.key_changes.len() < KEY_CHANGES_SHOWN {
                        est.key_changes.push(crate::dto::KeyChange {
                            from: f.key.clone(),
                            to: new_key.clone(),
                        });
                    }
                }
            }
            FileDecision::Unchanged(_) => {
                audio += 1;
                est.unchanged += 1;
            }
            FileDecision::NotAudio => {}
        }
        f.compression = Some(decision);
    }
    if audio == 0 {
        return None;
    }
    // Two files that end up under the same key (`a.wav` and `a.mp3`).
    let mut seen: HashMap<String, u32> = HashMap::new();
    for f in files.iter() {
        let key = match &f.compression {
            Some(FileDecision::Compress { new_key, .. }) => new_key.clone(),
            _ => f.key.clone(),
        };
        *seen.entry(key).or_default() += 1;
    }
    let mut collisions: Vec<String> = seen
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(k, _)| k)
        .collect();
    collisions.sort();
    collisions.truncate(KEY_CHANGES_SHOWN);
    est.collisions = collisions;
    Some(est)
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
                compression: None,
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

    fn wav(path: &std::path::Path, secs: u32) {
        let rate = 44_100u32;
        let data_len = rate * 4 * secs;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 4).to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        v.resize(v.len() + data_len as usize, 0);
        std::fs::write(path, v).unwrap();
    }

    fn file(local: std::path::PathBuf, key: &str) -> crate::state::UploadFile {
        let size = std::fs::metadata(&local).unwrap().len();
        crate::state::UploadFile {
            local,
            key: key.into(),
            size,
            identity: None,
            compression: None,
        }
    }

    #[test]
    fn compression_estimate_counts_and_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        wav(&d.join("a.wav"), 20);
        wav(&d.join("b.wav"), 20);
        std::fs::write(d.join("a.mp3"), b"not really audio").unwrap();
        std::fs::write(d.join("photo.jpg"), vec![0u8; 100]).unwrap();
        let mut files = vec![
            file(d.join("a.wav"), "x/a.wav"),
            file(d.join("b.wav"), "x/b.wav"),
            file(d.join("a.mp3"), "x/a.mp3"),
            file(d.join("photo.jpg"), "x/photo.jpg"),
        ];
        let est = super::compression_estimate(
            &mut files,
            &r2dock_core::compression::CompressionPrefs::default(),
            None,
        )
        .unwrap();
        assert_eq!(est.files, 2);
        assert_eq!(est.unchanged, 0);
        assert!(est.bytes_after < est.bytes_before / 5);
        assert_eq!(est.key_change_count, 2);
        assert_eq!(est.key_changes[0].to, "x/a.mp3");
        assert_eq!(
            est.collisions,
            vec!["x/a.mp3".to_string()],
            "a.wav becomes a.mp3"
        );
        assert!(!est.tools_ready);
        assert_eq!(est.video_files, 0);

        let mut photos = vec![file(d.join("photo.jpg"), "photo.jpg")];
        assert!(
            super::compression_estimate(
                &mut photos,
                &r2dock_core::compression::CompressionPrefs::default(),
                None
            )
            .is_none(),
            "no audio, no estimate"
        );
    }

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
