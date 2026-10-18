//! "Compare quality": 30 second samples of one file at each preset, so the
//! user can hear the difference before choosing. The original is included
//! as lossless FLAC from the same 30 seconds.
//!
//! Each label is made from the encoding that is actually produced, and
//! presets that produce the same encoding share one sample: an AAC file
//! stays AAC whatever the MP3 speech setting says.

use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;

use crate::classify::{Content, ContentOverride, classify};
use crate::ffmpeg::{EncodeError, Ffmpeg, args, middle_window};
use crate::plan::{
    AudioDecision, AudioInputs, AudioSettings, AudioTarget, Encoding, Mode, MusicQuality,
    SpeechQuality, plan_audio,
};
use crate::probe::{AudioProbe, ProbeError, probe_audio};

/// One playable sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    /// "Original", "MP3 32 kbps, mono", ...
    pub label: String,
    pub path: PathBuf,
    /// Size of one hour at this setting, from the sample.
    pub bytes_per_hour: u64,
}

/// The presets to compare for this content.
fn variants(content: Content, base: AudioSettings) -> Vec<AudioSettings> {
    let keep = AudioSettings {
        mode: Mode::KeepFormat,
        ..base
    };
    let smallest = AudioSettings {
        mode: Mode::Smallest,
        ..base
    };
    match content {
        Content::Speech => vec![
            AudioSettings {
                speech: SpeechQuality::Kbps32,
                ..keep
            },
            AudioSettings {
                speech: SpeechQuality::Kbps48,
                ..keep
            },
            smallest,
        ],
        Content::Music => vec![
            AudioSettings {
                music: MusicQuality::Standard,
                ..keep
            },
            AudioSettings {
                music: MusicQuality::Smaller,
                ..keep
            },
            smallest,
        ],
    }
}

/// A label that says what the sample really is: codec, bitrate, channels.
pub fn label(target: &AudioTarget) -> String {
    let codec = match target.encoding {
        Encoding::Mp3Cbr { kbps } => format!("MP3 {kbps} kbps"),
        Encoding::Mp3Vbr { approx_kbps, .. } => format!("MP3 about {approx_kbps} kbps"),
        Encoding::Aac { kbps } => format!("AAC {kbps} kbps"),
        Encoding::Opus { kbps } => format!("Opus {kbps} kbps"),
        Encoding::Flac => "FLAC (lossless)".to_string(),
    };
    let channels = match target.channels {
        1 => "mono".to_string(),
        2 => "stereo".to_string(),
        n => format!("{n} channels"),
    };
    format!("{codec}, {channels}")
}

/// The sample targets, one per distinct encoding, in preset order.
fn targets(probe: &AudioProbe, content: Content, base: AudioSettings) -> Vec<AudioTarget> {
    // Music is judged as music even when a single file looks like speech.
    let inputs = AudioInputs {
        content: match content {
            Content::Speech => ContentOverride::Speech,
            Content::Music => ContentOverride::Music,
        },
    };
    let mut out: Vec<AudioTarget> = Vec::new();
    for settings in variants(content, base) {
        if let AudioDecision::Compress(t) = plan_audio(probe, &settings, inputs)
            && !out.iter().any(|o| {
                (o.encoding, o.channels, o.sample_rate) == (t.encoding, t.channels, t.sample_rate)
            })
        {
            out.push(t);
        }
    }
    out
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
/// not make this file smaller are left out, and presets that produce the
/// same encoding are encoded once.
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
    let mut all = vec![("Original".to_string(), original)];
    all.extend(
        targets(&probe, content, AudioSettings::default())
            .into_iter()
            .map(|t| (label(&t), t)),
    );

    let mut samples = Vec::new();
    for (i, (label, target)) in all.into_iter().enumerate() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::LosslessAction;
    use crate::probe::AudioCodec;

    fn probe(codec: AudioCodec, rate: u32, channels: u8, kbps: u64) -> AudioProbe {
        AudioProbe {
            codec,
            sample_rate: Some(rate),
            channels: Some(channels),
            duration_secs: Some(3600.0),
            size: kbps * 1000 * 3600 / 8,
            picture_bytes: 0,
            has_video: false,
            audio_tracks: 1,
            marker: None,
        }
    }

    fn labels(p: &AudioProbe) -> Vec<String> {
        let content = classify(p, ContentOverride::Auto);
        targets(p, content, AudioSettings::default())
            .iter()
            .map(label)
            .collect()
    }

    #[test]
    fn labels_follow_the_real_encoding() {
        // AAC speech stays AAC 64 kbps whatever the MP3 setting: one sample.
        assert_eq!(
            labels(&probe(AudioCodec::Aac, 22_050, 1, 128)),
            ["AAC 64 kbps, mono", "Opus 24 kbps, mono"]
        );
        assert_eq!(
            labels(&probe(AudioCodec::Mp3, 22_050, 1, 128)),
            [
                "MP3 32 kbps, mono",
                "MP3 48 kbps, mono",
                "Opus 24 kbps, mono"
            ]
        );
        assert_eq!(
            labels(&probe(AudioCodec::Mp3, 44_100, 2, 320)),
            [
                "MP3 about 165 kbps, stereo",
                "MP3 about 115 kbps, stereo",
                "Opus 96 kbps, stereo"
            ]
        );
        // AAC music: both MP3 music presets give AAC 128 kbps.
        assert_eq!(
            labels(&probe(AudioCodec::Aac, 44_100, 2, 256)),
            ["AAC 128 kbps, stereo", "Opus 96 kbps, stereo"]
        );
        // Opus in Keep format and in Smallest is the same encoding.
        assert_eq!(
            labels(&probe(AudioCodec::Opus, 48_000, 2, 256)),
            ["Opus 96 kbps, stereo"]
        );
        // Stereo speech keeps both channels.
        assert_eq!(
            labels(&probe(AudioCodec::Mp3, 22_050, 2, 192)),
            [
                "MP3 64 kbps, stereo",
                "MP3 96 kbps, stereo",
                "Opus 48 kbps, stereo"
            ]
        );
        // Lossless to FLAC is the same for every preset: encoded once.
        let flac = AudioSettings {
            lossless: LosslessAction::Flac,
            ..AudioSettings::default()
        };
        let wav = probe(AudioCodec::Pcm, 44_100, 2, 1411);
        let t = targets(&wav, Content::Music, flac);
        assert_eq!(t.len(), 1);
        assert_eq!(label(&t[0]), "FLAC (lossless), stereo");
    }

    #[test]
    fn labels_are_unique() {
        for p in [
            probe(AudioCodec::Aac, 22_050, 1, 128),
            probe(AudioCodec::Aac, 44_100, 2, 256),
            probe(AudioCodec::Opus, 48_000, 2, 256),
            probe(AudioCodec::Mp3, 22_050, 2, 192),
            probe(AudioCodec::Pcm, 44_100, 2, 1411),
        ] {
            let l = labels(&p);
            let mut unique = l.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(l.len(), unique.len(), "{p:?}: {l:?}");
        }
    }

    #[test]
    fn samples_follow_the_channel_and_track_policy() {
        let surround = probe(AudioCodec::Pcm, 48_000, 6, 4608);
        assert!(labels(&surround).is_empty(), "6 channels are kept");
        let mut two = probe(AudioCodec::Aac, 44_100, 2, 256);
        two.audio_tracks = 2;
        assert!(labels(&two).is_empty(), "second track is kept");
    }
}
