//! Which video encoders work on this computer. Picked once per FFmpeg and
//! cached by the app: every listed candidate gets a tiny test encode,
//! because an encoder can be listed but unusable (no GPU, old driver).

use std::path::Path;
use std::time::Duration;

use super::args::test_encode;
use super::plan::{H264Encoder, VideoEncoder, VideoEncoders};

async fn works(ffmpeg: &Path, encoder: VideoEncoder) -> bool {
    let mut cmd = tokio::process::Command::new(ffmpeg);
    cmd.args(test_encode(encoder))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    matches!(
        tokio::time::timeout(Duration::from_secs(20), cmd.status()).await,
        Ok(Ok(status)) if status.success()
    )
}

/// Test the listed encoders (`listed` comes from `ffmpeg -encoders`).
pub async fn detect(ffmpeg: &Path, listed: &[String]) -> VideoEncoders {
    let has = |name: &str| listed.iter().any(|e| e == name);
    let mut found = VideoEncoders::default();
    if has("libsvtav1") {
        found.av1 = works(ffmpeg, VideoEncoder::SvtAv1).await;
    }
    for e in H264Encoder::PRIORITY {
        if !has(e.name()) {
            continue;
        }
        if works(ffmpeg, VideoEncoder::H264(e)).await {
            if e == H264Encoder::OpenH264 {
                found.openh264 = true;
            }
            if found.h264.is_none() {
                found.h264 = Some(e);
            }
            // Keep looking only for the software fallback.
            if found.openh264 || !has(H264Encoder::OpenH264.name()) {
                break;
            }
        }
    }
    found
}
