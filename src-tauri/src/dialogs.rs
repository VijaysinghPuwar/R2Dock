//! Native file pickers, run from Rust so local paths never come from the webview.

use std::path::PathBuf;

use r2dock_core::AppError;
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

fn closed() -> AppError {
    AppError::internal("The file picker closed unexpectedly.")
}

pub async fn pick_files(app: &AppHandle) -> Result<Option<Vec<PathBuf>>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Upload files")
        .pick_files(move |paths| {
            let _ = tx.send(paths);
        });
    let picked = rx.await.map_err(|_| closed())?;
    Ok(picked.map(|v| v.into_iter().filter_map(|p| p.into_path().ok()).collect()))
}

pub async fn pick_upload_folder(app: &AppHandle) -> Result<Option<PathBuf>, AppError> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
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
    let mut builder = app.dialog().file().set_title("Choose a folder");
    if let Some(dir) = start {
        builder = builder.set_directory(dir);
    }
    builder.pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let picked = rx.await.map_err(|_| closed())?;
    Ok(picked.and_then(|p| p.into_path().ok()))
}
