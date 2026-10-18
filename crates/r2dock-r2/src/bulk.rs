//! Bulk delete and move (PLAN.md 8.6, 8.7).
//!
//! R2 has no rename, move, or atomic folder operations. A move is copy,
//! verify, then delete, one object at a time with bounded concurrency, and
//! every partial failure is reported.

use std::collections::BTreeMap;

use futures_util::StreamExt;
use r2dock_core::keys::{base_name, validate_key, validate_name, validate_prefix};
use tokio_util::sync::CancellationToken;

use crate::client::R2Client;
use crate::dto::KeyFailure;
use crate::error::R2Error;
use crate::ops::{CopySpec, DELETE_BATCH, same_etag};

/// Parallel requests for bulk work.
pub const BULK_CONCURRENCY: usize = 4;

/// Refuse to plan more than this many objects in one operation.
pub const MAX_BULK_OBJECTS: usize = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectRef {
    pub key: String,
    pub size: u64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PlanError {
    #[error("Too many objects for one operation.")]
    TooMany,
    #[error("A folder cannot be moved into itself.")]
    IntoItself,
    #[error("Rename needs exactly one item.")]
    RenameNeedsOne,
    #[error("{0}")]
    InvalidName(String),
    #[error("Nothing to do.")]
    Empty,
    #[error("Two selected items would end up with the same name: {0}")]
    DuplicateDestination(String),
}

/// Every object addressed by `keys` plus everything under `prefixes`
/// (including folder markers). Keys are unique and sorted.
pub async fn collect_objects(
    client: &R2Client,
    bucket: &str,
    keys: &[String],
    prefixes: &[String],
    cancel: &CancellationToken,
) -> Result<Vec<ObjectRef>, R2Error> {
    let mut found: BTreeMap<String, u64> = BTreeMap::new();
    for key in keys {
        let size = match client.head_object(bucket, key).await {
            Ok(d) => d.size,
            Err(R2Error::NoSuchKey { .. }) => continue,
            Err(e) => return Err(e),
        };
        found.insert(key.clone(), size);
    }
    for prefix in prefixes {
        let mut too_many = false;
        client
            .for_each_object(bucket, prefix, cancel, |o| {
                found.insert(o.key, o.size);
                too_many = found.len() > MAX_BULK_OBJECTS;
                !too_many
            })
            .await?;
        if too_many {
            return Err(R2Error::Protocol {
                detail: "too many objects".to_string(),
            });
        }
    }
    Ok(found
        .into_iter()
        .map(|(key, size)| ObjectRef { key, size })
        .collect())
}

/// Delete keys in batches of up to 1,000, a few batches at a time.
/// `progress` receives the number of keys processed so far.
pub async fn delete_keys(
    client: &R2Client,
    bucket: &str,
    keys: Vec<String>,
    cancel: &CancellationToken,
    progress: impl Fn(u64),
) -> Result<(u64, Vec<KeyFailure>), R2Error> {
    let batches: Vec<Vec<String>> = keys.chunks(DELETE_BATCH).map(<[String]>::to_vec).collect();
    let futures: Vec<_> = batches
        .into_iter()
        .map(|batch| {
            Box::pin(async move {
                let n = batch.len() as u64;
                let result = client.delete_batch(bucket, &batch).await;
                (n, batch, result)
            })
        })
        .collect();
    let mut stream = futures_util::stream::iter(futures).buffer_unordered(BULK_CONCURRENCY);

    let mut processed = 0u64;
    let mut deleted = 0u64;
    let mut failures = Vec::new();
    loop {
        let next = tokio::select! {
            () = cancel.cancelled() => return Err(R2Error::Cancelled),
            n = stream.next() => n,
        };
        let Some((n, batch, result)) = next else {
            break;
        };
        processed += n;
        match result {
            Ok(failed) => {
                deleted += n.saturating_sub(failed.len() as u64);
                failures.extend(failed);
            }
            Err(e) => {
                let message = e.to_app_error().message;
                failures.extend(batch.into_iter().map(|key| KeyFailure {
                    key,
                    message: message.clone(),
                }));
            }
        }
        progress(processed);
    }
    Ok((deleted, failures))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveItem {
    pub src: String,
    pub dst: String,
    pub size: u64,
}

/// Plan where each object goes.
///
/// * `keys` are files moved into `dest_prefix` keeping their names.
/// * `prefixes` are folders moved into `dest_prefix` keeping their structure.
/// * `rename` renames the single selected item in place of moving it.
pub fn plan_move(
    objects: &[ObjectRef],
    keys: &[String],
    prefixes: &[String],
    dest_prefix: &str,
    rename: Option<&str>,
) -> Result<Vec<MoveItem>, PlanError> {
    validate_prefix(dest_prefix).map_err(|e| PlanError::InvalidName(e.to_string()))?;
    if let Some(name) = rename {
        validate_name(name).map_err(|e| PlanError::InvalidName(e.to_string()))?;
        if keys.len() + prefixes.len() != 1 {
            return Err(PlanError::RenameNeedsOne);
        }
    }
    if objects.len() > MAX_BULK_OBJECTS {
        return Err(PlanError::TooMany);
    }
    let mut items = Vec::new();
    for key in keys {
        let Some(obj) = objects.iter().find(|o| &o.key == key) else {
            continue;
        };
        let name = rename.unwrap_or_else(|| base_name(key));
        let dst = format!("{dest_prefix}{name}");
        validate_key(&dst).map_err(|e| PlanError::InvalidName(e.to_string()))?;
        if dst != *key {
            items.push(MoveItem {
                src: key.clone(),
                dst,
                size: obj.size,
            });
        }
    }
    for prefix in prefixes {
        let name = rename.unwrap_or_else(|| base_name(prefix));
        let new_prefix = format!("{dest_prefix}{name}/");
        if new_prefix.starts_with(prefix.as_str()) && new_prefix != *prefix {
            return Err(PlanError::IntoItself);
        }
        if new_prefix == *prefix {
            continue;
        }
        for obj in objects
            .iter()
            .filter(|o| o.key.starts_with(prefix.as_str()))
        {
            let dst = format!("{new_prefix}{}", &obj.key[prefix.len()..]);
            validate_key(&dst).map_err(|e| PlanError::InvalidName(e.to_string()))?;
            items.push(MoveItem {
                src: obj.key.clone(),
                dst,
                size: obj.size,
            });
        }
    }
    let mut seen = std::collections::HashSet::new();
    for item in &items {
        if !seen.insert(item.dst.as_str()) {
            return Err(PlanError::DuplicateDestination(item.dst.clone()));
        }
    }
    Ok(items)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveReport {
    pub moved: u64,
    /// Not copied, the destination already existed, or the destination
    /// changed right after copying: the source is kept.
    pub failures: Vec<KeyFailure>,
    /// Copied, but the original was kept: it could not be removed, it
    /// changed during the move, or the copy could not be confirmed.
    pub copied_not_deleted: Vec<KeyFailure>,
}

enum ItemResult {
    Moved,
    Failed(KeyFailure),
    CopiedOnly(KeyFailure),
}

const DEST_EXISTS: &str = "Something with that name already exists at the destination.";

/// Move one object: copy exactly the inspected source version, confirm the
/// destination is that copy, confirm the source is still that version, and
/// only then delete it.
///
/// R2 has no conditional DeleteObject, so a source changed between the last
/// check and the delete (one request) can still be removed. Every other
/// conflicting change keeps the source (PLAN.md 8.7).
async fn move_one(client: &R2Client, bucket: &str, item: &MoveItem, overwrite: bool) -> ItemResult {
    let fail = |message: &str| {
        ItemResult::Failed(KeyFailure {
            key: item.src.clone(),
            message: message.to_string(),
        })
    };
    let kept = |message: &str| {
        ItemResult::CopiedOnly(KeyFailure {
            key: item.src.clone(),
            message: message.to_string(),
        })
    };
    if !overwrite {
        // Early, friendly check. The copy itself is also conditional.
        match client.exists(bucket, &item.dst).await {
            Ok(true) => return fail(DEST_EXISTS),
            Ok(false) => {}
            Err(e) => return fail(&e.to_app_error().message),
        }
    }
    // Inspect the source now and copy exactly this version.
    let source = match client.head_object(bucket, &item.src).await {
        Ok(d) => d,
        Err(e) => return fail(&e.to_app_error().message),
    };
    let Some(source_etag) = source.etag.as_deref() else {
        return fail("The file could not be identified, so it was left in place.");
    };
    let spec = CopySpec {
        src_key: &item.src,
        dst_key: &item.dst,
        size: source.size,
        source_etag: Some(source_etag),
        create_only: !overwrite,
    };
    let copy_etag = match client.copy_object(bucket, &spec).await {
        Ok(etag) => etag,
        Err(R2Error::PreconditionFailed { .. }) => {
            // Either the source changed or the destination appeared.
            return match client.exists(bucket, &item.dst).await {
                Ok(true) if !overwrite => fail(DEST_EXISTS),
                _ => fail("The file changed while moving, so it was left in place."),
            };
        }
        Err(e) => return fail(&e.to_app_error().message),
    };
    // The destination must be the object this copy created, not just an
    // object of the same size.
    let Some(copy_etag) = copy_etag else {
        return kept("Copied, but the copy could not be confirmed, so the original was kept.");
    };
    match client.head_object(bucket, &item.dst).await {
        Ok(d)
            if d.size == source.size
                && d.etag.as_deref().is_some_and(|e| same_etag(e, &copy_etag)) => {}
        Ok(_) => {
            return fail("The destination changed right after copying, so the original was kept.");
        }
        Err(R2Error::NoSuchKey { .. }) => {
            return fail("The copy disappeared right after copying, so the original was kept.");
        }
        Err(e) => {
            return kept(&format!(
                "Copied, but the copy could not be checked, so the original was kept. {}",
                e.to_app_error().message
            ));
        }
    }
    // The source must still be the version that was copied.
    match client.head_object(bucket, &item.src).await {
        Ok(d)
            if d.size == source.size
                && d.etag.as_deref().is_some_and(|e| same_etag(e, source_etag)) => {}
        Ok(_) => {
            return kept("Copied, but the original changed during the move, so it was kept.");
        }
        // Someone else already removed it; the verified copy is in place.
        Err(R2Error::NoSuchKey { .. }) => return ItemResult::Moved,
        Err(e) => {
            return kept(&format!(
                "Copied, but the original could not be checked, so it was kept. {}",
                e.to_app_error().message
            ));
        }
    }
    match client.delete_object(bucket, &item.src).await {
        Ok(()) => ItemResult::Moved,
        Err(e) => kept(&format!(
            "Copied, but the original was not removed. {}",
            e.to_app_error().message
        )),
    }
}

/// Run a move plan with bounded concurrency. `progress` receives items processed.
pub async fn move_items(
    client: &R2Client,
    bucket: &str,
    items: Vec<MoveItem>,
    overwrite: bool,
    cancel: &CancellationToken,
    progress: impl Fn(u64),
) -> Result<MoveReport, R2Error> {
    let futures: Vec<_> = items
        .iter()
        .map(|item| Box::pin(move_one(client, bucket, item, overwrite)))
        .collect();
    let mut stream = futures_util::stream::iter(futures).buffer_unordered(BULK_CONCURRENCY);
    let mut report = MoveReport::default();
    let mut processed = 0u64;
    loop {
        let next = tokio::select! {
            () = cancel.cancelled() => return Err(R2Error::Cancelled),
            n = stream.next() => n,
        };
        let Some(result) = next else { break };
        processed += 1;
        match result {
            ItemResult::Moved => report.moved += 1,
            ItemResult::Failed(f) => report.failures.push(f),
            ItemResult::CopiedOnly(f) => report.copied_not_deleted.push(f),
        }
        progress(processed);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objs(keys: &[&str]) -> Vec<ObjectRef> {
        keys.iter()
            .map(|k| ObjectRef {
                key: (*k).to_string(),
                size: 1,
            })
            .collect()
    }

    #[test]
    fn plans_file_and_folder_moves() {
        let o = objs(&["a/x.txt", "a/sub/", "a/sub/y.txt", "b.txt"]);
        let items = plan_move(&o, &["b.txt".into()], &["a/".into()], "dest/", None).unwrap();
        let pairs: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.src.as_str(), i.dst.as_str()))
            .collect();
        assert!(pairs.contains(&("b.txt", "dest/b.txt")));
        assert!(pairs.contains(&("a/x.txt", "dest/a/x.txt")));
        assert!(pairs.contains(&("a/sub/", "dest/a/sub/")));
        assert!(pairs.contains(&("a/sub/y.txt", "dest/a/sub/y.txt")));
    }

    #[test]
    fn plans_renames() {
        let o = objs(&["docs/old.txt"]);
        let items = plan_move(&o, &["docs/old.txt".into()], &[], "docs/", Some("new.txt")).unwrap();
        assert_eq!(items[0].dst, "docs/new.txt");
        let o = objs(&["docs/", "docs/a.txt"]);
        let items = plan_move(&o, &[], &["docs/".into()], "", Some("papers")).unwrap();
        assert_eq!(items.len(), 2);
        assert!(items.iter().any(|i| i.dst == "papers/a.txt"));
        assert_eq!(
            plan_move(&o, &[], &["docs/".into()], "", Some("a/b")),
            Err(PlanError::InvalidName(
                "Folder names cannot contain \"/\".".into()
            ))
        );
    }

    #[test]
    fn refuses_moving_into_itself() {
        let o = objs(&["a/x.txt"]);
        assert_eq!(
            plan_move(&o, &[], &["a/".into()], "a/", None),
            Err(PlanError::IntoItself)
        );
        assert_eq!(
            plan_move(&o, &[], &["a/".into()], "a/deeper/", None),
            Err(PlanError::IntoItself)
        );
    }

    #[test]
    fn same_place_is_a_no_op() {
        let o = objs(&["a/x.txt"]);
        assert!(
            plan_move(&o, &["a/x.txt".into()], &[], "a/", None)
                .unwrap()
                .is_empty()
        );
        assert!(
            plan_move(&o, &[], &["a/".into()], "", None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn rejects_colliding_destinations() {
        let o = objs(&["a/x.txt", "b/x.txt"]);
        assert_eq!(
            plan_move(
                &o,
                &["a/x.txt".into(), "b/x.txt".into()],
                &[],
                "dest/",
                None
            ),
            Err(PlanError::DuplicateDestination("dest/x.txt".into()))
        );
    }

    #[test]
    fn rename_needs_one_item() {
        let o = objs(&["a", "b"]);
        assert_eq!(
            plan_move(&o, &["a".into(), "b".into()], &[], "", Some("c")),
            Err(PlanError::RenameNeedsOne)
        );
    }
}
