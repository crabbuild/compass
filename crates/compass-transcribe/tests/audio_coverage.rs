use std::error::Error;

use compass_transcribe::audio::{AudioLimits, WHISPER_SAMPLE_RATE, decode_audio, resample};
use compass_transcribe::{
    AudioDownloader, BackendRequest, BackendTranscript, NoUrlDownloader, WhisperBackend,
    build_whisper_prompt, transcribe_with,
};

struct UnusedBackend;

impl WhisperBackend for UnusedBackend {
    fn transcribe(&mut self, _request: &BackendRequest<'_>) -> Result<BackendTranscript, String> {
        Err("backend must not run".to_owned())
    }
}

#[test]
fn audio_rejections_and_resampling_cover_zero_identity_partial_and_full_chunks()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let missing = directory.path().join("missing.wav");
    assert!(decode_audio(&missing, AudioLimits::default()).is_err());

    let existing = directory.path().join("audio.wav");
    std::fs::write(&existing, b"not audio")?;
    assert!(
        decode_audio(
            &existing,
            AudioLimits {
                max_source_bytes: 100,
                max_duration_seconds: 0,
            },
        )
        .is_err()
    );
    assert!(decode_audio(directory.path(), AudioLimits::default()).is_err());
    assert!(decode_audio(&existing, AudioLimits::default()).is_err());

    assert!(resample(&[1.0], 0, WHISPER_SAMPLE_RATE).is_err());
    assert!(resample(&[1.0], WHISPER_SAMPLE_RATE, 0).is_err());
    assert!(resample(&[], 8_000, WHISPER_SAMPLE_RATE)?.is_empty());
    assert_eq!(resample(&[1.0, -1.0], 8_000, 8_000)?, [1.0, -1.0]);

    let partial = vec![0.25_f32; 513];
    let partial_output = resample(&partial, 8_000, WHISPER_SAMPLE_RATE)?;
    assert_eq!(partial_output.len(), 1_026);
    let full_chunks = (0..2_500)
        .map(|index| if index % 2 == 0 { 0.5 } else { -0.5 })
        .collect::<Vec<_>>();
    let full_output = resample(&full_chunks, 48_000, WHISPER_SAMPLE_RATE)?;
    assert_eq!(full_output.len(), 833);
    assert!(full_output.iter().all(|sample| sample.is_finite()));
    Ok(())
}

#[test]
fn default_prompt_and_no_url_downloader_fail_before_backend_execution() -> Result<(), Box<dyn Error>>
{
    let prompt = build_whisper_prompt(["Compass", "Graph"]);
    assert!(prompt.contains("Compass, Graph"));
    let directory = tempfile::tempdir()?;
    let mut downloader = NoUrlDownloader;
    assert!(
        downloader
            .download_audio("https://example.com/audio", directory.path())
            .is_err()
    );
    let mut backend = UnusedBackend;
    assert!(
        transcribe_with(
            "https://example.com/audio",
            directory.path(),
            None,
            false,
            &mut downloader,
            &mut backend,
        )
        .is_err()
    );
    Ok(())
}
