//! Bounded, engine-neutral OCR contracts and a managed local PP-OCR runtime.

mod engine;
mod models;

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use serde::{Deserialize, Serialize};

pub use engine::{
    ManagedOarEngine, OcrEngine, PreparedRaster, PreparedRasterTile, prepare_raster,
    prepare_raster_cancellable, prepared_raster_digest, tile_raster,
};
pub use models::{
    ArtifactFetcher, HttpsArtifactFetcher, ModelCache, ModelFiles, ModelProfile, ModelStatus,
    install_profile, list_profiles, profile_manifest_digest, verify_profile,
};

pub const OCR_SCHEMA: &str = "compass.ocr/1";
pub const OCR_PROTOCOL_SCHEMA: &str = "compass.ocr.protocol/1";
pub const OCR_POLICY_VERSION: u32 = 1;
pub const OCR_PREPROCESSING_VERSION: u32 = 2;

pub const OCR_MAX_RASTER_PIXELS: u64 = 24_000_000;
pub const OCR_MAX_RASTER_LONG_EDGE: u32 = 6_000;
pub const OCR_ENGINE_MAX_SIDE: u32 = 2_048;
pub const OCR_TILE_OVERLAP: u32 = 128;
pub const OCR_ENGINE_THREADS: usize = 1;
pub const OCR_MAX_DOCUMENT_WALL_TIME_SECS: u64 = 10 * 60;
pub const OCR_MAX_OBSERVATIONS_PER_RASTER: usize = 10_000;
pub const OCR_MAX_OBSERVATIONS_PER_DOCUMENT: usize = 100_000;
pub const OCR_MAX_TEXT_BYTES_PER_OBSERVATION: usize = 16 * 1024;
pub const OCR_MAX_TEXT_CHARS_PER_DOCUMENT: usize = 5_000_000;
pub const OCR_MAX_LANGUAGE_HINTS: usize = 32;
pub const OCR_MAX_PROFILE_FIELD_BYTES: usize = 256;

fn managed_runtime_supported_for(target_os: &str, target_arch: &str) -> bool {
    target_os != "macos" || target_arch != "x86_64"
}

#[must_use]
pub fn managed_runtime_available() -> bool {
    managed_runtime_supported_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub(crate) fn ensure_managed_runtime_available() -> Result<(), OcrError> {
    if managed_runtime_available() {
        Ok(())
    } else {
        Err(managed_runtime_unavailable_error())
    }
}

pub(crate) fn managed_runtime_unavailable_error() -> OcrError {
    OcrError::EngineUnavailable(
        "managed local OCR is unavailable on Intel macOS because the pinned ONNX Runtime does not provide a self-contained x86_64 macOS build; native PDF, DOCX, PPTX, and XLSX processing remains available with OCR off"
            .to_owned(),
    )
}

pub fn normalize_language_hints(hints: &[String]) -> Result<Vec<String>, OcrError> {
    if hints.len() > OCR_MAX_LANGUAGE_HINTS {
        return Err(OcrError::InvalidRequest(
            "too many language hints".to_owned(),
        ));
    }
    let mut normalized = Vec::with_capacity(hints.len());
    for hint in hints {
        if hint.is_empty()
            || hint.len() > 64
            || hint.split('-').any(|segment| {
                segment.is_empty()
                    || segment.len() > 8
                    || !segment.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            || hint.split('-').next().is_none_or(|first| {
                first.len() > 8 || !first.bytes().all(|byte| byte.is_ascii_alphabetic())
            })
        {
            return Err(OcrError::InvalidRequest(format!(
                "invalid BCP-47 language hint {hint:?}"
            )));
        }
        normalized.push(hint.to_ascii_lowercase());
    }
    normalized.sort();
    normalized.dedup();
    Ok(normalized)
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrMode {
    #[default]
    Off,
    Auto,
    Always,
}

impl FromStr for OcrMode {
    type Err = OcrError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "off" => Ok(Self::Off),
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            _ => Err(OcrError::InvalidRequest(format!(
                "unknown OCR mode {value:?}; expected off, auto, or always"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OcrProfileIdentity {
    pub engine: String,
    pub engine_version: String,
    pub profile: String,
    pub model_digests: BTreeMap<String, String>,
    pub languages: Vec<String>,
    pub preprocessing_version: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrSourceKind {
    PdfPage,
    EmbeddedImage,
    RasterImage,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OcrRequest {
    pub schema: String,
    pub request_id: String,
    pub source_kind: OcrSourceKind,
    pub width: u32,
    pub height: u32,
    pub language_hints: Vec<String>,
    pub image_digest: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct OcrPoint {
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OcrObservation {
    pub ordinal: u32,
    pub polygon: Vec<OcrPoint>,
    pub text: String,
    pub confidence_bps: u16,
    pub script: Option<String>,
    pub orientation_degrees: i16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OcrResponse {
    pub schema: String,
    pub request_id: String,
    pub profile: OcrProfileIdentity,
    pub observations: Vec<OcrObservation>,
}

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("invalid OCR request: {0}")]
    InvalidRequest(String),
    #[error("OCR output rejected: {0}")]
    InvalidOutput(String),
    #[error("OCR engine unavailable: {0}")]
    EngineUnavailable(String),
    #[error("OCR model unavailable: {0}")]
    ModelUnavailable(String),
    #[error("OCR model verification failed: {0}")]
    ModelVerification(String),
    #[error("OCR inference failed: {0}")]
    Inference(String),
    #[error("OCR processing was cancelled")]
    Cancelled,
    #[error("OCR processing timed out")]
    Timeout,
    #[error("OCR I/O failed for {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl OcrRequest {
    pub fn validate(&self) -> Result<(), OcrError> {
        if self.schema != OCR_SCHEMA {
            return Err(OcrError::InvalidRequest(format!(
                "unsupported schema {:?}",
                self.schema
            )));
        }
        validate_bounded_field("request ID", &self.request_id)?;
        validate_digest(&self.image_digest)?;
        validate_dimensions(self.width, self.height)?;
        if normalize_language_hints(&self.language_hints)? != self.language_hints {
            return Err(OcrError::InvalidRequest(
                "language hints must use canonical lowercase sorted unique BCP-47 tags".to_owned(),
            ));
        }
        Ok(())
    }
}

impl OcrProfileIdentity {
    pub fn validate(&self) -> Result<(), OcrError> {
        validate_bounded_field("engine", &self.engine)?;
        validate_bounded_field("engine version", &self.engine_version)?;
        validate_bounded_field("profile", &self.profile)?;
        if self.preprocessing_version != OCR_PREPROCESSING_VERSION {
            return Err(OcrError::InvalidOutput(format!(
                "unsupported preprocessing version {}",
                self.preprocessing_version
            )));
        }
        if self.model_digests.is_empty() || self.model_digests.len() > 16 {
            return Err(OcrError::InvalidOutput(
                "profile must identify 1 to 16 model artifacts".to_owned(),
            ));
        }
        for (name, digest) in &self.model_digests {
            validate_bounded_field("model artifact name", name)?;
            validate_digest(digest)?;
        }
        if self.languages.is_empty() || self.languages.len() > OCR_MAX_LANGUAGE_HINTS {
            return Err(OcrError::InvalidOutput(
                "profile language set is empty or excessive".to_owned(),
            ));
        }
        let unique = self.languages.iter().collect::<BTreeSet<_>>();
        if unique.len() != self.languages.len()
            || normalize_language_hints(&self.languages)? != self.languages
        {
            return Err(OcrError::InvalidOutput(
                "profile languages must be canonical BCP-47 tags".to_owned(),
            ));
        }
        Ok(())
    }
}

impl OcrResponse {
    pub fn validate_for(&self, request: &OcrRequest) -> Result<(), OcrError> {
        request.validate()?;
        if self.schema != OCR_SCHEMA {
            return Err(OcrError::InvalidOutput(format!(
                "unsupported schema {:?}",
                self.schema
            )));
        }
        if self.request_id != request.request_id {
            return Err(OcrError::InvalidOutput(
                "response request ID does not match".to_owned(),
            ));
        }
        self.profile.validate()?;
        if self.observations.len() > OCR_MAX_OBSERVATIONS_PER_RASTER {
            return Err(OcrError::InvalidOutput(
                "observation count exceeds raster limit".to_owned(),
            ));
        }
        let mut expected = 0_u32;
        for observation in &self.observations {
            if observation.ordinal != expected {
                return Err(OcrError::InvalidOutput(
                    "observation ordinals must be contiguous".to_owned(),
                ));
            }
            expected = expected
                .checked_add(1)
                .ok_or_else(|| OcrError::InvalidOutput("ordinal overflow".to_owned()))?;
            validate_observation(observation, request.width, request.height)?;
        }
        Ok(())
    }
}

pub fn validate_dimensions(width: u32, height: u32) -> Result<(), OcrError> {
    if width == 0 || height == 0 {
        return Err(OcrError::InvalidRequest(
            "raster dimensions must be nonzero".to_owned(),
        ));
    }
    if width > OCR_MAX_RASTER_LONG_EDGE || height > OCR_MAX_RASTER_LONG_EDGE {
        return Err(OcrError::InvalidRequest(
            "raster long edge exceeds limit".to_owned(),
        ));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| OcrError::InvalidRequest("raster pixel count overflow".to_owned()))?;
    if pixels > OCR_MAX_RASTER_PIXELS {
        return Err(OcrError::InvalidRequest(
            "raster pixel count exceeds limit".to_owned(),
        ));
    }
    Ok(())
}

fn validate_observation(
    observation: &OcrObservation,
    width: u32,
    height: u32,
) -> Result<(), OcrError> {
    if !(4..=16).contains(&observation.polygon.len()) {
        return Err(OcrError::InvalidOutput(
            "OCR polygons must contain 4 to 16 points".to_owned(),
        ));
    }
    if observation.text.len() > OCR_MAX_TEXT_BYTES_PER_OBSERVATION {
        return Err(OcrError::InvalidOutput(
            "OCR observation text exceeds limit".to_owned(),
        ));
    }
    if observation.text.trim().is_empty() {
        return Err(OcrError::InvalidOutput(
            "OCR observation text is empty".to_owned(),
        ));
    }
    if observation.text.chars().any(char::is_control) {
        return Err(OcrError::InvalidOutput(
            "OCR observation text contains control characters".to_owned(),
        ));
    }
    if observation.confidence_bps > 10_000 {
        return Err(OcrError::InvalidOutput(
            "OCR confidence is outside 0..=10000".to_owned(),
        ));
    }
    if !matches!(
        observation.orientation_degrees,
        -270 | -180 | -90 | 0 | 90 | 180 | 270
    ) {
        return Err(OcrError::InvalidOutput(
            "OCR orientation is unsupported".to_owned(),
        ));
    }
    if observation
        .polygon
        .iter()
        .any(|point| point.x >= width || point.y >= height)
    {
        return Err(OcrError::InvalidOutput(
            "OCR polygon lies outside the raster".to_owned(),
        ));
    }
    let doubled_area = polygon_doubled_area(&observation.polygon);
    if doubled_area == 0 {
        return Err(OcrError::InvalidOutput(
            "OCR polygon has zero area".to_owned(),
        ));
    }
    if let Some(script) = &observation.script {
        validate_bounded_field("script", script)?;
    }
    Ok(())
}

fn polygon_doubled_area(points: &[OcrPoint]) -> i128 {
    let mut area = 0_i128;
    for index in 0..points.len() {
        let current = points[index];
        let next = points[(index + 1) % points.len()];
        area +=
            i128::from(current.x) * i128::from(next.y) - i128::from(next.x) * i128::from(current.y);
    }
    area.abs()
}

fn validate_bounded_field(name: &str, value: &str) -> Result<(), OcrError> {
    if value.is_empty()
        || value.len() > OCR_MAX_PROFILE_FIELD_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(OcrError::InvalidRequest(format!(
            "{name} is empty or exceeds its bound"
        )));
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), OcrError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(OcrError::InvalidRequest(
            "digest must be 64 canonical lowercase hexadecimal characters".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> OcrRequest {
        OcrRequest {
            schema: OCR_SCHEMA.to_owned(),
            request_id: "raster-1".to_owned(),
            source_kind: OcrSourceKind::EmbeddedImage,
            width: 100,
            height: 50,
            language_hints: vec!["en".to_owned()],
            image_digest: "a".repeat(64),
        }
    }

    fn profile() -> OcrProfileIdentity {
        OcrProfileIdentity {
            engine: "oar-ocr".to_owned(),
            engine_version: "0.9.1".to_owned(),
            profile: "pp-ocrv6-small".to_owned(),
            model_digests: BTreeMap::from([("detector".to_owned(), "b".repeat(64))]),
            languages: vec!["mul".to_owned()],
            preprocessing_version: OCR_PREPROCESSING_VERSION,
        }
    }

    #[test]
    fn validates_bounded_geometry_and_identity() {
        let request = request();
        let response = OcrResponse {
            schema: OCR_SCHEMA.to_owned(),
            request_id: request.request_id.clone(),
            profile: profile(),
            observations: vec![OcrObservation {
                ordinal: 0,
                polygon: vec![
                    OcrPoint { x: 1, y: 1 },
                    OcrPoint { x: 10, y: 1 },
                    OcrPoint { x: 10, y: 10 },
                    OcrPoint { x: 1, y: 10 },
                ],
                text: "Compass".to_owned(),
                confidence_bps: 9_500,
                script: Some("Latn".to_owned()),
                orientation_degrees: 0,
            }],
        };
        assert!(response.validate_for(&request).is_ok());
    }

    #[test]
    fn rejects_out_of_bounds_and_zero_area_polygons() {
        let mut observation = OcrObservation {
            ordinal: 0,
            polygon: vec![
                OcrPoint { x: 1, y: 1 },
                OcrPoint { x: 2, y: 2 },
                OcrPoint { x: 3, y: 3 },
                OcrPoint { x: 4, y: 4 },
            ],
            text: "bad".to_owned(),
            confidence_bps: 1,
            script: None,
            orientation_degrees: 0,
        };
        assert!(validate_observation(&observation, 100, 50).is_err());
        observation.polygon[3] = OcrPoint { x: 100, y: 4 };
        assert!(validate_observation(&observation, 100, 50).is_err());
    }

    #[test]
    fn rejects_empty_control_text_and_noncanonical_digests() {
        let mut noncanonical_request = request();
        noncanonical_request.image_digest = "A".repeat(64);
        assert!(noncanonical_request.validate().is_err());

        let request = request();
        for text in ["   ", "unsafe\u{1b}[2J"] {
            let response = OcrResponse {
                schema: OCR_SCHEMA.to_owned(),
                request_id: request.request_id.clone(),
                profile: profile(),
                observations: vec![OcrObservation {
                    ordinal: 0,
                    polygon: vec![
                        OcrPoint { x: 1, y: 1 },
                        OcrPoint { x: 10, y: 1 },
                        OcrPoint { x: 10, y: 10 },
                        OcrPoint { x: 1, y: 10 },
                    ],
                    text: text.to_owned(),
                    confidence_bps: 9_000,
                    script: Some("Latn".to_owned()),
                    orientation_degrees: 0,
                }],
            };
            assert!(response.validate_for(&request).is_err());
        }
    }

    #[test]
    fn mode_parser_is_explicit() {
        assert_eq!(OcrMode::from_str("auto").ok(), Some(OcrMode::Auto));
        assert!(OcrMode::from_str("maybe").is_err());
    }

    #[test]
    fn language_hints_are_validated_and_canonicalized() {
        assert_eq!(
            normalize_language_hints(&["EN-us".to_owned(), "en-US".to_owned()]).ok(),
            Some(vec!["en-us".to_owned()])
        );
        assert_eq!(
            normalize_language_hints(&[
                "ZH-Hant-TW".to_owned(),
                "ar".to_owned(),
                "ja-JP".to_owned(),
            ])
            .ok(),
            Some(vec![
                "ar".to_owned(),
                "ja-jp".to_owned(),
                "zh-hant-tw".to_owned(),
            ])
        );
        assert!(normalize_language_hints(&["en--US".to_owned()]).is_err());
        assert!(normalize_language_hints(&["not_a_tag".to_owned()]).is_err());
    }

    #[test]
    fn accepts_bounded_multilingual_and_emoji_text() {
        let request = request();
        let response = OcrResponse {
            schema: OCR_SCHEMA.to_owned(),
            request_id: request.request_id.clone(),
            profile: profile(),
            observations: vec![OcrObservation {
                ordinal: 0,
                polygon: vec![
                    OcrPoint { x: 1, y: 1 },
                    OcrPoint { x: 90, y: 1 },
                    OcrPoint { x: 90, y: 20 },
                    OcrPoint { x: 1, y: 20 },
                ],
                text: "指南 مرحبا 🧭".to_owned(),
                confidence_bps: 8_500,
                script: None,
                orientation_degrees: 0,
            }],
        };
        assert!(response.validate_for(&request).is_ok());
    }

    #[test]
    fn raster_dimension_limits_accept_exact_and_reject_one_over() {
        assert!(validate_dimensions(6_000, 4_000).is_ok());
        assert!(validate_dimensions(6_000, 4_001).is_err());
        assert!(validate_dimensions(6_001, 1).is_err());
    }

    #[test]
    fn managed_runtime_support_matrix_excludes_intel_macos() {
        assert!(!managed_runtime_supported_for("macos", "x86_64"));
        assert!(managed_runtime_supported_for("macos", "aarch64"));
        assert!(managed_runtime_supported_for("linux", "x86_64"));
        assert!(managed_runtime_supported_for("windows", "x86_64"));
    }
}
