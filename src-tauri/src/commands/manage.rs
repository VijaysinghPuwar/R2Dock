//! Delete and move with backend confirmation tokens (PLAN.md 7.1).
//!
//! A destructive operation needs a token from its preview call. The token
//! names the exact keys counted in the preview, expires, and works once.
//! Large or folder deletes also need the typed confirmation phrase.

use r2dock_core::keys::{base_name, validate_name};
use r2dock_core::{AppError, ErrorCode};
use r2dock_r2::bulk::{collect_objects, delete_keys, move_items, plan_move};
use tauri::State;
use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{CmdResult, bucket_arg, key_arg, prefix_arg, with_client};
use crate::dto::{DeletePreview, DeleteResult, MovePreview, MoveResult, OpProgress};
use crate::state::{AppState, PendingDelete, PendingMove};

fn check_sources(keys: &[String], prefixes: &[String]) -> CmdResult<()> {
    if keys.is_empty() && prefixes.is_empty() {
        return Err(AppError::invalid_input("Nothing is selected."));
    }
    for k in keys {
        key_arg(k)?;
    }
    for p in prefixes {
        prefix_arg(p)?;
        if p.is_empty() {
            return Err(AppError::invalid_input(
                "A whole bucket cannot be selected this way.",
            ));
        }
    }
    Ok(())
}

/// The phrase a user types to confirm a delete.
pub fn typed_phrase(keys: &[String], prefixes: &[String], count: u64) -> String {
    if keys.is_empty() && prefixes.len() == 1 {
        base_name(&prefixes[0]).to_string()
    } else {
        format!("delete {count}")
    }
}

#[tauri::command]
pub async fn delete_preview(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    keys: Vec<String>,
    prefixes: Vec<String>,
) -> CmdResult<DeletePreview> {
    let bucket = bucket_arg(&bucket)?;
    check_sources(&keys, &prefixes)?;
    let (b, k, p) = (bucket.clone(), keys.clone(), prefixes.clone());
    let objects = with_client(&state, connection_id, |c| async move {
        collect_objects(&c, &b, &k, &p, &CancellationToken::new()).await
    })
    .await?;
    let count = objects.len() as u64;
    let bytes = objects.iter().map(|o| o.size).sum();
    let threshold = u64::from(state.settings.read().preferences.typed_confirm_threshold);
    let needs_typed = !prefixes.is_empty() || count > threshold;
    let phrase = typed_phrase(&keys, &prefixes, count);
    let sample = objects.iter().take(5).map(|o| o.key.clone()).collect();
    let token = state.deletes.insert(PendingDelete {
        connection: connection_id,
        bucket,
        keys: objects.into_iter().map(|o| o.key).collect(),
        needs_typed,
        typed_phrase: phrase.clone(),
    });
    Ok(DeletePreview {
        token,
        objects: count,
        bytes,
        needs_typed,
        typed_phrase: phrase,
        sample,
    })
}

#[tauri::command]
pub async fn delete_execute(
    state: State<'_, AppState>,
    token: String,
    typed: Option<String>,
    on_progress: Channel<OpProgress>,
) -> CmdResult<DeleteResult> {
    let pending = state.deletes.take(&token).ok_or_else(|| {
        AppError::new(
            ErrorCode::ConfirmationRequired,
            "This confirmation expired. Start the delete again.",
        )
    })?;
    if pending.needs_typed && typed.as_deref() != Some(pending.typed_phrase.as_str()) {
        return Err(AppError::new(
            ErrorCode::ConfirmationRequired,
            "The confirmation text did not match, so nothing was deleted.",
        ));
    }
    let total = pending.keys.len() as u64;
    let keys = pending.keys;
    let bucket = pending.bucket;
    let (deleted, failures) = with_client(&state, pending.connection, |c| async move {
        delete_keys(&c, &bucket, keys, &CancellationToken::new(), |done| {
            let _ = on_progress.send(OpProgress { done, total });
        })
        .await
    })
    .await?;
    tracing::info!("deleted {deleted} objects, {} failed", failures.len());
    Ok(DeleteResult { deleted, failures })
}

#[tauri::command]
pub async fn move_preview(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    keys: Vec<String>,
    prefixes: Vec<String>,
    dest_prefix: String,
    rename: Option<String>,
) -> CmdResult<MovePreview> {
    let bucket = bucket_arg(&bucket)?;
    check_sources(&keys, &prefixes)?;
    let dest_prefix = prefix_arg(&dest_prefix)?;
    let rename = rename.map(|r| r.trim().to_string());
    if let Some(r) = &rename {
        validate_name(r).map_err(|e| AppError::invalid_input(e.to_string()))?;
    }
    let (b, k, p) = (bucket.clone(), keys.clone(), prefixes.clone());
    let objects = with_client(&state, connection_id, |c| async move {
        collect_objects(&c, &b, &k, &p, &CancellationToken::new()).await
    })
    .await?;
    let items = plan_move(&objects, &keys, &prefixes, &dest_prefix, rename.as_deref())
        .map_err(|e| AppError::invalid_input(e.to_string()))?;
    if items.is_empty() {
        return Err(AppError::invalid_input(
            "Nothing to move: the items are already there.",
        ));
    }
    let count = items.len() as u64;
    let bytes = items.iter().map(|i| i.size).sum();
    let token = state.moves.insert(PendingMove {
        connection: connection_id,
        bucket: bucket.clone(),
        items,
    });
    Ok(MovePreview {
        token,
        objects: count,
        bytes,
        destination: if dest_prefix.is_empty() {
            format!("{bucket} (top level)")
        } else {
            dest_prefix
        },
    })
}

#[tauri::command]
pub async fn move_execute(
    state: State<'_, AppState>,
    token: String,
    on_progress: Channel<OpProgress>,
) -> CmdResult<MoveResult> {
    let pending = state.moves.take(&token).ok_or_else(|| {
        AppError::new(
            ErrorCode::ConfirmationRequired,
            "This confirmation expired. Start the move again.",
        )
    })?;
    let total = pending.items.len() as u64;
    let bucket = pending.bucket;
    let items = pending.items;
    let report = with_client(&state, pending.connection, |c| async move {
        move_items(
            &c,
            &bucket,
            items,
            false,
            &CancellationToken::new(),
            |done| {
                let _ = on_progress.send(OpProgress { done, total });
            },
        )
        .await
    })
    .await?;
    Ok(MoveResult {
        moved: report.moved,
        failures: report.failures,
        copied_not_deleted: report.copied_not_deleted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrases() {
        assert_eq!(typed_phrase(&[], &["a/ch 1-195/".into()], 195), "ch 1-195");
        assert_eq!(typed_phrase(&["x".into()], &["a/".into()], 7), "delete 7");
        assert_eq!(
            typed_phrase(&["x".into(), "y".into()], &[], 60),
            "delete 60"
        );
    }

    #[test]
    fn sources_are_validated() {
        assert!(check_sources(&[], &[]).is_err());
        assert!(check_sources(&[], &[String::new()]).is_err());
        assert!(check_sources(&[], &["no-slash".into()]).is_err());
        assert!(check_sources(&["a\nb".into()], &[]).is_err());
        assert!(check_sources(&["ok.txt".into()], &["dir/".into()]).is_ok());
    }
}
