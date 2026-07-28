use serde::{Deserialize, Serialize};

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
