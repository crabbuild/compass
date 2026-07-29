use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const MAX_RESOLUTION_CANDIDATES: usize = 20;
pub const OCCURRENCE_RULE_ATTRIBUTE: &str = "_occurrence_rule";
pub const ENDPOINT_REWRITE_RULES_ATTRIBUTE: &str = "_endpoint_rewrite_rules";
pub const TRUSTED_EDGE_RECORD_ATTRIBUTE: &str = "_compass_v1_edge_record";
pub const TRUSTED_NODE_RECORD_ATTRIBUTE: &str = "_compass_v1_node_record";

/// Immutable producer rule used to distinguish relationship occurrences.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OccurrenceRule(String);

impl OccurrenceRule {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Option<Self> {
        let value = value.into();
        (!value.trim().is_empty()).then_some(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_endpoint_rewrite(&self) -> bool {
        EndpointRewriteRule::from_wire_name(self.as_str()).is_some()
    }
}

/// Closed set of endpoint rewrites that can alter flexible graph facts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EndpointRewriteRule {
    CsharpNamespaceCanonicalization,
    LanguageFamilyStubResolution,
    PhpQualifiedTypeResolution,
    CanonicalImportTarget,
    UniqueStubEndpointResolution,
    SourceScopedNodeDisambiguation,
    HeaderImportDisambiguation,
    GraphSemanticIdRemap,
    GraphDocumentTwinRemap,
    GraphGhostEndpointRemap,
    GraphNormalizedIdRemap,
    IncrementalAstEndpointRemap,
}

impl EndpointRewriteRule {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CsharpNamespaceCanonicalization => "csharp-namespace-canonicalization",
            Self::LanguageFamilyStubResolution => "language-family-stub-resolution",
            Self::PhpQualifiedTypeResolution => "php-qualified-type-resolution",
            Self::CanonicalImportTarget => "canonical-import-target",
            Self::UniqueStubEndpointResolution => "unique-stub-endpoint-resolution",
            Self::SourceScopedNodeDisambiguation => "source-scoped-node-disambiguation",
            Self::HeaderImportDisambiguation => "header-import-disambiguation",
            Self::GraphSemanticIdRemap => "graph-semantic-id-remap",
            Self::GraphDocumentTwinRemap => "graph-document-twin-remap",
            Self::GraphGhostEndpointRemap => "graph-ghost-endpoint-remap",
            Self::GraphNormalizedIdRemap => "graph-normalized-id-remap",
            Self::IncrementalAstEndpointRemap => "incremental-ast-endpoint-remap",
        }
    }

    #[must_use]
    pub fn from_wire_name(value: &str) -> Option<Self> {
        serde_json::from_value(Value::String(value.to_owned())).ok()
    }
}

/// Typed evidence that an endpoint was rewritten after a producer emitted it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EndpointRewriteEvidence {
    pub rule: EndpointRewriteRule,
    pub score: f64,
}

/// Capture the open-ended producer rule before any endpoint mutation.
pub fn preserve_occurrence_rule(attributes: &mut Map<String, Value>) {
    if attributes.contains_key(OCCURRENCE_RULE_ATTRIBUTE)
        || attributes.contains_key(TRUSTED_EDGE_RECORD_ATTRIBUTE)
    {
        return;
    }
    let Some(rule) = attributes
        .get("rule")
        .and_then(Value::as_str)
        .and_then(|rule| OccurrenceRule::new(rule.to_owned()))
    else {
        return;
    };
    attributes.insert(OCCURRENCE_RULE_ATTRIBUTE.to_owned(), Value::String(rule.0));
}

/// Append endpoint-rewrite evidence without replacing the producer's evidence.
pub fn append_endpoint_rewrite_evidence(
    attributes: &mut Map<String, Value>,
    evidence: EndpointRewriteEvidence,
) {
    let mut entry = Map::new();
    for key in [
        "extractor",
        "source_file",
        "source_location",
        "source_anchor",
        "line_start",
        "line_end",
        "column_start",
        "column_end",
        "start_byte",
        "end_byte",
        "candidates",
    ] {
        if let Some(value) = attributes.get(key) {
            entry.insert(key.to_owned(), value.clone());
        }
    }
    entry.insert("_origin".to_owned(), Value::String("heuristic".to_owned()));
    entry.insert(
        "confidence".to_owned(),
        Value::String("INFERRED".to_owned()),
    );
    entry.insert(
        "rule".to_owned(),
        Value::String(evidence.rule.as_str().to_owned()),
    );
    entry.insert("score".to_owned(), Value::from(evidence.score));

    let mut entries = attributes
        .remove(ENDPOINT_REWRITE_RULES_ATTRIBUTE)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    entries.push(Value::Object(entry));
    entries.sort_by_cached_key(Value::to_string);
    entries.dedup();
    attributes.insert(
        ENDPOINT_REWRITE_RULES_ATTRIBUTE.to_owned(),
        Value::Array(entries),
    );
}

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

impl EvidenceOrigin {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ast => "ast",
            Self::Config => "config",
            Self::Convention => "convention",
            Self::Artifact => "artifact",
            Self::Heuristic => "heuristic",
        }
    }
}

/// The strongest claim Compass can make from a piece of evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceConfidence {
    Exact,
    Inferred,
    Ambiguous,
}

impl EvidenceConfidence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Inferred => "inferred",
            Self::Ambiguous => "ambiguous",
        }
    }

    #[must_use]
    pub const fn legacy_str(self) -> &'static str {
        match self {
            Self::Exact => "EXTRACTED",
            Self::Inferred => "INFERRED",
            Self::Ambiguous => "AMBIGUOUS",
        }
    }
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
        self.validate_endpoint_rewrite()?;
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

    pub fn validate_endpoint_rewrite(&self) -> Result<(), ProvenanceError> {
        let Some(rule) = self
            .rule
            .as_deref()
            .and_then(EndpointRewriteRule::from_wire_name)
        else {
            return Ok(());
        };
        if self.origin != EvidenceOrigin::Heuristic
            || self.confidence != EvidenceConfidence::Inferred
        {
            return Err(ProvenanceError::new(format!(
                "endpoint rewrite {} must be heuristic evidence with inferred confidence",
                rule.as_str()
            )));
        }
        if !self.anchors.is_empty() {
            return Err(ProvenanceError::new(format!(
                "endpoint rewrite {} must not contain direct anchors",
                rule.as_str()
            )));
        }
        if self
            .wiring_site
            .as_ref()
            .is_none_or(|site| !site.is_valid() || site.start_byte == site.end_byte)
        {
            return Err(ProvenanceError::new(format!(
                "endpoint rewrite {} requires a valid non-empty wiring site",
                rule.as_str()
            )));
        }
        if self
            .score
            .is_none_or(|score| !score.is_finite() || !(0.0..=1.0).contains(&score))
        {
            return Err(ProvenanceError::new(format!(
                "endpoint rewrite {} requires a finite score between 0.0 and 1.0",
                rule.as_str()
            )));
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
