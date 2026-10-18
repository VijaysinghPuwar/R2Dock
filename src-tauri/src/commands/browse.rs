use r2dock_core::AppError;

use r2dock_core::keys::validate_name;
use r2dock_r2::dto::{BucketEntry, ListPage, PrefixSize};
use tauri::State;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{CmdResult, bucket_arg, key_arg, logged, prefix_arg, with_client};
use crate::dto::{ObjectInfo, PublicLink, TextPreview};
use crate::media::MediaTarget;
use crate::state::AppState;

/// Text previews read at most this much.
pub const TEXT_PREVIEW_BYTES: u64 = 1024 * 1024;

#[tauri::command]
pub async fn bucket_list(
    state: State<'_, AppState>,
    connection_id: Uuid,
) -> CmdResult<Vec<BucketEntry>> {
    let act = state
        .activity
        .start_action("bucket_list")
        .connection(connection_id);
    let result = list_buckets_inner(&state, connection_id).await;
    let mut act = act;
    if let Ok(b) = &result {
        act.set("buckets", b.len());
    }
    act.end(&result);
    result
}

async fn list_buckets_inner(state: &AppState, connection_id: Uuid) -> CmdResult<Vec<BucketEntry>> {
    let cfg = state.connections.get(connection_id)?;
    let manual: Vec<BucketEntry> = cfg
        .manual_buckets
        .iter()
        .map(|b| BucketEntry {
            name: b.clone(),
            created: None,
        })
        .collect();
    if cfg.bucket_scoped {
        // Scoped keys cannot list buckets; use the ones the user added.
        state.connections.client(connection_id).await?;
        return Ok(manual);
    }
    let mut buckets = with_client(
        state,
        connection_id,
        |c| async move { c.list_buckets().await },
    )
    .await?;
    for m in manual {
        if !buckets.iter().any(|b| b.name == m.name) {
            buckets.push(m);
        }
    }
    buckets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(buckets)
}

#[tauri::command]
pub async fn object_list(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    prefix: String,
    token: Option<String>,
) -> CmdResult<ListPage> {
    let mut act = state
        .activity
        .start_action("folder_open")
        .connection(connection_id)
        .name("bucket", &bucket)
        .name("prefix", &prefix)
        .field("next_page", token.is_some());
    let result = async {
        let bucket = bucket_arg(&bucket)?;
        let prefix = prefix_arg(&prefix)?;
        with_client(&state, connection_id, |c| async move {
            c.list_page(&bucket, &prefix, token.as_deref(), None).await
        })
        .await
    }
    .await;
    if let Ok(page) = &result {
        act.set("folders", page.folders.len());
        act.set("files", page.objects.len());
        act.set("more", page.next_token.is_some());
    }
    act.end(&result);
    result
}

#[tauri::command]
pub async fn object_search(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    prefix: String,
    token: Option<String>,
) -> CmdResult<ListPage> {
    let act = state
        .activity
        .start_action("search_bucket")
        .connection(connection_id)
        .name("bucket", &bucket)
        .name("query", &prefix);
    logged(act, async {
        let bucket = bucket_arg(&bucket)?;
        if prefix.len() > r2dock_core::keys::MAX_KEY_BYTES {
            return Err(AppError::invalid_input("Search text is too long."));
        }
        with_client(&state, connection_id, |c| async move {
            c.search_prefix_page(&bucket, &prefix, token.as_deref())
                .await
        })
        .await
    })
    .await
}

#[tauri::command]
pub async fn object_head(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    key: String,
) -> CmdResult<ObjectInfo> {
    let act = state
        .activity
        .start_action("file_details")
        .connection(connection_id)
        .name("key", &key);
    logged(act, object_head_inner(&state, connection_id, bucket, key)).await
}

async fn object_head_inner(
    state: &AppState,
    connection_id: Uuid,
    bucket: String,
    key: String,
) -> CmdResult<ObjectInfo> {
    let bucket = bucket_arg(&bucket)?;
    let key = key_arg(&key)?;
    let details = with_client(state, connection_id, |c| async move {
        c.head_object(&bucket, &key).await
    })
    .await?;
    Ok(ObjectInfo { details })
}

/// The public link for an object: the bucket's manual public base URL, else
/// a custom domain or `r2.dev` domain found with the optional API token.
/// May call the Cloudflare API (cached briefly). The link itself is never
/// written to the activity log.
#[tauri::command]
pub async fn object_public_url(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    key: String,
) -> CmdResult<PublicLink> {
    let mut act = state
        .activity
        .start_action("public_link")
        .connection(connection_id)
        .name("key", &key);
    let result = async {
        let bucket = bucket_arg(&bucket)?;
        let key = key_arg(&key)?;
        state
            .public_links
            .resolve(&state.connections, connection_id, &bucket, &key)
            .await
    }
    .await;
    if let Ok(link) = &result {
        act.set("status", format!("{:?}", link.status));
        if let Some(source) = link.source {
            act.set("source", format!("{source:?}"));
        }
    }
    act.end(&result);
    result
}

#[tauri::command]
pub async fn prefix_size(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    prefix: String,
) -> CmdResult<PrefixSize> {
    let act = state
        .activity
        .start_action("size_calculate")
        .connection(connection_id)
        .name("prefix", &prefix);
    let bucket = bucket_arg(&bucket)?;
    let prefix = prefix_arg(&prefix)?;
    logged(
        act,
        with_client(&state, connection_id, |c| async move {
            let cancel = CancellationToken::new();
            let mut objects = 0u64;
            let mut bytes = 0u64;
            c.for_each_object(&bucket, &prefix, &cancel, |o| {
                objects += 1;
                bytes += o.size;
                true
            })
            .await?;
            Ok(PrefixSize {
                objects,
                bytes,
                calculated_at: chrono::Utc::now().to_rfc3339(),
            })
        }),
    )
    .await
}

#[tauri::command]
pub async fn folder_create(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    prefix: String,
    name: String,
) -> CmdResult<String> {
    let act = state
        .activity
        .start_action("folder_create")
        .connection(connection_id)
        .name("prefix", &prefix)
        .name("name", &name);
    logged(act, async {
        let bucket = bucket_arg(&bucket)?;
        let prefix = prefix_arg(&prefix)?;
        let name = name.trim().to_string();
        validate_name(&name).map_err(|e| AppError::invalid_input(e.to_string()))?;
        let key = format!("{prefix}{name}/");
        key_arg(&key)?;
        let k = key.clone();
        with_client(&state, connection_id, |c| async move {
            c.create_folder(&bucket, &k).await
        })
        .await?;
        Ok(key)
    })
    .await
}

/// Heuristic: text if there is no NUL byte and little invalid UTF-8.
fn looks_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    if sample.contains(&0) {
        return true;
    }
    let text = String::from_utf8_lossy(sample);
    let bad = text
        .chars()
        .filter(|c| *c == char::REPLACEMENT_CHARACTER)
        .count();
    // Allow a split character at the end of the sample.
    bad > 4
}

#[tauri::command]
pub async fn preview_text(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    key: String,
) -> CmdResult<TextPreview> {
    let act = state
        .activity
        .start_action("preview_text")
        .connection(connection_id)
        .name("key", &key);
    logged(act, async {
        let bucket = bucket_arg(&bucket)?;
        let key = key_arg(&key)?;
        let (bytes, truncated) = with_client(&state, connection_id, |c| async move {
            c.read_head_bytes(&bucket, &key, TEXT_PREVIEW_BYTES).await
        })
        .await?;
        let binary = looks_binary(&bytes);
        let text = if binary {
            String::new()
        } else {
            String::from_utf8_lossy(&bytes).into_owned()
        };
        Ok(TextPreview {
            text,
            truncated,
            binary,
        })
    })
    .await
}

#[tauri::command]
pub fn media_token(
    state: State<'_, AppState>,
    connection_id: Uuid,
    bucket: String,
    key: String,
) -> CmdResult<String> {
    let act = state
        .activity
        .start_action("preview_media")
        .connection(connection_id)
        .name("key", &key);
    let result = media_token_inner(&state, connection_id, bucket, key);
    act.end(&result);
    result
}

fn media_token_inner(
    state: &AppState,
    connection_id: Uuid,
    bucket: String,
    key: String,
) -> CmdResult<String> {
    let bucket = bucket_arg(&bucket)?;
    let key = key_arg(&key)?;
    state.connections.get(connection_id)?;
    Ok(state.media.issue(MediaTarget::R2 {
        connection: connection_id,
        bucket,
        key,
    }))
}

#[cfg(test)]
mod tests {
    use super::looks_binary;

    #[test]
    fn binary_detection() {
        assert!(!looks_binary(b"hello\nworld"));
        assert!(!looks_binary("caf\u{e9}".as_bytes()));
        assert!(looks_binary(&[0x89, b'P', b'N', b'G', 0, 0, 0]));
        assert!(looks_binary(&[0xff; 64]));
        assert!(!looks_binary(&[]));
    }
}
