//! Compression settings (docs/plans/media-compression.md). Plain data here,
//! so they can be stored with the other preferences; the decisions that use
//! them live in the `r2dock-media` crate.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Mode {
    /// MP3 stays MP3 and M4A stays M4A, so keys and links do not change.
    #[default]
    KeepFormat,
    /// Most efficient codecs (Opus); keys change extension.
    Smallest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SpeechQuality {
    #[default]
    Kbps32,
    Kbps48,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum MusicQuality {
    /// MP3 VBR quality 4, about 165 kbps.
    #[default]
    Standard,
    /// MP3 VBR quality 6, about 115 kbps.
    Smaller,
}

/// What to do with WAV, AIFF, FLAC, and ALAC sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum LosslessAction {
    /// Lossy, smallest: MP3 (Opus in Smallest mode).
    #[default]
    Mp3,
    /// Lossless, about half the size of WAV.
    Flac,
    Keep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AudioSettings {
    pub mode: Mode,
    pub speech: SpeechQuality,
    pub music: MusicQuality,
    pub lossless: LosslessAction,
    /// Minimum saving in percent; smaller savings upload the original.
    pub min_saving_percent: u8,
    /// Allow fewer channels: 5.1 and other multichannel audio mixed down to
    /// stereo (or mono for speech), and stereo speech mixed to mono. Off by
    /// default: channels are kept, except stereo whose two channels are
    /// proven identical, which becomes mono without losing anything.
    pub allow_downmix: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            mode: Mode::KeepFormat,
            speech: SpeechQuality::Kbps32,
            music: MusicQuality::Standard,
            lossless: LosslessAction::Mp3,
            min_saving_percent: 15,
            allow_downmix: false,
        }
    }
}

/// Codec for compressed video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VideoCodec {
    /// AV1: about a third of the size at the same quality. Plays in current
    /// Chrome, Edge, and Firefox, and in Safari on recent Apple devices.
    #[default]
    Av1,
    /// H.264: plays everywhere; saves less.
    H264,
}

/// Largest picture for compressed video (the shorter side, so portrait
/// phone videos are treated the same as landscape ones).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum VideoSize {
    Small720,
    #[default]
    Balanced1080,
    Keep,
    /// Do not compress video.
    Off,
}

impl VideoSize {
    /// Largest short side in pixels, if any.
    pub fn max_short_side(self) -> Option<u32> {
        match self {
            Self::Small720 => Some(720),
            Self::Balanced1080 => Some(1080),
            Self::Keep | Self::Off => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct VideoSettings {
    pub codec: VideoCodec,
    pub size: VideoSize,
}

/// What to do when compression fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CompressionFailure {
    #[default]
    UploadOriginal,
    SkipFile,
}

/// Settings > Compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct CompressionPrefs {
    /// Compress audio before upload. Off by default.
    pub enabled: bool,
    pub audio: AudioSettings,
    pub video: VideoSettings,
    pub on_failure: CompressionFailure,
    /// Files compressed at the same time (1 to 4).
    pub encode_limit: u8,
}

impl Default for CompressionPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            audio: AudioSettings::default(),
            video: VideoSettings::default(),
            on_failure: CompressionFailure::UploadOriginal,
            encode_limit: 1,
        }
    }
}

impl CompressionPrefs {
    /// Clamp numbers into their allowed ranges.
    pub fn sanitized(mut self) -> Self {
        self.audio.min_saving_percent = self.audio.min_saving_percent.clamp(5, 50);
        self.encode_limit = self.encode_limit.clamp(1, 4);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_plan() {
        let p = CompressionPrefs::default();
        assert!(!p.enabled, "off by default");
        assert_eq!(p.audio.speech, SpeechQuality::Kbps32);
        assert_eq!(p.audio.lossless, LosslessAction::Mp3);
        assert_eq!(p.audio.mode, Mode::KeepFormat);
        assert_eq!(p.audio.min_saving_percent, 15);
        assert_eq!(p.encode_limit, 1);
        assert_eq!(p.video.codec, VideoCodec::Av1);
        assert_eq!(p.video.size, VideoSize::Balanced1080);
        assert_eq!(VideoSize::Small720.max_short_side(), Some(720));
        assert_eq!(VideoSize::Keep.max_short_side(), None);
    }

    #[test]
    fn older_settings_get_video_defaults() {
        let p: CompressionPrefs = serde_json::from_str(r#"{"enabled":true}"#).unwrap();
        assert!(p.enabled);
        assert_eq!(p.video, VideoSettings::default());
    }

    #[test]
    fn sanitized_clamps() {
        let mut p = CompressionPrefs::default();
        p.audio.min_saving_percent = 99;
        p.encode_limit = 0;
        let p = p.sanitized();
        assert_eq!((p.audio.min_saving_percent, p.encode_limit), (50, 1));
    }
}
