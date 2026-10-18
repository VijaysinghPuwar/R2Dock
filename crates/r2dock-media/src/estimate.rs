//! Quick, probe-only estimates for the upload review dialog
//! ("Compress 195 audio files. Estimated 2.4 GB to 1.2 GB."). No FFmpeg is
//! started; the real numbers come later from the encode.

use std::path::Path;

use crate::classify::ContentOverride;
use crate::keys::output_key;
use crate::pipeline::Unchanged;
use crate::plan::{AudioDecision, AudioInputs, AudioSettings, plan_audio};
use crate::probe::{ProbeError, probe_audio};

/// Extensions worth probing. Content decides the codec; the extension only
/// keeps a folder of photos or documents from being opened one by one.
const AUDIO_EXTENSIONS: [&str; 13] = [
    "mp3", "m4a", "m4b", "aac", "mp4", "wav", "wave", "aif", "aiff", "aifc", "flac", "opus", "caf",
];

/// Whether a file might be audio worth compressing.
pub fn is_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileDecision {
    /// Will be compressed (subject to the sample test and final check).
    Compress {
        estimated_size: u64,
        new_key: String,
        video: bool,
    },
    /// An audio file that will be uploaded as it is.
    Unchanged(Unchanged),
    /// Not audio; compression does not apply.
    NotAudio,
}

/// Decide one file from its probe. Blocking (reads the file header).
pub fn decide_file(
    path: &Path,
    key: &str,
    settings: &AudioSettings,
    content: ContentOverride,
) -> FileDecision {
    if !is_candidate(path) {
        return FileDecision::NotAudio;
    }
    let probe = match probe_audio(path) {
        Ok(p) => p,
        Err(ProbeError::Unsupported | ProbeError::Io(_)) => return FileDecision::NotAudio,
    };
    match plan_audio(&probe, settings, AudioInputs { content }) {
        AudioDecision::Compress(t) => FileDecision::Compress {
            estimated_size: t.estimated_size,
            new_key: output_key(key, t.extension.as_deref()),
            video: false,
        },
        AudioDecision::Skip(reason) => FileDecision::Unchanged(Unchanged::Plan { reason }),
    }
}

/// Decide a video file with ffprobe (no encoding). `None` when the file
/// has no video stream (it may still be audio).
pub async fn decide_video(
    ff: &crate::ffmpeg::Ffmpeg,
    path: &Path,
    key: &str,
    audio: &AudioSettings,
    video: &r2dock_core::compression::VideoSettings,
) -> Option<FileDecision> {
    if !crate::video::run::is_video_candidate(path) {
        return None;
    }
    let probe = crate::video::probe::probe_video(ff.ffprobe.as_ref()?, path).await?;
    probe.video()?;
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    Some(
        match crate::video::plan::plan_video(
            &probe,
            video,
            &ff.video,
            audio.min_saving_percent,
            ext,
        ) {
            Ok(t) => FileDecision::Compress {
                estimated_size: t.estimated_size,
                new_key: output_key(key, t.extension.as_deref()),
                video: true,
            },
            Err(reason) => FileDecision::Unchanged(Unchanged::Plan { reason }),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn candidates() {
        for (name, want) in [
            ("a.mp3", true),
            ("A.WAV", true),
            ("book.m4b", true),
            ("x.flac", true),
            ("photo.jpg", false),
            ("notes.txt", false),
            ("noext", false),
            (".mp3", false),
        ] {
            assert_eq!(is_candidate(&PathBuf::from(name)), want, "{name}");
        }
    }

    #[test]
    fn wav_estimate_and_new_key() {
        let dir = tempfile::tempdir().unwrap();
        // 30 s of 16-bit stereo silence at 44.1 kHz (about 5.3 MB).
        let rate = 44_100u32;
        let data_len = rate * 2 * 2 * 30;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 4).to_le_bytes());
        v.extend_from_slice(&4u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        v.resize(v.len() + data_len as usize, 0);
        let path = dir.path().join("Take 1.wav");
        std::fs::write(&path, &v).unwrap();
        let d = decide_file(
            &path,
            "raw/Take 1.wav",
            &AudioSettings::default(),
            ContentOverride::Auto,
        );
        let FileDecision::Compress {
            estimated_size,
            new_key,
            ..
        } = d
        else {
            panic!("{d:?}");
        };
        assert_eq!(new_key, "raw/Take 1.mp3");
        assert!(estimated_size < v.len() as u64 / 5, "{estimated_size}");

        let text = dir.path().join("fake.mp3");
        std::fs::write(&text, b"not audio").unwrap();
        assert_eq!(
            decide_file(
                &text,
                "fake.mp3",
                &AudioSettings::default(),
                ContentOverride::Auto
            ),
            FileDecision::NotAudio
        );
    }
}
