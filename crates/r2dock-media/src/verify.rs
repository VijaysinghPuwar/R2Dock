//! Check an encoded file before it replaces the original in an upload:
//! it must open, hold the expected codec, and last as long as the source.
//! A copy that opens but lost a track or a channel is not a failure: the
//! pipeline checks it with `streams` and uploads the original instead.

use std::path::Path;

use thiserror::Error;

use crate::plan::{AudioTarget, Encoding};
use crate::probe::{AudioCodec, AudioProbe, probe_audio};

#[derive(Debug, Error, PartialEq)]
pub enum VerifyError {
    #[error("the compressed file could not be read")]
    Unreadable,
    #[error("the compressed file is empty")]
    Empty,
    #[error("the compressed file has the wrong format")]
    WrongCodec,
    #[error("the compressed file is {got:.1} s long instead of {expected:.1} s")]
    WrongDuration { expected: f64, got: f64 },
    #[error("the compressed file does not hold the same tracks as the original")]
    StreamsChanged,
    #[error("the compressed file has {got} audio channels instead of {expected}")]
    WrongChannels { expected: u32, got: u32 },
}

/// The codec an encoding produces.
pub fn codec_for(encoding: Encoding) -> AudioCodec {
    match encoding {
        Encoding::Mp3Cbr { .. } | Encoding::Mp3Vbr { .. } => AudioCodec::Mp3,
        Encoding::Aac { .. } => AudioCodec::Aac,
        Encoding::Opus { .. } => AudioCodec::Opus,
        Encoding::Flac => AudioCodec::Flac,
    }
}

/// Allowed difference in duration: 1 second or 0.5%, whichever is larger.
pub fn duration_tolerance(expected: f64) -> f64 {
    (expected * 0.005).max(1.0)
}

pub fn verify_audio(
    output: &Path,
    target: &AudioTarget,
    expected_secs: f64,
) -> Result<AudioProbe, VerifyError> {
    let probe = probe_audio(output).map_err(|_| VerifyError::Unreadable)?;
    if probe.size == 0 {
        return Err(VerifyError::Empty);
    }
    if probe.codec != codec_for(target.encoding) {
        return Err(VerifyError::WrongCodec);
    }
    let got = probe.duration_secs.ok_or(VerifyError::Unreadable)?;
    if (got - expected_secs).abs() > duration_tolerance(expected_secs) {
        return Err(VerifyError::WrongDuration {
            expected: expected_secs,
            got,
        });
    }
    Ok(probe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tolerance() {
        assert_eq!(duration_tolerance(60.0), 1.0);
        assert!((duration_tolerance(86_400.0) - 432.0).abs() < 1e-9);
    }

    #[test]
    fn unreadable_output_fails() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("out.mp3");
        std::fs::write(&p, b"not audio").unwrap();
        let t = AudioTarget {
            encoding: Encoding::Mp3Cbr { kbps: 32 },
            channels: 1,
            sample_rate: None,
            extension: None,
            content: crate::classify::Content::Speech,
            estimated_size: 0,
        };
        assert_eq!(verify_audio(&p, &t, 10.0), Err(VerifyError::Unreadable));
    }
}
