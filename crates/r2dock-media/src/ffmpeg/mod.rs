//! Running FFmpeg: arguments, progress, the process runner, and the
//! operations built on them (full encode and sample test).

pub mod args;
pub mod progress;
pub mod run;

use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::plan::AudioTarget;
use crate::verify::{VerifyError, verify_audio};
use args::{ArgsError, Window};
use run::{RunError, RunOptions, sanitize};

/// Length of the sample encoded before a long file.
pub const SAMPLE_SECS: f64 = 30.0;
/// Files longer than this get a sample test first.
pub const SAMPLE_TEST_MIN_SECS: f64 = 300.0;

#[derive(Debug, Error)]
pub enum EncodeError {
    #[error(transparent)]
    Args(#[from] ArgsError),
    #[error(transparent)]
    Run(#[from] RunError),
    #[error("{0}")]
    Failed(String),
    #[error(transparent)]
    Verify(#[from] VerifyError),
}

impl EncodeError {
    pub fn is_cancelled(&self) -> bool {
        matches!(self, Self::Run(RunError::Cancelled))
    }
}

/// The FFmpeg binary to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ffmpeg {
    pub bin: PathBuf,
    pub options: RunOptions,
    /// ffprobe, needed for video.
    pub ffprobe: Option<PathBuf>,
    /// Video encoders that passed a test encode (see `video::encoders`).
    pub video: crate::video::plan::VideoEncoders,
}

impl Ffmpeg {
    pub fn new(bin: PathBuf) -> Self {
        Self {
            bin,
            options: RunOptions::default(),
            ffprobe: None,
            video: crate::video::plan::VideoEncoders::default(),
        }
    }

    /// Add ffprobe and the detected video encoders.
    pub fn with_video(
        mut self,
        ffprobe: PathBuf,
        video: crate::video::plan::VideoEncoders,
    ) -> Self {
        self.ffprobe = Some(ffprobe);
        self.video = video;
        self
    }

    /// Run and turn a failure into a message without local folders.
    pub(crate) async fn run_clean(
        &self,
        args: &[std::ffi::OsString],
        paths: &[&Path],
        cancel: &CancellationToken,
        on_progress: impl FnMut(progress::Progress) + Send,
    ) -> Result<Vec<String>, EncodeError> {
        match run::run(&self.bin, args, cancel, self.options, on_progress).await {
            Err(RunError::Failed { code, last_line }) => {
                let detail = last_line
                    .map(|l| sanitize(&l, paths))
                    .unwrap_or_else(|| "no message".to_string());
                let code = code.map_or("signal".to_string(), |c| c.to_string());
                Err(EncodeError::Failed(format!(
                    "FFmpeg exited with {code}: {detail}"
                )))
            }
            other => Ok(other?),
        }
    }

    /// Encode `input` to `output`, verify the result, and return its probe.
    /// `on_fraction`
    /// receives progress from 0.0 to 1.0. `output` must be in R2Dock's own
    /// temporary folder; it is deleted again on any failure.
    pub async fn encode(
        &self,
        input: &Path,
        output: &Path,
        target: &AudioTarget,
        duration_secs: f64,
        cancel: &CancellationToken,
        mut on_fraction: impl FnMut(f64) + Send,
    ) -> Result<crate::probe::AudioProbe, EncodeError> {
        let args = args::encode_audio(input, output, target, None)?;
        let result = self
            .run_clean(&args, &[input, output], cancel, |p| {
                if let Some(secs) = p.out_secs
                    && duration_secs > 0.0
                {
                    on_fraction((secs / duration_secs).clamp(0.0, 1.0));
                }
            })
            .await
            .and_then(|_| Ok(verify_audio(output, target, duration_secs)?));
        if result.is_err() {
            let _ = std::fs::remove_file(output);
        }
        result
    }

    /// Encode a sample from the middle of the file and return the projected
    /// saving in percent (0 to 100). Used before spending minutes on a long
    /// file. `output` is a temporary path that is always deleted.
    pub async fn sample_saving(
        &self,
        input: &Path,
        output: &Path,
        target: &AudioTarget,
        source_size: u64,
        duration_secs: f64,
        cancel: &CancellationToken,
    ) -> Result<f64, EncodeError> {
        let window = middle_window(duration_secs);
        let args = args::encode_audio(input, output, target, Some(window))?;
        let result = self
            .run_clean(&args, &[input, output], cancel, |_| {})
            .await;
        let size = std::fs::metadata(output).map(|m| m.len());
        let _ = std::fs::remove_file(output);
        result?;
        let sample = size.map_err(|e| EncodeError::Failed(format!("sample missing: {e}")))?;
        #[allow(clippy::cast_precision_loss)]
        let projected = sample as f64 * duration_secs / window.len_secs;
        #[allow(clippy::cast_precision_loss)]
        let saving = 100.0 * (1.0 - projected / source_size.max(1) as f64);
        Ok(saving.clamp(0.0, 100.0))
    }
}

/// A sample window in the middle of the file (the whole file if short).
pub fn middle_window(duration_secs: f64) -> Window {
    if duration_secs <= SAMPLE_SECS {
        return Window {
            start_secs: 0.0,
            len_secs: duration_secs.max(0.1),
        };
    }
    Window {
        start_secs: (duration_secs - SAMPLE_SECS) / 2.0,
        len_secs: SAMPLE_SECS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows() {
        let w = middle_window(82_481.0);
        assert!((w.start_secs - 41_225.5).abs() < 1e-6);
        assert!((w.len_secs - 30.0).abs() < 1e-9);
        let short = middle_window(12.0);
        assert_eq!(short.start_secs, 0.0);
        assert!((short.len_secs - 12.0).abs() < 1e-9);
    }
}
