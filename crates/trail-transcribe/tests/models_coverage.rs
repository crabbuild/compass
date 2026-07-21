use std::error::Error;
use std::io::{Cursor, Read};

use trail_transcribe::models::{ArtifactFetcher, MODEL_SPECS, ModelCache, ModelError, model_spec};

struct FixtureFetcher(Result<Vec<u8>, String>);

impl ArtifactFetcher for FixtureFetcher {
    fn fetch(&self, _url: &str, _max_bytes: u64) -> Result<Box<dyn Read>, String> {
        self.0
            .as_ref()
            .map(|bytes| Box::new(Cursor::new(bytes.clone())) as Box<dyn Read>)
            .map_err(Clone::clone)
    }
}

#[test]
fn model_registry_aliases_and_unknown_models_are_explicit() -> Result<(), Box<dyn Error>> {
    assert_eq!(model_spec("large").map(|spec| spec.name), Some("large-v3"));
    assert_eq!(
        model_spec("turbo").map(|spec| spec.name),
        Some("large-v3-turbo")
    );
    assert!(model_spec("missing-model").is_none());
    assert!(MODEL_SPECS.len() >= 10);

    let directory = tempfile::tempdir()?;
    let cache = ModelCache::new(directory.path().join("models"));
    assert!(matches!(
        cache.ensure_model("missing-model", &FixtureFetcher(Ok(Vec::new()))),
        Err(ModelError::UnsupportedModel(_))
    ));
    Ok(())
}

#[test]
fn model_cache_bounds_directory_download_size_and_digest_failures() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let blocked = directory.path().join("blocked");
    std::fs::write(&blocked, b"file")?;
    assert!(matches!(
        ModelCache::new(blocked).ensure_model("tiny", &FixtureFetcher(Ok(Vec::new()))),
        Err(ModelError::Io { .. })
    ));

    let download_root = directory.path().join("download-error");
    assert!(matches!(
        ModelCache::new(download_root)
            .ensure_model("tiny", &FixtureFetcher(Err("offline".to_owned()))),
        Err(ModelError::Download { .. })
    ));

    let too_small_root = directory.path().join("too-small");
    assert!(matches!(
        ModelCache::new(too_small_root).ensure_model("tiny", &FixtureFetcher(Ok(vec![0; 1]))),
        Err(ModelError::Size { actual: 1, .. })
    ));

    let first_size = model_spec("tiny")
        .and_then(|spec| spec.artifacts.first())
        .map(|artifact| artifact.size)
        .ok_or("tiny model has no artifact")?;
    let too_large_root = directory.path().join("too-large");
    assert!(matches!(
        ModelCache::new(too_large_root).ensure_model(
            "tiny",
            &FixtureFetcher(Ok(vec![0; usize::try_from(first_size + 1)?]))
        ),
        Err(ModelError::Size { .. })
    ));

    let digest_root = directory.path().join("digest");
    assert!(matches!(
        ModelCache::new(digest_root).ensure_model(
            "tiny",
            &FixtureFetcher(Ok(vec![0; usize::try_from(first_size)?]))
        ),
        Err(ModelError::Digest { .. })
    ));
    Ok(())
}
