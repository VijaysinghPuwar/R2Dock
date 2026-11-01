//! Running FFmpeg: arguments, progress, the process runner, and the
//! operations built on them (full encode, sample test, channel check).

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
/// Left minus right peaking at or below this level, over the whole stream,
/// means the channels are identical. Lossy joint-stereo sources decode with
/// tiny differences even when both channels were the same; -60 dBFS is far
/// above that and far below anything audible as stereo content.
pub const IDENTICAL_CHANNELS_DB: f64 = -60.0;
/// How far the channel difference must stay below the audio's own peak.
pub const SIGNAL_MARGIN_DB: f64 = 40.0;

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

    /// Run and turn a failure into a message without local folders. FFmpeg
    /// stopped from outside (not by R2Dock) is run once more before it
    /// counts as a failure; every run overwrites its own output.
    pub(crate) async fn run_clean(
        &self,
        args: &[std::ffi::OsString],
        paths: &[&Path],
        cancel: &CancellationToken,
        mut on_progress: impl FnMut(progress::Progress) + Send,
    ) -> Result<Vec<String>, EncodeError> {
        let mut retried = false;
        loop {
            match run::run(&self.bin, args, cancel, self.options, &mut on_progress).await {
                Err(RunError::Failed { code, .. })
                    if !retried && run::stopped_from_outside(code) && !cancel.is_cancelled() =>
                {
                    retried = true;
                }
                Err(RunError::Failed { code, last_line }) => {
                    let text = run::exit_text(code);
                    return Err(EncodeError::Failed(match last_line {
                        Some(l) => format!("{text}: {}", sanitize(&l, paths)),
                        None => text,
                    }));
                }
                other => return Ok(other?),
            }
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

impl Ffmpeg {
    /// Whether left and right of a stereo file are identical everywhere:
    /// the whole first audio track is decoded once and the loudest
    /// difference must stay at or below [`IDENTICAL_CHANNELS_DB`]. A sample
    /// is never enough: a file can match in the middle and differ at the
    /// start. Cancellable; uses the same runner and watchdog as encodes.
    pub async fn channels_identical(
        &self,
        input: &Path,
        cancel: &CancellationToken,
    ) -> Result<bool, EncodeError> {
        let args = args::channel_difference(input)?;
        let lines = self.run_clean(&args, &[input], cancel, |_| {}).await?;
        let difference = max_volume(&lines, "Parsed_volumedetect_2");
        let signal = max_volume(&lines, "Parsed_volumedetect_4");
        Ok(channels_match(difference, signal))
    }
}

/// The difference must be tiny in absolute terms and also far below the
/// audio itself, so quiet audio that differs between channels (which an
/// absolute limit alone would accept) keeps both channels.
fn channels_match(difference: Option<f64>, signal: Option<f64>) -> bool {
    match (difference, signal) {
        (Some(d), Some(s)) => {
            d <= IDENTICAL_CHANNELS_DB && (d == f64::NEG_INFINITY || d <= s - SIGNAL_MARGIN_DB)
        }
        _ => false,
    }
}

/// `max_volume` reported by one volumedetect filter, in dB (`-inf` is
/// negative infinity).
fn max_volume(lines: &[String], filter: &str) -> Option<f64> {
    lines
        .iter()
        .rev()
        .filter(|l| l.contains(filter))
        .find_map(|l| {
            let rest = l.split("max_volume:").nth(1)?;
            let value = rest.trim().trim_end_matches("dB").trim();
            if value == "-inf" {
                Some(f64::NEG_INFINITY)
            } else {
                value.parse().ok()
            }
        })
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

    #[test]
    fn parses_max_volume() {
        let lines = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
        let log = lines(
            "[Parsed_volumedetect_4 @ 0x2] max_volume: -10.8 dB\n[Parsed_volumedetect_2 @ 0x1] mean_volume: -91.0 dB\n[Parsed_volumedetect_2 @ 0x1] max_volume: -inf dB",
        );
        assert_eq!(
            max_volume(&log, "Parsed_volumedetect_2"),
            Some(f64::NEG_INFINITY)
        );
        assert_eq!(max_volume(&log, "Parsed_volumedetect_4"), Some(-10.8));
        // No measurement (an empty track) is never taken as identical.
        assert_eq!(max_volume(&lines("nothing"), "Parsed_volumedetect_2"), None);
    }

    /// A fake FFmpeg that is stopped from outside on its first run (like
    /// the 0xC000013A exit seen in a user's log) and succeeds on the next.
    /// With `always`, it is stopped every time.
    fn stopped_once(dir: &Path, always: bool) -> PathBuf {
        let marker = dir.join("ran");
        #[cfg(windows)]
        {
            let bin = dir.join("ffmpeg.bat");
            let body = if always {
                "@exit /b -1073741510\r\n".to_string()
            } else {
                format!(
                    "@if exist \"{m}\" (echo second run 1>&2 & exit /b 0)\r\n@echo.>\"{m}\"\r\n@exit /b -1073741510\r\n",
                    m = marker.display()
                )
            };
            std::fs::write(&bin, body).unwrap();
            bin
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bin = dir.join("ffmpeg");
            let body = if always {
                "#!/bin/sh\nkill -TERM $$\n".to_string()
            } else {
                format!(
                    "#!/bin/sh\nif [ -e '{m}' ]; then echo second run >&2; exit 0; fi\ntouch '{m}'\nkill -TERM $$\n",
                    m = marker.display()
                )
            };
            std::fs::write(&bin, body).unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            bin
        }
    }

    #[tokio::test]
    async fn runs_again_once_after_an_outside_stop() {
        let dir = tempfile::tempdir().unwrap();
        let ff = Ffmpeg::new(stopped_once(dir.path(), false));
        let lines = ff
            .run_clean(&[], &[], &CancellationToken::new(), |_| {})
            .await
            .unwrap();
        assert_eq!(lines.last().map(|l| l.trim()), Some("second run"));

        let dir = tempfile::tempdir().unwrap();
        let ff = Ffmpeg::new(stopped_once(dir.path(), true));
        let err = ff
            .run_clean(&[], &[], &CancellationToken::new(), |_| {})
            .await
            .unwrap_err()
            .to_string();
        #[cfg(windows)]
        assert_eq!(err, "FFmpeg was stopped by the system (0xC000013A)");
        #[cfg(unix)]
        assert_eq!(err, "FFmpeg was stopped by a signal");
    }

    #[test]
    fn channel_match_rules() {
        let inf = f64::NEG_INFINITY;
        // Bit-identical, and lossy-identical well below the audio.
        assert!(channels_match(Some(inf), Some(-10.8)));
        assert!(channels_match(Some(-91.0), Some(-10.8)));
        // Digital silence in both channels.
        assert!(channels_match(Some(inf), Some(inf)));
        // Quiet opposite-phase audio: tiny difference, but the mix is silent.
        assert!(!channels_match(Some(-68.0), Some(-91.0)));
        // Quiet audio whose difference is close to its own level.
        assert!(!channels_match(Some(-70.0), Some(-45.0)));
        // Audible difference.
        assert!(!channels_match(Some(-31.0), Some(-3.0)));
        // Missing measurements never count as identical.
        assert!(!channels_match(None, Some(-10.0)));
        assert!(!channels_match(Some(inf), None));
    }
}
