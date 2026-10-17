//! Video decisions: what to encode, or why the file is left alone.
//!
//! Targets come from measurements on real video (see
//! docs/benchmarks/2026-10-17-video-encoders.md): SVT-AV1 at CRF 35 keeps a
//! VMAF above 96 at about 30% of a typical H.264 web video; hardware H.264
//! at high quality saves little on files that are already compressed, so the
//! sample test decides for those.

use r2dock_core::compression::{VideoCodec, VideoSettings, VideoSize};

use super::probe::{StreamKind, VideoProbe};
use crate::plan::{MIN_DURATION_SECS, MIN_SIZE, SkipReason};

/// H.264 encoders, best first. Hardware encoders are fast and use little
/// CPU; OpenH264 is the software fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Encoder {
    VideoToolbox,
    Nvenc,
    Qsv,
    Amf,
    MediaFoundation,
    Vaapi,
    OpenH264,
}

impl H264Encoder {
    pub const PRIORITY: [Self; 7] = [
        Self::VideoToolbox,
        Self::Nvenc,
        Self::Qsv,
        Self::Amf,
        Self::MediaFoundation,
        Self::Vaapi,
        Self::OpenH264,
    ];

    /// FFmpeg encoder name.
    pub fn name(self) -> &'static str {
        match self {
            Self::VideoToolbox => "h264_videotoolbox",
            Self::Nvenc => "h264_nvenc",
            Self::Qsv => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::MediaFoundation => "h264_mf",
            Self::Vaapi => "h264_vaapi",
            Self::OpenH264 => "libopenh264",
        }
    }

    pub fn is_hardware(self) -> bool {
        self != Self::OpenH264
    }
}

/// Video encoders that passed a test encode on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VideoEncoders {
    /// Best working H.264 encoder.
    pub h264: Option<H264Encoder>,
    /// OpenH264 works (fallback when a hardware encoder fails mid-file).
    pub openh264: bool,
    /// SVT-AV1 works.
    pub av1: bool,
}

/// The video encoder for one job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    SvtAv1,
    H264(H264Encoder),
}

impl VideoEncoder {
    pub fn codec_name(self) -> &'static str {
        match self {
            Self::SvtAv1 => "av1",
            Self::H264(_) => "h264",
        }
    }

    /// Short name stored in object metadata.
    pub fn preset(self) -> String {
        match self {
            Self::SvtAv1 => "av1-crf35".into(),
            Self::H264(e) => format!("h264-{}", e.name()),
        }
    }
}

/// What to do with one audio track of a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackAudio {
    Copy,
    Aac { kbps: u32 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoTarget {
    pub encoder: VideoEncoder,
    /// Index of the main video stream in the input.
    pub video_index: u32,
    /// Largest short side after scaling, when the picture is reduced.
    pub scale_short_side: Option<u32>,
    /// Keep 10-bit color (SDR 10-bit sources, AV1 only).
    pub ten_bit: bool,
    /// (input stream index, plan) for every audio track, in order.
    pub audio: Vec<(u32, TrackAudio)>,
    /// Input indexes of text subtitles, converted to `mov_text`.
    pub subtitles: Vec<u32>,
    /// Expected video bitrate, for the estimate and the OpenH264 target.
    pub video_bps: u64,
    pub estimated_size: u64,
    /// New file extension (`mp4`) when the input is not MP4.
    pub extension: Option<String>,
    /// Length of the input in seconds.
    pub duration_secs: f64,
}

/// Image subtitle codecs MP4 cannot hold.
const IMAGE_SUBTITLES: [&str; 4] = ["hdmv_pgs_subtitle", "dvd_subtitle", "dvb_subtitle", "xsub"];
/// Text subtitle codecs that convert to `mov_text`.
const TEXT_SUBTITLES: [&str; 6] = ["mov_text", "subrip", "ass", "ssa", "webvtt", "text"];
/// Data tracks that can be dropped safely (timecode, Apple metadata).
const HARMLESS_DATA: [&str; 3] = ["tmcd", "mebx", "Core Media Metadata"];
/// Codecs already as efficient as H.264 or better.
const EFFICIENT: [&str; 4] = ["hevc", "av1", "vp9", "vvc"];

/// Bitrate for 1080p at 30 fps, measured; other sizes scale from it.
fn reference_bps(encoder: VideoEncoder) -> f64 {
    match encoder {
        VideoEncoder::SvtAv1 => 1_100_000.0,
        VideoEncoder::H264(H264Encoder::OpenH264) => 4_000_000.0,
        VideoEncoder::H264(_) => 3_500_000.0,
    }
}

/// Expected bitrate for a picture of `w` x `h` at `fps`.
pub fn expected_bps(encoder: VideoEncoder, w: u32, h: u32, fps: f64) -> u64 {
    let pixels = f64::from(w.max(1)) * f64::from(h.max(1));
    let scale = (pixels / (1920.0 * 1080.0)).powf(0.75);
    let motion = (fps.clamp(10.0, 120.0) / 30.0).sqrt();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bps = (reference_bps(encoder) * scale * motion) as u64;
    bps
}

/// Picture size after scaling the short side down to `max_short` (even
/// numbers, aspect ratio kept). Rotation does not matter: only the short
/// side is compared.
pub fn scaled_size(w: u32, h: u32, max_short: Option<u32>) -> (u32, u32, bool) {
    let short = w.min(h);
    match max_short {
        Some(max) if short > max && short > 0 => {
            let ratio = f64::from(max) / f64::from(short);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let scale = |v: u32| ((f64::from(v) * ratio / 2.0).round() as u32) * 2;
            (scale(w), scale(h), true)
        }
        _ => (w, h, false),
    }
}

fn audio_plan(codec: &str, bit_rate: Option<u64>, channels: u32) -> (TrackAudio, u64) {
    let target = match channels {
        0 | 1 => 96,
        2 => 128,
        n => (64 * n).min(384),
    };
    if codec == "aac" && bit_rate.is_some_and(|b| b <= u64::from(target + 32) * 1000) {
        return (TrackAudio::Copy, bit_rate.unwrap_or(0));
    }
    (TrackAudio::Aac { kbps: target }, u64::from(target) * 1000)
}

/// Decide one video file.
pub fn plan_video(
    probe: &VideoProbe,
    settings: &VideoSettings,
    encoders: &VideoEncoders,
    min_saving_percent: u8,
    input_ext: &str,
) -> Result<VideoTarget, SkipReason> {
    if settings.size == VideoSize::Off {
        return Err(SkipReason::VideoOff);
    }
    let video = probe.video().ok_or(SkipReason::UnsupportedFormat)?;
    if probe.size < MIN_SIZE {
        return Err(SkipReason::TooSmall);
    }
    let duration = probe.duration_secs.ok_or(SkipReason::UnknownDuration)?;
    if duration < MIN_DURATION_SECS {
        return Err(SkipReason::TooShort);
    }
    if matches!(video.color_transfer.as_str(), "smpte2084" | "arib-std-b67") {
        return Err(SkipReason::Hdr);
    }
    for s in &probe.streams {
        let unsupported = match s.kind {
            StreamKind::Subtitle => IMAGE_SUBTITLES.contains(&s.codec.as_str()),
            StreamKind::Data => !HARMLESS_DATA.contains(&s.tag.as_str()),
            StreamKind::Attachment | StreamKind::Other => true,
            _ => false,
        };
        if unsupported {
            return Err(SkipReason::UnsupportedStreams);
        }
    }

    // AV1 when chosen and available; otherwise H.264.
    let encoder = match (settings.codec, encoders.av1, encoders.h264) {
        (VideoCodec::Av1, true, _) => VideoEncoder::SvtAv1,
        (_, _, Some(h)) => VideoEncoder::H264(h),
        (VideoCodec::Av1, false, None) | (VideoCodec::H264, _, None) => {
            return Err(SkipReason::NoVideoEncoder);
        }
    };
    let source = video.codec.as_str();
    let already_efficient = match encoder {
        VideoEncoder::SvtAv1 => source == "av1",
        VideoEncoder::H264(_) => EFFICIENT.contains(&source),
    };
    if already_efficient {
        return Err(SkipReason::AlreadyEfficient {
            codec: video.codec.clone(),
        });
    }

    let (w, h, scaled) = scaled_size(video.width, video.height, settings.size.max_short_side());
    let video_bps = expected_bps(encoder, w, h, video.fps.unwrap_or(30.0));
    let source_bps = probe.video_bitrate().unwrap_or(0);
    // Within 3% of the target counts as already there (container overhead).
    if !scaled && source_bps > 0 && source_bps <= video_bps + video_bps * 3 / 100 {
        return Err(SkipReason::AlreadyCompressed {
            kbps: u32::try_from(source_bps / 1000).unwrap_or(u32::MAX),
        });
    }
    let video_bps = if source_bps > 0 {
        video_bps.min(source_bps)
    } else {
        video_bps
    };

    let mut audio = Vec::new();
    let mut audio_bps = 0u64;
    for a in probe.audio() {
        let (plan, bps) = audio_plan(&a.codec, a.bit_rate, a.channels);
        audio.push((a.index, plan));
        audio_bps += bps;
    }
    let subtitles = probe
        .streams
        .iter()
        .filter(|s| s.kind == StreamKind::Subtitle && TEXT_SUBTITLES.contains(&s.codec.as_str()))
        .map(|s| s.index)
        .collect();

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let estimated_size = ((video_bps + audio_bps) as f64 * duration / 8.0 * 1.01) as u64;
    #[allow(clippy::cast_precision_loss)]
    let saving = 100.0 * (1.0 - estimated_size as f64 / probe.size.max(1) as f64);
    if saving < f64::from(min_saving_percent) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        return Err(SkipReason::SavingTooSmall {
            percent: saving.floor().clamp(0.0, 100.0) as u8,
        });
    }
    let ext = input_ext.to_ascii_lowercase();
    let extension = (ext != "mp4" && ext != "m4v").then(|| "mp4".to_string());
    let ten_bit = encoder == VideoEncoder::SvtAv1 && video.pix_fmt.contains("10");
    Ok(VideoTarget {
        encoder,
        video_index: video.index,
        scale_short_side: scaled.then(|| settings.size.max_short_side()).flatten(),
        ten_bit,
        audio,
        subtitles,
        video_bps,
        estimated_size,
        extension,
        duration_secs: duration,
    })
}

#[cfg(test)]
mod tests;
