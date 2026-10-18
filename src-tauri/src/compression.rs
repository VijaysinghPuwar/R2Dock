//! Compression tools for the app: which FFmpeg to use, installing and
//! removing the pinned download, and telling the transfer engine.
//! The logic lives in `r2dock-media`; this file only holds app state.

use std::path::PathBuf;

use parking_lot::{Mutex, RwLock};
use r2dock_core::AppError;
use r2dock_media::ffmpeg::Ffmpeg;
use r2dock_media::tools::detect::{self, BuildInfo};
use r2dock_media::tools::install::{self, HttpFetch};
use r2dock_media::tools::{Tools, ToolsError, manifest};
use r2dock_media::video::encoders;
use r2dock_media::video::plan::VideoEncoders;
use r2dock_transfer::JobState;
use r2dock_transfer::engine::MediaRuntime;
use tokio_util::sync::CancellationToken;

use crate::dto::{CompressionStatus, ToolsState};
use crate::state::AppState;

pub struct CompressionService {
    /// Downloaded tools (inside the app data folder).
    pub tools_root: PathBuf,
    /// Compressed copies waiting to upload (inside the app cache folder).
    pub work_dir: PathBuf,
    current: RwLock<Option<(Tools, BuildInfo)>>,
    /// Video encoders that passed a test encode, for the current FFmpeg.
    video: RwLock<VideoEncoders>,
    problem: RwLock<Option<String>>,
    installing: Mutex<Option<CancellationToken>>,
    /// The running "Compare quality", stopped when another one starts.
    preview: Mutex<Option<CancellationToken>>,
}

/// Replacing or deleting FFmpeg under a running encode fails part way on
/// Windows, so both wait until no file is being compressed.
fn ensure_not_compressing(state: &AppState, action: &str) -> Result<(), AppError> {
    if state
        .engine
        .snapshot()
        .iter()
        .any(|j| j.state == JobState::Compressing)
    {
        return Err(AppError::invalid_input(format!(
            "Files are being compressed. Pause or cancel them, then {action} the tools."
        )));
    }
    Ok(())
}

fn tools_error(e: &ToolsError) -> AppError {
    match e {
        ToolsError::Cancelled => AppError::new(r2dock_core::ErrorCode::Cancelled, "Cancelled."),
        ToolsError::UnsupportedPlatform => AppError::invalid_input(
            "Compression tools are not available for this system. Use FFmpeg from your system instead.",
        ),
        ToolsError::Download(_) => AppError::new(
            r2dock_core::ErrorCode::Network,
            "The compression tools could not be downloaded. Check your connection and try again.",
        )
        .retryable(true),
        other => AppError::internal(format!("Compression tools: {other}.")),
    }
}

impl CompressionService {
    pub fn new(tools_root: PathBuf, work_dir: PathBuf) -> Self {
        Self {
            tools_root,
            work_dir,
            current: RwLock::new(None),
            video: RwLock::new(VideoEncoders::default()),
            problem: RwLock::new(None),
            installing: Mutex::new(None),
            preview: Mutex::new(None),
        }
    }

    /// Give the engine the current tools and encode limit (or none).
    pub fn apply(&self, state: &AppState) {
        let limit = usize::from(state.settings.read().preferences.compression.encode_limit);
        let media = self.ffmpeg().map(|ffmpeg| MediaRuntime {
            ffmpeg,
            work_dir: self.work_dir.clone(),
            encode_limit: limit,
        });
        state.engine.set_media(media);
    }

    /// Work out which FFmpeg to use (the picked one, else the download),
    /// check it, and tell the engine.
    pub async fn refresh(&self, state: &AppState) {
        let system = state.settings.read().ffmpeg_path.clone();
        let result = match system {
            Some(path) => detect::use_system(&path).await,
            None => {
                let root = self.tools_root.clone();
                let checked = tokio::task::spawn_blocking(move || {
                    install::check_installed(&root, manifest::for_this_platform())
                })
                .await
                .unwrap_or(Err(ToolsError::NotInstalled));
                match checked {
                    Ok(tools) => detect::inspect(&tools.ffmpeg)
                        .await
                        .map(|info| (tools, info)),
                    Err(e) => Err(e),
                }
            }
        };
        match result {
            Ok(ok) => {
                // Prove each video encoder once per FFmpeg.
                *self.video.write() = encoders::detect(&ok.0.ffmpeg, &ok.1.encoders).await;
                *self.current.write() = Some(ok);
                *self.problem.write() = None;
            }
            Err(ToolsError::NotInstalled) => {
                *self.current.write() = None;
                *self.problem.write() = None;
            }
            Err(e) => {
                tracing::warn!("compression tools unusable: {e}");
                *self.current.write() = None;
                *self.problem.write() = Some(e.to_string());
            }
        }
        self.apply(state);
    }

    pub fn status(&self, state: &AppState) -> CompressionStatus {
        let current = self.current.read().clone();
        let installing = self.installing.lock().is_some();
        let problem = self.problem.read().clone();
        let system = state.settings.read().ffmpeg_path.clone();
        let tools_state = if installing {
            ToolsState::Installing
        } else if current.is_some() {
            ToolsState::Ready
        } else if problem.is_some() {
            ToolsState::Unusable
        } else {
            ToolsState::NotInstalled
        };
        CompressionStatus {
            state: tools_state,
            source: current.as_ref().map(|(t, _)| t.source),
            version: current.as_ref().map(|(t, _)| t.version.clone()),
            license: current.as_ref().map(|(_, i)| i.license),
            download_bytes: manifest::for_this_platform().map(|p| p.total_download()),
            system_path: system.map(|p| state.display_path(&p)),
            problem,
        }
    }

    /// Download and install the pinned tools. `on_progress` gets
    /// (done, total) bytes.
    pub async fn install(
        &self,
        state: &AppState,
        on_progress: impl FnMut(u64, u64) + Send,
    ) -> Result<(), AppError> {
        let pkg = manifest::for_this_platform()
            .ok_or_else(|| tools_error(&ToolsError::UnsupportedPlatform))?;
        ensure_not_compressing(state, "install")?;
        let token = {
            let mut slot = self.installing.lock();
            if slot.is_some() {
                return Err(AppError::invalid_input(
                    "The tools are already being installed.",
                ));
            }
            let token = CancellationToken::new();
            *slot = Some(token.clone());
            token
        };
        let agent = format!("R2Dock/{}", env!("CARGO_PKG_VERSION"));
        let result = match HttpFetch::new(&agent) {
            Ok(fetch) => install::install(&self.tools_root, pkg, &fetch, &token, on_progress).await,
            Err(e) => Err(e),
        };
        *self.installing.lock() = None;
        let result = result.map_err(|e| tools_error(&e));
        if result.is_ok() {
            // The download replaces a picked system FFmpeg.
            state.update_settings(|s| s.ffmpeg_path = None).await?;
        }
        self.refresh(state).await;
        result.map(|_| ())
    }

    pub fn cancel_install(&self) {
        if let Some(t) = self.installing.lock().as_ref() {
            t.cancel();
        }
    }

    /// Delete the downloaded tools. Blocking file removal runs off the
    /// async runtime. The tools are checked again even when removal fails
    /// part way, so a half-deleted folder is never used.
    pub async fn remove(&self, state: &AppState) -> Result<(), AppError> {
        ensure_not_compressing(state, "remove")?;
        if self.installing.lock().is_some() {
            return Err(AppError::invalid_input(
                "The tools are being installed. Cancel the install first.",
            ));
        }
        let root = self.tools_root.clone();
        let result = tokio::task::spawn_blocking(move || install::remove(&root))
            .await
            .map_err(|_| AppError::internal("A background task stopped unexpectedly."))
            .and_then(|r| r.map_err(|e| tools_error(&e)));
        self.refresh(state).await;
        result
    }

    /// Start a "Compare quality" run, stopping the one before it.
    pub fn start_preview(&self) -> CancellationToken {
        let token = CancellationToken::new();
        if let Some(old) = self.preview.lock().replace(token.clone()) {
            old.cancel();
        }
        token
    }

    /// Use an FFmpeg the user picked; check it first.
    pub async fn use_system(&self, state: &AppState, path: PathBuf) -> Result<(), AppError> {
        detect::use_system(&path)
            .await
            .map_err(|e| AppError::invalid_input(e.to_string()))?;
        state
            .update_settings(|s| s.ffmpeg_path = Some(path))
            .await?;
        self.refresh(state).await;
        Ok(())
    }

    /// Go back to the downloaded tools.
    pub async fn use_downloaded(&self, state: &AppState) -> Result<(), AppError> {
        state.update_settings(|s| s.ffmpeg_path = None).await?;
        self.refresh(state).await;
        Ok(())
    }

    /// The FFmpeg to run now, with ffprobe and the working video encoders.
    pub fn ffmpeg(&self) -> Option<Ffmpeg> {
        let video = *self.video.read();
        self.current
            .read()
            .as_ref()
            .map(|(t, _)| Ffmpeg::new(t.ffmpeg.clone()).with_video(t.ffprobe.clone(), video))
    }

    /// Whether compression can run now.
    pub fn ready(&self) -> bool {
        self.current.read().is_some()
    }
}
