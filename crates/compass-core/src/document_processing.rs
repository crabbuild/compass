//! Application-owned preparation of rich documents for structural and
//! semantic consumers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use compass_media::document::DocumentArtifact;
use compass_ocr::{
    ManagedOarEngine, ModelProfile, OCR_POLICY_VERSION, OCR_PREPROCESSING_VERSION,
    OCR_PROTOCOL_SCHEMA, OCR_SCHEMA, OcrEngine, OcrMode, normalize_language_hints,
    profile_manifest_digest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_CACHED_DOCUMENT_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreDocumentProcessingOptions {
    pub ocr_mode: OcrMode,
    pub ocr_profile: ModelProfile,
    pub language_hints: Vec<String>,
    pub allow_partial: bool,
    pub cache_directory: Option<PathBuf>,
}

impl Default for CoreDocumentProcessingOptions {
    fn default() -> Self {
        Self {
            ocr_mode: OcrMode::Off,
            ocr_profile: ModelProfile::PpOcrV6Small,
            language_hints: Vec::new(),
            allow_partial: false,
            cache_directory: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PreparedDocument {
    pub artifact: Arc<DocumentArtifact>,
    pub semantic_text: Arc<str>,
    pub cache_identity: String,
    pub ocr_mode: OcrMode,
}

#[derive(Clone, Debug)]
pub struct PreparedDocumentSet {
    pub documents: BTreeMap<PathBuf, PreparedDocument>,
    pub cache_identity: String,
}

impl Default for PreparedDocumentSet {
    fn default() -> Self {
        Self {
            documents: BTreeMap::new(),
            cache_identity: cache_identity(&CoreDocumentProcessingOptions::default()),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CachedDocumentArtifact {
    schema: String,
    source_digest: String,
    cache_identity: String,
    artifact: DocumentArtifact,
}

impl PreparedDocumentSet {
    #[must_use]
    pub fn get(&self, path: &Path) -> Option<&PreparedDocument> {
        self.documents.get(path)
    }
}

pub fn prepare_document_set(
    paths: &[PathBuf],
    options: &CoreDocumentProcessingOptions,
) -> Result<PreparedDocumentSet, String> {
    let mut rich_paths = paths
        .iter()
        .filter(|path| is_rich_document(path))
        .cloned()
        .collect::<Vec<_>>();
    rich_paths.sort();
    rich_paths.dedup();
    let mut engine = None;
    let language_hints =
        normalize_language_hints(&options.language_hints).map_err(|error| error.to_string())?;
    let mut canonical_options = options.clone();
    canonical_options.language_hints.clone_from(&language_hints);
    let processing = compass_media::DocumentProcessingOptions {
        ocr_mode: options.ocr_mode,
        language_hints,
        allow_partial: options.allow_partial,
    };
    let cache_identity = cache_identity(&canonical_options);
    let mut documents = BTreeMap::new();
    for path in rich_paths {
        let bytes = compass_media::read_document_bounded(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let source_digest = format!("sha256:{:x}", Sha256::digest(&bytes));
        let cache_path = options
            .cache_directory
            .as_deref()
            .map(|directory| document_cache_path(directory, &source_digest, &cache_identity));
        let artifact = match cache_path.as_deref() {
            Some(cache_path) if cache_path.is_file() => {
                load_cached_document(cache_path, &source_digest, &cache_identity)?
            }
            _ => {
                if options.ocr_mode != OcrMode::Off && engine.is_none() {
                    engine = Some(
                        ManagedOarEngine::load(options.ocr_profile)
                            .map_err(|error| error.to_string())?,
                    );
                }
                let artifact = compass_media::decode_document_with_ocr(
                    &path,
                    &bytes,
                    &processing,
                    engine.as_ref().map(|engine| engine as &dyn OcrEngine),
                )
                .map_err(|error| format!("{}: {error}", path.display()))?;
                if artifact.complete
                    && !matches!(
                        artifact.visual_coverage,
                        compass_media::document::VisualCoverage::Partial
                            | compass_media::document::VisualCoverage::Failed
                    )
                    && let Some(cache_path) = cache_path.as_deref()
                {
                    let cached = CachedDocumentArtifact {
                        schema: "compass.document.cache/1".to_owned(),
                        source_digest: source_digest.clone(),
                        cache_identity: cache_identity.clone(),
                        artifact: artifact.clone(),
                    };
                    compass_files::write_json_atomic(cache_path, &cached, false)
                        .map_err(|error| error.to_string())?;
                }
                artifact
            }
        };
        let semantic_text = compass_media::render_document_markdown(&artifact)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        documents.insert(
            path,
            PreparedDocument {
                artifact: Arc::new(artifact),
                semantic_text: Arc::<str>::from(semantic_text),
                cache_identity: cache_identity.clone(),
                ocr_mode: options.ocr_mode,
            },
        );
    }
    Ok(PreparedDocumentSet {
        documents,
        cache_identity,
    })
}

fn document_cache_path(directory: &Path, source_digest: &str, cache_identity: &str) -> PathBuf {
    let key = format!(
        "{:x}",
        Sha256::digest(format!("{source_digest}\0{cache_identity}"))
    );
    directory.join(&key[..2]).join(format!("{key}.json"))
}

fn load_cached_document(
    path: &Path,
    source_digest: &str,
    cache_identity: &str,
) -> Result<DocumentArtifact, String> {
    let bytes =
        compass_files::read_bytes_bounded(path, MAX_CACHED_DOCUMENT_BYTES).map_err(|error| {
            format!(
                "could not read bounded document cache {}: {error}",
                path.display()
            )
        })?;
    let cached: CachedDocumentArtifact = serde_json::from_slice(&bytes)
        .map_err(|error| format!("document cache {} is corrupt: {error}", path.display()))?;
    if cached.schema != "compass.document.cache/1"
        || cached.source_digest != source_digest
        || cached.cache_identity != cache_identity
    {
        return Err(format!(
            "document cache {} has an incompatible identity",
            path.display()
        ));
    }
    cached
        .artifact
        .validate()
        .map_err(|error| format!("document cache {} is invalid: {error}", path.display()))?;
    Ok(cached.artifact)
}

fn cache_identity(options: &CoreDocumentProcessingOptions) -> String {
    let mut languages = options.language_hints.clone();
    languages.sort();
    languages.dedup();
    let mode = match options.ocr_mode {
        OcrMode::Off => "off",
        OcrMode::Auto => "auto",
        OcrMode::Always => "always",
    };
    format!(
        "schema={};normalizer={};ocr_schema={};ocr_protocol={};ocr_policy={};preprocess={};rasterizer={};mode={};profile={};model_manifest={};languages={};raw_bytes={};pdf_pages={};office_images={};raster_pixels={};raster_edge={};engine_side={};tile_overlap={};aggregate_pixels={};regions_raster={};regions_document={};text_region={};text_document={};wall_time_seconds={}",
        compass_media::DOCUMENT_SCHEMA,
        compass_media::DOCUMENT_NORMALIZER_VERSION,
        OCR_SCHEMA,
        OCR_PROTOCOL_SCHEMA,
        OCR_POLICY_VERSION,
        OCR_PREPROCESSING_VERSION,
        compass_media::PDF_RASTERIZER_IDENTITY,
        mode,
        options.ocr_profile.name(),
        profile_manifest_digest(options.ocr_profile),
        languages.join(","),
        compass_media::MEDIA_MAX_RAW_BYTES,
        compass_media::OCR_MAX_PDF_PAGES,
        compass_media::OCR_MAX_OOXML_IMAGES,
        compass_ocr::OCR_MAX_RASTER_PIXELS,
        compass_ocr::OCR_MAX_RASTER_LONG_EDGE,
        compass_ocr::OCR_ENGINE_MAX_SIDE,
        compass_ocr::OCR_TILE_OVERLAP,
        compass_media::OCR_MAX_AGGREGATE_PIXELS,
        compass_ocr::OCR_MAX_OBSERVATIONS_PER_RASTER,
        compass_ocr::OCR_MAX_OBSERVATIONS_PER_DOCUMENT,
        compass_ocr::OCR_MAX_TEXT_BYTES_PER_OBSERVATION,
        compass_ocr::OCR_MAX_TEXT_CHARS_PER_DOCUMENT,
        compass_ocr::OCR_MAX_DOCUMENT_WALL_TIME_SECS,
    )
}

fn is_rich_document(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["pdf", "docx", "xlsx", "pptx"]
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

pub(crate) fn project_document(
    source_file: &str,
    path: &Path,
    artifact: &DocumentArtifact,
    cache_identity: &str,
    ocr_mode: OcrMode,
) -> Result<compass_languages::Extraction, String> {
    artifact.validate().map_err(|error| error.to_string())?;
    let file_id = compass_languages::make_id(&[source_file]);
    let mut extraction = compass_languages::Extraction {
        raw_calls: None,
        ..compass_languages::Extraction::default()
    };
    let mut root = serde_json::Map::new();
    root.insert(
        "label".to_owned(),
        serde_json::Value::String(
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(source_file)
                .to_owned(),
        ),
    );
    root.insert("file_type".to_owned(), serde_json::json!("document"));
    root.insert("document_kind".to_owned(), serde_json::json!("document"));
    root.insert(
        "document_format".to_owned(),
        serde_json::to_value(artifact.format).map_err(|error| error.to_string())?,
    );
    root.insert("source_file".to_owned(), serde_json::json!(source_file));
    root.insert("source_location".to_owned(), serde_json::json!("document"));
    root.insert("_origin".to_owned(), serde_json::json!("artifact"));
    root.insert(
        "document_schema".to_owned(),
        serde_json::json!(artifact.schema),
    );
    root.insert(
        "document_normalizer_version".to_owned(),
        serde_json::json!(artifact.normalizer_version),
    );
    root.insert(
        "document_complete".to_owned(),
        serde_json::json!(artifact.complete),
    );
    root.insert(
        "document_visual_coverage".to_owned(),
        serde_json::to_value(artifact.visual_coverage).map_err(|error| error.to_string())?,
    );
    root.insert(
        "document_ocr_mode".to_owned(),
        serde_json::to_value(ocr_mode).map_err(|error| error.to_string())?,
    );
    if !artifact.metadata.is_empty() {
        root.insert(
            "document_metadata".to_owned(),
            serde_json::to_value(&artifact.metadata).map_err(|error| error.to_string())?,
        );
    }
    if let Some(profile) = &artifact.ocr_profile {
        root.insert(
            "document_ocr_profile".to_owned(),
            serde_json::to_value(profile).map_err(|error| error.to_string())?,
        );
    }
    extraction.nodes.push(compass_languages::RawNodeRecord {
        id: file_id.clone(),
        attributes: root,
    });
    let mut block_ids = BTreeMap::new();
    for block in &artifact.blocks {
        let ordinal = block.ordinal.to_string();
        let id = compass_languages::make_id(&[source_file, "document-block", &ordinal]);
        block_ids.insert(block.ordinal, id.clone());
        let locator = serde_json::to_value(&block.locator).map_err(|error| error.to_string())?;
        let location = serde_json::to_string(&locator).map_err(|error| error.to_string())?;
        let kind = serde_json::to_value(&block.kind).map_err(|error| error.to_string())?;
        let kind_name = kind
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("other");
        let mut attributes = serde_json::Map::new();
        attributes.insert(
            "label".to_owned(),
            serde_json::json!(bounded_label(&block.text)),
        );
        attributes.insert("file_type".to_owned(), serde_json::json!("document"));
        attributes.insert("document_kind".to_owned(), serde_json::json!(kind_name));
        attributes.insert("source_file".to_owned(), serde_json::json!(source_file));
        attributes.insert("source_location".to_owned(), serde_json::json!(location));
        attributes.insert("document_locator".to_owned(), locator);
        attributes.insert("block_index".to_owned(), serde_json::json!(block.ordinal));
        attributes.insert("_origin".to_owned(), serde_json::json!("artifact"));
        attributes.insert("document_text".to_owned(), serde_json::json!(block.text));
        attributes.insert(
            "document_origin".to_owned(),
            serde_json::to_value(&block.origin).map_err(|error| error.to_string())?,
        );
        if !block.metadata.is_empty() {
            attributes.insert(
                "document_metadata".to_owned(),
                serde_json::to_value(&block.metadata).map_err(|error| error.to_string())?,
            );
        }
        extraction
            .nodes
            .push(compass_languages::RawNodeRecord { id, attributes });
    }
    for block in &artifact.blocks {
        let Some(target) = block_ids.get(&block.ordinal) else {
            return Err("prepared document block identity disappeared".to_owned());
        };
        let source = match block.parent {
            Some(parent) => block_ids
                .get(&parent)
                .ok_or_else(|| "prepared document parent identity disappeared".to_owned())?,
            None => &file_id,
        };
        let mut attributes = serde_json::Map::new();
        attributes.insert("relation".to_owned(), serde_json::json!("contains"));
        attributes.insert("confidence".to_owned(), serde_json::json!("EXTRACTED"));
        attributes.insert("source_file".to_owned(), serde_json::json!(source_file));
        attributes.insert("source_location".to_owned(), serde_json::json!("document"));
        attributes.insert("_origin".to_owned(), serde_json::json!("artifact"));
        attributes.insert("weight".to_owned(), serde_json::json!(1.0));
        extraction.edges.push(compass_languages::RawEdgeRecord {
            source: source.clone(),
            target: target.clone(),
            attributes,
        });
    }
    if !artifact.links.is_empty() {
        extraction.extensions.insert(
            "document_links".to_owned(),
            serde_json::to_value(&artifact.links).map_err(|error| error.to_string())?,
        );
    }
    if !artifact.diagnostics.is_empty() {
        extraction.extensions.insert(
            "document_diagnostics".to_owned(),
            serde_json::to_value(&artifact.diagnostics).map_err(|error| error.to_string())?,
        );
    }
    extraction.extensions.insert(
        "document_cache_identity".to_owned(),
        serde_json::json!(cache_identity),
    );
    Ok(extraction)
}

fn bounded_label(text: &str) -> String {
    let mut output = text.chars().take(512).collect::<String>();
    if text.chars().count() > 512 {
        output.push('…');
    }
    output
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write as _;

    use super::*;

    #[test]
    fn prepared_document_cache_is_reused_and_corruption_is_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let document = directory.path().join("report.docx");
        let file = fs::File::create(&document)?;
        let mut archive = zip::ZipWriter::new(file);
        archive.start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )?;
        archive.write_all(br#"<w:document xmlns:w="urn:w"><w:body><w:p><w:r><w:t>Sentinel text</w:t></w:r></w:p></w:body></w:document>"#)?;
        archive.finish()?;
        let options = CoreDocumentProcessingOptions {
            cache_directory: Some(directory.path().join("cache")),
            ..CoreDocumentProcessingOptions::default()
        };
        let first = prepare_document_set(std::slice::from_ref(&document), &options)?;
        let prepared = first.get(&document).ok_or("prepared document is missing")?;
        assert!(prepared.semantic_text.contains("Sentinel text"));
        let source_digest = format!("sha256:{:x}", Sha256::digest(fs::read(&document)?));
        let cache_path = document_cache_path(
            options
                .cache_directory
                .as_deref()
                .ok_or("cache directory missing")?,
            &source_digest,
            &first.cache_identity,
        );
        assert!(cache_path.is_file());
        let second = prepare_document_set(std::slice::from_ref(&document), &options)?;
        assert_eq!(
            first
                .get(&document)
                .map(|value| value.semantic_text.as_ref()),
            second
                .get(&document)
                .map(|value| value.semantic_text.as_ref())
        );
        let mut unknown: serde_json::Value = serde_json::from_slice(&fs::read(&cache_path)?)?;
        unknown["unexpected"] = serde_json::json!(true);
        fs::write(&cache_path, serde_json::to_vec(&unknown)?)?;
        let error = prepare_document_set(std::slice::from_ref(&document), &options)
            .err()
            .ok_or("cache with an unknown field was accepted")?;
        assert!(error.contains("corrupt"));

        fs::write(&cache_path, b"{")?;
        let error = prepare_document_set(std::slice::from_ref(&document), &options)
            .err()
            .ok_or("corrupt cache was accepted")?;
        assert!(error.contains("corrupt"));

        let oversized = fs::File::create(&cache_path)?;
        oversized.set_len(MAX_CACHED_DOCUMENT_BYTES + 1)?;
        let error = prepare_document_set(std::slice::from_ref(&document), &options)
            .err()
            .ok_or("oversized cache was accepted")?;
        assert!(error.contains("bounded document cache"));
        Ok(())
    }

    #[test]
    fn projection_retains_document_blocks_and_typed_locators()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut artifact = DocumentArtifact::new(compass_media::document::DocumentFormat::Docx);
        artifact.push_block(
            None,
            compass_media::document::DocumentBlockKind::Paragraph,
            "Projected sentinel".to_owned(),
            compass_media::document::DocumentLocator::Package {
                part: "word/document.xml".to_owned(),
                path: "body/p[1]".to_owned(),
            },
        )?;
        let extraction = project_document(
            "docs/report.docx",
            Path::new("report.docx"),
            &artifact,
            "fixture-identity",
            OcrMode::Off,
        )?;
        assert_eq!(extraction.nodes.len(), 2);
        assert_eq!(extraction.edges.len(), 1);
        assert_eq!(
            extraction.nodes[1]
                .attributes
                .get("document_text")
                .and_then(serde_json::Value::as_str),
            Some("Projected sentinel")
        );
        assert_eq!(
            extraction.nodes[0].attributes.get("document_ocr_mode"),
            Some(&serde_json::json!("off"))
        );
        assert_eq!(
            extraction.extensions.get("document_cache_identity"),
            Some(&serde_json::json!("fixture-identity"))
        );
        Ok(())
    }
}
