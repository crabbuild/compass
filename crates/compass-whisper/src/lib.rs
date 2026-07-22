//! Bounded native Whisper inference for Compass.
//!
//! The inference implementation is derived from `whisper-candle-core` 0.1.2
//! at commit `a6c94f583603c605330bc21f1da6b0b255a3d32e` under the MIT license.
//! Compass deliberately omits its network, file decoding, and output-writing
//! layers so those boundaries remain governed by Compass's own limits.

pub mod audio;
pub mod decode;
pub mod model;
pub mod nn;
pub mod timing;
pub mod tokenizer;
pub mod transcribe;
pub mod utils;

pub use decode::{DecodingOptions, DecodingResult, decode, detect_language};
pub use model::WhisperModel;
pub use tokenizer::{Task, Tokenizer, get_tokenizer};
pub use transcribe::{Segment, TranscribeOptions, TranscribeResult, transcribe};

use anyhow::Result;
use candle_core::Device;

/// Pick an inference device. Phase 5 intentionally exposes CPU only so every
/// release artifact has the same dependency-free behavior.
pub fn device(name: &str) -> Result<Device> {
    match name {
        "cpu" => Ok(Device::Cpu),
        other => anyhow::bail!("device {other} is not available; Compass currently supports CPU"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_device_is_portable_and_accelerators_stay_hidden() -> Result<()> {
        assert!(matches!(device("cpu")?, Device::Cpu));
        assert!(device("gpu").is_err());
        Ok(())
    }

    #[test]
    fn bundled_english_tokenizer_round_trips_text() -> Result<()> {
        let tokenizer = get_tokenizer(false, 99, Some("en"), Some(Task::Transcribe))?;
        let text = " Compass maps code into a knowledge graph.";
        assert_eq!(tokenizer.decode(&tokenizer.encode(text)), text);
        Ok(())
    }

    #[test]
    fn log_mel_shape_is_whisper_compatible() -> Result<()> {
        let silence = vec![0.0; 1_600];
        let spectrogram = audio::log_mel_spectrogram(&silence, 80, audio::N_SAMPLES)?;
        assert_eq!(spectrogram.n_mels, 80);
        assert!(spectrogram.n_frames >= audio::N_FRAMES);
        assert!(spectrogram.data.iter().all(|sample| sample.is_finite()));
        Ok(())
    }
}
