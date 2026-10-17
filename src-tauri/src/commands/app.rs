use tauri::State;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use super::CmdResult;
use crate::dto::{AppInfo, Platform};
use crate::state::AppState;
use r2dock_core::AppError;

const MAX_CLIPBOARD: usize = 1024 * 1024;

pub fn platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else if cfg!(target_os = "windows") {
        Platform::Windows
    } else {
        Platform::Linux
    }
}

#[tauri::command]
pub fn app_info(app: tauri::AppHandle, state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: r2dock_core::APP_NAME.to_string(),
        version: app.package_info().version.to_string(),
        platform: platform(),
        credential_store: state.credential_store_label.clone(),
        credential_store_available: state.connections.keychain_available(),
    }
}

#[tauri::command]
pub fn clipboard_write(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> CmdResult<()> {
    // The copied text itself is never recorded.
    let act = state
        .activity
        .start_action("copy_to_clipboard")
        .field("length", text.len());
    let result = clipboard_write_inner(&app, text);
    act.end(&result);
    result
}

fn clipboard_write_inner(app: &tauri::AppHandle, text: String) -> CmdResult<()> {
    if text.len() > MAX_CLIPBOARD {
        return Err(AppError::invalid_input("Text is too long to copy."));
    }
    app.clipboard()
        .write_text(text)
        .map_err(|_| AppError::internal("Could not copy to the clipboard."))
}

/// Open an https link in the default browser. Other schemes are refused.
#[tauri::command]
pub fn open_external(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> CmdResult<()> {
    // Only the host is recorded, never the path or query.
    let host = url
        .strip_prefix("https://")
        .and_then(|r| r.split('/').next())
        .unwrap_or("")
        .to_string();
    let act = state.activity.start_action("open_link").field("host", host);
    let result = open_external_inner(&app, url);
    act.end(&result);
    result
}

fn open_external_inner(app: &tauri::AppHandle, url: String) -> CmdResult<()> {
    let ok = url.starts_with("https://")
        && url.len() < 4096
        && !url.chars().any(|c| c.is_control() || c.is_whitespace());
    if !ok {
        return Err(AppError::invalid_input("Only https links can be opened."));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| AppError::internal("Could not open the link."))
}

#[tauri::command]
pub fn open_log_folder(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let _ = std::fs::create_dir_all(&state.paths.logs);
    app.opener()
        .open_path(state.paths.logs.to_string_lossy(), None::<&str>)
        .map_err(|_| AppError::internal("Could not open the log folder."))
}
