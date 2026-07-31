use compass_ir::SourceAnchor;
use serde::{Deserialize, Serialize};

pub const UNIVERSAL_EVIDENCE_SCHEMA: &str = "compass.languages.evidence/1";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterProfile {
    Legacy,
    UniversalCandidate,
    UniversalComplete,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterCapability {
    Declarations,
    LexicalScopes,
    Namespaces,
    Overloads,
    Annotations,
    Inheritance,
    Interfaces,
    Traits,
    ImplOwnership,
    Macros,
    Imports,
    Calls,
    ExternalPackages,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdapterDescriptor {
    pub id: String,
    pub language: String,
    pub version: u32,
    pub evidence_schema: String,
    pub profile: AdapterProfile,
    pub capabilities: Vec<AdapterCapability>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclarationKind {
    Module,
    Trait,
    Struct,
    Enum,
    TypeAlias,
    Function,
    Method,
    Field,
    Constant,
    Macro,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DeclarationFact {
    pub symbol: String,
    pub name: String,
    pub kind: DeclarationKind,
    pub owner: Option<String>,
    pub anchor: SourceAnchor,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct ScopeFact {
    pub id: String,
    pub owner: String,
    pub parent: Option<String>,
    pub anchor: SourceAnchor,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    Import,
    Alias,
    Module,
    Package,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct BindingFact {
    pub scope: String,
    pub spelling: String,
    pub identity: String,
    pub kind: BindingKind,
    pub anchor: SourceAnchor,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OccurrenceRole {
    Call,
    Import,
    TypeReference,
    TraitBound,
    MacroInvocation,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct OccurrenceFact {
    pub owner: String,
    pub role: OccurrenceRole,
    pub spelling: String,
    pub qualifier: Option<String>,
    pub anchor: SourceAnchor,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct RelationshipCandidate {
    pub owner: String,
    pub role: OccurrenceRole,
    pub spelling: String,
    pub qualifier: Option<String>,
    pub anchor: SourceAnchor,
    pub target_kinds: Vec<DeclarationKind>,
    pub external_identity: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UniversalEvidence {
    pub schema: String,
    pub adapter_id: String,
    pub adapter_version: u32,
    pub profile: AdapterProfile,
    #[serde(default)]
    pub declarations: Vec<DeclarationFact>,
    #[serde(default)]
    pub scopes: Vec<ScopeFact>,
    #[serde(default)]
    pub bindings: Vec<BindingFact>,
    #[serde(default)]
    pub occurrences: Vec<OccurrenceFact>,
    #[serde(default)]
    pub relationship_candidates: Vec<RelationshipCandidate>,
}

impl UniversalEvidence {
    #[must_use]
    pub fn new(adapter: &AdapterDescriptor) -> Self {
        Self {
            schema: adapter.evidence_schema.clone(),
            adapter_id: adapter.id.clone(),
            adapter_version: adapter.version,
            profile: adapter.profile,
            declarations: Vec::new(),
            scopes: Vec::new(),
            bindings: Vec::new(),
            occurrences: Vec::new(),
            relationship_candidates: Vec::new(),
        }
    }
}
