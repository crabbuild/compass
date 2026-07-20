//! Bounded, pure-Rust audio extraction and resampling for Whisper.

use std::fs::File;
use std::path::{Path, PathBuf};

use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;

pub const WHISPER_SAMPLE_RATE: usize = 16_000;
pub const DEFAULT_MAX_SOURCE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const DEFAULT_MAX_DURATION_SECONDS: usize = 4 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioLimits {
    pub max_source_bytes: u64,
    pub max_duration_seconds: usize,
}

impl Default for AudioLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: DEFAULT_MAX_SOURCE_BYTES,
            max_duration_seconds: DEFAULT_MAX_DURATION_SECONDS,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("could not access audio {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("audio rejected: {0}")]
    Rejected(String),
    #[error("unsupported audio: {0}")]
    Unsupported(String),
    #[error("audio decode failed: {0}")]
    Decode(String),
}

/// Decode the first audio track to bounded 16 kHz mono PCM.
pub fn decode_audio(path: &Path, limits: AudioLimits) -> Result<DecodedAudio, AudioError> {
    if limits.max_duration_seconds == 0 {
        return Err(AudioError::Rejected(
            "maximum duration must be greater than zero".to_owned(),
        ));
    }
    let metadata = path.metadata().map_err(|source| AudioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > limits.max_source_bytes {
        return Err(AudioError::Rejected(format!(
            "{} is {} bytes; maximum is {}",
            path.display(),
            metadata.len(),
            limits.max_source_bytes
        )));
    }
    if crate::avi::is_avi(path)? {
        return crate::avi::decode_avi_audio(path, limits);
    }
    let file = File::open(path).map_err(|source| AudioError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    decode_audio_file(
        file,
        path.extension().and_then(|extension| extension.to_str()),
        limits,
    )
}

pub(crate) fn decode_audio_file(
    file: File,
    extension: Option<&str>,
    limits: AudioLimits,
) -> Result<DecodedAudio, AudioError> {
    let source = MediaSourceStream::new(Box::new(file), MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    if let Some(extension) = extension {
        hint.with_extension(extension);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            source,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|error| AudioError::Unsupported(error.to_string()))?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or_else(|| AudioError::Unsupported("container has no audio track".to_owned()))?;
    let track_id = track.id;
    let codec_parameters = track
        .codec_params
        .as_ref()
        .and_then(|parameters| parameters.audio())
        .cloned()
        .ok_or_else(|| AudioError::Unsupported("audio codec parameters are missing".to_owned()))?;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&codec_parameters, &AudioDecoderOptions::default())
        .map_err(|error| AudioError::Unsupported(error.to_string()))?;

    let mut source_rate = None;
    let mut pcm = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(SymphoniaError::ResetRequired) => {
                return Err(AudioError::Decode(
                    "container changed streams during decoding".to_owned(),
                ));
            }
            Err(error) => return Err(AudioError::Decode(error.to_string())),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(error) => return Err(AudioError::Decode(error.to_string())),
        };
        let spec = decoded.spec();
        let rate = spec.rate() as usize;
        if rate == 0 {
            return Err(AudioError::Rejected("audio sample rate is zero".to_owned()));
        }
        if source_rate
            .replace(rate)
            .is_some_and(|previous| previous != rate)
        {
            return Err(AudioError::Unsupported(
                "audio sample rate changes mid-stream".to_owned(),
            ));
        }
        let channels = spec.channels().count();
        if channels == 0 {
            return Err(AudioError::Rejected("audio has no channels".to_owned()));
        }
        let mut samples = vec![0.0_f32; decoded.samples_interleaved()];
        decoded.copy_to_slice_interleaved(&mut samples);
        let packet_samples = samples.as_slice();
        let packet_frames = packet_samples.len() / channels;
        let max_source_frames = rate
            .checked_mul(limits.max_duration_seconds)
            .ok_or_else(|| AudioError::Rejected("audio duration limit overflowed".to_owned()))?;
        if pcm.len().saturating_add(packet_frames) > max_source_frames {
            return Err(AudioError::Rejected(format!(
                "decoded audio exceeds {} seconds",
                limits.max_duration_seconds
            )));
        }
        if channels == 1 {
            pcm.extend_from_slice(packet_samples);
        } else {
            pcm.extend(
                packet_samples
                    .chunks_exact(channels)
                    .map(|frame| frame.iter().copied().sum::<f32>() / channels as f32),
            );
        }
    }

    let rate = source_rate.ok_or_else(|| AudioError::Decode("audio is empty".to_owned()))?;
    finalize_pcm(pcm, rate, limits)
}

pub(crate) fn finalize_pcm(
    pcm: Vec<f32>,
    rate: usize,
    limits: AudioLimits,
) -> Result<DecodedAudio, AudioError> {
    let samples = if rate == WHISPER_SAMPLE_RATE {
        pcm
    } else {
        resample(&pcm, rate, WHISPER_SAMPLE_RATE)?
    };
    let max_output_samples = WHISPER_SAMPLE_RATE
        .checked_mul(limits.max_duration_seconds)
        .ok_or_else(|| AudioError::Rejected("audio duration limit overflowed".to_owned()))?;
    if samples.len() > max_output_samples {
        return Err(AudioError::Rejected(format!(
            "resampled audio exceeds {} seconds",
            limits.max_duration_seconds
        )));
    }
    Ok(DecodedAudio {
        samples,
        sample_rate: WHISPER_SAMPLE_RATE,
    })
}

/// Windowed-sinc mono resampling with deterministic output length.
pub fn resample(input: &[f32], from_rate: usize, to_rate: usize) -> Result<Vec<f32>, AudioError> {
    if from_rate == 0 || to_rate == 0 {
        return Err(AudioError::Rejected(
            "resampling rates must be greater than zero".to_owned(),
        ));
    }
    if input.is_empty() || from_rate == to_rate {
        return Ok(input.to_vec());
    }
    let ratio = to_rate as f64 / from_rate as f64;
    const SINC_LENGTH: usize = 256;
    const CHUNK_SIZE: usize = 1024;
    let parameters = SincInterpolationParameters {
        sinc_len: SINC_LENGTH,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Cubic,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(ratio, 2.0, parameters, CHUNK_SIZE, 1)
        .map_err(|error| AudioError::Decode(error.to_string()))?;
    let delay = resampler
        .output_delay()
        .saturating_sub(((SINC_LENGTH / 2) as f64 * ratio).ceil() as usize);
    let expected = (input.len() as f64 * ratio).round() as usize;
    let mut output = Vec::with_capacity(expected.saturating_add(delay).saturating_add(CHUNK_SIZE));
    let mut position = 0;
    while position + CHUNK_SIZE <= input.len() {
        let chunks = resampler
            .process(&[&input[position..position + CHUNK_SIZE]], None)
            .map_err(|error| AudioError::Decode(error.to_string()))?;
        output.extend_from_slice(&chunks[0]);
        position += CHUNK_SIZE;
    }
    if position < input.len() {
        let chunks = resampler
            .process_partial(Some(&[&input[position..]]), None)
            .map_err(|error| AudioError::Decode(error.to_string()))?;
        output.extend_from_slice(&chunks[0]);
    }
    while output.len() < delay.saturating_add(expected) {
        let chunks = resampler
            .process_partial::<&[f32]>(None, None)
            .map_err(|error| AudioError::Decode(error.to_string()))?;
        if chunks[0].is_empty() {
            break;
        }
        output.extend_from_slice(&chunks[0]);
    }
    let start = delay.min(output.len());
    let end = delay.saturating_add(expected).min(output.len());
    Ok(output[start..end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};

    #[test]
    fn decodes_stereo_wav_to_mono_without_external_codecs() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("stereo.wav");
        let frames = [[i16::MAX, i16::MIN], [8_000, 4_000], [-4_000, -8_000]];
        fs::write(&path, wav_pcm16(WHISPER_SAMPLE_RATE as u32, 2, &frames))?;
        let decoded = decode_audio(&path, AudioLimits::default())?;
        assert_eq!(decoded.sample_rate, WHISPER_SAMPLE_RATE);
        assert_eq!(decoded.samples.len(), frames.len());
        assert!(decoded.samples[0].abs() < 0.000_1);
        assert!((decoded.samples[1] - (6_000.0 / 32_768.0)).abs() < 0.000_1);
        Ok(())
    }

    #[test]
    fn resamples_wav_to_whisper_rate_with_bounded_length() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("eight-khz.wav");
        let frames = (0..800)
            .map(|index| {
                let sample = if index % 2 == 0 { 1_000 } else { -1_000 };
                [sample]
            })
            .collect::<Vec<_>>();
        fs::write(&path, wav_pcm16(8_000, 1, &frames))?;
        let decoded = decode_audio(&path, AudioLimits::default())?;
        assert_eq!(decoded.samples.len(), 1_600);
        assert!(decoded.samples.iter().all(|sample| sample.is_finite()));
        Ok(())
    }

    #[test]
    fn rejects_source_before_parser_reads_it() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("large.wav");
        let mut file = File::create(&path)?;
        file.seek(SeekFrom::Start(1_024))?;
        file.write_all(&[0])?;
        let result = decode_audio(
            &path,
            AudioLimits {
                max_source_bytes: 1_024,
                max_duration_seconds: 1,
            },
        );
        let error = match result {
            Ok(_) => return Err("oversized source should fail".into()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("maximum is 1024"));
        Ok(())
    }

    fn wav_pcm16<const CHANNELS: usize>(
        sample_rate: u32,
        channels: u16,
        frames: &[[i16; CHANNELS]],
    ) -> Vec<u8> {
        let data_bytes = u32::try_from(frames.len() * CHANNELS * 2).unwrap_or_default();
        let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16_u32.to_le_bytes());
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
        bytes.extend_from_slice(&(channels * 2).to_le_bytes());
        bytes.extend_from_slice(&16_u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_bytes.to_le_bytes());
        for frame in frames {
            for sample in frame {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        }
        bytes
    }
}
