use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeDelta, Utc};
use r2dock_core::AppError;
use r2dock_core::settings::{Preferences, SettingsFile};
use tauri::State;
use tauri_plugin_clipboard_manager::ClipboardExt;

use super::CmdResult;
use crate::activity::export;
use crate::dialogs;
use crate::dto::{ExportRange, LogKind, SettingsView};
use crate::state::AppState;

/// Held from changing the settings until they are on disk, so saves from
/// these commands run one at a time and an older snapshot never lands last.
static SAVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub fn view(state: &AppState) -> SettingsView {
    let preferences = state.settings.read().preferences.clone();
    SettingsView {
        preferences,
        download_dir: state
            .download_dir()
            .map(|p| state.display_path(&p))
            .unwrap_or_else(|| "Not set".to_string()),
        activity_dir: state.display_path(&state.activity_dir()),
        activity_dir_is_default: state.settings.read().activity_dir.is_none(),
        log_dir: state.display_path(&state.paths.logs),
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
    state.compression.apply(state);
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
    let _saving = SAVE_LOCK.lock().await;
    let old = std::mem::replace(&mut state.settings.write().preferences, new.clone());
    // Record which settings changed (names only, never values of paths).
    let act = state
        .activity
        .start_action("settings_set")
        .field("changed", changed_fields(&old, &new));
    let result = state.save_settings().await;
    if result.is_ok() {
        apply(&state, log.0.as_ref());
    } else {
        // Keep memory in step with the file on disk.
        state.settings.write().preferences = old;
    }
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
        let _saving = SAVE_LOCK.lock().await;
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
        let _saving = SAVE_LOCK.lock().await;
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

/// Open a log folder in the system file manager, creating it if needed.
pub fn open_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>, dir: &Path) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    std::fs::create_dir_all(dir)
        .map_err(|e| r2dock_core::app_error::io_error(&e, "open the log folder"))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|_| AppError::internal("Could not open the log folder."))
}

/// Open the activity log folder in the system file manager.
#[tauri::command]
pub fn open_activity_folder(app: tauri::AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    open_dir(&app, &state.activity_dir())
}

/// Return to the default activity log folder.
#[tauri::command]
pub async fn activity_reset_dir(
    state: State<'_, AppState>,
    log: State<'_, crate::LogHandle>,
) -> CmdResult<SettingsView> {
    let _saving = SAVE_LOCK.lock().await;
    state.settings.write().activity_dir = None;
    let result = state.save_settings().await;
    apply(&state, log.0.as_ref());
    state
        .activity
        .start_action("activity_folder_reset")
        .end(&result);
    result?;
    Ok(view(&state))
}

fn log_dir(state: &AppState, kind: LogKind) -> PathBuf {
    match kind {
        LogKind::Activity => state.activity_dir(),
        LogKind::Technical => state.paths.logs.clone(),
    }
}

/// Copy the full path of a log folder. The path is put on the clipboard by
/// Rust, so it never passes through the webview.
#[tauri::command]
pub fn log_copy_path(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    kind: LogKind,
) -> CmdResult<()> {
    let dir = log_dir(&state, kind);
    let result = app
        .clipboard()
        .write_text(dir.to_string_lossy().into_owned())
        .map_err(|_| AppError::internal("Could not copy to the clipboard."));
    state
        .activity
        .start_action("log_path_copied")
        .field("log", format!("{kind:?}"))
        .end(&result);
    result
}

/// Start of an export range, or `None` for everything.
fn range_start(range: ExportRange, now: DateTime<Local>) -> Option<DateTime<Utc>> {
    let days_ago = |days: i64| Some((now - TimeDelta::days(days)).with_timezone(&Utc));
    match range {
        ExportRange::Today => now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
            .map(|d| d.with_timezone(&Utc)),
        ExportRange::Last7Days => days_ago(7),
        ExportRange::Last30Days => days_ago(30),
        ExportRange::All => None,
    }
}

/// Whether `dest` would replace one of the daily activity files. Paths are
/// resolved first and names compared without regard to case, because
/// `Activity-2026-10-18.CSV` is the same file as `activity-2026-10-18.csv`
/// on Windows and macOS.
fn is_daily_file(dest: &Path, activity_dir: &Path) -> bool {
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    // An existing file resolves to its real (long, correctly cased) name.
    let dest = canonical(dest);
    let same_dir = dest.parent().is_some_and(|p| {
        let (a, b) = (canonical(p), canonical(activity_dir));
        a == b || a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    });
    // Windows ignores trailing dots and spaces in file names.
    let daily_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| export::file_date(n.trim_end_matches(['.', ' '])))
        .is_some();
    same_dir && daily_name
}

/// Merge the activity files into one CSV file. The destination comes from
/// the Rust save dialog, never from the webview. Returns the number of rows,
/// or `None` when the user cancels.
#[tauri::command]
pub async fn activity_export_csv(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    range: ExportRange,
) -> CmdResult<Option<u32>> {
    let now = Local::now();
    let file_name = format!("r2dock-activity-{}.csv", now.format("%Y-%m-%d"));
    let Some(dest) = dialogs::save_csv(&app, &file_name).await? else {
        return Ok(None);
    };
    let dir = state.activity_dir();
    if is_daily_file(&dest, &dir) {
        return Err(AppError::invalid_input(
            "Choose another name. This name is used by the daily activity files.",
        ));
    }
    let since = range_start(range, now);
    let mut act = state
        .activity
        .start_action("activity_export")
        .field("range", format!("{range:?}"));
    let result = tokio::task::spawn_blocking(move || export::export_csv(&dir, since, &dest))
        .await
        .map_err(|_| AppError::internal("A background task stopped unexpectedly."))
        .and_then(|r| {
            r.map_err(|e| r2dock_core::app_error::io_error(&e, "export the activity log"))
        })
        .map(|rows| u32::try_from(rows).unwrap_or(u32::MAX));
    if let Ok(rows) = &result {
        act.set("rows", *rows);
    }
    act.end(&result);
    result.map(Some)
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
    let _saving = SAVE_LOCK.lock().await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    #[test]
    fn export_ranges() {
        let now = Local.with_ymd_and_hms(2026, 10, 18, 15, 30, 0).unwrap();
        let today = range_start(ExportRange::Today, now).unwrap();
        let local = today.with_timezone(&Local);
        assert_eq!(local.date_naive(), now.date_naive());
        assert_eq!((local.hour(), local.minute()), (0, 0));
        assert_eq!(
            range_start(ExportRange::Last7Days, now).unwrap(),
            (now - TimeDelta::days(7)).with_timezone(&Utc)
        );
        assert_eq!(
            range_start(ExportRange::Last30Days, now).unwrap(),
            (now - TimeDelta::days(30)).with_timezone(&Utc)
        );
        assert_eq!(range_start(ExportRange::All, now), None);
    }

    #[test]
    fn refuses_to_replace_daily_files() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        assert!(is_daily_file(&d.join("activity-2026-10-18.csv"), d));
        assert!(is_daily_file(&d.join("activity-2026-10-18.jsonl"), d));
        assert!(!is_daily_file(&d.join("r2dock-activity-2026-10-18.csv"), d));
        // Case-only differences name the same file on Windows and macOS.
        assert!(is_daily_file(&d.join("Activity-2026-10-18.CSV"), d));
        assert!(is_daily_file(&d.join("ACTIVITY-2026-10-18.JSONL"), d));
        assert!(is_daily_file(&d.join("activity-2026-10-18.csv."), d));
        assert!(is_daily_file(
            &d.join("activity-2026-10-18-ab12cd34.csv"),
            d
        ));
        // An existing daily file reached through a differently cased name.
        std::fs::write(d.join("activity-2026-10-17.csv"), "x").unwrap();
        assert!(is_daily_file(&d.join("ACTIVITY-2026-10-17.Csv"), d));
        let other = tempfile::tempdir().unwrap();
        assert!(!is_daily_file(
            &other.path().join("activity-2026-10-18.csv"),
            d
        ));
    }
}
