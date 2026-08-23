//! Selective OCR policy and provenance-preserving artifact fusion.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use compass_ocr::{
    OCR_MAX_OBSERVATIONS_PER_DOCUMENT, OCR_MAX_TEXT_CHARS_PER_DOCUMENT, OCR_SCHEMA, OcrEngine,
    OcrMode, OcrObservation, OcrRequest, OcrResponse, OcrSourceKind, PreparedRaster,
    normalize_language_hints, prepare_raster_cancellable, prepared_raster_digest, tile_raster,
};
use unicode_normalization::UnicodeNormalization;

use crate::document::{
    DiagnosticSeverity, DocumentArtifact, DocumentBlockKind, DocumentDiagnostic, DocumentError,
    DocumentFormat, DocumentLocator, DocumentOrigin, VisualCoverage,
};
use crate::limits::{OCR_MAX_AGGREGATE_PIXELS, OCR_MAX_OOXML_IMAGES, OCR_MAX_PDF_PAGES};
use crate::{decode_document, raster_candidates, rasterize_pdf_pages_cancellable};

#[derive(Clone, Debug)]
pub struct DocumentProcessingOptions {
    pub ocr_mode: OcrMode,
    pub language_hints: Vec<String>,
    pub allow_partial: bool,
}

impl Default for DocumentProcessingOptions {
    fn default() -> Self {
        Self {
            ocr_mode: OcrMode::Off,
            language_hints: Vec::new(),
            allow_partial: false,
        }
    }
}

pub fn decode_document_with_ocr(
    logical_path: &Path,
    bytes: &[u8],
    options: &DocumentProcessingOptions,
    engine: Option<&dyn OcrEngine>,
) -> Result<DocumentArtifact, DocumentError> {
    decode_document_with_ocr_cancellable(
        logical_path,
        bytes,
        options,
        engine,
        &AtomicBool::new(false),
    )
}

pub fn decode_document_with_ocr_cancellable(
    logical_path: &Path,
    bytes: &[u8],
    options: &DocumentProcessingOptions,
    engine: Option<&dyn OcrEngine>,
    cancellation: &AtomicBool,
) -> Result<DocumentArtifact, DocumentError> {
    let started = Instant::now();
    check_cancelled(cancellation)?;
    let mut artifact = decode_document(logical_path, bytes)?;
    if options.ocr_mode == OcrMode::Off {
        return Ok(artifact);
    }
    let engine = engine.ok_or_else(|| {
        DocumentError::Ocr(compass_ocr::OcrError::EngineUnavailable(
            "OCR was requested but no verified engine profile was loaded".to_owned(),
        ))
    })?;
    engine.identity().validate().map_err(DocumentError::Ocr)?;
    let languages =
        normalize_language_hints(&options.language_hints).map_err(DocumentError::Ocr)?;
    let extension = logical_path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let mut requests = Vec::new();
    let mut aggregate_pixels = 0_u64;
    let mut candidate_failed = false;
    match artifact.format {
        DocumentFormat::Pdf => {
            let selected = selected_pdf_pages(&artifact, options.ocr_mode)?;
            for candidate in rasterize_pdf_pages_cancellable(bytes, &selected, cancellation)? {
                check_deadline(started)?;
                reserve_aggregate_pixels(&mut aggregate_pixels, &candidate.raster)?;
                requests.push((
                    candidate.id,
                    candidate.owner,
                    OcrSourceKind::PdfPage,
                    candidate.raster,
                ));
            }
        }
        DocumentFormat::Docx | DocumentFormat::Xlsx | DocumentFormat::Pptx => {
            let candidates = raster_candidates(&extension, bytes)?;
            if candidates.len() > OCR_MAX_OOXML_IMAGES {
                return Err(DocumentError::Rejected(
                    "embedded image count exceeds OCR limit".to_owned(),
                ));
            }
            for candidate in candidates {
                check_cancelled(cancellation)?;
                check_deadline(started)?;
                match prepare_raster_cancellable(&candidate.bytes, cancellation) {
                    Ok(raster) => {
                        if options.ocr_mode == OcrMode::Auto
                            && (raster.width < 64
                                || raster.height < 64
                                || u64::from(raster.width) * u64::from(raster.height) < 4_096)
                        {
                            artifact.diagnostics.push(DocumentDiagnostic {
                                code: "ocr_candidate_skipped_too_small".to_owned(),
                                severity: DiagnosticSeverity::Info,
                                locator: Some(candidate.owner),
                                message: "Embedded image is below the automatic OCR size threshold"
                                    .to_owned(),
                            });
                            continue;
                        }
                        reserve_aggregate_pixels(&mut aggregate_pixels, &raster)?;
                        requests.push((
                            candidate.id,
                            candidate.owner,
                            OcrSourceKind::EmbeddedImage,
                            raster,
                        ));
                    }
                    Err(error) if options.allow_partial => {
                        candidate_failed = true;
                        artifact.complete = false;
                        record_ocr_failure(&mut artifact, candidate.owner, &error);
                    }
                    Err(error) => return Err(DocumentError::Ocr(error)),
                }
            }
        }
        _ => {
            return Err(DocumentError::Unsupported(format!(
                "OCR for {:?}",
                artifact.format
            )));
        }
    }
    let mut aggregate_observations = 0_usize;
    let mut aggregate_text = 0_usize;
    let mut profile = Some(engine.identity().clone());
    let mut failed = candidate_failed;
    let mut succeeded = 0_usize;
    let mut reused_responses = BTreeMap::<String, compass_ocr::OcrResponse>::new();
    let native_page_text = native_pdf_page_text(&artifact);
    for (candidate_id, owner, source_kind, raster) in requests {
        check_cancelled(cancellation)?;
        check_deadline(started)?;
        let digest = prepared_raster_digest(&raster);
        let request = OcrRequest {
            schema: OCR_SCHEMA.to_owned(),
            request_id: candidate_id.clone(),
            source_kind,
            width: raster.width,
            height: raster.height,
            language_hints: languages.clone(),
            image_digest: digest.clone(),
        };
        let response = match reused_responses.get(&digest) {
            Some(cached) => {
                let mut reused = cached.clone();
                reused.request_id.clone_from(&request.request_id);
                reused
            }
            None => match recognize_tiled(engine, &request, &raster, cancellation, started) {
                Ok(response) => response,
                Err(compass_ocr::OcrError::Cancelled) => {
                    return Err(DocumentError::Ocr(compass_ocr::OcrError::Cancelled));
                }
                Err(error) if options.allow_partial => {
                    failed = true;
                    artifact.complete = false;
                    record_ocr_failure(&mut artifact, owner, &error);
                    continue;
                }
                Err(error) => return Err(DocumentError::Ocr(error)),
            },
        };
        check_deadline(started)?;
        if let Err(error) = response.validate_for(&request) {
            if options.allow_partial {
                failed = true;
                artifact.complete = false;
                record_ocr_failure(&mut artifact, owner, &error);
                continue;
            }
            return Err(DocumentError::Ocr(error));
        }
        reused_responses
            .entry(digest)
            .or_insert_with(|| response.clone());
        succeeded = succeeded.saturating_add(1);
        if profile
            .as_ref()
            .is_some_and(|identity| identity != &response.profile)
        {
            return Err(DocumentError::InvalidArtifact(
                "OCR engine profile changed within one document".to_owned(),
            ));
        }
        profile = Some(response.profile.clone());
        aggregate_observations = aggregate_observations
            .checked_add(response.observations.len())
            .ok_or_else(|| DocumentError::Rejected("OCR observation overflow".to_owned()))?;
        if aggregate_observations > OCR_MAX_OBSERVATIONS_PER_DOCUMENT {
            return Err(DocumentError::Rejected(
                "OCR observations exceed document limit".to_owned(),
            ));
        }
        let parent = parent_for_locator(&artifact, &owner);
        let reading_order_approximate = response
            .observations
            .iter()
            .any(|observation| observation.script.as_deref() != Some("Latn"));
        for observation in response.observations {
            aggregate_text = aggregate_text
                .checked_add(observation.text.chars().count())
                .ok_or_else(|| DocumentError::Rejected("OCR text size overflow".to_owned()))?;
            if aggregate_text > OCR_MAX_TEXT_CHARS_PER_DOCUMENT {
                return Err(DocumentError::Rejected(
                    "OCR text exceeds document limit".to_owned(),
                ));
            }
            let locator = DocumentLocator::Ocr {
                owner: Box::new(owner.clone()),
                candidate_id: candidate_id.clone(),
                width: raster.width,
                height: raster.height,
                polygon: observation.polygon,
                occurrence: observation.ordinal,
            };
            let ordinal = artifact.push_block(
                parent,
                DocumentBlockKind::Paragraph,
                observation.text,
                locator.clone(),
            )?;
            let block = artifact.blocks.get_mut(ordinal as usize).ok_or_else(|| {
                DocumentError::InvalidArtifact("new OCR block disappeared".to_owned())
            })?;
            block.origin = DocumentOrigin::Ocr {
                profile: response.profile.clone(),
                confidence_bps: observation.confidence_bps,
            };
            block.metadata.insert(
                "ocr_orientation_degrees".to_owned(),
                serde_json::json!(observation.orientation_degrees),
            );
            if let Some(script) = observation.script {
                block
                    .metadata
                    .insert("ocr_script".to_owned(), serde_json::json!(script));
            }
            if observation.confidence_bps < 5_000 {
                artifact.diagnostics.push(DocumentDiagnostic {
                    code: "ocr_low_confidence".to_owned(),
                    severity: DiagnosticSeverity::Warning,
                    locator: Some(locator),
                    message: format!(
                        "OCR confidence is {} basis points",
                        observation.confidence_bps
                    ),
                });
            }
        }
        if reading_order_approximate {
            artifact.diagnostics.push(DocumentDiagnostic {
                code: "ocr_reading_order_approximate".to_owned(),
                severity: DiagnosticSeverity::Info,
                locator: Some(owner.clone()),
                message: "OCR regions use deterministic geometric order because the engine did not provide a fully supported writing direction".to_owned(),
            });
        }
        if let DocumentLocator::Pdf { page, .. } = owner
            && native_page_text
                .get(&page)
                .is_some_and(|text| !text.trim().is_empty())
        {
            artifact.diagnostics.push(DocumentDiagnostic {
                code: "ocr_native_text_preferred".to_owned(),
                severity: DiagnosticSeverity::Info,
                locator: Some(DocumentLocator::Pdf { page, item: 1 }),
                message: "Native PDF text remains authoritative; OCR is separate derived evidence"
                    .to_owned(),
            });
        }
    }
    artifact.ocr_profile = profile;
    artifact.visual_coverage = if failed && succeeded == 0 {
        VisualCoverage::Failed
    } else if failed {
        VisualCoverage::Partial
    } else {
        VisualCoverage::Complete
    };
    artifact
        .diagnostics
        .retain(|diagnostic| diagnostic.code != "embedded_images_available_for_ocr");
    if !failed
        && !artifact
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    {
        artifact.complete = true;
    }
    artifact.validate()?;
    check_cancelled(cancellation)?;
    Ok(artifact)
}

fn check_cancelled(cancellation: &AtomicBool) -> Result<(), DocumentError> {
    if cancellation.load(Ordering::Acquire) {
        Err(DocumentError::Ocr(compass_ocr::OcrError::Cancelled))
    } else {
        Ok(())
    }
}

fn check_deadline(started: Instant) -> Result<(), DocumentError> {
    check_ocr_deadline(started).map_err(DocumentError::Ocr)
}

fn check_ocr_deadline(started: Instant) -> Result<(), compass_ocr::OcrError> {
    if started.elapsed() > Duration::from_secs(compass_ocr::OCR_MAX_DOCUMENT_WALL_TIME_SECS) {
        Err(compass_ocr::OcrError::Timeout)
    } else {
        Ok(())
    }
}

fn reserve_aggregate_pixels(total: &mut u64, raster: &PreparedRaster) -> Result<(), DocumentError> {
    let pixels = u64::from(raster.width)
        .checked_mul(u64::from(raster.height))
        .ok_or_else(|| DocumentError::Rejected("OCR raster pixel count overflow".to_owned()))?;
    *total = total
        .checked_add(pixels)
        .ok_or_else(|| DocumentError::Rejected("aggregate OCR pixels overflow".to_owned()))?;
    if *total > OCR_MAX_AGGREGATE_PIXELS {
        return Err(DocumentError::Rejected(
            "aggregate OCR pixels exceed document limit".to_owned(),
        ));
    }
    Ok(())
}

fn selected_pdf_pages(
    artifact: &DocumentArtifact,
    mode: OcrMode,
) -> Result<Vec<u32>, DocumentError> {
    let mut page_text = native_pdf_page_text(artifact);
    let mut pages = artifact
        .blocks
        .iter()
        .filter_map(|block| match block.locator {
            DocumentLocator::Pdf { page, .. } if matches!(block.kind, DocumentBlockKind::Page) => {
                Some(page)
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|page| {
            mode == OcrMode::Always
                || page_text
                    .remove(page)
                    .unwrap_or_default()
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .count()
                    < 24
        })
        .collect::<Vec<_>>();
    pages.sort_unstable();
    if pages.len() > OCR_MAX_PDF_PAGES {
        return Err(DocumentError::Rejected(
            "eligible PDF pages exceed OCR limit".to_owned(),
        ));
    }
    Ok(pages)
}

fn native_pdf_page_text(artifact: &DocumentArtifact) -> BTreeMap<u32, String> {
    let mut text = BTreeMap::<u32, String>::new();
    for block in &artifact.blocks {
        if !matches!(block.origin, DocumentOrigin::Native) {
            continue;
        }
        let page = match block.locator {
            DocumentLocator::Pdf { page, .. } => Some(page),
            _ => block
                .parent
                .and_then(|parent| artifact.blocks.get(parent as usize))
                .and_then(|parent| match parent.locator {
                    DocumentLocator::Pdf { page, .. } => Some(page),
                    _ => None,
                }),
        };
        if let Some(page) = page {
            text.entry(page).or_default().push_str(&block.text);
        }
    }
    text
}

fn parent_for_locator(artifact: &DocumentArtifact, owner: &DocumentLocator) -> Option<u32> {
    artifact
        .blocks
        .iter()
        .find(|block| &block.locator == owner)
        .map(|block| block.ordinal)
}

fn bounded_message(message: &str) -> String {
    message.chars().take(1_024).collect()
}

fn recognize_tiled(
    engine: &dyn OcrEngine,
    request: &OcrRequest,
    raster: &PreparedRaster,
    cancellation: &AtomicBool,
    started: Instant,
) -> Result<OcrResponse, compass_ocr::OcrError> {
    let tiles = tile_raster(raster)?;
    let mut observations = Vec::new();
    for tile in tiles {
        check_ocr_deadline(started)?;
        check_cancelled(cancellation).map_err(|error| match error {
            DocumentError::Ocr(error) => error,
            other => compass_ocr::OcrError::Inference(other.to_string()),
        })?;
        let tile_digest = prepared_raster_digest(&tile.raster);
        let tile_request = OcrRequest {
            schema: OCR_SCHEMA.to_owned(),
            request_id: format!("tile-{}-{}", &tile_digest[..16], tile.ordinal),
            source_kind: request.source_kind,
            width: tile.raster.width,
            height: tile.raster.height,
            language_hints: request.language_hints.clone(),
            image_digest: tile_digest,
        };
        let response = engine.recognize_cancellable(&tile_request, &tile.raster, cancellation)?;
        check_ocr_deadline(started)?;
        response.validate_for(&tile_request)?;
        if response.profile != *engine.identity() {
            return Err(compass_ocr::OcrError::InvalidOutput(
                "OCR engine response profile does not match its loaded identity".to_owned(),
            ));
        }
        for mut observation in response.observations {
            for point in &mut observation.polygon {
                point.x = point.x.checked_add(tile.x).ok_or_else(|| {
                    compass_ocr::OcrError::InvalidOutput("tile x overflow".to_owned())
                })?;
                point.y = point.y.checked_add(tile.y).ok_or_else(|| {
                    compass_ocr::OcrError::InvalidOutput("tile y overflow".to_owned())
                })?;
                if point.x >= raster.width || point.y >= raster.height {
                    return Err(compass_ocr::OcrError::InvalidOutput(
                        "mapped tile geometry lies outside the source raster".to_owned(),
                    ));
                }
            }
            observations.push(observation);
        }
    }
    observations.sort_by(|left, right| {
        observation_geometry_key(left)
            .cmp(&observation_geometry_key(right))
            .then_with(|| left.text.cmp(&right.text))
            .then_with(|| right.confidence_bps.cmp(&left.confidence_bps))
    });
    let mut deduplicated = Vec::<OcrObservation>::new();
    for observation in observations {
        let normalized = comparison_text(&observation.text);
        let duplicate = deduplicated.iter().position(|existing| {
            comparison_text(&existing.text) == normalized
                && polygon_box_iou(existing, &observation) >= 0.5
        });
        if let Some(index) = duplicate {
            if observation.confidence_bps > deduplicated[index].confidence_bps {
                deduplicated[index] = observation;
            }
        } else {
            deduplicated.push(observation);
        }
    }
    if deduplicated.len() > compass_ocr::OCR_MAX_OBSERVATIONS_PER_RASTER {
        return Err(compass_ocr::OcrError::InvalidOutput(
            "merged OCR observation count exceeds raster limit".to_owned(),
        ));
    }
    deduplicated.sort_by(|left, right| {
        observation_geometry_key(left)
            .cmp(&observation_geometry_key(right))
            .then_with(|| left.text.cmp(&right.text))
    });
    for (ordinal, observation) in deduplicated.iter_mut().enumerate() {
        observation.ordinal = u32::try_from(ordinal).map_err(|_| {
            compass_ocr::OcrError::InvalidOutput("merged OCR ordinal overflow".to_owned())
        })?;
    }
    let response = OcrResponse {
        schema: OCR_SCHEMA.to_owned(),
        request_id: request.request_id.clone(),
        profile: engine.identity().clone(),
        observations: deduplicated,
    };
    response.validate_for(request)?;
    Ok(response)
}

fn observation_geometry_key(observation: &OcrObservation) -> (u32, u32, u32, u32) {
    let min_x = observation
        .polygon
        .iter()
        .map(|point| point.x)
        .min()
        .unwrap_or(0);
    let min_y = observation
        .polygon
        .iter()
        .map(|point| point.y)
        .min()
        .unwrap_or(0);
    let max_x = observation
        .polygon
        .iter()
        .map(|point| point.x)
        .max()
        .unwrap_or(0);
    let max_y = observation
        .polygon
        .iter()
        .map(|point| point.y)
        .max()
        .unwrap_or(0);
    (min_y, min_x, max_y, max_x)
}

fn comparison_text(text: &str) -> String {
    let normalized = text.nfkc().collect::<String>();
    normalized
        .split_whitespace()
        .flat_map(|word| {
            word.chars()
                .flat_map(char::to_lowercase)
                .chain(std::iter::once(' '))
        })
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn polygon_box_iou(left: &OcrObservation, right: &OcrObservation) -> f64 {
    let (left_y1, left_x1, left_y2, left_x2) = observation_geometry_key(left);
    let (right_y1, right_x1, right_y2, right_x2) = observation_geometry_key(right);
    let overlap_width = left_x2.min(right_x2).saturating_sub(left_x1.max(right_x1));
    let overlap_height = left_y2.min(right_y2).saturating_sub(left_y1.max(right_y1));
    let intersection = u64::from(overlap_width) * u64::from(overlap_height);
    let left_area =
        u64::from(left_x2.saturating_sub(left_x1)) * u64::from(left_y2.saturating_sub(left_y1));
    let right_area =
        u64::from(right_x2.saturating_sub(right_x1)) * u64::from(right_y2.saturating_sub(right_y1));
    let union = left_area
        .saturating_add(right_area)
        .saturating_sub(intersection);
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

fn record_ocr_failure(
    artifact: &mut DocumentArtifact,
    locator: DocumentLocator,
    error: &compass_ocr::OcrError,
) {
    let code = match error {
        compass_ocr::OcrError::Timeout => "ocr_engine_timeout",
        compass_ocr::OcrError::InvalidOutput(_) | compass_ocr::OcrError::InvalidRequest(_) => {
            "ocr_engine_output_rejected"
        }
        compass_ocr::OcrError::Cancelled => "ocr_engine_cancelled",
        compass_ocr::OcrError::EngineUnavailable(_)
        | compass_ocr::OcrError::ModelUnavailable(_)
        | compass_ocr::OcrError::ModelVerification(_)
        | compass_ocr::OcrError::Inference(_)
        | compass_ocr::OcrError::Io { .. } => "ocr_engine_unavailable",
    };
    artifact.diagnostics.push(DocumentDiagnostic {
        code: code.to_owned(),
        severity: DiagnosticSeverity::Warning,
        locator: Some(locator.clone()),
        message: bounded_message(&error.to_string()),
    });
    artifact.diagnostics.push(DocumentDiagnostic {
        code: "ocr_partial_visual_coverage".to_owned(),
        severity: DiagnosticSeverity::Warning,
        locator: Some(locator),
        message: "One selected OCR raster could not be processed".to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write as _};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use compass_ocr::{OcrObservation, OcrPoint, OcrProfileIdentity, OcrResponse};
    use image::{DynamicImage, ImageFormat, RgbImage};
    use zip::write::SimpleFileOptions;

    use super::*;

    struct FixtureEngine {
        calls: AtomicUsize,
        fail: bool,
        invalid: bool,
    }

    struct OverlapEngine;

    impl OcrEngine for FixtureEngine {
        fn identity(&self) -> &OcrProfileIdentity {
            static PROFILE: std::sync::OnceLock<OcrProfileIdentity> = std::sync::OnceLock::new();
            PROFILE.get_or_init(fixture_profile)
        }

        fn recognize(
            &self,
            request: &OcrRequest,
            _raster: &compass_ocr::PreparedRaster,
        ) -> Result<OcrResponse, compass_ocr::OcrError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                return Err(compass_ocr::OcrError::Inference(
                    "fixture failure".to_owned(),
                ));
            }
            Ok(OcrResponse {
                schema: OCR_SCHEMA.to_owned(),
                request_id: if self.invalid {
                    "wrong-request".to_owned()
                } else {
                    request.request_id.clone()
                },
                profile: fixture_profile(),
                observations: vec![OcrObservation {
                    ordinal: 0,
                    polygon: vec![
                        OcrPoint { x: 1, y: 1 },
                        OcrPoint { x: 80, y: 1 },
                        OcrPoint { x: 80, y: 30 },
                        OcrPoint { x: 1, y: 30 },
                    ],
                    text: "OCR sentinel".to_owned(),
                    confidence_bps: 9_500,
                    script: Some("Latn".to_owned()),
                    orientation_degrees: 0,
                }],
            })
        }
    }

    impl OcrEngine for OverlapEngine {
        fn identity(&self) -> &OcrProfileIdentity {
            static PROFILE: std::sync::OnceLock<OcrProfileIdentity> = std::sync::OnceLock::new();
            PROFILE.get_or_init(fixture_profile)
        }

        fn recognize(
            &self,
            request: &OcrRequest,
            _raster: &PreparedRaster,
        ) -> Result<OcrResponse, compass_ocr::OcrError> {
            let second = request.request_id.ends_with("-1");
            let (left, right, confidence) = if second {
                (0, 127, 9_500)
            } else {
                (1_920, 2_047, 8_000)
            };
            Ok(OcrResponse {
                schema: OCR_SCHEMA.to_owned(),
                request_id: request.request_id.clone(),
                profile: self.identity().clone(),
                observations: vec![OcrObservation {
                    ordinal: 0,
                    polygon: vec![
                        OcrPoint { x: left, y: 1 },
                        OcrPoint { x: right, y: 1 },
                        OcrPoint { x: right, y: 30 },
                        OcrPoint { x: left, y: 30 },
                    ],
                    text: "overlap sentinel".to_owned(),
                    confidence_bps: confidence,
                    script: Some("Latn".to_owned()),
                    orientation_degrees: 0,
                }],
            })
        }
    }

    fn fixture_profile() -> OcrProfileIdentity {
        OcrProfileIdentity {
            engine: "fixture".to_owned(),
            engine_version: "1".to_owned(),
            profile: "fixture".to_owned(),
            model_digests: BTreeMap::from([("model".to_owned(), "a".repeat(64))]),
            languages: vec!["en".to_owned()],
            preprocessing_version: compass_ocr::OCR_PREPROCESSING_VERSION,
        }
    }

    fn docx_with_image() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 100, image::Rgb([255, 255, 255])))
            .write_to(&mut png, ImageFormat::Png)?;
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut archive = zip::ZipWriter::new(&mut bytes);
            archive.start_file("word/document.xml", SimpleFileOptions::default())?;
            archive.write_all(br#"<w:document xmlns:w="urn:w" xmlns:a="urn:a" xmlns:r="urn:r"><w:body><w:p><w:r><w:t>Native sentinel</w:t></w:r><w:r><a:blip r:embed="rIdImage1"/></w:r></w:p><w:p><w:r><a:blip r:embed="rIdImage1"/></w:r></w:p></w:body></w:document>"#)?;
            archive.start_file("word/_rels/document.xml.rels", SimpleFileOptions::default())?;
            archive.write_all(br#"<Relationships xmlns="urn:relationships"><Relationship Id="rIdImage1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/></Relationships>"#)?;
            archive.start_file("word/media/image1.png", SimpleFileOptions::default())?;
            archive.write_all(png.get_ref())?;
            archive.finish()?;
        }
        Ok(bytes.into_inner())
    }

    #[test]
    fn office_ocr_preserves_native_text_and_exact_visual_provenance()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = docx_with_image()?;
        let engine = FixtureEngine {
            calls: AtomicUsize::new(0),
            fail: false,
            invalid: false,
        };
        let artifact = decode_document_with_ocr(
            Path::new("report.docx"),
            &bytes,
            &DocumentProcessingOptions {
                ocr_mode: OcrMode::Auto,
                language_hints: vec!["en".to_owned()],
                allow_partial: false,
            },
            Some(&engine),
        )?;
        assert_eq!(engine.calls.load(Ordering::Relaxed), 1);
        assert!(artifact.blocks.iter().any(|block| {
            block.text == "Native sentinel" && matches!(block.origin, DocumentOrigin::Native)
        }));
        let ocr = artifact
            .blocks
            .iter()
            .find(|block| block.text == "OCR sentinel")
            .ok_or("OCR block missing")?;
        assert!(matches!(
            ocr.origin,
            DocumentOrigin::Ocr {
                confidence_bps: 9_500,
                ..
            }
        ));
        assert!(matches!(
            ocr.locator,
            DocumentLocator::Ocr { occurrence: 0, .. }
        ));
        let repeated = artifact
            .blocks
            .iter()
            .filter(|block| block.text == "OCR sentinel")
            .collect::<Vec<_>>();
        assert_eq!(repeated.len(), 2);
        assert_ne!(repeated[0].locator, repeated[1].locator);
        assert_eq!(artifact.visual_coverage, VisualCoverage::Complete);
        assert!(artifact.complete);
        Ok(())
    }

    #[test]
    fn partial_ocr_is_never_reported_complete() -> Result<(), Box<dyn std::error::Error>> {
        let bytes = docx_with_image()?;
        let engine = FixtureEngine {
            calls: AtomicUsize::new(0),
            fail: true,
            invalid: false,
        };
        let artifact = decode_document_with_ocr(
            Path::new("report.docx"),
            &bytes,
            &DocumentProcessingOptions {
                ocr_mode: OcrMode::Always,
                language_hints: Vec::new(),
                allow_partial: true,
            },
            Some(&engine),
        )?;
        assert_eq!(artifact.visual_coverage, VisualCoverage::Failed);
        assert!(!artifact.complete);
        assert!(artifact.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "ocr_partial_visual_coverage"
                && diagnostic.severity == DiagnosticSeverity::Warning
        }));
        Ok(())
    }

    #[test]
    fn allowed_invalid_engine_output_is_rejected_and_reported_as_failed_coverage()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = docx_with_image()?;
        let engine = FixtureEngine {
            calls: AtomicUsize::new(0),
            fail: false,
            invalid: true,
        };
        let artifact = decode_document_with_ocr(
            Path::new("report.docx"),
            &bytes,
            &DocumentProcessingOptions {
                ocr_mode: OcrMode::Always,
                language_hints: Vec::new(),
                allow_partial: true,
            },
            Some(&engine),
        )?;
        assert_eq!(artifact.visual_coverage, VisualCoverage::Failed);
        assert!(!artifact.complete);
        assert!(
            artifact
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ocr_engine_output_rejected")
        );
        assert!(
            artifact
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ocr_partial_visual_coverage")
        );
        Ok(())
    }

    #[test]
    fn native_off_is_identical_and_invalid_engine_output_is_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = docx_with_image()?;
        let native = decode_document(Path::new("report.docx"), &bytes)?;
        let off = decode_document_with_ocr(
            Path::new("report.docx"),
            &bytes,
            &DocumentProcessingOptions::default(),
            None,
        )?;
        assert_eq!(native, off);
        assert!(off.complete);

        let engine = FixtureEngine {
            calls: AtomicUsize::new(0),
            fail: false,
            invalid: true,
        };
        let result = decode_document_with_ocr(
            Path::new("report.docx"),
            &bytes,
            &DocumentProcessingOptions {
                ocr_mode: OcrMode::Always,
                language_hints: vec!["en".to_owned()],
                allow_partial: false,
            },
            Some(&engine),
        );
        assert!(matches!(result, Err(DocumentError::Ocr(_))));
        Ok(())
    }

    #[test]
    fn automatic_pdf_selection_skips_pages_with_sufficient_native_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut artifact = DocumentArtifact::new(DocumentFormat::Pdf);
        let first = artifact.push_block(
            None,
            DocumentBlockKind::Page,
            String::new(),
            DocumentLocator::Pdf { page: 1, item: 1 },
        )?;
        artifact.push_block(
            Some(first),
            DocumentBlockKind::Paragraph,
            "This born-digital page contains enough native text.".to_owned(),
            DocumentLocator::Pdf { page: 1, item: 2 },
        )?;
        artifact.push_block(
            None,
            DocumentBlockKind::Page,
            String::new(),
            DocumentLocator::Pdf { page: 2, item: 1 },
        )?;
        assert_eq!(selected_pdf_pages(&artifact, OcrMode::Auto)?, vec![2]);
        assert_eq!(selected_pdf_pages(&artifact, OcrMode::Always)?, vec![1, 2]);
        Ok(())
    }

    #[test]
    fn always_pdf_selection_rejects_one_page_over_the_hard_cap()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut artifact = DocumentArtifact::new(DocumentFormat::Pdf);
        for page in 1..=u32::try_from(OCR_MAX_PDF_PAGES + 1)? {
            artifact.push_block(
                None,
                DocumentBlockKind::Page,
                String::new(),
                DocumentLocator::Pdf { page, item: 1 },
            )?;
        }
        assert!(selected_pdf_pages(&artifact, OcrMode::Always).is_err());
        Ok(())
    }

    #[test]
    fn document_ocr_honors_pre_cancel_without_decoding_or_inference()
    -> Result<(), Box<dyn std::error::Error>> {
        let bytes = docx_with_image()?;
        let engine = FixtureEngine {
            calls: AtomicUsize::new(0),
            fail: false,
            invalid: false,
        };
        let cancellation = AtomicBool::new(true);
        let result = decode_document_with_ocr_cancellable(
            Path::new("report.docx"),
            &bytes,
            &DocumentProcessingOptions {
                ocr_mode: OcrMode::Always,
                language_hints: Vec::new(),
                allow_partial: false,
            },
            Some(&engine),
            &cancellation,
        );
        assert!(matches!(
            result,
            Err(DocumentError::Ocr(compass_ocr::OcrError::Cancelled))
        ));
        assert_eq!(engine.calls.load(Ordering::Relaxed), 0);
        Ok(())
    }

    #[test]
    fn tiled_regions_map_to_source_geometry_and_overlap_is_deduplicated()
    -> Result<(), Box<dyn std::error::Error>> {
        let raster = PreparedRaster {
            image: RgbImage::new(3_000, 100),
            width: 3_000,
            height: 100,
        };
        let request = OcrRequest {
            schema: OCR_SCHEMA.to_owned(),
            request_id: "source-raster".to_owned(),
            source_kind: OcrSourceKind::EmbeddedImage,
            width: raster.width,
            height: raster.height,
            language_hints: Vec::new(),
            image_digest: prepared_raster_digest(&raster),
        };
        let response = recognize_tiled(
            &OverlapEngine,
            &request,
            &raster,
            &AtomicBool::new(false),
            Instant::now(),
        )?;
        assert_eq!(response.observations.len(), 1);
        assert_eq!(response.observations[0].confidence_bps, 9_500);
        assert_eq!(response.observations[0].polygon[0].x, 1_920);
        Ok(())
    }

    #[test]
    fn tiled_recognition_checks_the_document_deadline_before_inference()
    -> Result<(), Box<dyn std::error::Error>> {
        let raster = PreparedRaster {
            image: RgbImage::new(100, 100),
            width: 100,
            height: 100,
        };
        let request = OcrRequest {
            schema: OCR_SCHEMA.to_owned(),
            request_id: "expired-raster".to_owned(),
            source_kind: OcrSourceKind::EmbeddedImage,
            width: raster.width,
            height: raster.height,
            language_hints: Vec::new(),
            image_digest: prepared_raster_digest(&raster),
        };
        let engine = FixtureEngine {
            calls: AtomicUsize::new(0),
            fail: false,
            invalid: false,
        };
        let started = Instant::now()
            .checked_sub(Duration::from_secs(
                compass_ocr::OCR_MAX_DOCUMENT_WALL_TIME_SECS + 1,
            ))
            .ok_or("could not construct expired deadline")?;
        let result = recognize_tiled(&engine, &request, &raster, &AtomicBool::new(false), started);
        assert!(matches!(result, Err(compass_ocr::OcrError::Timeout)));
        assert_eq!(engine.calls.load(Ordering::Relaxed), 0);
        Ok(())
    }
}
