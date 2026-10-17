//! What an FFmpeg binary can do: version, license, and encoders.
//! Used for the downloaded tools and for an FFmpeg the user picked.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{Tools, ToolsError, ToolsSource};

/// License of an FFmpeg build, from its configure flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum License {
    LgplV21,
    LgplV3,
    GplV2,
    GplV3,
    /// Built with `--enable-nonfree`; cannot be redistributed.
    Nonfree,
}

/// Audio encoders R2Dock uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioEncoders {
    pub mp3: bool,
    pub aac: bool,
    pub opus: bool,
    pub flac: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub version: String,
    pub license: License,
    pub audio: AudioEncoders,
    /// Every encoder name, for the video encoder choice.
    pub encoders: Vec<String>,
}

/// Version from the first line of `ffmpeg -version`
/// (`ffmpeg version n9.0.2-17-g2a571b6068 Copyright ...` gives `9.0.2`).
pub fn parse_version(text: &str) -> Option<String> {
    let first = text.lines().next()?;
    let raw = first
        .strip_prefix("ffmpeg version ")?
        .split_whitespace()
        .next()?;
    let raw = raw.strip_prefix('n').unwrap_or(raw);
    let version: String = raw
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if version.is_empty() {
        // Development builds: `N-127032-g6ae491a26c`.
        Some(raw.to_string())
    } else {
        Some(version.trim_end_matches('.').to_string())
    }
}

/// License from the `configuration:` line of `ffmpeg -version`.
pub fn parse_license(text: &str) -> License {
    let config = text
        .lines()
        .find(|l| l.trim_start().starts_with("configuration:"))
        .unwrap_or("");
    let has = |flag: &str| config.split_whitespace().any(|f| f == flag);
    match (
        has("--enable-nonfree"),
        has("--enable-gpl"),
        has("--enable-version3"),
    ) {
        (true, _, _) => License::Nonfree,
        (false, true, true) => License::GplV3,
        (false, true, false) => License::GplV2,
        (false, false, true) => License::LgplV3,
        (false, false, false) => License::LgplV21,
    }
}

/// Encoder names from `ffmpeg -hide_banner -encoders`. Lines look like
/// ` A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3)`.
pub fn parse_encoders(text: &str) -> Vec<String> {
    let mut seen_header = false;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim_start().starts_with("------") {
            seen_header = true;
            continue;
        }
        if !seen_header {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(flags), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if flags.len() == 6 && flags.starts_with(['A', 'V', 'S']) {
            out.push(name.to_string());
        }
    }
    out
}

pub fn audio_encoders(encoders: &[String]) -> AudioEncoders {
    let has = |n: &str| encoders.iter().any(|e| e == n);
    AudioEncoders {
        mp3: has("libmp3lame"),
        aac: has("aac"),
        opus: has("libopus"),
        flac: has("flac"),
    }
}

async fn output(bin: &Path, args: &[&str]) -> Result<String, ToolsError> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(Duration::from_secs(20), cmd.output())
        .await
        .map_err(|_| ToolsError::Unusable("it did not answer".into()))?
        .map_err(|_| ToolsError::Unusable("it could not be started".into()))?;
    if !out.status.success() {
        return Err(ToolsError::Unusable("it reported an error".into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Ask a binary what it is and what it can encode.
pub async fn inspect(ffmpeg: &Path) -> Result<BuildInfo, ToolsError> {
    let version_text = output(ffmpeg, &["-hide_banner", "-version"]).await?;
    // `-hide_banner` drops the first line of `-version` on some builds.
    let version_text = if version_text.starts_with("ffmpeg version") {
        version_text
    } else {
        output(ffmpeg, &["-version"]).await?
    };
    let version = parse_version(&version_text)
        .ok_or_else(|| ToolsError::Unusable("this is not FFmpeg".into()))?;
    let encoders = parse_encoders(&output(ffmpeg, &["-hide_banner", "-encoders"]).await?);
    Ok(BuildInfo {
        version,
        license: parse_license(&version_text),
        audio: audio_encoders(&encoders),
        encoders,
    })
}

/// Check an FFmpeg the user picked. ffprobe must be next to it.
pub async fn use_system(ffmpeg: &Path) -> Result<(Tools, BuildInfo), ToolsError> {
    let meta =
        std::fs::metadata(ffmpeg).map_err(|_| ToolsError::Unusable("file not found".into()))?;
    if !meta.is_file() {
        return Err(ToolsError::Unusable(
            "choose the ffmpeg program file".into(),
        ));
    }
    let info = inspect(ffmpeg).await?;
    if !info.audio.mp3 {
        return Err(ToolsError::Unusable(
            "it has no MP3 encoder (libmp3lame)".into(),
        ));
    }
    let ffprobe = sibling_ffprobe(ffmpeg);
    if !ffprobe.is_file() {
        return Err(ToolsError::Unusable(
            "ffprobe is not in the same folder".into(),
        ));
    }
    let tools = Tools {
        ffmpeg: ffmpeg.to_path_buf(),
        ffprobe,
        version: info.version.clone(),
        source: ToolsSource::System,
    };
    Ok((tools, info))
}

fn sibling_ffprobe(ffmpeg: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    ffmpeg.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION_TEXT: &str = "ffmpeg version n9.0.2-17-g2a571b6068-20260930 Copyright (c) 2000-2026 the FFmpeg developers\n\
        built with gcc 15.2.0 (crosstool-NG 1.28.0.1_403899e)\n\
        configuration: --prefix=/ffbuild/prefix --pkg-config-flags=--static --enable-version3 --disable-debug --enable-libmp3lame --enable-libopus\n\
        libavutil      60.  8.100 / 60.  8.100\n";

    #[test]
    fn versions() {
        assert_eq!(parse_version(VERSION_TEXT).as_deref(), Some("9.0.2"));
        assert_eq!(
            parse_version("ffmpeg version 9.0.1 Copyright (c) 2000-2026").as_deref(),
            Some("9.0.1")
        );
        assert_eq!(
            parse_version("ffmpeg version 4.4.2-0ubuntu0.22.04.1 Copyright").as_deref(),
            Some("4.4.2")
        );
        assert_eq!(
            parse_version("ffmpeg version N-127032-g6ae491a26c-20260930 Copyright").as_deref(),
            Some("N-127032-g6ae491a26c-20260930")
        );
        assert_eq!(parse_version("ffprobe version 9.0"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn licenses() {
        assert_eq!(parse_license(VERSION_TEXT), License::LgplV3);
        let cfg = |flags: &str| format!("ffmpeg version 9\nconfiguration: {flags}\n");
        assert_eq!(
            parse_license(&cfg("--enable-gpl --enable-version3")),
            License::GplV3
        );
        assert_eq!(parse_license(&cfg("--enable-gpl")), License::GplV2);
        assert_eq!(
            parse_license(&cfg("--enable-gpl --enable-nonfree")),
            License::Nonfree
        );
        assert_eq!(parse_license(&cfg("--prefix=/x")), License::LgplV21);
        // A flag name inside another value does not count.
        assert_eq!(
            parse_license(&cfg("--extra-cflags=--enable-gpl-ish")),
            License::LgplV21
        );
    }

    #[test]
    fn encoders() {
        let text = "Encoders:\n V..... = Video\n A..... = Audio\n ------\n \
            V....D libx264              libx264 H.264\n \
            V....D h264_videotoolbox    VideoToolbox H.264 Encoder (codec h264)\n \
            A....D aac                  AAC (Advanced Audio Coding)\n \
            A....D flac                 FLAC (Free Lossless Audio Codec)\n \
            A....D libmp3lame           libmp3lame MP3 (MPEG audio layer 3) (codec mp3)\n \
            A....D libopus              libopus Opus (codec opus)\n \
            S..... mov_text             3GPP Timed Text subtitle\n";
        let list = parse_encoders(text);
        assert_eq!(list.len(), 7);
        assert!(list.contains(&"h264_videotoolbox".to_string()));
        let audio = audio_encoders(&list);
        assert_eq!(
            audio,
            AudioEncoders {
                mp3: true,
                aac: true,
                opus: true,
                flac: true
            }
        );
        assert_eq!(audio_encoders(&[]), AudioEncoders::default());
    }
}
