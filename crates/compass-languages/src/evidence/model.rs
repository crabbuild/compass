use serde::{Deserialize, Serialize};

/// Identity and truthful capability declaration for one semantic adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterIdentity {
    pub language: String,
    pub producer: String,
    pub capabilities: Vec<LanguageCapability>,
}

/// A byte-exact source occurrence.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceRange {
    pub source_file: String,
    pub start_byte: u64,
    pub end_byte: u64,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

/// Semantic operations that an adapter may truthfully advertise.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageCapability {
    Declarations,
    LexicalScopes,
    Imports,
    Reexports,
    Aliases,
    Calls,
    Construction,
    Decorators,
    TypeReferences,
    BaseTypes,
    Members,
    Ownership,
    Receivers,
    Embedding,
    ExternalReferences,
}

/// The source-level role played by an exact occurrence.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticRole {
    Import,
    Reexport,
    Alias,
    Call,
    CallableReference,
    Construction,
    Decorator,
    Annotation,
    BaseType,
    TypeReference,
    MemberAccess,
    Ownership,
    Receiver,
    Embedding,
}

impl SemanticRole {
    #[must_use]
    pub const fn required_capability(self) -> LanguageCapability {
        match self {
            Self::Import => LanguageCapability::Imports,
            Self::Reexport => LanguageCapability::Reexports,
            Self::Alias => LanguageCapability::Aliases,
            Self::Call | Self::CallableReference => LanguageCapability::Calls,
            Self::Construction => LanguageCapability::Construction,
            Self::Decorator => LanguageCapability::Decorators,
            Self::Annotation | Self::TypeReference => LanguageCapability::TypeReferences,
            Self::BaseType => LanguageCapability::BaseTypes,
            Self::MemberAccess => LanguageCapability::Members,
            Self::Ownership => LanguageCapability::Ownership,
            Self::Receiver => LanguageCapability::Receivers,
            Self::Embedding => LanguageCapability::Embedding,
        }
    }
}

/// The binding rule established at an exact source range.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingKind {
    Import,
    ImportAlias,
    Reexport,
    LocalAlias,
    Package,
    Member,
}

impl BindingKind {
    #[must_use]
    pub const fn required_capability(self) -> LanguageCapability {
        match self {
            Self::Import | Self::Package => LanguageCapability::Imports,
            Self::ImportAlias | Self::LocalAlias => LanguageCapability::Aliases,
            Self::Reexport => LanguageCapability::Reexports,
            Self::Member => LanguageCapability::Members,
        }
    }
}

/// A relationship that the shared resolver may materialize.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateRelation {
    Calls,
    IndirectCalls,
    Constructs,
    Decorates,
    Annotates,
    Extends,
    Implements,
    References,
    AccessesMember,
    Contains,
    Owns,
    Embeds,
    Imports,
    Reexports,
}

impl CandidateRelation {
    #[must_use]
    pub const fn required_capability(self) -> LanguageCapability {
        match self {
            Self::Calls | Self::IndirectCalls => LanguageCapability::Calls,
            Self::Constructs => LanguageCapability::Construction,
            Self::Decorates => LanguageCapability::Decorators,
            Self::Annotates | Self::References | Self::Implements => {
                LanguageCapability::TypeReferences
            }
            Self::Extends => LanguageCapability::BaseTypes,
            Self::AccessesMember => LanguageCapability::Members,
            Self::Contains | Self::Owns => LanguageCapability::Ownership,
            Self::Embeds => LanguageCapability::Embedding,
            Self::Imports => LanguageCapability::Imports,
            Self::Reexports => LanguageCapability::Reexports,
        }
    }

    #[must_use]
    pub const fn requires_occurrence(self) -> bool {
        !matches!(self, Self::Contains | Self::Owns)
    }
}

/// One source declaration that may become a graph node.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeclarationFact {
    pub id: String,
    pub language: String,
    /// Existing raw graph identity enriched by this declaration.
    pub graph_node_id: String,
    pub kind: String,
    pub name: String,
    pub qualified_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_or_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    pub range: EvidenceRange,
}

/// One lexical scope with optional declaration ownership.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeFact {
    pub id: String,
    pub language: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_declaration_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_scope_id: Option<String>,
    pub range: EvidenceRange,
}

/// One explicit name binding.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingFact {
    pub id: String,
    pub language: String,
    pub kind: BindingKind,
    pub spelling: String,
    pub qualified_target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_declaration_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub range: EvidenceRange,
}

/// One exact semantic use site.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccurrenceFact {
    pub id: String,
    pub language: String,
    pub role: SemanticRole,
    pub owner_declaration_id: String,
    pub spelling: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    pub range: EvidenceRange,
}

/// Constraints applied before a target can be selected.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolutionConstraint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_or_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_target_kinds: Vec<String>,
    #[serde(default)]
    pub allow_external: bool,
}

/// A source-grounded relationship awaiting constrained resolution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelationshipCandidate {
    pub id: String,
    pub language: String,
    pub relation: CandidateRelation,
    pub source_declaration_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurrence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_id: Option<String>,
    pub target_spelling: String,
    pub constraints: ResolutionConstraint,
}

/// A bounded extraction diagnostic tied to source when available.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceDiagnostic {
    pub code: String,
    pub language: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<EvidenceRange>,
    pub message: String,
}

/// Direct output of one universal semantic adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SemanticEvidenceBatch {
    pub adapter: AdapterIdentity,
    pub declarations: Vec<DeclarationFact>,
    pub scopes: Vec<ScopeFact>,
    pub bindings: Vec<BindingFact>,
    pub occurrences: Vec<OccurrenceFact>,
    pub candidates: Vec<RelationshipCandidate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<EvidenceDiagnostic>,
}
