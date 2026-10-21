//! Native file pickers, run from Rust so local paths never come from the webview.

use std::path::PathBuf;

use r2dock_core::AppError;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, FileDialogBuilder};

fn closed() -> AppError {
    AppError::internal("The file picker closed unexpectedly.")
}

/// A file dialog owned by the main window, so it is modal: the app cannot
/// be used (and start a conflicting action) while a picker is open.
fn file_dialog(app: &AppHandle) -> FileDialogBuilder<tauri::Wry> {
    let builder = app.dialog().file();
    match app.get_webview_window("main") {
        Some(window) => builder.set_parent(&window),
        None => builder,
    }
}

pub async fn pick_files(app: &AppHandle) -> Result<Option<Vec<PathBuf>>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    file_dialog(app)
        .set_title("Upload files")
        .pick_files(move |paths| {
            let _ = tx.send(paths);
        });
    let picked = rx.await.map_err(|_| closed())?;
    Ok(picked.map(|v| v.into_iter().filter_map(|p| p.into_path().ok()).collect()))
}

pub async fn pick_upload_folder(app: &AppHandle) -> Result<Option<PathBuf>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    file_dialog(app)
        .set_title("Upload folder")
        .pick_folder(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx.await.map_err(|_| closed())?;
    Ok(picked.and_then(|p| p.into_path().ok()))
}

pub async fn pick_folder(
    app: &AppHandle,
    start: Option<PathBuf>,
) -> Result<Option<PathBuf>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut builder = file_dialog(app).set_title("Choose a folder");
    if let Some(dir) = start {
        builder = builder.set_directory(dir);
    }
    builder.pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let picked = rx.await.map_err(|_| closed())?;
    Ok(picked.and_then(|p| p.into_path().ok()))
}

/// Ask where to save a CSV file. Returns the chosen path with a `.csv`
/// extension.
pub async fn save_csv(app: &AppHandle, file_name: &str) -> Result<Option<PathBuf>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    file_dialog(app)
        .set_title("Export activity")
        .set_file_name(file_name)
        .add_filter("CSV", &["csv"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx.await.map_err(|_| closed())?;
    Ok(picked.and_then(|p| p.into_path().ok()).map(|mut p| {
        let is_csv = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv"));
        if !is_csv {
            p.as_mut_os_string().push(".csv");
        }
        p
    }))
}

/// Pick an FFmpeg program for "Use FFmpeg from my system".
pub async fn pick_ffmpeg(app: &AppHandle) -> Result<Option<PathBuf>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    file_dialog(app)
        .set_title("Choose the ffmpeg program")
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx.await.map_err(|_| closed())?;
    Ok(picked.and_then(|p| p.into_path().ok()))
}

/// Pick one audio file for "Compare quality".
pub async fn pick_audio(app: &AppHandle) -> Result<Option<PathBuf>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    file_dialog(app)
        .set_title("Choose an audio file to compare")
        .add_filter(
            "Audio",
            &[
                "mp3", "m4a", "m4b", "aac", "wav", "aif", "aiff", "flac", "opus",
            ],
        )
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx.await.map_err(|_| closed())?;
    Ok(picked.and_then(|p| p.into_path().ok()))
}
