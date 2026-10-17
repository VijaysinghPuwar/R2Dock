//! "Compare quality": 30 second samples of one file at each preset, so the
//! user can hear the difference before choosing. The original is included
//! as lossless FLAC from the same 30 seconds.

use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use crate::classify::{Content, ContentOverride, classify};
use crate::ffmpeg::{EncodeError, Ffmpeg, args, middle_window};
use crate::plan::{
    AudioDecision, AudioInputs, AudioSettings, AudioTarget, Encoding, Mode, MusicQuality,
    SpeechQuality, plan_audio,
};
use crate::probe::{ProbeError, probe_audio};

/// One playable sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// "Original", "Speech 32 kbps", ...
    pub label: String,
    pub path: PathBuf,
    /// Size of one hour at this setting, from the sample.
    pub bytes_per_hour: u64,
}

fn variants(content: Content) -> Vec<(&'static str, AudioSettings)> {
    let base = AudioSettings::default();
    match content {
        Content::Speech => vec![
            (
                "Speech 32 kbps (MP3)",
                AudioSettings {
                    speech: SpeechQuality::Kbps32,
                    ..base
                },
            ),
            (
                "Speech 48 kbps (MP3)",
                AudioSettings {
                    speech: SpeechQuality::Kbps48,
                    ..base
                },
            ),
            (
                "Smallest (Opus 24 kbps)",
                AudioSettings {
                    mode: Mode::Smallest,
                    ..base
                },
            ),
        ],
        Content::Music => vec![
            (
                "Music Standard (MP3)",
                AudioSettings {
                    music: MusicQuality::Standard,
                    ..base
                },
            ),
            (
                "Music Smaller (MP3)",
                AudioSettings {
                    music: MusicQuality::Smaller,
                    ..base
                },
            ),
            (
                "Smallest (Opus)",
                AudioSettings {
                    mode: Mode::Smallest,
                    ..base
                },
            ),
        ],
    }
}

fn ext_for(encoding: Encoding) -> &'static str {
    match encoding {
        Encoding::Mp3Cbr { .. } | Encoding::Mp3Vbr { .. } => "mp3",
        Encoding::Aac { .. } => "m4a",
        Encoding::Opus { .. } => "opus",
        Encoding::Flac => "flac",
    }
}

/// Encode the samples into `out_dir` (emptied first). Presets that would
/// not make this file smaller are left out.
pub async fn compare(
    ff: &Ffmpeg,
    input: &Path,
    out_dir: &Path,
    cancel: &CancellationToken,
) -> Result<Vec<Sample>, EncodeError> {
    let path = input.to_path_buf();
    let probe = match tokio::task::spawn_blocking(move || probe_audio(&path)).await {
        Ok(Ok(p)) => p,
        Ok(Err(ProbeError::Unsupported)) => {
            return Err(EncodeError::Failed(
                "this file is not audio R2Dock can read".into(),
            ));
        }
        _ => return Err(EncodeError::Failed("the file could not be read".into())),
    };
    let duration = probe
        .duration_secs
        .ok_or_else(|| EncodeError::Failed("the length of this file could not be read".into()))?;
    let _ = std::fs::remove_dir_all(out_dir);
    std::fs::create_dir_all(out_dir).map_err(|e| {
        EncodeError::Failed(format!("could not create the preview folder: {}", e.kind()))
    })?;
    let window = middle_window(duration);
    let content = classify(&probe, ContentOverride::Auto);

    let original = AudioTarget {
        encoding: Encoding::Flac,
        channels: probe.channels.unwrap_or(2),
        sample_rate: probe.sample_rate,
        extension: Some("flac".into()),
        content,
        estimated_size: 0,
    };
    let mut targets = vec![("Original".to_string(), original)];
    for (label, settings) in variants(content) {
        // Music is judged as music even when a single file looks like speech.
        let inputs = AudioInputs {
            content: match content {
                Content::Speech => ContentOverride::Speech,
                Content::Music => ContentOverride::Music,
            },
            dual_mono: false,
        };
        if let AudioDecision::Compress(t) = plan_audio(&probe, &settings, inputs) {
            targets.push((label.to_string(), t));
        }
    }

    let mut samples = Vec::new();
    for (i, (label, target)) in targets.into_iter().enumerate() {
        let out = out_dir.join(format!(
            "{i}-{}.{}",
            target.encoding.preset(),
            ext_for(target.encoding)
        ));
        let a = args::encode_audio(input, &out, &target, Some(window))?;
        crate::ffmpeg::run::run(&ff.bin, &a, cancel, ff.options, |_| {}).await?;
        let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
        // The original plays from a lossless copy, but its size is the real
        // file's, so the comparison is honest.
        let (bytes, secs) = if i == 0 {
            (probe.size, duration)
        } else {
            (size, window.len_secs)
        };
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let per_hour = (bytes as f64 * 3600.0 / secs.max(0.1)) as u64;
        samples.push(Sample {
            label,
            path: out,
            bytes_per_hour: per_hour,
        });
    }
    Ok(samples)
}
