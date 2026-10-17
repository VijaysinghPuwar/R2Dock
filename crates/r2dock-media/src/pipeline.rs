//! The whole decision for one file, cheapest checks first:
//! probe, duplicated-stereo check, plan, sample test, disk space, encode,
//! and the final size check. Anything that does not pay off leaves the file
//! unchanged with a reason the interface can show.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use ts_rs::TS;

use crate::classify::ContentOverride;
use crate::ffmpeg::{EncodeError, Ffmpeg, SAMPLE_TEST_MIN_SECS};
use crate::keys::output_key;
use crate::plan::{
    AudioDecision, AudioInputs, AudioSettings, AudioTarget, Encoding, SkipReason, plan_audio,
};
use crate::probe::{ProbeError, probe_audio};

/// Free space needed for the output: 1.2 times the estimate.
const SPACE_FACTOR: f64 = 1.2;

/// Why a file is uploaded as it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
#[ts(export)]
pub enum Unchanged {
    /// Not audio (images, documents, archives, anything else).
    NotAudio,
    /// The plan left it alone.
    Plan { reason: SkipReason },
    /// The sample test saved less than the minimum.
    SampleSavedTooLittle { percent: u8 },
    /// The finished file was not smaller by the minimum.
    ResultSavedTooLittle { percent: u8 },
    /// Not enough free disk space for the compressed copy.
    NoDiskSpace,
    /// FFmpeg is not installed or could not be used.
    ToolsMissing,
}

/// A compressed copy ready to upload in place of the original.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compressed {
    pub output: PathBuf,
    pub size: u64,
    pub original_size: u64,
    /// Preset name, also stored as the marker and in object metadata.
    pub preset: String,
    /// New file extension, or `None` when the key stays the same.
    pub extension: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Prepared {
    Compressed(Compressed),
    Unchanged(Unchanged),
}

/// The output file's extension: the target's new extension, or one that
/// matches the codec when the key keeps its own name.
fn output_extension(input: &Path, target: &AudioTarget) -> String {
    if let Some(ext) = &target.extension {
        return ext.clone();
    }
    let input_ext = input
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match target.encoding {
        Encoding::Mp3Cbr { .. } | Encoding::Mp3Vbr { .. } => "mp3".into(),
        Encoding::Aac { .. } => match input_ext.as_str() {
            "m4a" | "m4b" | "mp4" | "aac" => input_ext,
            _ => "m4a".into(),
        },
        Encoding::Opus { .. } => "opus".into(),
        Encoding::Flac => "flac".into(),
    }
}

fn percent(saving: f64) -> u8 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let p = saving.floor().clamp(0.0, 100.0) as u8;
    p
}

/// One file to prepare.
#[derive(Debug, Clone, Copy)]
pub struct PrepareJob<'a> {
    pub input: &'a Path,
    /// R2Dock's own temporary folder.
    pub work_dir: &'a Path,
    /// A unique name for this job (the job id).
    pub stem: &'a str,
    pub settings: &'a AudioSettings,
    pub video: &'a r2dock_core::compression::VideoSettings,
    pub content: ContentOverride,
}

/// Prepare one file. The compressed copy is `<work_dir>/<stem>.<ext>`;
/// nothing else is left behind.
pub async fn prepare(
    ff: &Ffmpeg,
    job: PrepareJob<'_>,
    cancel: &CancellationToken,
    on_fraction: impl FnMut(f64) + Send,
) -> Result<Prepared, EncodeError> {
    let PrepareJob {
        input,
        work_dir,
        stem,
        settings,
        content,
        ..
    } = job;
    let path = input.to_path_buf();
    let video_candidate = crate::video::run::is_video_candidate(input);
    // Video containers are checked with ffprobe first: the in-process
    // audio probe may see only the audio track of a video file.
    if video_candidate
        && let Some(ffprobe) = &ff.ffprobe
        && crate::video::probe::probe_video(ffprobe, input)
            .await
            .is_some_and(|p| p.video().is_some())
    {
        return crate::video::run::prepare_video(ff, job, cancel, on_fraction).await;
    }
    let probe = match tokio::task::spawn_blocking(move || probe_audio(&path)).await {
        // Audio-only MP4 and M4A stay on the audio path.
        Ok(Ok(p)) if !p.has_video => p,
        Ok(Ok(_)) => return crate::video::run::prepare_video(ff, job, cancel, on_fraction).await,
        Ok(Err(ProbeError::Unsupported)) if video_candidate => {
            return crate::video::run::prepare_video(ff, job, cancel, on_fraction).await;
        }
        Ok(Err(ProbeError::Unsupported)) => return Ok(Prepared::Unchanged(Unchanged::NotAudio)),
        Ok(Err(ProbeError::Io(e))) => {
            return Err(EncodeError::Failed(format!(
                "could not read the file: {}",
                e.kind()
            )));
        }
        Err(_) => return Err(EncodeError::Failed("the probe stopped unexpectedly".into())),
    };
    let duration = probe.duration_secs.unwrap_or(0.0);

    // Only stereo lossy or lossless music benefits; speech becomes mono anyway.
    let first = plan_audio(
        &probe,
        settings,
        AudioInputs {
            content,
            dual_mono: false,
        },
    );
    let dual_mono = match &first {
        AudioDecision::Compress(t)
            if probe.channels == Some(2) && t.channels == 2 && t.encoding != Encoding::Flac =>
        {
            ff.is_dual_mono(input, duration, cancel).await?
        }
        _ => false,
    };
    let decision = if dual_mono {
        plan_audio(&probe, settings, AudioInputs { content, dual_mono })
    } else {
        first
    };
    let target = match decision {
        AudioDecision::Compress(t) => t,
        AudioDecision::Skip(reason) => return Ok(Prepared::Unchanged(Unchanged::Plan { reason })),
    };

    std::fs::create_dir_all(work_dir).map_err(|e| {
        EncodeError::Failed(format!("could not create the work folder: {}", e.kind()))
    })?;
    let ext = output_extension(input, &target);
    let output = work_dir.join(format!("{stem}.{ext}"));
    let margin = f64::from(settings.min_saving_percent);

    // Lossless sources always shrink, so only lossy sources get a sample test.
    if duration > SAMPLE_TEST_MIN_SECS && !probe.codec.is_lossless() {
        let sample = work_dir.join(format!("{stem}.sample.{ext}"));
        let saving = ff
            .sample_saving(input, &sample, &target, probe.size, duration, cancel)
            .await?;
        if saving < margin {
            return Ok(Prepared::Unchanged(Unchanged::SampleSavedTooLittle {
                percent: percent(saving),
            }));
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let needed = (target.estimated_size as f64 * SPACE_FACTOR) as u64;
    if fs4::available_space(work_dir).is_ok_and(|free| free < needed) {
        return Ok(Prepared::Unchanged(Unchanged::NoDiskSpace));
    }

    let result = ff
        .encode(input, &output, &target, duration, cancel, on_fraction)
        .await?;
    #[allow(clippy::cast_precision_loss)]
    let saving = 100.0 * (1.0 - result.size as f64 / probe.size.max(1) as f64);
    if saving < margin {
        let _ = std::fs::remove_file(&output);
        return Ok(Prepared::Unchanged(Unchanged::ResultSavedTooLittle {
            percent: percent(saving),
        }));
    }
    Ok(Prepared::Compressed(Compressed {
        output,
        size: result.size,
        original_size: probe.size,
        preset: target.encoding.preset(),
        extension: target.extension.clone(),
    }))
}

/// The key a prepared file is uploaded to.
pub fn upload_key(original_key: &str, prepared: &Compressed) -> String {
    output_key(original_key, prepared.extension.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::Content;

    fn target(encoding: Encoding, extension: Option<&str>) -> AudioTarget {
        AudioTarget {
            encoding,
            channels: 1,
            sample_rate: None,
            extension: extension.map(str::to_string),
            content: Content::Speech,
            estimated_size: 0,
        }
    }

    #[test]
    fn output_extensions() {
        let mp3 = Encoding::Mp3Cbr { kbps: 32 };
        let aac = Encoding::Aac { kbps: 64 };
        let cases = [
            ("a.MP3", target(mp3, None), "mp3"),
            ("no-ext", target(mp3, None), "mp3"),
            ("book.M4B", target(aac, None), "m4b"),
            ("song.mp4", target(aac, None), "mp4"),
            ("weird.bin", target(aac, None), "m4a"),
            ("take.wav", target(mp3, Some("mp3")), "mp3"),
            ("take.wav", target(Encoding::Flac, Some("flac")), "flac"),
            (
                "speech.opus",
                target(Encoding::Opus { kbps: 24 }, None),
                "opus",
            ),
        ];
        for (input, t, want) in cases {
            assert_eq!(output_extension(Path::new(input), &t), want, "{input}");
        }
    }

    #[test]
    fn percents() {
        assert_eq!(percent(-5.0), 0);
        assert_eq!(percent(14.99), 14);
        assert_eq!(percent(150.0), 100);
    }
}
