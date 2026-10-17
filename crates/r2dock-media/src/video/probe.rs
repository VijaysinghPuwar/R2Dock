//! Video probing with ffprobe's JSON output (Symphonia cannot read video).

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::ffmpeg::args::file_url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Data,
    Attachment,
    Other,
}

/// One stream as R2Dock needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub index: u32,
    pub kind: StreamKind,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    pub color_transfer: String,
    /// Bits per second, when known.
    pub bit_rate: Option<u64>,
    pub channels: u32,
    /// Cover art inside a video file.
    pub attached_pic: bool,
    /// Rotation in degrees from the display matrix (phone videos).
    pub rotation: i32,
    /// Frames per second (average).
    pub fps: Option<f64>,
    /// Data track handler or tag (for example `gpmd` for GoPro telemetry).
    pub tag: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoProbe {
    pub size: u64,
    pub duration_secs: Option<f64>,
    pub streams: Vec<StreamInfo>,
    /// The container's own bitrate (bits per second), when known.
    pub bit_rate: Option<u64>,
}

impl VideoProbe {
    /// The main video stream (not cover art).
    pub fn video(&self) -> Option<&StreamInfo> {
        self.streams
            .iter()
            .find(|s| s.kind == StreamKind::Video && !s.attached_pic)
    }

    pub fn audio(&self) -> impl Iterator<Item = &StreamInfo> {
        self.streams.iter().filter(|s| s.kind == StreamKind::Audio)
    }

    /// Real average video bitrate: the stream's own figure, or the file's
    /// bitrate minus audio.
    pub fn video_bitrate(&self) -> Option<u64> {
        if let Some(b) = self.video().and_then(|v| v.bit_rate) {
            return Some(b);
        }
        let secs = self.duration_secs.filter(|d| *d > 0.0)?;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let total = self
            .bit_rate
            .unwrap_or((self.size as f64 * 8.0 / secs) as u64);
        let audio: u64 = self.audio().filter_map(|a| a.bit_rate).sum();
        Some(total.saturating_sub(audio))
    }
}

#[derive(Deserialize)]
struct Raw {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawStream {
    index: u32,
    codec_type: String,
    codec_name: String,
    codec_tag_string: String,
    width: u32,
    height: u32,
    pix_fmt: String,
    color_transfer: String,
    bit_rate: Option<String>,
    channels: u32,
    avg_frame_rate: String,
    disposition: RawDisposition,
    side_data_list: Vec<RawSideData>,
    tags: RawTags,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawDisposition {
    attached_pic: u8,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawSideData {
    rotation: Option<f64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawTags {
    rotate: Option<String>,
    handler_name: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawFormat {
    duration: Option<String>,
    size: Option<String>,
    bit_rate: Option<String>,
}

fn fraction(s: &str) -> Option<f64> {
    let (a, b) = s.split_once('/')?;
    let (a, b): (f64, f64) = (a.parse().ok()?, b.parse().ok()?);
    (b > 0.0 && a > 0.0).then(|| a / b)
}

/// Parse ffprobe's `-show_format -show_streams` JSON.
pub fn parse(json: &str) -> Option<VideoProbe> {
    let raw: Raw = serde_json::from_str(json).ok()?;
    let format = raw.format.unwrap_or_default();
    let streams = raw
        .streams
        .into_iter()
        .map(|s| {
            let kind = match s.codec_type.as_str() {
                "video" => StreamKind::Video,
                "audio" => StreamKind::Audio,
                "subtitle" => StreamKind::Subtitle,
                "data" => StreamKind::Data,
                "attachment" => StreamKind::Attachment,
                _ => StreamKind::Other,
            };
            let rotation = s
                .side_data_list
                .iter()
                .find_map(|d| d.rotation)
                .or_else(|| s.tags.rotate.as_deref().and_then(|r| r.parse().ok()))
                .unwrap_or(0.0);
            #[allow(clippy::cast_possible_truncation)]
            let rotation = rotation.round() as i32;
            let tag = if s.codec_tag_string.is_empty() || s.codec_tag_string.starts_with("[0]") {
                s.tags.handler_name.unwrap_or_default()
            } else {
                s.codec_tag_string
            };
            StreamInfo {
                index: s.index,
                kind,
                codec: s.codec_name,
                width: s.width,
                height: s.height,
                pix_fmt: s.pix_fmt,
                color_transfer: s.color_transfer,
                bit_rate: s.bit_rate.and_then(|b| b.parse().ok()),
                channels: s.channels,
                attached_pic: s.disposition.attached_pic == 1,
                rotation,
                fps: fraction(&s.avg_frame_rate),
                tag,
            }
        })
        .collect();
    Some(VideoProbe {
        size: format.size.and_then(|s| s.parse().ok()).unwrap_or(0),
        duration_secs: format.duration.and_then(|d| d.parse().ok()),
        streams,
        bit_rate: format.bit_rate.and_then(|b| b.parse().ok()),
    })
}

/// Run ffprobe on a file. `None` when it is not something ffprobe reads.
pub async fn probe_video(ffprobe: &Path, input: &Path) -> Option<VideoProbe> {
    let url = file_url(input).ok()?;
    let mut cmd = tokio::process::Command::new(ffprobe);
    cmd.args([
        "-hide_banner",
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-print_format",
        "json",
        "-show_format",
        "-show_streams",
    ])
    .arg(url)
    .stdin(std::process::Stdio::null())
    .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let out = tokio::time::timeout(Duration::from_secs(60), cmd.output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut probe = parse(&String::from_utf8_lossy(&out.stdout))?;
    if probe.size == 0 {
        probe.size = std::fs::metadata(input).map(|m| m.len()).unwrap_or(0);
    }
    Some(probe)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: &str = r#"{
      "streams": [
        {"index": 0, "codec_type": "video", "codec_name": "hevc", "codec_tag_string": "hvc1",
         "width": 1920, "height": 1080, "pix_fmt": "yuv420p10le", "color_transfer": "arib-std-b67",
         "bit_rate": "9000000", "avg_frame_rate": "30000/1001",
         "side_data_list": [{"side_data_type": "Display Matrix", "rotation": -90}],
         "disposition": {"attached_pic": 0}},
        {"index": 1, "codec_type": "audio", "codec_name": "aac", "channels": 2, "bit_rate": "190000"},
        {"index": 2, "codec_type": "data", "codec_name": "none", "codec_tag_string": "mebx",
         "tags": {"handler_name": "Core Media Metadata"}},
        {"index": 3, "codec_type": "video", "codec_name": "mjpeg", "width": 300, "height": 300,
         "disposition": {"attached_pic": 1}}
      ],
      "format": {"duration": "12.5", "size": "15000000", "bit_rate": "9600000"}
    }"#;

    #[test]
    fn parses_a_phone_video() {
        let p = parse(PHONE).unwrap();
        let v = p.video().unwrap();
        assert_eq!((v.codec.as_str(), v.width, v.height), ("hevc", 1920, 1080));
        assert_eq!(v.rotation, -90);
        assert_eq!(v.color_transfer, "arib-std-b67");
        assert!((v.fps.unwrap() - 29.97).abs() < 0.01);
        assert_eq!(p.audio().count(), 1);
        assert_eq!(p.streams[2].kind, StreamKind::Data);
        assert_eq!(p.streams[2].tag, "mebx");
        assert!(p.streams[3].attached_pic, "cover art is not the main video");
        assert_eq!(p.size, 15_000_000);
        assert_eq!(p.duration_secs, Some(12.5));
        assert_eq!(p.video_bitrate(), Some(9_000_000));
    }

    #[test]
    fn video_bitrate_falls_back_to_file_minus_audio() {
        let json = r#"{"streams":[{"index":0,"codec_type":"video","codec_name":"h264"},
            {"index":1,"codec_type":"audio","codec_name":"aac","bit_rate":"128000"}],
            "format":{"duration":"10","size":"5000000","bit_rate":"4000000"}}"#;
        assert_eq!(parse(json).unwrap().video_bitrate(), Some(3_872_000));
        assert!(parse("not json").is_none());
        assert_eq!(fraction("0/0"), None);
    }
}
