//! Compatibility contracts and bounded orchestration for Trail transcription.
//!
//! Model inference and URL acquisition live behind explicit traits so the CLI
//! cannot accidentally expose an incomplete or platform-dependent backend.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha1::{Digest, Sha1};

pub mod audio;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    ".mp4", ".mov", ".webm", ".mkv", ".avi", ".m4v", ".mp3", ".wav", ".m4a", ".ogg",
];
pub const URL_PREFIXES: &[&str] = &["http://", "https://", "www."];
pub const DEFAULT_MODEL: &str = "base";
pub const FALLBACK_PROMPT: &str = "Use proper punctuation and paragraph breaks.";
pub const BEAM_SIZE: usize = 5;

const CACHED_AUDIO_EXTENSIONS: &[&str] = &["m4a", "opus", "mp3", "ogg", "wav", "webm"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendRequest<'a> {
    pub audio_path: &'a Path,
    pub model: &'a str,
    pub beam_size: usize,
    pub initial_prompt: &'a str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BackendTranscript {
    pub segments: Vec<String>,
    pub language: Option<String>,
}

pub trait WhisperBackend {
    fn transcribe(&mut self, request: &BackendRequest<'_>) -> Result<BackendTranscript, String>;
}

pub trait AudioDownloader {
    fn download_audio(&mut self, url: &str, output_dir: &Path) -> Result<PathBuf, String>;
}

#[derive(Debug, Default)]
pub struct NoUrlDownloader;

impl AudioDownloader for NoUrlDownloader {
    fn download_audio(&mut self, _url: &str, _output_dir: &Path) -> Result<PathBuf, String> {
        Err("URL audio download is not available in this build".to_owned())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TranscriptionError {
    #[error("could not {action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not download audio from {url}: {message}")]
    Download { url: String, message: String },
    #[error("could not transcribe {path}: {message}")]
    Backend { path: PathBuf, message: String },
    #[error("input has no usable file name: {0}")]
    MissingFileName(PathBuf),
    #[error("could not persist transcript {path}: {message}")]
    Persist { path: PathBuf, message: String },
}

#[must_use]
pub fn is_url(input: &str) -> bool {
    URL_PREFIXES.iter().any(|prefix| input.starts_with(prefix))
}

#[must_use]
pub fn model_name() -> String {
    std::env::var("GRAPHIFY_WHISPER_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_owned())
}

#[must_use]
pub fn build_whisper_prompt<'a>(labels: impl IntoIterator<Item = &'a str>) -> String {
    let override_prompt = std::env::var("GRAPHIFY_WHISPER_PROMPT").ok();
    build_whisper_prompt_with_override(labels, override_prompt.as_deref())
}

#[must_use]
pub fn build_whisper_prompt_with_override<'a>(
    labels: impl IntoIterator<Item = &'a str>,
    override_prompt: Option<&str>,
) -> String {
    let labels = labels
        .into_iter()
        .take(10)
        .filter(|label| !label.is_empty())
        .take(5)
        .collect::<Vec<_>>();
    if labels.is_empty() {
        return FALLBACK_PROMPT.to_owned();
    }
    if let Some(prompt) = override_prompt.filter(|prompt| !prompt.is_empty()) {
        return prompt.to_owned();
    }
    format!(
        "Technical discussion about {}. Use proper punctuation and paragraph breaks.",
        labels.join(", ")
    )
}

#[must_use]
pub fn audio_cache_key(url: &str) -> String {
    let digest = Sha1::digest(url.as_bytes());
    format!("{digest:x}")[..12].to_owned()
}

#[must_use]
pub fn cached_audio_path(url: &str, output_dir: &Path) -> Option<PathBuf> {
    let key = audio_cache_key(url);
    CACHED_AUDIO_EXTENSIONS
        .iter()
        .map(|extension| output_dir.join(format!("yt_{key}.{extension}")))
        .find(|candidate| candidate.exists())
}

pub fn transcribe_with<B: WhisperBackend, D: AudioDownloader>(
    input: &str,
    output_dir: &Path,
    initial_prompt: Option<&str>,
    force: bool,
    downloader: &mut D,
    backend: &mut B,
) -> Result<PathBuf, TranscriptionError> {
    create_dir_all(output_dir)?;
    let audio_path = if is_url(input) {
        let downloads = output_dir.join("downloads");
        create_dir_all(&downloads)?;
        downloader
            .download_audio(input, &downloads)
            .map_err(|message| TranscriptionError::Download {
                url: input.to_owned(),
                message,
            })?
    } else {
        PathBuf::from(input)
    };

    let stem = audio_path
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| TranscriptionError::MissingFileName(audio_path.clone()))?;
    let transcript_path = output_dir.join(stem).with_extension("txt");
    if transcript_path.exists() && !force {
        return Ok(transcript_path);
    }

    let prompt = initial_prompt.unwrap_or(FALLBACK_PROMPT);
    let selected_model = model_name();
    let result = backend
        .transcribe(&BackendRequest {
            audio_path: &audio_path,
            model: &selected_model,
            beam_size: BEAM_SIZE,
            initial_prompt: prompt,
        })
        .map_err(|message| TranscriptionError::Backend {
            path: audio_path,
            message,
        })?;
    let transcript = result
        .segments
        .iter()
        .map(|segment| segment.trim())
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    write_atomic(&transcript_path, transcript.as_bytes())?;
    Ok(transcript_path)
}

pub fn transcribe_all_with<B, D, W>(
    inputs: &[String],
    output_dir: &Path,
    initial_prompt: Option<&str>,
    downloader: &mut D,
    backend: &mut B,
    mut warn: W,
) -> Vec<PathBuf>
where
    B: WhisperBackend,
    D: AudioDownloader,
    W: FnMut(&str, &TranscriptionError),
{
    inputs
        .iter()
        .filter_map(|input| {
            match transcribe_with(
                input,
                output_dir,
                initial_prompt,
                false,
                downloader,
                backend,
            ) {
                Ok(path) => Some(path),
                Err(error) => {
                    warn(input, &error);
                    None
                }
            }
        })
        .collect()
}

fn create_dir_all(path: &Path) -> Result<(), TranscriptionError> {
    fs::create_dir_all(path).map_err(|source| TranscriptionError::Io {
        action: "create directory",
        path: path.to_path_buf(),
        source,
    })
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), TranscriptionError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| TranscriptionError::Io {
            action: "create temporary transcript in",
            path: parent.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(contents)
        .and_then(|()| temporary.flush())
        .map_err(|source| TranscriptionError::Io {
            action: "write transcript",
            path: path.to_path_buf(),
            source,
        })?;
    temporary
        .persist(path)
        .map_err(|error| TranscriptionError::Persist {
            path: path.to_path_buf(),
            message: error.error.to_string(),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Default)]
    struct StubBackend {
        calls: usize,
        results: VecDeque<Result<BackendTranscript, String>>,
        observed: Vec<(PathBuf, String, usize, String)>,
    }

    impl WhisperBackend for StubBackend {
        fn transcribe(
            &mut self,
            request: &BackendRequest<'_>,
        ) -> Result<BackendTranscript, String> {
            self.calls += 1;
            self.observed.push((
                request.audio_path.to_path_buf(),
                request.model.to_owned(),
                request.beam_size,
                request.initial_prompt.to_owned(),
            ));
            self.results
                .pop_front()
                .unwrap_or_else(|| Err("missing stub result".to_owned()))
        }
    }

    #[derive(Default)]
    struct StubDownloader {
        path: Option<PathBuf>,
        calls: usize,
    }

    impl AudioDownloader for StubDownloader {
        fn download_audio(&mut self, _url: &str, _output_dir: &Path) -> Result<PathBuf, String> {
            self.calls += 1;
            self.path
                .clone()
                .ok_or_else(|| "missing stub download".to_owned())
        }
    }

    #[test]
    fn compatibility_constants_match_python_surface() {
        assert!(VIDEO_EXTENSIONS.contains(&".mp4"));
        assert!(VIDEO_EXTENSIONS.contains(&".mp3"));
        assert!(VIDEO_EXTENSIONS.contains(&".wav"));
        assert!(VIDEO_EXTENSIONS.contains(&".mov"));
        assert!(!VIDEO_EXTENSIONS.contains(&".py"));
        assert_eq!(DEFAULT_MODEL, "base");
        assert_eq!(BEAM_SIZE, 5);
    }

    #[test]
    fn url_detection_is_intentionally_prefix_based() {
        assert!(is_url("http://example.com/a"));
        assert!(is_url("https://example.com/a"));
        assert!(is_url("www.example.com/a"));
        assert!(!is_url("HTTPS://example.com/a"));
        assert!(!is_url("/tmp/https://clip.mp4"));
    }

    #[test]
    fn prompts_match_python_selection_and_fallback_rules() {
        assert_eq!(
            build_whisper_prompt_with_override([], None),
            FALLBACK_PROMPT
        );
        let labels = ["one", "", "two", "three", "four", "five", "six"];
        assert_eq!(
            build_whisper_prompt_with_override(labels, None),
            "Technical discussion about one, two, three, four, five. Use proper punctuation and paragraph breaks."
        );
        assert_eq!(
            build_whisper_prompt_with_override(["Rust"], Some("Custom domain hint.")),
            "Custom domain hint."
        );
        assert_eq!(
            build_whisper_prompt_with_override([], Some("ignored without nodes")),
            FALLBACK_PROMPT
        );
    }

    #[test]
    fn audio_cache_hash_and_extension_order_are_stable() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        assert_eq!(
            audio_cache_key("https://example.com/watch?v=42"),
            "0b8aecba6b5b"
        );
        let key = audio_cache_key("https://example.com/watch?v=42");
        let webm = directory.path().join(format!("yt_{key}.webm"));
        let m4a = directory.path().join(format!("yt_{key}.m4a"));
        fs::write(&webm, [])?;
        fs::write(&m4a, [])?;
        assert_eq!(
            cached_audio_path("https://example.com/watch?v=42", directory.path()),
            Some(m4a)
        );
        Ok(())
    }

    #[test]
    fn existing_transcript_short_circuits_backend() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let audio = directory.path().join("lecture.mp4");
        fs::write(&audio, b"fake")?;
        let output = directory.path().join("transcripts");
        fs::create_dir_all(&output)?;
        let cached = output.join("lecture.txt");
        fs::write(&cached, b"Cached transcript content.")?;
        let mut backend = StubBackend::default();
        let mut downloader = StubDownloader::default();
        let result = transcribe_with(
            audio.to_string_lossy().as_ref(),
            &output,
            None,
            false,
            &mut downloader,
            &mut backend,
        )?;
        assert_eq!(result, cached);
        assert_eq!(backend.calls, 0);
        Ok(())
    }

    #[test]
    fn force_replaces_transcript_and_preserves_request_contract()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let audio = directory.path().join("talk.mp4");
        fs::write(&audio, b"fake")?;
        let output = directory.path().join("transcripts");
        fs::create_dir_all(&output)?;
        fs::write(output.join("talk.txt"), b"Old transcript.")?;
        let mut backend = StubBackend {
            results: VecDeque::from([Ok(BackendTranscript {
                segments: vec![
                    "  First. ".to_owned(),
                    "".to_owned(),
                    " Second.  ".to_owned(),
                ],
                language: Some("en".to_owned()),
            })]),
            ..StubBackend::default()
        };
        let mut downloader = StubDownloader::default();
        let result = transcribe_with(
            audio.to_string_lossy().as_ref(),
            &output,
            Some("Domain prompt."),
            true,
            &mut downloader,
            &mut backend,
        )?;
        assert_eq!(fs::read_to_string(result)?, "First.\nSecond.");
        assert_eq!(backend.calls, 1);
        assert_eq!(backend.observed[0].2, BEAM_SIZE);
        assert_eq!(backend.observed[0].3, "Domain prompt.");
        Ok(())
    }

    #[test]
    fn url_inputs_download_before_transcription() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let audio = directory.path().join("downloaded.m4a");
        fs::write(&audio, b"fake")?;
        let mut downloader = StubDownloader {
            path: Some(audio.clone()),
            calls: 0,
        };
        let mut backend = StubBackend {
            results: VecDeque::from([Ok(BackendTranscript {
                segments: vec!["Transcript".to_owned()],
                language: None,
            })]),
            ..StubBackend::default()
        };
        let transcript = transcribe_with(
            "https://example.com/video",
            &directory.path().join("out"),
            None,
            false,
            &mut downloader,
            &mut backend,
        )?;
        assert_eq!(downloader.calls, 1);
        assert_eq!(backend.observed[0].0, audio);
        assert_eq!(
            transcript.file_name(),
            Some(std::ffi::OsStr::new("downloaded.txt"))
        );
        Ok(())
    }

    #[test]
    fn batch_transcription_skips_failures_and_reports_them()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let first = directory.path().join("first.wav");
        let second = directory.path().join("second.wav");
        fs::write(&first, [])?;
        fs::write(&second, [])?;
        let mut backend = StubBackend {
            results: VecDeque::from([
                Err("bad audio".to_owned()),
                Ok(BackendTranscript {
                    segments: vec!["ok".to_owned()],
                    language: Some("en".to_owned()),
                }),
            ]),
            ..StubBackend::default()
        };
        let mut downloader = StubDownloader::default();
        let mut warnings = Vec::new();
        let paths = transcribe_all_with(
            &[
                first.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
            ],
            &directory.path().join("out"),
            None,
            &mut downloader,
            &mut backend,
            |input, error| warnings.push((input.to_owned(), error.to_string())),
        );
        assert_eq!(paths.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].1.contains("bad audio"));
        Ok(())
    }
}
