//! Fixed FFmpeg argument arrays. Never a shell, never a string to split.
//!
//! - Inputs and outputs are passed as `file:<absolute path>` with
//!   `-protocol_whitelist file`, so a crafted name such as `concat:a|b.mp3`,
//!   `http://x.mp3`, or `-i.mp3` can never be read as a URL or an option.
//! - `-nostdin -y` and an explicit output path mean FFmpeg never waits on a
//!   prompt. Progress goes to stdout with `-progress pipe:1 -nostats`.

use std::ffi::OsString;
use std::path::Path;

use thiserror::Error;

use crate::plan::{AudioTarget, Encoding};
use crate::probe::MARKER;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArgsError {
    #[error("paths given to FFmpeg must be absolute")]
    RelativePath,
    #[error("no output format for this file extension")]
    UnknownContainer,
}

/// Output container, chosen from the output file's extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp3,
    /// `.m4a` and `.m4b` (iPod-style MP4 audio).
    Ipod,
    Mp4,
    Adts,
    Opus,
    Flac,
}

impl Container {
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "mp3" => Self::Mp3,
            "m4a" | "m4b" => Self::Ipod,
            "mp4" => Self::Mp4,
            "aac" => Self::Adts,
            "opus" => Self::Opus,
            "flac" => Self::Flac,
            _ => return None,
        })
    }

    fn muxer(self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Ipod => "ipod",
            Self::Mp4 => "mp4",
            Self::Adts => "adts",
            Self::Opus => "opus",
            Self::Flac => "flac",
        }
    }

    /// Whether cover art (an attached picture stream) can be kept.
    fn keeps_cover_art(self) -> bool {
        matches!(self, Self::Mp3 | Self::Ipod | Self::Mp4 | Self::Flac)
    }

    fn is_mp4(self) -> bool {
        matches!(self, Self::Ipod | Self::Mp4)
    }
}

/// `file:` URL for an absolute path.
pub fn file_url(path: &Path) -> Result<OsString, ArgsError> {
    if !path.is_absolute() {
        return Err(ArgsError::RelativePath);
    }
    let mut s = OsString::from("file:");
    s.push(path.as_os_str());
    Ok(s)
}

fn common_start() -> Vec<OsString> {
    ["-hide_banner", "-nostdin", "-y", "-loglevel", "error"]
        .into_iter()
        .map(OsString::from)
        .collect()
}

fn push(args: &mut Vec<OsString>, items: &[&str]) {
    args.extend(items.iter().map(OsString::from));
}

/// A part of the input to encode instead of the whole file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub start_secs: f64,
    pub len_secs: f64,
}

/// Arguments to encode `input` to `output` as `target`, with the R2Dock
/// marker tag. `window` limits the encode to a sample.
pub fn encode_audio(
    input: &Path,
    output: &Path,
    target: &AudioTarget,
    window: Option<Window>,
) -> Result<Vec<OsString>, ArgsError> {
    let container = Container::from_path(output).ok_or(ArgsError::UnknownContainer)?;
    let mut a = common_start();
    push(&mut a, &["-protocol_whitelist", "file"]);
    if let Some(w) = window {
        push(
            &mut a,
            &[
                "-ss",
                &format!("{:.3}", w.start_secs),
                "-t",
                &format!("{:.3}", w.len_secs),
            ],
        );
    }
    a.push("-i".into());
    a.push(file_url(input)?);

    push(&mut a, &["-map", "0:a:0"]);
    if container.keeps_cover_art() && window.is_none() {
        push(
            &mut a,
            &[
                "-map",
                "0:v?",
                "-c:v",
                "copy",
                "-disposition:v",
                "attached_pic",
            ],
        );
    } else {
        a.push("-vn".into());
    }
    push(&mut a, &["-map_metadata", "0", "-map_chapters", "0"]);

    match target.encoding {
        Encoding::Mp3Cbr { kbps } => {
            push(&mut a, &["-c:a", "libmp3lame", "-b:a", &format!("{kbps}k")]);
        }
        Encoding::Mp3Vbr { quality, .. } => {
            push(
                &mut a,
                &["-c:a", "libmp3lame", "-q:a", &quality.to_string()],
            );
        }
        Encoding::Aac { kbps } => push(&mut a, &["-c:a", "aac", "-b:a", &format!("{kbps}k")]),
        Encoding::Opus { kbps } => {
            let application = match target.content {
                crate::classify::Content::Speech => "voip",
                crate::classify::Content::Music => "audio",
            };
            push(
                &mut a,
                &[
                    "-c:a",
                    "libopus",
                    "-b:a",
                    &format!("{kbps}k"),
                    "-application",
                    application,
                ],
            );
        }
        Encoding::Flac => push(&mut a, &["-c:a", "flac", "-compression_level", "8"]),
    }
    push(&mut a, &["-ac", &target.channels.to_string()]);
    if let Some(rate) = target.sample_rate {
        push(&mut a, &["-ar", &rate.to_string()]);
    }
    match container {
        // ID3v2.3 is the most widely read version; the Xing/LAME header
        // keeps gapless playback and an accurate duration.
        Container::Mp3 => push(&mut a, &["-id3v2_version", "3", "-write_xing", "1"]),
        c if c.is_mp4() => push(&mut a, &["-movflags", "+faststart"]),
        _ => {}
    }
    // MP4 gets no marker: FFmpeg writes custom MP4 tags only with
    // `use_metadata_tags`, which stores every other tag (title, author) in
    // a form many players ignore. The bitrate checks in `plan` keep MP4
    // output from being compressed twice.
    if !container.is_mp4() {
        push(
            &mut a,
            &[
                "-metadata",
                &format!("{MARKER}={}", target.encoding.preset()),
            ],
        );
    }
    push(
        &mut a,
        &["-progress", "pipe:1", "-nostats", "-f", container.muxer()],
    );
    a.push(file_url(output)?);
    Ok(a)
}

#[cfg(test)]
mod tests;
