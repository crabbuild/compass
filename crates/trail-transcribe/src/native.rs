//! Native Candle Whisper backend over Trail's bounded media and model layers.

use crate::audio::{AudioLimits, decode_audio};
use crate::models::ModelFiles;
use crate::{BackendRequest, BackendTranscript, WhisperBackend};
use trail_whisper::{TranscribeOptions, WhisperModel};

pub struct NativeWhisperBackend {
    model_name: String,
    model: WhisperModel,
    audio_limits: AudioLimits,
}

impl NativeWhisperBackend {
    pub fn load(
        model_name: impl Into<String>,
        files: &ModelFiles,
        audio_limits: AudioLimits,
    ) -> Result<Self, String> {
        let model_name = model_name.into();
        let device = trail_whisper::device("cpu").map_err(|error| error.to_string())?;
        let mut model = WhisperModel::load(&files.config, &files.weights, &device)
            .map_err(|error| format!("could not load model {model_name}: {error}"))?;
        model
            .set_alignment_heads_from_file(&files.generation_config)
            .map_err(|error| format!("could not load model generation config: {error}"))?;
        Ok(Self {
            model_name,
            model,
            audio_limits,
        })
    }
}

impl WhisperBackend for NativeWhisperBackend {
    fn transcribe(&mut self, request: &BackendRequest<'_>) -> Result<BackendTranscript, String> {
        if request.model != self.model_name {
            return Err(format!(
                "loaded model is {:?}, but request selected {:?}",
                self.model_name, request.model
            ));
        }
        let decoded = decode_audio(request.audio_path, self.audio_limits)
            .map_err(|error| error.to_string())?;
        let options = native_options(request)?;
        let result = trail_whisper::transcribe(&mut self.model, &decoded.samples, &options)
            .map_err(|error| error.to_string())?;
        Ok(BackendTranscript {
            segments: result
                .segments
                .into_iter()
                .map(|segment| segment.text)
                .collect(),
            language: Some(result.language),
        })
    }
}

fn native_options(request: &BackendRequest<'_>) -> Result<TranscribeOptions, String> {
    if request.beam_size == 0 {
        return Err("beam size must be greater than zero".to_owned());
    }
    let mut options = TranscribeOptions {
        initial_prompt: Some(request.initial_prompt.to_owned()),
        ..TranscribeOptions::default()
    };
    options.decode_options.beam_size = Some(request.beam_size);
    options.decode_options.best_of = None;
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn native_options_match_faster_whisper_request_contract() -> Result<(), String> {
        let request = BackendRequest {
            audio_path: Path::new("clip.wav"),
            model: "base",
            beam_size: 5,
            initial_prompt: "Technical discussion about Trail.",
        };
        let options = native_options(&request)?;
        assert_eq!(
            options.initial_prompt.as_deref(),
            Some(request.initial_prompt)
        );
        assert_eq!(options.decode_options.beam_size, Some(5));
        assert_eq!(options.decode_options.best_of, None);
        assert!(options.condition_on_previous_text);
        Ok(())
    }

    #[test]
    fn zero_beam_size_is_rejected() {
        let request = BackendRequest {
            audio_path: Path::new("clip.wav"),
            model: "base",
            beam_size: 0,
            initial_prompt: "prompt",
        };
        assert!(native_options(&request).is_err());
    }
}
