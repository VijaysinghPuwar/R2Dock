//! Speech or music, from what the probe found.

use serde::{Deserialize, Serialize};

use crate::probe::AudioProbe;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Content {
    Speech,
    Music,
}

/// A per-upload choice that overrides the automatic guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentOverride {
    #[default]
    Auto,
    Speech,
    Music,
}

/// Highest sample rate that still counts as speech.
const SPEECH_MAX_RATE: u32 = 24_000;
/// Highest real bitrate that still counts as speech.
const SPEECH_MAX_BPS: u64 = 64_000;

/// Speech if mono, or 24 kHz or lower, or 64 kbps or lower; otherwise music.
/// Lossless sources are judged by channels and sample rate only, since their
/// bitrate says nothing about the content.
pub fn classify(probe: &AudioProbe, choice: ContentOverride) -> Content {
    match choice {
        ContentOverride::Speech => return Content::Speech,
        ContentOverride::Music => return Content::Music,
        ContentOverride::Auto => {}
    }
    let mono = probe.channels == Some(1);
    let low_rate = probe.sample_rate.is_some_and(|r| r <= SPEECH_MAX_RATE);
    let low_bitrate =
        !probe.codec.is_lossless() && probe.real_bitrate().is_some_and(|b| b <= SPEECH_MAX_BPS);
    if mono || low_rate || low_bitrate {
        Content::Speech
    } else {
        Content::Music
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::AudioCodec;

    fn probe(codec: AudioCodec, rate: u32, channels: u8, kbps: u64) -> AudioProbe {
        AudioProbe {
            codec,
            sample_rate: Some(rate),
            channels: Some(channels),
            duration_secs: Some(1000.0),
            size: kbps * 1000 * 1000 / 8,
            picture_bytes: 0,
            has_video: false,
            marker: None,
        }
    }

    #[test]
    fn decision_table() {
        use AudioCodec::*;
        use Content::*;
        let table = [
            (probe(Mp3, 22_050, 1, 64), Speech, "mono audiobook"),
            (probe(Mp3, 24_000, 1, 48), Speech, "mono 48 kbps"),
            (probe(Mp3, 44_100, 2, 64), Speech, "low bitrate stereo"),
            (probe(Mp3, 22_050, 2, 128), Speech, "low sample rate"),
            (probe(Mp3, 44_100, 2, 192), Music, "stereo song"),
            (probe(Aac, 48_000, 2, 256), Music, "AAC song"),
            (probe(Pcm, 44_100, 2, 1411), Music, "CD quality WAV"),
            (probe(Pcm, 44_100, 1, 705), Speech, "mono WAV"),
            (probe(Flac, 48_000, 2, 40), Music, "FLAC ignores bitrate"),
        ];
        for (p, want, what) in table {
            assert_eq!(classify(&p, ContentOverride::Auto), want, "{what}");
        }
    }

    #[test]
    fn override_wins() {
        let song = probe(AudioCodec::Mp3, 44_100, 2, 192);
        assert_eq!(classify(&song, ContentOverride::Speech), Content::Speech);
        let book = probe(AudioCodec::Mp3, 22_050, 1, 64);
        assert_eq!(classify(&book, ContentOverride::Music), Content::Music);
    }
}
