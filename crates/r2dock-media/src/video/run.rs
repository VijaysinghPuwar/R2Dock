//! The video branch of the pipeline: probe, plan, sample test, disk space,
//! encode (with one software retry after a hardware failure), verify, and
//! the final size check.

use std::path::Path;

use tokio_util::sync::CancellationToken;

use super::args::encode_video;
use super::plan::{H264Encoder, VideoEncoder, VideoTarget, plan_video};
use super::probe::probe_video;
use crate::ffmpeg::{EncodeError, Ffmpeg, SAMPLE_TEST_MIN_SECS, middle_window};
use crate::pipeline::{Compressed, PrepareJob, Prepared, Unchanged};
use crate::verify::{VerifyError, duration_tolerance};

/// Free space needed for the output: 1.2 times the estimate.
const SPACE_FACTOR: f64 = 1.2;

/// Video extensions worth probing with ffprobe.
pub const VIDEO_EXTENSIONS: [&str; 10] = [
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "3gp", "wmv",
];

pub fn is_video_candidate(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

fn percent(saving: f64) -> u8 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let p = saving.floor().clamp(0.0, 100.0) as u8;
    p
}

async fn verify(
    ffprobe: &Path,
    output: &Path,
    target: &VideoTarget,
    encoder: VideoEncoder,
    expected_secs: f64,
) -> Result<u64, EncodeError> {
    let probe = probe_video(ffprobe, output)
        .await
        .ok_or(EncodeError::Verify(VerifyError::Unreadable))?;
    let video = probe
        .video()
        .ok_or(EncodeError::Verify(VerifyError::WrongCodec))?;
    if video.codec != encoder.codec_name() || probe.audio().count() != target.audio.len() {
        return Err(EncodeError::Verify(VerifyError::WrongCodec));
    }
    let got = probe
        .duration_secs
        .ok_or(EncodeError::Verify(VerifyError::Unreadable))?;
    if (got - expected_secs).abs() > duration_tolerance(expected_secs) {
        return Err(EncodeError::Verify(VerifyError::WrongDuration {
            expected: expected_secs,
            got,
        }));
    }
    Ok(probe.size)
}

/// Encode once with `encoder`; delete the output on any failure.
async fn encode_once(
    ff: &Ffmpeg,
    job: &PrepareJob<'_>,
    output: &Path,
    target: &VideoTarget,
    encoder: VideoEncoder,
    cancel: &CancellationToken,
    on_fraction: &mut (dyn FnMut(f64) + Send),
) -> Result<u64, EncodeError> {
    let duration = target.duration_secs;
    let Some(ffprobe) = &ff.ffprobe else {
        return Err(EncodeError::Failed("ffprobe is missing".into()));
    };
    let args = encode_video(job.input, output, target, encoder, None)?;
    let result = ff
        .run_clean(&args, &[job.input, output], cancel, |p| {
            if let Some(secs) = p.out_secs
                && duration > 0.0
            {
                on_fraction((secs / duration).clamp(0.0, 1.0));
            }
        })
        .await;
    let result = match result {
        Ok(_) => verify(ffprobe, output, target, encoder, duration).await,
        Err(e) => Err(e),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}

/// Prepare one video file.
pub async fn prepare_video(
    ff: &Ffmpeg,
    job: PrepareJob<'_>,
    cancel: &CancellationToken,
    mut on_fraction: impl FnMut(f64) + Send,
) -> Result<Prepared, EncodeError> {
    let Some(ffprobe) = &ff.ffprobe else {
        return Ok(Prepared::Unchanged(Unchanged::ToolsMissing));
    };
    let Some(probe) = probe_video(ffprobe, job.input).await else {
        return Ok(Prepared::Unchanged(Unchanged::NotAudio));
    };
    if probe.video().is_none() {
        return Ok(Prepared::Unchanged(Unchanged::NotAudio));
    }
    let ext = job.input.extension().and_then(|e| e.to_str()).unwrap_or("");
    let target = match plan_video(
        &probe,
        job.video,
        &ff.video,
        job.settings.min_saving_percent,
        ext,
    ) {
        Ok(t) => t,
        Err(reason) => return Ok(Prepared::Unchanged(Unchanged::Plan { reason })),
    };
    let duration = probe.duration_secs.unwrap_or(0.0);
    std::fs::create_dir_all(job.work_dir).map_err(|e| {
        EncodeError::Failed(format!("could not create the work folder: {}", e.kind()))
    })?;
    let output = job.work_dir.join(format!("{}.mp4", job.stem));
    let margin = f64::from(job.settings.min_saving_percent);

    if duration > SAMPLE_TEST_MIN_SECS {
        let window = middle_window(duration);
        let sample = job.work_dir.join(format!("{}.sample.mp4", job.stem));
        let args = encode_video(job.input, &sample, &target, target.encoder, Some(window))?;
        let run = ff
            .run_clean(&args, &[job.input, &sample], cancel, |_| {})
            .await;
        let size = std::fs::metadata(&sample).map(|m| m.len());
        let _ = std::fs::remove_file(&sample);
        run?;
        let sample_size = size.map_err(|e| EncodeError::Failed(format!("sample missing: {e}")))?;
        #[allow(clippy::cast_precision_loss)]
        let projected = sample_size as f64 * duration / window.len_secs;
        #[allow(clippy::cast_precision_loss)]
        let saving = 100.0 * (1.0 - projected / probe.size.max(1) as f64);
        if saving < margin {
            return Ok(Prepared::Unchanged(Unchanged::SampleSavedTooLittle {
                percent: percent(saving),
            }));
        }
    }

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let needed = (target.estimated_size as f64 * SPACE_FACTOR) as u64;
    if fs4::available_space(job.work_dir).is_ok_and(|free| free < needed) {
        return Ok(Prepared::Unchanged(Unchanged::NoDiskSpace));
    }

    let mut encoder = target.encoder;
    let mut size = encode_once(
        ff,
        &job,
        &output,
        &target,
        encoder,
        cancel,
        &mut on_fraction,
    )
    .await;
    // A hardware encoder that fails mid-file gets one retry in software.
    if let (Err(e), VideoEncoder::H264(h)) = (&size, encoder)
        && !e.is_cancelled()
        && h.is_hardware()
        && ff.video.openh264
    {
        encoder = VideoEncoder::H264(H264Encoder::OpenH264);
        size = encode_once(
            ff,
            &job,
            &output,
            &target,
            encoder,
            cancel,
            &mut on_fraction,
        )
        .await;
    }
    let size = size?;
    #[allow(clippy::cast_precision_loss)]
    let saving = 100.0 * (1.0 - size as f64 / probe.size.max(1) as f64);
    if saving < margin {
        let _ = std::fs::remove_file(&output);
        return Ok(Prepared::Unchanged(Unchanged::ResultSavedTooLittle {
            percent: percent(saving),
        }));
    }
    Ok(Prepared::Compressed(Compressed {
        output,
        size,
        original_size: probe.size,
        preset: encoder.preset(),
        extension: target.extension,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_candidates() {
        for (name, want) in [
            ("a.MP4", true),
            ("b.mov", true),
            ("c.mkv", true),
            ("d.mp3", false),
            ("e", false),
        ] {
            assert_eq!(is_video_candidate(Path::new(name)), want, "{name}");
        }
    }
}
