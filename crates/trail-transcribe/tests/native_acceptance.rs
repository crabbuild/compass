use std::error::Error;
use std::path::PathBuf;

use trail_transcribe::audio::AudioLimits;
use trail_transcribe::models::{HttpsArtifactFetcher, ModelCache};
use trail_transcribe::native::NativeWhisperBackend;
use trail_transcribe::{BackendRequest, WhisperBackend};

#[test]
#[ignore = "downloads a pinned model and performs native CPU inference"]
fn native_whisper_transcribes_real_speech() -> Result<(), Box<dyn Error>> {
    let audio = std::env::var_os("TRAIL_WHISPER_ACCEPTANCE_AUDIO")
        .map(PathBuf::from)
        .ok_or("set TRAIL_WHISPER_ACCEPTANCE_AUDIO to a real speech file")?;
    let model_name =
        std::env::var("TRAIL_WHISPER_ACCEPTANCE_MODEL").unwrap_or_else(|_| "tiny.en".to_owned());
    let cache = ModelCache::from_environment()?;
    let files = cache.ensure_model(&model_name, &HttpsArtifactFetcher::default())?;
    let mut backend =
        NativeWhisperBackend::load(model_name.clone(), &files, AudioLimits::default())
            .map_err(|error| format!("native backend load failed: {error}"))?;
    let result = backend
        .transcribe(&BackendRequest {
            audio_path: &audio,
            model: &model_name,
            beam_size: 5,
            initial_prompt: "Technical discussion about Trail knowledge graphs.",
        })
        .map_err(|error| format!("native transcription failed: {error}"))?;
    let text = result.segments.join(" ").to_lowercase();
    assert!(!text.trim().is_empty(), "native Whisper returned no text");
    assert!(
        text.contains("trail") || text.contains("knowledge graph"),
        "unexpected transcript: {text}"
    );
    Ok(())
}
