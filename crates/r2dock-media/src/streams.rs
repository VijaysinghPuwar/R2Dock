//! Stream inventory: what a file holds, and whether a compressed copy holds
//! the same. A smaller file is only a success when nothing was dropped on
//! the way: every audio, video, and subtitle track the plan promised to keep
//! must be in the output, with its channels and language.
//!
//! Deliberate exceptions, all listed here: cover art (an attached picture)
//! is not a video track; timecode, Apple camera metadata, and the QuickTime
//! chapter track are not content (chapters are copied with
//! `-map_chapters`); cover art in an Opus file is not kept.

use crate::verify::VerifyError;
use crate::video::probe::{StreamInfo, StreamKind, VideoProbe};

/// Data track tags that hold nothing the compressed copy must carry:
/// timecode, Apple camera metadata, and the QuickTime chapter track.
const NOT_CONTENT: [&str; 4] = ["tmcd", "mebx", "Core Media Metadata", "text"];

/// Whether an audio-only file holds only what the audio path keeps: one
/// audio track, plus cover art and chapters.
pub fn audio_only_supported(probe: &VideoProbe) -> bool {
    let mut audio = 0;
    for s in &probe.streams {
        match s.kind {
            StreamKind::Audio => audio += 1,
            StreamKind::Video if s.attached_pic => {}
            StreamKind::Data if NOT_CONTENT.contains(&s.tag.as_str()) => {}
            _ => return false,
        }
    }
    audio == 1
}

/// Video tracks that are not cover art.
pub fn video_tracks(probe: &VideoProbe) -> usize {
    probe
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Video && !s.attached_pic)
        .count()
}

/// One audio track the output must hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedAudio {
    /// Channel count, when known.
    pub channels: Option<u32>,
    /// Language to keep. `None` when the output format has no per-track
    /// language (MP3, FLAC) and there is only one track to tell apart.
    pub language: Option<String>,
}

impl ExpectedAudio {
    /// The source track as it is, with `channels` in the output.
    pub fn from_source(source: &StreamInfo, channels: Option<u32>) -> Self {
        Self {
            channels,
            language: Some(source.language.clone()),
        }
    }
}

/// What the plan promised the output holds, in order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expected {
    pub video: usize,
    pub audio: Vec<ExpectedAudio>,
    pub subtitles: usize,
}

/// No language and `und` (undetermined) mean the same.
fn same_language(a: &str, b: &str) -> bool {
    let norm = |s: &str| if s.is_empty() { "und" } else { s }.to_ascii_lowercase();
    norm(a) == norm(b)
}

/// Check the output's inventory against the plan.
pub fn check(output: &VideoProbe, expected: &Expected) -> Result<(), VerifyError> {
    let subtitles = output
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Subtitle)
        .count();
    let audio: Vec<&StreamInfo> = output.audio().collect();
    if video_tracks(output) != expected.video
        || audio.len() != expected.audio.len()
        || subtitles != expected.subtitles
    {
        return Err(VerifyError::StreamsChanged);
    }
    for (got, want) in audio.iter().zip(&expected.audio) {
        if let Some(channels) = want.channels
            && got.channels != channels
        {
            return Err(VerifyError::WrongChannels {
                expected: channels,
                got: got.channels,
            });
        }
        if want
            .language
            .as_deref()
            .is_some_and(|lang| !same_language(&got.language, lang))
        {
            return Err(VerifyError::StreamsChanged);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::probe::parse;

    fn probe(streams: &str) -> VideoProbe {
        parse(&format!(
            r#"{{"streams":[{streams}],"format":{{"duration":"60","size":"5000000"}}}}"#
        ))
        .unwrap()
    }

    const AUDIO: &str = r#"{"index":0,"codec_type":"audio","codec_name":"aac","channels":2}"#;
    const COVER: &str =
        r#"{"index":1,"codec_type":"video","codec_name":"mjpeg","disposition":{"attached_pic":1}}"#;
    const CHAPTERS: &str =
        r#"{"index":2,"codec_type":"data","codec_name":"bin_data","codec_tag_string":"text"}"#;

    #[test]
    fn audio_only_inventory() {
        let ok = [
            AUDIO.to_string(),
            format!("{AUDIO},{COVER}"),
            format!("{AUDIO},{COVER},{CHAPTERS}"),
        ];
        for streams in ok {
            assert!(audio_only_supported(&probe(&streams)), "{streams}");
        }
        let second = r#"{"index":3,"codec_type":"audio","codec_name":"aac","channels":1}"#;
        let ttml = r#"{"index":3,"codec_type":"subtitle","codec_name":"ttml"}"#;
        let video = r#"{"index":3,"codec_type":"video","codec_name":"h264"}"#;
        let gpmd =
            r#"{"index":3,"codec_type":"data","codec_name":"bin_data","codec_tag_string":"gpmd"}"#;
        let attachment = r#"{"index":3,"codec_type":"attachment","codec_name":"ttf"}"#;
        for extra in [second, ttml, video, gpmd, attachment] {
            let streams = format!("{AUDIO},{extra}");
            assert!(!audio_only_supported(&probe(&streams)), "{streams}");
        }
        assert!(!audio_only_supported(&probe(COVER)), "no audio at all");
    }

    fn expected(audio: &[(Option<u32>, &str)], video: usize, subtitles: usize) -> Expected {
        Expected {
            video,
            audio: audio
                .iter()
                .map(|(channels, language)| ExpectedAudio {
                    channels: *channels,
                    language: Some((*language).to_string()),
                })
                .collect(),
            subtitles,
        }
    }

    #[test]
    fn output_must_match_the_plan() {
        let out = probe(
            r#"{"index":0,"codec_type":"video","codec_name":"av1"},
               {"index":1,"codec_type":"audio","codec_name":"aac","channels":2,"tags":{"language":"eng"}},
               {"index":2,"codec_type":"audio","codec_name":"aac","channels":6,"tags":{"language":"spa"}},
               {"index":3,"codec_type":"subtitle","codec_name":"mov_text"},
               {"index":4,"codec_type":"data","codec_name":"bin_data","codec_tag_string":"text"}"#,
        );
        let good = expected(&[(Some(2), "eng"), (Some(6), "spa")], 1, 1);
        assert_eq!(check(&out, &good), Ok(()));
        let unknown_channels = expected(&[(None, "eng"), (None, "spa")], 1, 1);
        assert_eq!(check(&out, &unknown_channels), Ok(()));

        let missing_audio = expected(&[(Some(2), "eng")], 1, 1);
        assert_eq!(
            check(&out, &missing_audio),
            Err(VerifyError::StreamsChanged)
        );
        let missing_subtitle = expected(&[(Some(2), "eng"), (Some(6), "spa")], 1, 2);
        assert_eq!(
            check(&out, &missing_subtitle),
            Err(VerifyError::StreamsChanged)
        );
        let missing_video = expected(&[(Some(2), "eng"), (Some(6), "spa")], 2, 1);
        assert_eq!(
            check(&out, &missing_video),
            Err(VerifyError::StreamsChanged)
        );
        let downmixed = expected(&[(Some(2), "eng"), (Some(8), "spa")], 1, 1);
        assert_eq!(
            check(&out, &downmixed),
            Err(VerifyError::WrongChannels {
                expected: 8,
                got: 6
            })
        );
        let swapped = expected(&[(Some(2), "spa"), (Some(6), "eng")], 1, 1);
        assert_eq!(check(&out, &swapped), Err(VerifyError::StreamsChanged));
    }

    #[test]
    fn undetermined_language_matches_none() {
        assert!(same_language("", "und"));
        assert!(same_language("ENG", "eng"));
        assert!(!same_language("eng", "und"));
    }
}
