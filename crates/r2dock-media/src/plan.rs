//! The bitrate ladder and the checks that leave a file unchanged.
//!
//! [`plan_audio`] turns a probe and the user's settings into either a target
//! encoding or the reason the original is uploaded as is. The checks run
//! cheapest first; nothing here starts a process. The sample test and the
//! final size check run later, after FFmpeg has encoded something.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::classify::{Content, ContentOverride, classify};
use crate::probe::{AudioCodec, AudioProbe};

// The settings types live in r2dock-core so they can be stored with the
// other preferences.
pub use r2dock_core::compression::{
    AudioSettings, LosslessAction, Mode, MusicQuality, SpeechQuality,
};

/// Facts about one upload that are not in the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioInputs {
    pub content: ContentOverride,
    /// Left and right are identical over the whole stream (proven by
    /// decoding all of it, see `Ffmpeg::channels_identical`). Mono then
    /// loses nothing, so no downmix permission is needed.
    pub identical_channels: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "codec")]
pub enum Encoding {
    Mp3Cbr { kbps: u32 },
    Mp3Vbr { quality: u8, approx_kbps: u32 },
    Aac { kbps: u32 },
    Opus { kbps: u32 },
    Flac,
}

impl Encoding {
    /// Expected average bitrate, for lossy encodings.
    pub fn nominal_bps(self) -> Option<u64> {
        match self {
            Self::Mp3Cbr { kbps } | Self::Aac { kbps } | Self::Opus { kbps } => {
                Some(u64::from(kbps) * 1000)
            }
            Self::Mp3Vbr { approx_kbps, .. } => Some(u64::from(approx_kbps) * 1000),
            Self::Flac => None,
        }
    }

    /// Short name stored in the marker tag and object metadata.
    pub fn preset(self) -> String {
        match self {
            Self::Mp3Cbr { kbps } => format!("mp3-{kbps}k"),
            Self::Mp3Vbr { quality, .. } => format!("mp3-vbr{quality}"),
            Self::Aac { kbps } => format!("aac-{kbps}k"),
            Self::Opus { kbps } => format!("opus-{kbps}k"),
            Self::Flac => "flac".to_string(),
        }
    }
}

/// The encoding to produce for one file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTarget {
    pub encoding: Encoding,
    pub channels: u8,
    /// `None` keeps the encoder's choice (Opus always works at 48 kHz).
    pub sample_rate: Option<u32>,
    /// New file extension, or `None` when the key does not change.
    pub extension: Option<String>,
    pub content: Content,
    pub estimated_size: u64,
}

/// Why a file is uploaded unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", tag = "reason")]
#[ts(export)]
pub enum SkipReason {
    /// Handled by the video path.
    Video,
    /// R2Dock compressed it before.
    AlreadyCompressedByR2dock { preset: String },
    /// Under 1 MB.
    TooSmall,
    /// Under 10 seconds.
    TooShort,
    /// The probe could not tell how long it is.
    UnknownDuration,
    /// A format R2Dock does not re-encode in this mode.
    UnsupportedFormat,
    /// The lossless setting is Keep.
    LosslessKept,
    /// More than two channels and downmix is not allowed.
    Multichannel { channels: u8 },
    /// The real bitrate is already at or below the target.
    AlreadyCompressed { kbps: u32 },
    /// The projected saving is below the minimum.
    SavingTooSmall { percent: u8 },
    /// Video compression is turned off.
    VideoOff,
    /// HDR video (converting it would wash out the colors).
    Hdr,
    /// Tracks the compressed copy could not keep: a second audio or video
    /// track, subtitles that cannot be converted, attachments, or data.
    UnsupportedStreams,
    /// No working video encoder on this computer.
    NoVideoEncoder,
    /// Already in a codec as efficient as the target.
    AlreadyEfficient { codec: String },
    /// The compressed copy did not hold every track or channel of the
    /// original (checked after encoding), so the original is uploaded.
    TracksNotKept,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum AudioDecision {
    Compress(AudioTarget),
    Skip(SkipReason),
}

pub const MIN_SIZE: u64 = 1_000_000;
pub const MIN_DURATION_SECS: f64 = 10.0;

/// Sample rates the MP3 encoder accepts.
const MP3_RATES: [u32; 9] = [
    8_000, 11_025, 12_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000,
];
const SPEECH_RATE: u32 = 22_050;
const MUSIC_RATE: u32 = 44_100;

/// The highest MP3 rate at or below both the wanted rate and the source.
fn mp3_rate(wanted: u32, source: Option<u32>) -> u32 {
    let cap = source.map_or(wanted, |s| s.min(wanted));
    MP3_RATES
        .iter()
        .rev()
        .copied()
        .find(|r| *r <= cap)
        .unwrap_or(MP3_RATES[0])
}

/// The ladder step for this content and settings, before any checks.
/// `mono` is whether the output has one channel. The speech and music
/// targets are per output: stereo gets about twice the mono bitrate, so
/// keeping both channels does not cost quality.
fn ladder(
    probe: &AudioProbe,
    content: Content,
    mono: bool,
    s: &AudioSettings,
) -> Result<(Encoding, Option<&'static str>), SkipReason> {
    use AudioCodec::*;
    let opus = |content| match (content, mono) {
        (Content::Speech, true) => Encoding::Opus { kbps: 24 },
        (Content::Speech, false) => Encoding::Opus { kbps: 48 },
        (Content::Music, true) => Encoding::Opus { kbps: 64 },
        (Content::Music, false) => Encoding::Opus { kbps: 96 },
    };
    let mp3 = |content| match (content, s.speech, s.music) {
        (Content::Speech, SpeechQuality::Kbps32, _) => Encoding::Mp3Cbr {
            kbps: if mono { 32 } else { 64 },
        },
        (Content::Speech, SpeechQuality::Kbps48, _) => Encoding::Mp3Cbr {
            kbps: if mono { 48 } else { 96 },
        },
        (Content::Music, _, MusicQuality::Standard) => Encoding::Mp3Vbr {
            quality: 4,
            approx_kbps: if mono { 85 } else { 165 },
        },
        (Content::Music, _, MusicQuality::Smaller) => Encoding::Mp3Vbr {
            quality: 6,
            approx_kbps: if mono { 60 } else { 115 },
        },
    };
    if probe.codec.is_lossless() {
        return match (s.lossless, s.mode) {
            (LosslessAction::Keep, _) => Err(SkipReason::LosslessKept),
            (LosslessAction::Flac, _) => Ok((Encoding::Flac, Some("flac"))),
            (LosslessAction::Mp3, Mode::KeepFormat) => Ok((mp3(content), Some("mp3"))),
            (LosslessAction::Mp3, Mode::Smallest) => Ok((opus(content), Some("opus"))),
        };
    }
    match (s.mode, probe.codec) {
        (Mode::Smallest, Mp3 | Aac | Opus) => Ok((opus(content), Some("opus"))),
        (Mode::KeepFormat, Mp3) => Ok((mp3(content), None)),
        (Mode::KeepFormat, Aac) => {
            let kbps = match content {
                Content::Speech => 64,
                Content::Music => 128,
            };
            Ok((Encoding::Aac { kbps }, None))
        }
        (Mode::KeepFormat, Opus) => Ok((opus(content), None)),
        _ => Err(SkipReason::UnsupportedFormat),
    }
}

/// Decide what to do with one audio file.
pub fn plan_audio(probe: &AudioProbe, s: &AudioSettings, inputs: AudioInputs) -> AudioDecision {
    match decide(probe, s, inputs) {
        Ok(target) => AudioDecision::Compress(target),
        Err(reason) => AudioDecision::Skip(reason),
    }
}

fn decide(
    probe: &AudioProbe,
    s: &AudioSettings,
    inputs: AudioInputs,
) -> Result<AudioTarget, SkipReason> {
    if probe.has_video {
        return Err(SkipReason::Video);
    }
    // FFmpeg encodes one audio track; a second one would be lost.
    if probe.audio_tracks > 1 {
        return Err(SkipReason::UnsupportedStreams);
    }
    if let Some(preset) = &probe.marker {
        return Err(SkipReason::AlreadyCompressedByR2dock {
            preset: preset.clone(),
        });
    }
    if probe.size < MIN_SIZE {
        return Err(SkipReason::TooSmall);
    }
    let duration = probe.duration_secs.ok_or(SkipReason::UnknownDuration)?;
    if duration < MIN_DURATION_SECS {
        return Err(SkipReason::TooShort);
    }
    let content = classify(probe, inputs.content);
    let source_channels = probe.channels.unwrap_or(2);
    // Channels are only reduced when that loses nothing (left and right
    // proven identical) or the user allows it: a mono mix erases sound that
    // differs between channels. With permission, speech becomes mono and
    // anything else at most stereo.
    let mono = source_channels == 1
        || (source_channels == 2 && inputs.identical_channels)
        || (s.allow_downmix && content == Content::Speech);
    let (encoding, extension) = ladder(probe, content, mono, s)?;
    let channels = match encoding {
        // Lossless output keeps every channel.
        Encoding::Flac => source_channels,
        // The veto comes before any other channel decision.
        _ if source_channels > 2 && !s.allow_downmix => {
            return Err(SkipReason::Multichannel {
                channels: source_channels,
            });
        }
        _ if mono => 1,
        _ => source_channels.min(2),
    };
    let sample_rate = match encoding {
        Encoding::Mp3Cbr { .. } | Encoding::Mp3Vbr { .. } => {
            let wanted = match content {
                Content::Speech => SPEECH_RATE,
                Content::Music => MUSIC_RATE,
            };
            Some(mp3_rate(wanted, probe.sample_rate))
        }
        Encoding::Aac { .. } | Encoding::Flac => probe.sample_rate,
        Encoding::Opus { .. } => None,
    };

    // Never raise the bitrate: a lossy source at or below the target stays.
    // Headers and tags add a little to the measured bitrate, so a source
    // within 3% of the target counts as already there.
    let real = probe.real_bitrate().unwrap_or(0);
    let nominal = encoding.nominal_bps();
    if !probe.codec.is_lossless()
        && let Some(target) = nominal
        && real <= target + target * 3 / 100
    {
        return Err(SkipReason::AlreadyCompressed {
            kbps: u32::try_from(real / 1000).unwrap_or(u32::MAX),
        });
    }

    let estimated_size = estimate(probe, encoding, nominal, duration);
    let saving = 1.0 - estimated_size as f64 / probe.size.max(1) as f64;
    let percent = (saving * 100.0).floor().clamp(0.0, 100.0);
    if percent < f64::from(s.min_saving_percent) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Err(SkipReason::SavingTooSmall {
            percent: percent as u8,
        });
    }
    Ok(AudioTarget {
        encoding,
        channels,
        sample_rate,
        extension: extension.map(str::to_string),
        content,
        estimated_size,
    })
}

/// Projected output size: audio at the target bitrate, plus cover art,
/// plus a small container overhead.
fn estimate(probe: &AudioProbe, encoding: Encoding, nominal: Option<u64>, duration: f64) -> u64 {
    let audio = match (encoding, nominal) {
        (_, Some(bps)) => duration * bps as f64 / 8.0,
        // FLAC is about 55% of PCM; already-compressed lossless barely shrinks.
        (Encoding::Flac, None) => match probe.codec {
            AudioCodec::Pcm => probe.size as f64 * 0.55,
            _ => probe.size as f64,
        },
        (_, None) => probe.size as f64,
    };
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let audio = (audio * 1.005) as u64;
    audio + probe.picture_bytes
}

#[cfg(test)]
mod tests;
