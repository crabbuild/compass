//! Versioned, provenance-preserving document artifact contract.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use compass_ocr::{OcrPoint, OcrProfileIdentity};
use serde::{Deserialize, Serialize};

use crate::limits::{
    DOCUMENT_MAX_BLOCKS, DOCUMENT_MAX_DEPTH, DOCUMENT_MAX_DIAGNOSTIC_MESSAGE_BYTES,
    DOCUMENT_MAX_DIAGNOSTICS, DOCUMENT_MAX_FIELD_BYTES, DOCUMENT_MAX_LINKS,
    DOCUMENT_MAX_METADATA_ENTRIES, DOCUMENT_MAX_TEXT_CHARS,
};

pub const DOCUMENT_SCHEMA: &str = "compass.document/1";
pub const DOCUMENT_INSPECT_SCHEMA: &str = "compass.document.inspect/1";
pub const DOCUMENT_NORMALIZER_VERSION: u32 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    Text,
    Markdown,
    Html,
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Rtf,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentBlockKind {
    DocumentTitle,
    Heading { level: u8 },
    Paragraph,
    List,
    ListItem,
    Code,
    Quote,
    Table,
    Row,
    Cell,
    Page,
    Sheet,
    Slide,
    Note,
    Other { role: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentLocator {
    TextRange {
        start_byte: u64,
        end_byte: u64,
        start_line: u32,
        end_line: u32,
    },
    Package {
        part: String,
        path: String,
    },
    Pdf {
        page: u32,
        item: u32,
    },
    Spreadsheet {
        sheet: String,
        row: u32,
        column: u16,
    },
    Slide {
        slide: u32,
        shape: u32,
    },
    Ocr {
        owner: Box<DocumentLocator>,
        candidate_id: String,
        width: u32,
        height: u32,
        polygon: Vec<OcrPoint>,
        occurrence: u32,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentOrigin {
    #[default]
    Native,
    Ocr {
        profile: OcrProfileIdentity,
        confidence_bps: u16,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentBlock {
    pub ordinal: u32,
    pub parent: Option<u32>,
    pub kind: DocumentBlockKind,
    pub text: String,
    pub locator: DocumentLocator,
    #[serde(default)]
    pub origin: DocumentOrigin,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentLinkKind {
    Hyperlink,
    Relationship,
    Image,
    Attachment,
    Other,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentLink {
    pub source_block: u32,
    pub destination: String,
    pub label: Option<String>,
    pub relationship: DocumentLinkKind,
    pub locator: DocumentLocator,
    pub external: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub locator: Option<DocumentLocator>,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualCoverage {
    #[default]
    NotRequested,
    Complete,
    Partial,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DocumentArtifact {
    pub schema: String,
    pub normalizer_version: u32,
    pub format: DocumentFormat,
    pub blocks: Vec<DocumentBlock>,
    pub links: Vec<DocumentLink>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub diagnostics: Vec<DocumentDiagnostic>,
    pub complete: bool,
    #[serde(default)]
    pub visual_coverage: VisualCoverage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_profile: Option<OcrProfileIdentity>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("unsupported document format {0:?}")]
    Unsupported(String),
    #[error("document rejected: {0}")]
    Rejected(String),
    #[error("document parse failed: {0}")]
    Parse(String),
    #[error("invalid document artifact: {0}")]
    InvalidArtifact(String),
    #[error("could not access {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Ocr(#[from] compass_ocr::OcrError),
}

impl DocumentArtifact {
    #[must_use]
    pub fn new(format: DocumentFormat) -> Self {
        Self {
            schema: DOCUMENT_SCHEMA.to_owned(),
            normalizer_version: DOCUMENT_NORMALIZER_VERSION,
            format,
            blocks: Vec::new(),
            links: Vec::new(),
            metadata: BTreeMap::new(),
            diagnostics: Vec::new(),
            complete: true,
            visual_coverage: VisualCoverage::NotRequested,
            ocr_profile: None,
        }
    }

    pub fn push_block(
        &mut self,
        parent: Option<u32>,
        kind: DocumentBlockKind,
        text: String,
        locator: DocumentLocator,
    ) -> Result<u32, DocumentError> {
        if self.blocks.len() >= DOCUMENT_MAX_BLOCKS {
            return Err(DocumentError::Rejected(
                "document block count exceeds limit".to_owned(),
            ));
        }
        let ordinal = u32::try_from(self.blocks.len())
            .map_err(|_| DocumentError::Rejected("document ordinal overflow".to_owned()))?;
        self.blocks.push(DocumentBlock {
            ordinal,
            parent,
            kind,
            text,
            locator,
            origin: DocumentOrigin::Native,
            metadata: BTreeMap::new(),
        });
        Ok(ordinal)
    }

    pub fn validate(&self) -> Result<(), DocumentError> {
        if self.schema != DOCUMENT_SCHEMA {
            return Err(DocumentError::InvalidArtifact(format!(
                "unsupported schema {:?}",
                self.schema
            )));
        }
        if self.normalizer_version != DOCUMENT_NORMALIZER_VERSION {
            return Err(DocumentError::InvalidArtifact(format!(
                "unsupported normalizer version {}",
                self.normalizer_version
            )));
        }
        if self.blocks.len() > DOCUMENT_MAX_BLOCKS
            || self.links.len() > DOCUMENT_MAX_LINKS
            || self.diagnostics.len() > DOCUMENT_MAX_DIAGNOSTICS
            || self.metadata.len() > DOCUMENT_MAX_METADATA_ENTRIES
        {
            return Err(DocumentError::InvalidArtifact(
                "artifact collection exceeds limit".to_owned(),
            ));
        }
        let mut text_chars = 0_usize;
        for (index, block) in self.blocks.iter().enumerate() {
            let expected = u32::try_from(index)
                .map_err(|_| DocumentError::InvalidArtifact("ordinal overflow".to_owned()))?;
            if block.ordinal != expected {
                return Err(DocumentError::InvalidArtifact(
                    "block ordinals must be contiguous".to_owned(),
                ));
            }
            if block.parent.is_some_and(|parent| parent >= block.ordinal) {
                return Err(DocumentError::InvalidArtifact(
                    "block parent must precede its child".to_owned(),
                ));
            }
            if depth_for(&self.blocks, block.ordinal)? > DOCUMENT_MAX_DEPTH {
                return Err(DocumentError::InvalidArtifact(
                    "block nesting exceeds limit".to_owned(),
                ));
            }
            text_chars = text_chars
                .checked_add(block.text.chars().count())
                .ok_or_else(|| DocumentError::InvalidArtifact("text size overflow".to_owned()))?;
            if block.metadata.len() > DOCUMENT_MAX_METADATA_ENTRIES {
                return Err(DocumentError::InvalidArtifact(
                    "block metadata exceeds limit".to_owned(),
                ));
            }
            validate_block_kind(&block.kind)?;
            validate_locator(&block.locator, 0)?;
            validate_origin(&block.origin)?;
            validate_metadata(&block.metadata)?;
        }
        if text_chars > DOCUMENT_MAX_TEXT_CHARS {
            return Err(DocumentError::InvalidArtifact(
                "document text exceeds limit".to_owned(),
            ));
        }
        for link in &self.links {
            if usize::try_from(link.source_block)
                .ok()
                .is_none_or(|source| source >= self.blocks.len())
            {
                return Err(DocumentError::InvalidArtifact(
                    "link references a missing block".to_owned(),
                ));
            }
            validate_field("link destination", &link.destination)?;
            if let Some(label) = &link.label {
                validate_field("link label", label)?;
            }
            validate_locator(&link.locator, 0)?;
        }
        validate_metadata(&self.metadata)?;
        for diagnostic in &self.diagnostics {
            validate_field("diagnostic code", &diagnostic.code)?;
            if diagnostic.message.len() > DOCUMENT_MAX_DIAGNOSTIC_MESSAGE_BYTES {
                return Err(DocumentError::InvalidArtifact(
                    "diagnostic message exceeds limit".to_owned(),
                ));
            }
            if let Some(locator) = &diagnostic.locator {
                validate_locator(locator, 0)?;
            }
        }
        if let Some(profile) = &self.ocr_profile {
            profile.validate()?;
        }
        if self.visual_coverage == VisualCoverage::NotRequested && self.ocr_profile.is_some() {
            return Err(DocumentError::InvalidArtifact(
                "OCR profile present when visual coverage was not requested".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn to_canonical_json(&self) -> Result<Vec<u8>, DocumentError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| DocumentError::InvalidArtifact(error.to_string()))
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, DocumentError> {
        let artifact: Self = serde_json::from_slice(bytes)
            .map_err(|error| DocumentError::InvalidArtifact(error.to_string()))?;
        artifact.validate()?;
        Ok(artifact)
    }
}

fn depth_for(blocks: &[DocumentBlock], ordinal: u32) -> Result<usize, DocumentError> {
    let mut depth = 0_usize;
    let mut current = Some(ordinal);
    while let Some(value) = current {
        let index = usize::try_from(value)
            .map_err(|_| DocumentError::InvalidArtifact("invalid block ordinal".to_owned()))?;
        let block = blocks.get(index).ok_or_else(|| {
            DocumentError::InvalidArtifact("parent references a missing block".to_owned())
        })?;
        current = block.parent;
        depth = depth
            .checked_add(1)
            .ok_or_else(|| DocumentError::InvalidArtifact("block depth overflow".to_owned()))?;
        if depth > DOCUMENT_MAX_DEPTH + 1 {
            return Ok(depth);
        }
    }
    Ok(depth)
}

fn validate_block_kind(kind: &DocumentBlockKind) -> Result<(), DocumentError> {
    match kind {
        DocumentBlockKind::Heading { level } if !(1..=6).contains(level) => Err(
            DocumentError::InvalidArtifact("heading level must be in 1..=6".to_owned()),
        ),
        DocumentBlockKind::Other { role } => validate_field("block role", role),
        _ => Ok(()),
    }
}

fn validate_origin(origin: &DocumentOrigin) -> Result<(), DocumentError> {
    if let DocumentOrigin::Ocr {
        profile,
        confidence_bps,
    } = origin
    {
        profile.validate()?;
        if *confidence_bps > 10_000 {
            return Err(DocumentError::InvalidArtifact(
                "OCR confidence exceeds 10000 basis points".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_locator(locator: &DocumentLocator, depth: usize) -> Result<(), DocumentError> {
    if depth > DOCUMENT_MAX_DEPTH {
        return Err(DocumentError::InvalidArtifact(
            "locator nesting exceeds limit".to_owned(),
        ));
    }
    match locator {
        DocumentLocator::TextRange {
            start_byte,
            end_byte,
            start_line,
            end_line,
        } if start_byte > end_byte || start_line > end_line => Err(DocumentError::InvalidArtifact(
            "invalid text range".to_owned(),
        )),
        DocumentLocator::Package { part, path } => {
            validate_package_part(part)?;
            validate_field("package block path", path)
        }
        DocumentLocator::Spreadsheet { sheet, row, column } => {
            validate_field("sheet name", sheet)?;
            if *row == 0 || *column == 0 {
                return Err(DocumentError::InvalidArtifact(
                    "spreadsheet coordinates are one-based".to_owned(),
                ));
            }
            Ok(())
        }
        DocumentLocator::Slide { slide, shape } if *slide == 0 || *shape == 0 => Err(
            DocumentError::InvalidArtifact("slide coordinates are one-based".to_owned()),
        ),
        DocumentLocator::Pdf { page, item } if *page == 0 || *item == 0 => Err(
            DocumentError::InvalidArtifact("PDF coordinates are one-based".to_owned()),
        ),
        DocumentLocator::Ocr {
            owner,
            candidate_id,
            width,
            height,
            polygon,
            occurrence: _,
        } => {
            validate_locator(owner, depth + 1)?;
            validate_field("OCR candidate ID", candidate_id)?;
            compass_ocr::validate_dimensions(*width, *height)?;
            if !(4..=16).contains(&polygon.len())
                || polygon
                    .iter()
                    .any(|point| point.x >= *width || point.y >= *height)
            {
                return Err(DocumentError::InvalidArtifact(
                    "invalid OCR locator polygon".to_owned(),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_package_part(value: &str) -> Result<(), DocumentError> {
    validate_field("package part", value)?;
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || value.contains('\\')
    {
        return Err(DocumentError::InvalidArtifact(
            "package part is absolute or escapes its package".to_owned(),
        ));
    }
    Ok(())
}

fn validate_field(name: &str, value: &str) -> Result<(), DocumentError> {
    if value.is_empty() || value.len() > DOCUMENT_MAX_FIELD_BYTES || value.contains('\0') {
        return Err(DocumentError::InvalidArtifact(format!(
            "{name} is empty or exceeds its bound"
        )));
    }
    Ok(())
}

fn validate_metadata(metadata: &BTreeMap<String, serde_json::Value>) -> Result<(), DocumentError> {
    if metadata.len() > DOCUMENT_MAX_METADATA_ENTRIES {
        return Err(DocumentError::InvalidArtifact(
            "metadata count exceeds limit".to_owned(),
        ));
    }
    let mut keys = BTreeSet::new();
    for (key, value) in metadata {
        validate_field("metadata key", key)?;
        if !keys.insert(key) {
            return Err(DocumentError::InvalidArtifact(
                "duplicate metadata key".to_owned(),
            ));
        }
        let encoded = serde_json::to_vec(value)
            .map_err(|error| DocumentError::InvalidArtifact(error.to_string()))?;
        if encoded.len() > DOCUMENT_MAX_FIELD_BYTES {
            return Err(DocumentError::InvalidArtifact(
                "metadata value exceeds limit".to_owned(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_round_trip_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
        let mut artifact = DocumentArtifact::new(DocumentFormat::Docx);
        artifact
            .metadata
            .insert("title".to_owned(), serde_json::json!("Guide"));
        artifact.push_block(
            None,
            DocumentBlockKind::Heading { level: 1 },
            "Guide".to_owned(),
            DocumentLocator::Package {
                part: "word/document.xml".to_owned(),
                path: "body/p[1]".to_owned(),
            },
        )?;
        let first = artifact.to_canonical_json()?;
        let decoded = DocumentArtifact::from_json(&first)?;
        assert_eq!(decoded, artifact);
        assert_eq!(decoded.to_canonical_json()?, first);
        Ok(())
    }

    #[test]
    fn rejects_unknown_schema_parent_and_package_escape() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut artifact = DocumentArtifact::new(DocumentFormat::Docx);
        artifact.schema = "compass.document/2".to_owned();
        assert!(artifact.validate().is_err());

        let mut artifact = DocumentArtifact::new(DocumentFormat::Docx);
        artifact.push_block(
            Some(0),
            DocumentBlockKind::Paragraph,
            "bad".to_owned(),
            DocumentLocator::Package {
                part: "../word/document.xml".to_owned(),
                path: "body/p[1]".to_owned(),
            },
        )?;
        assert!(artifact.validate().is_err());
        Ok(())
    }
}
