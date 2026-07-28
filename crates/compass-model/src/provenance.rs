use serde::{Deserialize, Serialize};

pub const MAX_RESOLUTION_CANDIDATES: usize = 20;

/// A repository-relative, half-open source range.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceAnchor {
    pub file: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

/// The evidence source used to establish a structural graph fact.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceOrigin {
    Ast,
    Config,
    Convention,
    Artifact,
    Heuristic,
}

/// The strongest claim Compass can make from a piece of evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceConfidence {
    Exact,
    Inferred,
    Ambiguous,
}

/// The outcome of resolving a symbolic or framework reference.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionState {
    Exact,
    Ambiguous,
    Unresolved,
}

/// One retained candidate when static evidence does not identify one target.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolutionCandidate {
    pub node_id: String,
    pub reason: String,
    pub confidence: EvidenceConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<SourceAnchor>,
}

/// Structured, auditable evidence for one node or relationship.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Provenance {
    pub origin: EvidenceOrigin,
    pub extractor: String,
    pub confidence: EvidenceConfidence,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<SourceAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiring_site: Option<SourceAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<ResolutionCandidate>,
}

impl SourceAnchor {
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.file.is_empty()
            && !std::path::Path::new(&self.file).is_absolute()
            && !self.file.contains('\\')
            && self.start_byte <= self.end_byte
            && self.start_line > 0
            && self.end_line > 0
            && (self.start_line < self.end_line
                || (self.start_line == self.end_line && self.start_column <= self.end_column))
    }
}

impl Provenance {
    pub fn direct(
        origin: EvidenceOrigin,
        extractor: impl Into<String>,
        confidence: EvidenceConfidence,
        anchor: SourceAnchor,
    ) -> Result<Self, ProvenanceError> {
        let evidence = Self {
            origin,
            extractor: extractor.into(),
            confidence,
            rule: None,
            anchors: vec![anchor],
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        };
        evidence.validate()?;
        Ok(evidence)
    }

    pub fn convention(
        extractor: impl Into<String>,
        rule: impl Into<String>,
        anchor: SourceAnchor,
    ) -> Result<Self, ProvenanceError> {
        let evidence = Self {
            origin: EvidenceOrigin::Convention,
            extractor: extractor.into(),
            confidence: EvidenceConfidence::Exact,
            rule: Some(rule.into()),
            anchors: vec![anchor],
            wiring_site: None,
            score: None,
            candidates: Vec::new(),
        };
        evidence.validate()?;
        Ok(evidence)
    }

    pub fn heuristic(
        extractor: impl Into<String>,
        rule: impl Into<String>,
        wiring_site: SourceAnchor,
        candidates: Vec<ResolutionCandidate>,
    ) -> Result<Self, ProvenanceError> {
        let evidence = Self {
            origin: EvidenceOrigin::Heuristic,
            extractor: extractor.into(),
            confidence: if candidates.len() > 1 {
                EvidenceConfidence::Ambiguous
            } else {
                EvidenceConfidence::Inferred
            },
            rule: Some(rule.into()),
            anchors: Vec::new(),
            wiring_site: Some(wiring_site),
            score: None,
            candidates,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    pub fn validate(&self) -> Result<(), ProvenanceError> {
        if self.extractor.trim().is_empty() {
            return Err(ProvenanceError::new("extractor must not be empty"));
        }
        if self.score.is_some_and(|score| !score.is_finite()) {
            return Err(ProvenanceError::new("score must be finite"));
        }
        for anchor in &self.anchors {
            if !anchor.is_valid() {
                return Err(ProvenanceError::new("evidence anchor is invalid"));
            }
        }
        if self
            .wiring_site
            .as_ref()
            .is_some_and(|site| !site.is_valid())
        {
            return Err(ProvenanceError::new("wiring site is invalid"));
        }

        match self.origin {
            EvidenceOrigin::Ast | EvidenceOrigin::Config | EvidenceOrigin::Artifact => {
                if self.anchors.is_empty() {
                    return Err(ProvenanceError::new(
                        "direct evidence requires at least one source anchor",
                    ));
                }
                if self
                    .anchors
                    .iter()
                    .any(|anchor| anchor.start_byte == anchor.end_byte)
                {
                    return Err(ProvenanceError::new(
                        "direct evidence requires a non-empty source range",
                    ));
                }
            }
            EvidenceOrigin::Convention => {
                if self.anchors.is_empty() || empty_optional(&self.rule) {
                    return Err(ProvenanceError::new(
                        "convention evidence requires a rule and input-file anchor",
                    ));
                }
            }
            EvidenceOrigin::Heuristic => {
                if empty_optional(&self.rule) || self.wiring_site.is_none() {
                    return Err(ProvenanceError::new(
                        "heuristic evidence requires a rule and wiring site",
                    ));
                }
                if self
                    .wiring_site
                    .as_ref()
                    .is_some_and(|anchor| anchor.start_byte == anchor.end_byte)
                {
                    return Err(ProvenanceError::new(
                        "heuristic wiring sites require a non-empty source range",
                    ));
                }
            }
        }

        if self.candidates.len() > MAX_RESOLUTION_CANDIDATES {
            return Err(ProvenanceError::new(
                "resolution candidates exceed the supported bound",
            ));
        }
        let mut previous = None;
        for candidate in &self.candidates {
            if candidate.node_id.trim().is_empty() || candidate.reason.trim().is_empty() {
                return Err(ProvenanceError::new(
                    "resolution candidates require node ID and reason",
                ));
            }
            if candidate.score.is_some_and(|score| !score.is_finite()) {
                return Err(ProvenanceError::new(
                    "resolution candidate score must be finite",
                ));
            }
            if candidate
                .anchor
                .as_ref()
                .is_some_and(|anchor| !anchor.is_valid())
            {
                return Err(ProvenanceError::new(
                    "resolution candidate anchor is invalid",
                ));
            }
            if previous.is_some_and(|value: &str| value >= candidate.node_id.as_str()) {
                return Err(ProvenanceError::new(
                    "resolution candidates must be unique and sorted by node ID",
                ));
            }
            previous = Some(candidate.node_id.as_str());
        }
        if self.confidence == EvidenceConfidence::Ambiguous && self.candidates.len() < 2 {
            return Err(ProvenanceError::new(
                "ambiguous evidence requires at least two candidates",
            ));
        }
        Ok(())
    }
}

fn empty_optional(value: &Option<String>) -> bool {
    value.as_ref().is_none_or(|text| text.trim().is_empty())
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("invalid provenance: {message}")]
pub struct ProvenanceError {
    message: String,
}

impl ProvenanceError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
