use tauri::State;
use uuid::Uuid;

use super::{CmdResult, logged};
use crate::dto::{ConnectionInput, ConnectionSummary, TestResult};
use crate::state::AppState;

#[tauri::command]
pub fn connection_list(state: State<'_, AppState>) -> Vec<ConnectionSummary> {
    state.connections.list()
}

/// Bucket names typed in the Connect form, for the activity log count.
fn bucket_count(input: &ConnectionInput) -> usize {
    input
        .bucket
        .as_deref()
        .unwrap_or_default()
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|b| !b.is_empty())
        .count()
}

/// The secret in `input` is used for this call only and never returned or logged.
#[tauri::command]
pub async fn connection_test(
    state: State<'_, AppState>,
    input: ConnectionInput,
) -> CmdResult<TestResult> {
    let act = state
        .activity
        .start_action("connection_test")
        .field("editing", input.id.is_some())
        .field("buckets_entered", bucket_count(&input))
        .field("jurisdiction", format!("{:?}", input.jurisdiction));
    logged(act, state.connections.test(&input)).await
}

#[tauri::command]
pub async fn connection_save(
    state: State<'_, AppState>,
    input: ConnectionInput,
) -> CmdResult<ConnectionSummary> {
    let act = state
        .activity
        .start_action("connection_save")
        .field("editing", input.id.is_some())
        .field(
            "new_secret",
            input.secret.as_deref().is_some_and(|s| !s.is_empty()),
        )
        .field("buckets_entered", bucket_count(&input))
        .field(
            "new_api_token",
            input
                .api_token
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty()),
        )
        .field("remove_api_token", input.remove_api_token);
    logged(act, state.connections.save(&input)).await
}

#[tauri::command]
pub async fn connection_remove(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    let act = state
        .activity
        .start_action("connection_remove")
        .connection(id);
    logged(act, async {
        state.connections.remove(id).await?;
        state
            .engine
            .fail_connection(id, "This connection was removed.");
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn connection_set_active(state: State<'_, AppState>, id: Uuid) -> CmdResult<()> {
    let act = state
        .activity
        .start_action("connection_switch")
        .connection(id);
    logged(act, state.connections.set_active(id)).await
}

#[tauri::command]
pub async fn connection_add_bucket(
    state: State<'_, AppState>,
    id: Uuid,
    bucket: String,
) -> CmdResult<ConnectionSummary> {
    let act = state
        .activity
        .start_action("bucket_add")
        .connection(id)
        .name("bucket", &bucket);
    logged(act, state.connections.add_bucket(id, &bucket)).await
}

#[tauri::command]
pub async fn connection_remove_bucket(
    state: State<'_, AppState>,
    id: Uuid,
    bucket: String,
) -> CmdResult<ConnectionSummary> {
    let act = state
        .activity
        .start_action("bucket_remove")
        .connection(id)
        .name("bucket", &bucket);
    logged(act, state.connections.remove_bucket(id, &bucket)).await
}

#[tauri::command]
pub async fn connection_set_public_url(
    state: State<'_, AppState>,
    id: Uuid,
    bucket: String,
    url: Option<String>,
) -> CmdResult<ConnectionSummary> {
    let act = state
        .activity
        .start_action("public_url_set")
        .connection(id)
        .name("bucket", &bucket)
        .field(
            "cleared",
            url.as_deref().is_none_or(|u| u.trim().is_empty()),
        );
    logged(
        act,
        state
            .connections
            .set_public_url(id, &bucket, url.as_deref()),
    )
    .await
}

#[tauri::command]
pub async fn connection_toggle_favorite(
    state: State<'_, AppState>,
    id: Uuid,
    bucket: String,
) -> CmdResult<ConnectionSummary> {
    let act = state
        .activity
        .start_action("favorite_toggle")
        .connection(id)
        .name("bucket", &bucket);
    logged(act, state.connections.toggle_favorite(id, &bucket)).await
}
