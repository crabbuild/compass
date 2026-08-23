use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use compass_media::document::{DocumentArtifact, DocumentOrigin};
use compass_media::{
    DocumentProcessingOptions, decode_document_with_ocr, render_document_markdown,
};
use compass_ocr::{ManagedOarEngine, ModelProfile, OcrMode, normalize_language_hints};
use serde::Serialize;

use crate::Outcome;

#[derive(Serialize)]
struct InspectEnvelope<'a> {
    schema: &'static str,
    source: String,
    artifact: &'a DocumentArtifact,
    processing: InspectProcessing<'a>,
    limits: InspectLimits,
}

#[derive(Serialize)]
struct InspectProcessing<'a> {
    ocr_mode: OcrMode,
    ocr_profile: ModelProfile,
    language_hints: &'a [String],
    allow_partial: bool,
}

#[derive(Serialize)]
struct InspectLimits {
    max_raw_bytes: u64,
    max_pdf_pages: usize,
    max_office_images: usize,
    max_raster_pixels: u64,
    max_raster_long_edge: u32,
    max_aggregate_pixels: u64,
    max_observations_per_raster: usize,
    max_observations_per_document: usize,
    max_text_bytes_per_observation: usize,
    max_text_chars_per_document: usize,
}

pub(crate) fn command(args: &[String]) -> Outcome {
    match args.first().map(String::as_str) {
        Some("inspect") => inspect(&args[1..]),
        Some(other) => Outcome::from_command_output(
            2,
            String::new(),
            format!("error: unknown document command {other:?}"),
        ),
        None => Outcome::from_command_output(
            2,
            String::new(),
            "error: missing document command; expected inspect".to_owned(),
        ),
    }
}

fn inspect(args: &[String]) -> Outcome {
    let mut path = None;
    let mut format = "text";
    let mut mode = OcrMode::Off;
    let mut profile = ModelProfile::PpOcrV6Small;
    let mut languages = Vec::new();
    let mut allow_partial = false;
    let mut index = 0_usize;
    while index < args.len() {
        match args[index].as_str() {
            "--format" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--format requires text or json");
                };
                if !matches!(value.as_str(), "text" | "json") {
                    return usage_error("--format requires text or json");
                }
                format = value;
            }
            "--ocr" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--ocr requires off, auto, or always");
                };
                mode = match OcrMode::from_str(value) {
                    Ok(mode) => mode,
                    Err(error) => return usage_error(&error.to_string()),
                };
            }
            "--ocr-profile" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--ocr-profile requires a profile name");
                };
                profile = match ModelProfile::from_str(value) {
                    Ok(profile) => profile,
                    Err(error) => return usage_error(&error.to_string()),
                };
            }
            "--ocr-language" => {
                index += 1;
                let Some(value) = args.get(index) else {
                    return usage_error("--ocr-language requires a BCP-47 language tag");
                };
                languages.push(value.clone());
            }
            "--allow-partial" => allow_partial = true,
            value if value.starts_with('-') => {
                return usage_error(&format!("unknown document option {value:?}"));
            }
            value if path.is_none() => path = Some(PathBuf::from(value)),
            value => return usage_error(&format!("unexpected argument {value:?}")),
        }
        index += 1;
    }
    let Some(path) = path else {
        return usage_error("document inspect requires a file");
    };
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return Outcome::failure(format!("error: could not stat {}: {error}", path.display()));
        }
    };
    if metadata.len() > compass_media::MEDIA_MAX_RAW_BYTES {
        return Outcome::failure(format!(
            "error: {} exceeds the document source limit of {} bytes",
            path.display(),
            compass_media::MEDIA_MAX_RAW_BYTES
        ));
    }
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Outcome::failure(format!("error: could not read {}: {error}", path.display()));
        }
    };
    if bytes.len() as u64 > compass_media::MEDIA_MAX_RAW_BYTES {
        return Outcome::failure(format!(
            "error: {} changed while reading and exceeds the document source limit",
            path.display()
        ));
    }
    languages = match normalize_language_hints(&languages) {
        Ok(languages) => languages,
        Err(error) => return usage_error(&error.to_string()),
    };
    let engine = if mode == OcrMode::Off {
        None
    } else {
        match ManagedOarEngine::load(profile) {
            Ok(engine) => Some(engine),
            Err(error) => {
                return Outcome::failure(format!(
                    "error: {error}\nhelp: Compass manages all OCR runtime and model dependencies; no system OCR package is required"
                ));
            }
        }
    };
    let options = DocumentProcessingOptions {
        ocr_mode: mode,
        language_hints: languages.clone(),
        allow_partial,
    };
    let artifact = match decode_document_with_ocr(
        &path,
        &bytes,
        &options,
        engine.as_ref().map(|engine| engine as _),
    ) {
        Ok(artifact) => artifact,
        Err(error) => {
            return Outcome::failure(format!(
                "error: could not process {}: {error}",
                path.display()
            ));
        }
    };
    if format == "json" {
        let envelope = InspectEnvelope {
            schema: compass_media::DOCUMENT_INSPECT_SCHEMA,
            source: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("document")
                .to_owned(),
            artifact: &artifact,
            processing: InspectProcessing {
                ocr_mode: mode,
                ocr_profile: profile,
                language_hints: &languages,
                allow_partial,
            },
            limits: InspectLimits {
                max_raw_bytes: compass_media::MEDIA_MAX_RAW_BYTES,
                max_pdf_pages: compass_media::OCR_MAX_PDF_PAGES,
                max_office_images: compass_media::OCR_MAX_OOXML_IMAGES,
                max_raster_pixels: compass_ocr::OCR_MAX_RASTER_PIXELS,
                max_raster_long_edge: compass_ocr::OCR_MAX_RASTER_LONG_EDGE,
                max_aggregate_pixels: compass_media::OCR_MAX_AGGREGATE_PIXELS,
                max_observations_per_raster: compass_ocr::OCR_MAX_OBSERVATIONS_PER_RASTER,
                max_observations_per_document: compass_ocr::OCR_MAX_OBSERVATIONS_PER_DOCUMENT,
                max_text_bytes_per_observation: compass_ocr::OCR_MAX_TEXT_BYTES_PER_OBSERVATION,
                max_text_chars_per_document: compass_ocr::OCR_MAX_TEXT_CHARS_PER_DOCUMENT,
            },
        };
        return match serde_json::to_string_pretty(&envelope) {
            Ok(json) => Outcome::success(json),
            Err(error) => {
                Outcome::failure(format!("error: could not encode document JSON: {error}"))
            }
        };
    }
    let mut output = match render_document_markdown(&artifact) {
        Ok(output) => output,
        Err(error) => {
            return Outcome::failure(format!("error: could not render document: {error}"));
        }
    };
    output.push_str(&format!(
        "\n\n[document processing: ocr={mode:?}, profile={}, allow_partial={allow_partial}, complete={}]",
        profile.name(), artifact.complete
    ));
    let ocr_blocks = artifact
        .blocks
        .iter()
        .filter_map(|block| match &block.origin {
            DocumentOrigin::Ocr { confidence_bps, .. } => Some((block, confidence_bps)),
            DocumentOrigin::Native => None,
        })
        .collect::<Vec<_>>();
    if !ocr_blocks.is_empty() {
        output.push_str("\n\n## OCR-derived evidence\n");
        for (block, confidence) in ocr_blocks {
            output.push_str(&format!(
                "\n- [OCR {:02}.{:02}% @ {:?}] {}",
                confidence / 100,
                confidence % 100,
                block.locator,
                block.text
            ));
        }
    }
    if !artifact.diagnostics.is_empty() {
        output.push_str("\n\n## Diagnostics\n");
        for diagnostic in &artifact.diagnostics {
            output.push_str(&format!(
                "\n- {} ({:?}): {}",
                diagnostic.code, diagnostic.severity, diagnostic.message
            ));
        }
    }
    Outcome::success(output)
}

fn usage_error(message: &str) -> Outcome {
    Outcome::from_command_output(
        2,
        String::new(),
        format!(
            "error: {message}\nusage: compass document inspect <FILE> [--format text|json] [--ocr off|auto|always] [--ocr-profile <NAME>] [--ocr-language <TAG>]..."
        ),
    )
}
