//! In-process audio probing with Symphonia: codec, sample rate, channels,
//! duration, and the R2Dock compression marker. No process is started.
//!
//! Symphonia reports no bitrate, so the real bitrate is computed from the
//! file size and duration (see [`AudioProbe::real_bitrate`]). This is correct
//! for variable-bitrate MP3, whose header bitrate is misleading.

use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::CodecParameters;
use symphonia::core::codecs::audio::well_known::{
    CODEC_ID_AAC, CODEC_ID_ALAC, CODEC_ID_FLAC, CODEC_ID_MP3, CODEC_ID_OPUS, CODEC_ID_PCM_MULAW,
    CODEC_ID_PCM_S32LE, CODEC_ID_VORBIS,
};
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, RawValue, Tag};
use thiserror::Error;

/// Tag that marks a file R2Dock already compressed: ID3 `TXXX` with this
/// description, or a comment of the form `R2DOCK_COMPRESSED=<preset>`.
pub const MARKER: &str = "R2DOCK_COMPRESSED";

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("could not read the file")]
    Io(#[from] std::io::Error),
    #[error("not a supported audio file")]
    Unsupported,
}

/// The audio codec, decided by the content, never by the file name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodec {
    Mp3,
    Aac,
    Opus,
    Vorbis,
    Flac,
    Alac,
    /// Uncompressed PCM (WAV, AIFF).
    Pcm,
    Other,
}

impl AudioCodec {
    /// Lossless sources are always worth compressing (subject to settings).
    pub fn is_lossless(self) -> bool {
        matches!(self, Self::Flac | Self::Alac | Self::Pcm)
    }
}

/// What the probe found.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioProbe {
    pub codec: AudioCodec,
    pub sample_rate: Option<u32>,
    pub channels: Option<u8>,
    pub duration_secs: Option<f64>,
    /// File size in bytes.
    pub size: u64,
    /// Bytes of embedded pictures (cover art), excluded from the bitrate.
    pub picture_bytes: u64,
    /// The file contains a video track (handled by the video path).
    pub has_video: bool,
    /// Preset from an R2Dock marker tag, if present.
    pub marker: Option<String>,
}

impl AudioProbe {
    /// Real average bitrate in bits per second: audio bytes x 8 / duration.
    pub fn real_bitrate(&self) -> Option<u64> {
        let secs = self.duration_secs.filter(|d| *d > 0.0)?;
        let audio = self.size.saturating_sub(self.picture_bytes);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bps = (audio as f64 * 8.0 / secs).round() as u64;
        Some(bps)
    }
}

fn codec_of(params: &CodecParameters) -> Option<(AudioCodec, Option<u32>, Option<u8>)> {
    let CodecParameters::Audio(a) = params else {
        return None;
    };
    let codec = match a.codec {
        c if c == CODEC_ID_MP3 => AudioCodec::Mp3,
        c if c == CODEC_ID_AAC => AudioCodec::Aac,
        c if c == CODEC_ID_OPUS => AudioCodec::Opus,
        c if c == CODEC_ID_VORBIS => AudioCodec::Vorbis,
        c if c == CODEC_ID_FLAC => AudioCodec::Flac,
        c if c == CODEC_ID_ALAC => AudioCodec::Alac,
        // Every PCM variant has an id in this range.
        c if (CODEC_ID_PCM_S32LE..=CODEC_ID_PCM_MULAW).contains(&c) => AudioCodec::Pcm,
        _ => AudioCodec::Other,
    };
    let channels = a
        .channels
        .as_ref()
        .and_then(|c| u8::try_from(c.count()).ok());
    Some((codec, a.sample_rate, channels))
}

/// The marker preset in a tag, if the tag is an R2Dock marker.
fn marker_of(tag: &Tag) -> Option<String> {
    let value = match &tag.raw.value {
        RawValue::String(s) => s.as_str().to_string(),
        RawValue::StringList(l) => l.first().cloned().unwrap_or_default(),
        _ => return None,
    };
    let described = tag
        .raw
        .sub_fields
        .iter()
        .flatten()
        .any(|f| matches!(&f.value, RawValue::String(s) if s.eq_ignore_ascii_case(MARKER)));
    if described || tag.raw.key.eq_ignore_ascii_case(MARKER) {
        return Some(value);
    }
    let (key, rest) = value.split_once('=')?;
    key.trim()
        .eq_ignore_ascii_case(MARKER)
        .then(|| rest.trim().to_string())
}

fn read_metadata(reader: &mut dyn FormatReader, probe: &mut AudioProbe) {
    let mut metadata = reader.metadata();
    let Some(rev) = metadata.skip_to_latest() else {
        return;
    };
    let containers = std::iter::once(&rev.media).chain(rev.per_track.iter().map(|t| &t.metadata));
    for container in containers {
        for tag in &container.tags {
            if probe.marker.is_none() {
                probe.marker = marker_of(tag);
            }
        }
        probe.picture_bytes += container
            .visuals
            .iter()
            .map(|v| v.data.len() as u64)
            .sum::<u64>();
    }
}

/// Probe an audio file. Blocking; run it on a blocking thread.
pub fn probe_audio(path: &Path) -> Result<AudioProbe, ProbeError> {
    let file = File::open(path)?;
    let size = file.metadata()?.len();
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(&ext.to_ascii_lowercase());
    }
    let mut reader = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|e| match e {
            symphonia::core::errors::Error::IoError(io) => ProbeError::Io(io),
            _ => ProbeError::Unsupported,
        })?;

    let has_video = reader
        .tracks()
        .iter()
        .any(|t| matches!(t.codec_params, Some(CodecParameters::Video(_))));
    let track = reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.as_ref().and_then(codec_of).is_some())
        .ok_or(ProbeError::Unsupported)?;
    let (codec, sample_rate, channels) = track
        .codec_params
        .as_ref()
        .and_then(codec_of)
        .ok_or(ProbeError::Unsupported)?;
    let duration_secs = match (track.time_base, track.num_frames, track.duration) {
        (Some(tb), _, Some(d)) => tb.calc_duration(d).map(time_secs),
        (_, Some(frames), _) => sample_rate
            .filter(|r| *r > 0)
            .map(|r| frames as f64 / f64::from(r)),
        _ => None,
    };
    let mut probe = AudioProbe {
        codec,
        sample_rate,
        channels,
        duration_secs,
        size,
        picture_bytes: 0,
        has_video,
        marker: None,
    };
    read_metadata(reader.as_mut(), &mut probe);
    Ok(probe)
}

fn time_secs(t: symphonia::core::units::Time) -> f64 {
    let (secs, nanos) = t.parts();
    secs as f64 + f64::from(nanos) / 1e9
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::meta::{RawTag, RawTagSubField};

    /// A 16-bit PCM WAV file of `secs` seconds of silence.
    fn wav(rate: u32, channels: u16, secs: u32) -> Vec<u8> {
        let data_len = rate * u32::from(channels) * 2 * secs;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&channels.to_le_bytes());
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
        v.extend_from_slice(&(channels * 2).to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        v.resize(v.len() + data_len as usize, 0);
        v
    }

    fn write(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn probes_wav() {
        let dir = tempfile::tempdir().unwrap();
        let p = probe_audio(&write(dir.path(), "a.wav", &wav(44_100, 2, 3))).unwrap();
        assert_eq!(p.codec, AudioCodec::Pcm);
        assert_eq!((p.sample_rate, p.channels), (Some(44_100), Some(2)));
        let d = p.duration_secs.unwrap();
        assert!((d - 3.0).abs() < 0.01, "{d}");
        assert!(!p.has_video && p.marker.is_none());
        let kbps = p.real_bitrate().unwrap() / 1000;
        assert!((1411..=1413).contains(&kbps), "{kbps}");
    }

    #[test]
    fn content_decides_not_the_name() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["really-wav.mp3", "UPPER.WAV", "no-extension"] {
            let p = probe_audio(&write(dir.path(), name, &wav(22_050, 1, 1))).unwrap();
            assert_eq!(p.codec, AudioCodec::Pcm, "{name}");
            assert_eq!(p.channels, Some(1));
        }
    }

    #[test]
    fn rejects_non_audio_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let text = write(dir.path(), "notes.mp3", b"just some text, not audio at all");
        assert!(matches!(probe_audio(&text), Err(ProbeError::Unsupported)));
        let empty = write(dir.path(), "empty.wav", b"");
        assert!(probe_audio(&empty).is_err());
        let mut cut = wav(44_100, 2, 1);
        cut.truncate(20);
        let truncated = write(dir.path(), "cut.wav", &cut);
        assert!(probe_audio(&truncated).is_err());
        assert!(matches!(
            probe_audio(&dir.path().join("missing.wav")),
            Err(ProbeError::Io(_))
        ));
    }

    #[test]
    fn marker_tags() {
        let txxx = Tag::new(RawTag::new_with_sub_fields(
            "TXXX",
            "mp3-32k",
            vec![RawTagSubField::new("DESCRIPTION", MARKER)].into_boxed_slice(),
        ));
        assert_eq!(marker_of(&txxx).as_deref(), Some("mp3-32k"));
        let comment = Tag::new(RawTag::new("©cmt", "R2DOCK_COMPRESSED=aac-64k"));
        assert_eq!(marker_of(&comment).as_deref(), Some("aac-64k"));
        let lower = Tag::new(RawTag::new("COMMENT", "r2dock_compressed = opus-24k"));
        assert_eq!(marker_of(&lower).as_deref(), Some("opus-24k"));
        let other = Tag::new(RawTag::new_with_sub_fields(
            "TXXX",
            "x",
            vec![RawTagSubField::new("DESCRIPTION", "SOMETHING")].into_boxed_slice(),
        ));
        assert_eq!(marker_of(&other), None);
        assert_eq!(marker_of(&Tag::new(RawTag::new("TIT2", "A title"))), None);
    }
}
