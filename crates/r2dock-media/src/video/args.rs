//! FFmpeg arguments for video. Same rules as audio: an argument array,
//! `file:` paths with `-protocol_whitelist file`, no prompts, progress on
//! stdout. Output is always MP4 with `+faststart` so playback can start
//! before the download finishes.

use std::ffi::OsString;
use std::path::Path;

use super::plan::{H264Encoder, TrackAudio, VideoEncoder, VideoTarget};
use crate::ffmpeg::args::{ArgsError, Window, file_url};

/// VAAPI render device on Linux.
const VAAPI_DEVICE: &str = "/dev/dri/renderD128";

fn push(args: &mut Vec<OsString>, items: &[&str]) {
    args.extend(items.iter().map(OsString::from));
}

/// Video filter: scale the short side down (after FFmpeg's automatic
/// rotation, so phone videos are not sideways), then the pixel format.
fn video_filter(target: &VideoTarget, encoder: VideoEncoder) -> String {
    let mut parts = Vec::new();
    if let Some(s) = target.scale_short_side {
        parts.push(format!(
            "scale=w='if(gte(iw,ih),-2,{s})':h='if(gte(iw,ih),{s},-2)'"
        ));
    }
    match encoder {
        VideoEncoder::SvtAv1 if target.ten_bit => parts.push("format=yuv420p10le".into()),
        VideoEncoder::H264(H264Encoder::Vaapi) => parts.push("format=nv12,hwupload".into()),
        _ => parts.push("format=yuv420p".into()),
    }
    parts.join(",")
}

fn encoder_args(a: &mut Vec<OsString>, encoder: VideoEncoder, video_bps: u64) {
    match encoder {
        // Measured: CRF 35 keeps VMAF above 96 at about 30% of the source.
        VideoEncoder::SvtAv1 => push(
            a,
            &[
                "-c:v",
                "libsvtav1",
                "-crf",
                "35",
                "-preset",
                "8",
                "-g",
                "240",
            ],
        ),
        VideoEncoder::H264(e) => {
            push(a, &["-c:v", e.name()]);
            match e {
                H264Encoder::VideoToolbox => push(a, &["-q:v", "60", "-profile:v", "high"]),
                H264Encoder::Nvenc => push(
                    a,
                    &[
                        "-preset",
                        "p5",
                        "-rc",
                        "vbr",
                        "-cq",
                        "23",
                        "-b:v",
                        "0",
                        "-profile:v",
                        "high",
                    ],
                ),
                H264Encoder::Qsv => push(a, &["-global_quality", "23", "-profile:v", "high"]),
                H264Encoder::Amf => push(
                    a,
                    &["-rc", "cqp", "-qp_i", "22", "-qp_p", "24", "-qp_b", "26"],
                ),
                H264Encoder::MediaFoundation => {
                    push(a, &["-rate_control", "quality", "-quality", "60"]);
                }
                H264Encoder::Vaapi => push(a, &["-qp", "23"]),
                // OpenH264 has no constant-quality mode: target the estimate.
                H264Encoder::OpenH264 => {
                    let bps = video_bps.max(300_000);
                    push(
                        a,
                        &[
                            "-b:v",
                            &bps.to_string(),
                            "-maxrate",
                            &(bps * 3 / 2).to_string(),
                            "-bufsize",
                            &(bps * 3).to_string(),
                        ],
                    );
                }
            }
        }
    }
}

/// Arguments to encode a video. `encoder` overrides the target's encoder
/// (used for the software retry after a hardware failure).
pub fn encode_video(
    input: &Path,
    output: &Path,
    target: &VideoTarget,
    encoder: VideoEncoder,
    window: Option<Window>,
) -> Result<Vec<OsString>, ArgsError> {
    let mut a: Vec<OsString> = Vec::new();
    push(
        &mut a,
        &["-hide_banner", "-nostdin", "-y", "-loglevel", "error"],
    );
    if encoder == VideoEncoder::H264(H264Encoder::Vaapi) {
        push(&mut a, &["-vaapi_device", VAAPI_DEVICE]);
    }
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

    push(&mut a, &["-map", &format!("0:{}", target.video_index)]);
    for (index, _) in &target.audio {
        push(&mut a, &["-map", &format!("0:{index}")]);
    }
    for index in &target.subtitles {
        push(&mut a, &["-map", &format!("0:{index}")]);
    }
    push(&mut a, &["-map_metadata", "0", "-map_chapters", "0", "-dn"]);

    push(&mut a, &["-vf", &video_filter(target, encoder)]);
    encoder_args(&mut a, encoder, target.video_bps);
    // Variable frame rate (phones, screen recordings) is kept as is.
    push(&mut a, &["-fps_mode", "passthrough"]);

    for (n, (_, plan)) in target.audio.iter().enumerate() {
        match plan {
            TrackAudio::Copy => push(&mut a, &[&format!("-c:a:{n}"), "copy"]),
            TrackAudio::Aac { kbps } => push(
                &mut a,
                &[
                    &format!("-c:a:{n}"),
                    "aac",
                    &format!("-b:a:{n}"),
                    &format!("{kbps}k"),
                ],
            ),
        }
    }
    if !target.subtitles.is_empty() {
        push(&mut a, &["-c:s", "mov_text"]);
    }
    push(
        &mut a,
        &[
            "-movflags",
            "+faststart",
            "-progress",
            "pipe:1",
            "-nostats",
            "-f",
            "mp4",
        ],
    );
    a.push(file_url(output)?);
    Ok(a)
}

/// Arguments for a tiny test encode that proves an encoder works on this
/// machine (an encoder can be listed but unusable: no GPU, old driver).
pub fn test_encode(encoder: VideoEncoder) -> Vec<OsString> {
    let mut a: Vec<OsString> = Vec::new();
    push(&mut a, &["-hide_banner", "-nostdin", "-loglevel", "error"]);
    let vaapi = encoder == VideoEncoder::H264(H264Encoder::Vaapi);
    if vaapi {
        push(&mut a, &["-vaapi_device", VAAPI_DEVICE]);
    }
    push(
        &mut a,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240:r=10:d=0.5",
            "-frames:v",
            "3",
        ],
    );
    let filter = if vaapi {
        "format=nv12,hwupload"
    } else {
        "format=yuv420p"
    };
    push(&mut a, &["-vf", filter]);
    let probe_target = VideoTarget {
        encoder,
        video_index: 0,
        scale_short_side: None,
        ten_bit: false,
        audio: Vec::new(),
        subtitles: Vec::new(),
        video_bps: 300_000,
        estimated_size: 0,
        extension: None,
        duration_secs: 0.0,
    };
    encoder_args(&mut a, encoder, probe_target.video_bps);
    push(&mut a, &["-f", "null", "-"]);
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(encoder: VideoEncoder) -> VideoTarget {
        VideoTarget {
            encoder,
            video_index: 0,
            scale_short_side: Some(1080),
            ten_bit: false,
            audio: vec![(1, TrackAudio::Aac { kbps: 128 }), (3, TrackAudio::Copy)],
            subtitles: vec![4],
            video_bps: 1_000_000,
            estimated_size: 0,
            extension: None,
            duration_secs: 60.0,
        }
    }

    fn text(a: &[OsString]) -> Vec<String> {
        a.iter().map(|s| s.to_string_lossy().into_owned()).collect()
    }

    fn after<'a>(a: &'a [String], flag: &str) -> Option<&'a str> {
        let i = a.iter().position(|x| x == flag)?;
        a.get(i + 1).map(String::as_str)
    }

    #[test]
    fn av1_snapshot_parts() {
        let input = std::env::temp_dir().join("in.mov");
        let output = std::env::temp_dir().join("out.mp4");
        let t = target(VideoEncoder::SvtAv1);
        let a = text(&encode_video(&input, &output, &t, t.encoder, None).unwrap());
        let maps: Vec<&str> = a
            .windows(2)
            .filter(|w| w[0] == "-map")
            .map(|w| w[1].as_str())
            .collect();
        assert_eq!(maps, ["0:0", "0:1", "0:3", "0:4"]);
        assert_eq!(after(&a, "-c:v"), Some("libsvtav1"));
        assert_eq!(after(&a, "-crf"), Some("35"));
        assert_eq!(
            after(&a, "-vf"),
            Some("scale=w='if(gte(iw,ih),-2,1080)':h='if(gte(iw,ih),1080,-2)',format=yuv420p")
        );
        assert_eq!(after(&a, "-c:a:0"), Some("aac"));
        assert_eq!(after(&a, "-b:a:0"), Some("128k"));
        assert_eq!(after(&a, "-c:a:1"), Some("copy"));
        assert_eq!(after(&a, "-c:s"), Some("mov_text"));
        assert_eq!(after(&a, "-fps_mode"), Some("passthrough"));
        assert_eq!(after(&a, "-movflags"), Some("+faststart"));
        assert_eq!(after(&a, "-f"), Some("mp4"));
        assert!(a.contains(&"-dn".to_string()));
        assert!(
            a.windows(2)
                .any(|w| w[0] == "-protocol_whitelist" && w[1] == "file")
        );
        assert_eq!(a.last().unwrap(), &format!("file:{}", output.display()));
    }

    #[test]
    fn hardware_encoders() {
        let input = std::env::temp_dir().join("in.mp4");
        let output = std::env::temp_dir().join("out.mp4");
        for (e, flag, value) in [
            (H264Encoder::VideoToolbox, "-q:v", "60"),
            (H264Encoder::Nvenc, "-cq", "23"),
            (H264Encoder::Qsv, "-global_quality", "23"),
            (H264Encoder::Amf, "-qp_p", "24"),
            (H264Encoder::MediaFoundation, "-quality", "60"),
            (H264Encoder::Vaapi, "-qp", "23"),
            (H264Encoder::OpenH264, "-b:v", "1000000"),
        ] {
            let t = target(VideoEncoder::H264(e));
            let a = text(&encode_video(&input, &output, &t, t.encoder, None).unwrap());
            assert_eq!(after(&a, "-c:v"), Some(e.name()));
            assert_eq!(after(&a, flag), Some(value), "{}", e.name());
        }
        let t = target(VideoEncoder::H264(H264Encoder::Vaapi));
        let a = text(&encode_video(&input, &output, &t, t.encoder, None).unwrap());
        let device = a.iter().position(|x| x == "-vaapi_device").unwrap();
        let i = a.iter().position(|x| x == "-i").unwrap();
        assert!(device < i, "VAAPI device is a global option");
        assert!(after(&a, "-vf").unwrap().ends_with("format=nv12,hwupload"));
    }

    #[test]
    fn ten_bit_and_no_scaling() {
        let mut t = target(VideoEncoder::SvtAv1);
        t.ten_bit = true;
        t.scale_short_side = None;
        t.subtitles.clear();
        let a = text(
            &encode_video(
                &std::env::temp_dir().join("a.mkv"),
                &std::env::temp_dir().join("b.mp4"),
                &t,
                t.encoder,
                None,
            )
            .unwrap(),
        );
        assert_eq!(after(&a, "-vf"), Some("format=yuv420p10le"));
        assert!(!a.contains(&"-c:s".to_string()));
    }

    #[test]
    fn test_encodes() {
        let a = text(&test_encode(VideoEncoder::H264(H264Encoder::Nvenc)));
        assert_eq!(after(&a, "-c:v"), Some("h264_nvenc"));
        assert_eq!(a.last().map(String::as_str), Some("-"));
        let v = text(&test_encode(VideoEncoder::H264(H264Encoder::Vaapi)));
        assert!(v.contains(&"-vaapi_device".to_string()));
    }
}
