use serde::{Deserialize, Serialize};

use crate::UniversalAdapterProfile;

/// Identity and truthful capability declaration for one semantic adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdapterIdentity {
    pub id: String,
    pub language: String,
    /// Parser dialect preserved separately from the semantic language family.
    /// `None` is retained for legacy adapters that predate dialect metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dialect: Option<String>,
    pub version: u32,
    pub evidence_schema: String,
    pub profile: UniversalAdapterProfile,
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
    Namespaces,
    Traits,
    ImplOwnership,
    Macros,
    Tests,
    Imports,
    Reexports,
    Aliases,
    Calls,
    Construction,
    Decorators,
    TypeReferences,
    BaseTypes,
    HierarchyDispatch,
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
    Override,
    TypeReference,
    MemberAccess,
    Ownership,
    Receiver,
    Embedding,
    TraitBound,
    MacroInvocation,
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
            Self::Override => LanguageCapability::HierarchyDispatch,
            Self::MemberAccess => LanguageCapability::Members,
            Self::Ownership => LanguageCapability::Ownership,
            Self::Receiver => LanguageCapability::Receivers,
            Self::Embedding => LanguageCapability::Embedding,
            Self::TraitBound => LanguageCapability::Traits,
            Self::MacroInvocation => LanguageCapability::Macros,
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
    CallResult,
    Package,
    Member,
}

/// The TypeScript/JavaScript symbol space occupied by a declaration or
/// binding. ECMAScript modules expose values, types, and namespaces through
/// overlapping source spellings, so retaining the space is required before a
/// resolver can safely select one identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolNamespace {
    Value,
    Type,
    Namespace,
    ValueAndType,
}

impl BindingKind {
    #[must_use]
    pub const fn required_capability(self) -> LanguageCapability {
        match self {
            Self::Import | Self::Package => LanguageCapability::Imports,
            Self::ImportAlias | Self::LocalAlias => LanguageCapability::Aliases,
            Self::CallResult => LanguageCapability::TypeReferences,
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
    UsesTrait,
    Overrides,
    References,
    TypeOf,
    Returns,
    AccessesMember,
    Contains,
    Owns,
    Embeds,
    Imports,
    Reexports,
    InvokesMacro,
    Tests,
}

impl CandidateRelation {
    #[must_use]
    pub const fn required_capability(self) -> LanguageCapability {
        match self {
            Self::Calls | Self::IndirectCalls => LanguageCapability::Calls,
            Self::Constructs => LanguageCapability::Construction,
            Self::Decorates => LanguageCapability::Decorators,
            Self::Annotates
            | Self::References
            | Self::TypeOf
            | Self::Returns
            | Self::Implements => LanguageCapability::TypeReferences,
            Self::Extends => LanguageCapability::BaseTypes,
            Self::UsesTrait => LanguageCapability::Traits,
            Self::Overrides => LanguageCapability::HierarchyDispatch,
            Self::AccessesMember => LanguageCapability::Members,
            Self::Contains | Self::Owns => LanguageCapability::Ownership,
            Self::Embeds => LanguageCapability::Embedding,
            Self::Imports => LanguageCapability::Imports,
            Self::Reexports => LanguageCapability::Reexports,
            Self::InvokesMacro => LanguageCapability::Macros,
            Self::Tests => LanguageCapability::Tests,
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
    /// Optional value/type/namespace identity. `None` preserves legacy
    /// adapters that predate explicit symbol-space evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<SymbolNamespace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_or_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Number of source-level parameters when this declaration is callable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_count: Option<u32>,
    /// Canonical source-level parameter types, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameter_types: Vec<String>,
    /// Whether all direct base types for this declaration were parsed and emitted.
    #[serde(default, skip_serializing_if = "is_false")]
    pub direct_bases_complete: bool,
    /// Whether the final source-level parameter accepts a variable arity.
    #[serde(default)]
    pub variadic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    /// Full syntax-node start for legacy callable identities. The canonical
    /// evidence range may intentionally begin at the identifier, while the
    /// public graph identity historically used the enclosing declaration
    /// start (for example `export async function` begins after `export`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_start_byte: Option<u64>,
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

fn is_false(value: &bool) -> bool {
    !*value
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
    /// Optional symbol space selected by the source construct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<SymbolNamespace>,
    /// Whether this import or re-export is explicitly type-only in source.
    #[serde(default, skip_serializing_if = "is_false")]
    pub type_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_declaration_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    /// Zero-based result selected by a source-level destructuring assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_index: Option<u32>,
    /// Exact nominal result type proven by the adapter for this call result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_type_qualified_name: Option<String>,
    /// Earlier call result whose nominal type receives this method call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_binding_id: Option<String>,
    /// Prior binding retained when project-wide call-result evidence is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_binding_id: Option<String>,
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
    /// Exact declaration proven by the adapter from the same source construct.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_target_declaration_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact_language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_or_package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    /// Number of source-level arguments at a callable occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_count: Option<u32>,
    /// Canonical argument types when statically proven at the call site.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub argument_types: Vec<Option<String>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_target_kinds: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hierarchy: Option<HierarchyConstraint>,
    #[serde(default)]
    pub allow_external: bool,
}

/// Typed hierarchy evidence used by shared receiver dispatch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HierarchyConstraint {
    /// One ordered direct-base occurrence. `base_set_complete` describes the
    /// complete source-level base list for the owning declaration.
    DirectBase { base_set_complete: bool },
    /// Resolve a member after the receiver in a proven source hierarchy.
    ReceiverDispatch {
        receiver_qualified_name: String,
        strategy: ReceiverDispatchStrategy,
    },
    /// Resolve `Self::Type` through one exact Rust trait hierarchy and the
    /// corresponding associated-type realization for the concrete receiver.
    RustAssociatedType {
        receiver_declaration_id: String,
        receiver_qualified_name: String,
        trait_qualified_name: String,
    },
}

/// Language-selected linearization implemented by the shared resolver.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverDispatchStrategy {
    C3FromReceiver,
    C3AfterReceiver,
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
