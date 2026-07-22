use std::error::Error;
use std::path::PathBuf;

use compass_transcribe::audio::AudioLimits;
use compass_transcribe::models::{HttpsArtifactFetcher, ModelCache};
use compass_transcribe::native::NativeWhisperBackend;
use compass_transcribe::{BackendRequest, WhisperBackend};

#[test]
#[ignore = "downloads a pinned model and performs native CPU inference"]
fn native_whisper_transcribes_real_speech() -> Result<(), Box<dyn Error>> {
    let audio = std::env::var_os("COMPASS_WHISPER_ACCEPTANCE_AUDIO")
        .map(PathBuf::from)
        .ok_or("set COMPASS_WHISPER_ACCEPTANCE_AUDIO to a real speech file")?;
    let model_name =
        std::env::var("COMPASS_WHISPER_ACCEPTANCE_MODEL").unwrap_or_else(|_| "tiny.en".to_owned());
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
            initial_prompt: "Technical discussion about Compass knowledge graphs.",
        })
        .map_err(|error| format!("native transcription failed: {error}"))?;
    let text = result.segments.join(" ").to_lowercase();
    assert!(!text.trim().is_empty(), "native Whisper returned no text");
    assert!(
        text.contains("compass") || text.contains("knowledge graph"),
        "unexpected transcript: {text}"
    );
    Ok(())
}
