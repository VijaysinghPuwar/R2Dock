use r2dock_core::AppError;
use r2dock_core::settings::{Preferences, SettingsFile};
use tauri::State;

use super::CmdResult;
use crate::dialogs;
use crate::dto::SettingsView;
use crate::state::AppState;

pub fn view(state: &AppState) -> SettingsView {
    let preferences = state.settings.read().preferences.clone();
    SettingsView {
        preferences,
        download_dir: state
            .download_dir()
            .map(|p| state.display_path(&p))
            .unwrap_or_else(|| "Not set".to_string()),
        activity_dir: state.display_path(&state.activity_dir()),
        credential_store: state.credential_store_label.clone(),
        credential_store_available: state.connections.keychain_available(),
    }
}

/// Push settings that affect running services.
pub fn apply(state: &AppState, log: Option<&crate::logging::ReloadHandle>) {
    let p = state.settings.read().preferences.clone();
    state.engine.set_concurrency(usize::from(p.concurrency));
    state
        .engine
        .set_multipart_threshold(u64::from(p.multipart_threshold_mib) * r2dock_core::parts::MIB);
    if let Some(handle) = log {
        let _ = handle.reload(crate::logging::filter_for(p.log_level));
    }
    state.activity.configure(
        p.activity_log,
        p.activity_include_names,
        state.activity_dir(),
    );
}

/// Names of the preferences that differ, for the activity log.
fn changed_fields(old: &Preferences, new: &Preferences) -> Vec<String> {
    let (Ok(serde_json::Value::Object(a)), Ok(serde_json::Value::Object(b))) =
        (serde_json::to_value(old), serde_json::to_value(new))
    else {
        return Vec::new();
    };
    b.iter()
        .filter(|(k, v)| a.get(*k) != Some(*v))
        .map(|(k, _)| k.clone())
        .collect()
}

#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> SettingsView {
    view(&state)
}

#[tauri::command]
pub async fn settings_set(
    state: State<'_, AppState>,
    log: State<'_, crate::LogHandle>,
    preferences: Preferences,
) -> CmdResult<SettingsView> {
    let new = preferences.sanitized();
    let old = std::mem::replace(&mut state.settings.write().preferences, new.clone());
    // Record which settings changed (names only, never values of paths).
    let act = state
        .activity
        .start_action("settings_set")
        .field("changed", changed_fields(&old, &new));
    let result = state.save_settings().await;
    apply(&state, log.0.as_ref());
    act.end(&result);
    result?;
    Ok(view(&state))
}

#[tauri::command]
pub async fn settings_choose_download_dir(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<SettingsView> {
    if let Some(dir) = dialogs::pick_folder(&app, state.download_dir()).await? {
        if !dir.is_dir() {
            return Err(AppError::invalid_input("Choose an existing folder."));
        }
        state.settings.write().download_dir = Some(dir);
        let result = state.save_settings().await;
        state
            .activity
            .start_action("download_folder_changed")
            .end(&result);
        result?;
    }
    Ok(view(&state))
}

/// Choose the folder where activity logs are written.
#[tauri::command]
pub async fn settings_choose_activity_dir(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    log: State<'_, crate::LogHandle>,
) -> CmdResult<SettingsView> {
    if let Some(dir) = dialogs::pick_folder(&app, Some(state.activity_dir())).await? {
        if !dir.is_dir() {
            return Err(AppError::invalid_input("Choose an existing folder."));
        }
        state.settings.write().activity_dir = Some(dir);
        let result = state.save_settings().await;
        apply(&state, log.0.as_ref());
        state
            .activity
            .start_action("activity_folder_changed")
            .end(&result);
        result?;
    }
    Ok(view(&state))
}

/// Open the activity log folder in the system file manager.
#[tauri::command]
pub fn open_activity_folder(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let dir = state.activity_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| r2dock_core::app_error::io_error(&e, "open the activity folder"))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|_| AppError::internal("Could not open the activity folder."))
}

/// Record a problem seen by the interface (for example an unexpected error),
/// so it appears in the activity log next to the backend events.
#[tauri::command]
pub fn activity_note(state: State<'_, AppState>, kind: String, message: String) {
    let kind: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(40)
        .collect();
    let message: String = message
        .chars()
        .filter(|c| !c.is_control())
        .take(500)
        .collect();
    let mut fields = serde_json::Map::new();
    fields.insert("kind".into(), serde_json::Value::String(kind));
    fields.insert("message".into(), serde_json::Value::String(message));
    state.activity.event("ui_problem", fields);
}

#[tauri::command]
pub async fn settings_reset(
    state: State<'_, AppState>,
    log: State<'_, crate::LogHandle>,
) -> CmdResult<SettingsView> {
    *state.settings.write() = SettingsFile::default();
    let result = state.save_settings().await;
    apply(&state, log.0.as_ref());
    state.activity.start_action("settings_reset").end(&result);
    result?;
    Ok(view(&state))
}

#[tauri::command]
pub async fn history_clear(state: State<'_, AppState>) -> CmdResult<()> {
    let engine = state.engine.clone();
    let result = tokio::task::spawn_blocking(move || engine.clear_history())
        .await
        .map_err(|_| AppError::internal("A background task stopped unexpectedly."));
    state.activity.start_action("history_cleared").end(&result);
    result
}
