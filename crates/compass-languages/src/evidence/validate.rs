use std::collections::BTreeSet;
use std::path::{Component, Path};

use ahash::AHashMap;
use serde::{Deserialize, Serialize};

use super::model::{
    BindingFact, CandidateRelation, DeclarationFact, EvidenceRange, HierarchyConstraint,
    LanguageCapability, OccurrenceFact, RelationshipCandidate, ScopeFact, SemanticEvidenceBatch,
    SemanticRole, SymbolNamespace,
};

/// Hard resource ceilings for a single adapter evidence batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EvidenceLimits {
    pub declarations: usize,
    pub scopes: usize,
    pub bindings: usize,
    pub occurrences: usize,
    pub candidates: usize,
    pub diagnostics: usize,
    pub callable_types_per_fact: usize,
    pub callable_type_bytes: usize,
    pub allowed_target_kinds_per_candidate: usize,
    pub diagnostic_message_bytes: usize,
}

impl Default for EvidenceLimits {
    fn default() -> Self {
        Self {
            declarations: 100_000,
            scopes: 100_000,
            bindings: 100_000,
            occurrences: 500_000,
            candidates: 500_000,
            diagnostics: 10_000,
            callable_types_per_fact: 256,
            callable_type_bytes: 1_024,
            allowed_target_kinds_per_candidate: 64,
            diagnostic_message_bytes: 4_096,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceErrorCode {
    InvalidAdapter,
    ResourceLimit,
    DuplicateId,
    InvalidPath,
    InvalidRange,
    MissingReference,
    LanguageMismatch,
    MissingOccurrence,
    UndeclaredCapability,
    InvalidFact,
}

/// Stable validation failure. Messages are deliberately bounded.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?}: {message}")]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceError {
    pub code: EvidenceErrorCode,
    pub message: String,
}

const MAX_ERROR_MESSAGE_BYTES: usize = 512;

impl EvidenceError {
    pub(super) fn new(code: EvidenceErrorCode, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.len() > MAX_ERROR_MESSAGE_BYTES {
            let mut boundary = MAX_ERROR_MESSAGE_BYTES;
            while boundary > 0 && !message.is_char_boundary(boundary) {
                boundary -= 1;
            }
            message.truncate(boundary);
        }
        Self { code, message }
    }
}

/// Validate all references, source evidence, capabilities, and resource bounds.
///
/// Fact traversal is sorted by typed ID so the first returned failure is
/// independent of input ordering.
pub fn validate_evidence(
    batch: &SemanticEvidenceBatch,
    limits: EvidenceLimits,
) -> Result<(), EvidenceError> {
    validate_adapter(batch)?;
    validate_limits(batch, limits)?;

    let capabilities: BTreeSet<_> = batch.adapter.capabilities.iter().copied().collect();
    if capabilities.len() != batch.adapter.capabilities.len() {
        return Err(EvidenceError::new(
            EvidenceErrorCode::InvalidAdapter,
            "adapter capabilities must be unique",
        ));
    }

    let mut ids = Vec::with_capacity(
        batch.declarations.len()
            + batch.scopes.len()
            + batch.bindings.len()
            + batch.occurrences.len()
            + batch.candidates.len(),
    );
    ids.extend(
        batch
            .declarations
            .iter()
            .map(|fact| (fact.id.as_str(), "declaration")),
    );
    ids.extend(batch.scopes.iter().map(|fact| (fact.id.as_str(), "scope")));
    ids.extend(
        batch
            .bindings
            .iter()
            .map(|fact| (fact.id.as_str(), "binding")),
    );
    ids.extend(
        batch
            .occurrences
            .iter()
            .map(|fact| (fact.id.as_str(), "occurrence")),
    );
    ids.extend(
        batch
            .candidates
            .iter()
            .map(|fact| (fact.id.as_str(), "candidate")),
    );
    ids.sort_unstable();

    for pair in ids.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err(EvidenceError::new(
                EvidenceErrorCode::DuplicateId,
                format!("duplicate evidence id {:?}", pair[0].0),
            ));
        }
    }
    if let Some((_, kind)) = ids.iter().find(|(id, _)| id.is_empty()) {
        return Err(EvidenceError::new(
            EvidenceErrorCode::InvalidFact,
            format!("{kind} id must not be empty"),
        ));
    }

    let declarations: AHashMap<_, _> = batch
        .declarations
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    let scopes: AHashMap<_, _> = batch
        .scopes
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    let bindings: AHashMap<_, _> = batch
        .bindings
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    let occurrences: AHashMap<_, _> = batch
        .occurrences
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();

    let mut facts = Vec::with_capacity(ids.len());
    facts.extend(batch.declarations.iter().map(Fact::Declaration));
    facts.extend(batch.scopes.iter().map(Fact::Scope));
    facts.extend(batch.bindings.iter().map(Fact::Binding));
    facts.extend(batch.occurrences.iter().map(Fact::Occurrence));
    facts.extend(batch.candidates.iter().map(Fact::Candidate));
    facts.sort_unstable_by(|left, right| left.id().cmp(right.id()));

    for fact in facts {
        validate_fact(
            fact,
            batch.adapter.language.as_str(),
            &capabilities,
            &declarations,
            &scopes,
            &bindings,
            &occurrences,
            limits,
        )?;
    }
    validate_binding_chains(&bindings)?;

    let mut diagnostics: Vec<_> = batch.diagnostics.iter().collect();
    diagnostics.sort_unstable_by(|left, right| {
        left.fact_id
            .cmp(&right.fact_id)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    for diagnostic in diagnostics {
        if diagnostic.code.is_empty() || diagnostic.language != batch.adapter.language {
            return Err(EvidenceError::new(
                EvidenceErrorCode::InvalidFact,
                "diagnostic code and adapter language must be valid",
            ));
        }
        if diagnostic.message.len() > limits.diagnostic_message_bytes {
            return Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                format!(
                    "diagnostic {:?} exceeds message byte limit {}",
                    diagnostic.code, limits.diagnostic_message_bytes
                ),
            ));
        }
        if let Some(fact_id) = diagnostic.fact_id.as_deref()
            && ids.binary_search_by_key(&fact_id, |(id, _)| *id).is_err()
        {
            return Err(EvidenceError::new(
                EvidenceErrorCode::MissingReference,
                format!("diagnostic references missing fact {fact_id:?}"),
            ));
        }
        if let Some(range) = diagnostic.range.as_ref() {
            validate_range(range, &diagnostic.code, false)?;
        }
    }

    Ok(())
}

fn validate_adapter(batch: &SemanticEvidenceBatch) -> Result<(), EvidenceError> {
    if batch.adapter.language.trim().is_empty() || batch.adapter.producer.trim().is_empty() {
        return Err(EvidenceError::new(
            EvidenceErrorCode::InvalidAdapter,
            "adapter language and producer must not be empty",
        ));
    }
    Ok(())
}

fn validate_limits(
    batch: &SemanticEvidenceBatch,
    limits: EvidenceLimits,
) -> Result<(), EvidenceError> {
    let counts = [
        (
            "declarations",
            batch.declarations.len(),
            limits.declarations,
        ),
        ("scopes", batch.scopes.len(), limits.scopes),
        ("bindings", batch.bindings.len(), limits.bindings),
        ("occurrences", batch.occurrences.len(), limits.occurrences),
        ("candidates", batch.candidates.len(), limits.candidates),
        ("diagnostics", batch.diagnostics.len(), limits.diagnostics),
    ];
    if let Some((name, count, limit)) = counts.into_iter().find(|(_, count, limit)| count > limit) {
        return Err(EvidenceError::new(
            EvidenceErrorCode::ResourceLimit,
            format!("{name} count {count} exceeds limit {limit}"),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Fact<'a> {
    Declaration(&'a DeclarationFact),
    Scope(&'a ScopeFact),
    Binding(&'a BindingFact),
    Occurrence(&'a OccurrenceFact),
    Candidate(&'a RelationshipCandidate),
}

impl<'a> Fact<'a> {
    fn id(self) -> &'a str {
        match self {
            Self::Declaration(fact) => &fact.id,
            Self::Scope(fact) => &fact.id,
            Self::Binding(fact) => &fact.id,
            Self::Occurrence(fact) => &fact.id,
            Self::Candidate(fact) => &fact.id,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_fact(
    fact: Fact<'_>,
    adapter_language: &str,
    capabilities: &BTreeSet<LanguageCapability>,
    declarations: &AHashMap<&str, &DeclarationFact>,
    scopes: &AHashMap<&str, &ScopeFact>,
    bindings: &AHashMap<&str, &BindingFact>,
    occurrences: &AHashMap<&str, &OccurrenceFact>,
    limits: EvidenceLimits,
) -> Result<(), EvidenceError> {
    match fact {
        Fact::Declaration(fact) => {
            validate_language(&fact.id, &fact.language, adapter_language)?;
            validate_range(&fact.range, &fact.id, fact.kind == "file")?;
            require_capability(&fact.id, LanguageCapability::Declarations, capabilities)?;
            if fact.graph_node_id.is_empty()
                || fact.kind.is_empty()
                || fact.name.is_empty()
                || fact.qualified_name.is_empty()
            {
                return Err(invalid_fact(&fact.id, "declaration identity is empty"));
            }
            require_optional_reference(&fact.id, "scope", fact.scope_id.as_deref(), scopes)?;
            validate_callable_types(
                &fact.id,
                &fact.parameter_types,
                fact.parameter_count,
                limits,
            )?;
            if fact.direct_bases_complete
                && !matches!(
                    (fact.language.as_str(), fact.kind.as_str()),
                    (
                        "java",
                        "class" | "interface" | "enum" | "record" | "annotation_type"
                    ) | ("csharp", "class" | "interface" | "record" | "struct")
                        | ("rust", "trait")
                )
            {
                return Err(invalid_fact(
                    &fact.id,
                    "complete direct-base evidence requires a qualified nominal type declaration",
                ));
            }
        }
        Fact::Scope(fact) => {
            validate_language(&fact.id, &fact.language, adapter_language)?;
            let empty_file_scope = fact.kind == "module"
                && fact
                    .owner_declaration_id
                    .as_deref()
                    .and_then(|id| declarations.get(id))
                    .is_some_and(|owner| owner.kind == "file" && owner.range == fact.range);
            validate_range(&fact.range, &fact.id, empty_file_scope)?;
            require_capability(&fact.id, LanguageCapability::LexicalScopes, capabilities)?;
            if fact.kind.is_empty() {
                return Err(invalid_fact(&fact.id, "scope kind is empty"));
            }
            require_optional_reference(
                &fact.id,
                "owner declaration",
                fact.owner_declaration_id.as_deref(),
                declarations,
            )?;
            require_optional_reference(
                &fact.id,
                "parent scope",
                fact.parent_scope_id.as_deref(),
                scopes,
            )?;
        }
        Fact::Binding(fact) => {
            validate_language(&fact.id, &fact.language, adapter_language)?;
            validate_range(&fact.range, &fact.id, false)?;
            require_capability(&fact.id, fact.kind.required_capability(), capabilities)?;
            if fact.spelling.is_empty() || fact.qualified_target.is_empty() {
                return Err(invalid_fact(&fact.id, "binding identity is empty"));
            }
            if fact.type_only
                && !matches!(
                    fact.kind,
                    crate::BindingKind::Import
                        | crate::BindingKind::ImportAlias
                        | crate::BindingKind::Reexport
                )
            {
                return Err(invalid_fact(
                    &fact.id,
                    "only import and re-export bindings may be type-only",
                ));
            }
            if fact.type_only
                && !matches!(
                    fact.namespace,
                    Some(SymbolNamespace::Type | SymbolNamespace::Namespace)
                )
            {
                return Err(invalid_fact(
                    &fact.id,
                    "type-only bindings must identify the type or namespace symbol space",
                ));
            }
            if fact.output_index.is_some() && fact.kind != crate::BindingKind::CallResult {
                return Err(invalid_fact(
                    &fact.id,
                    "only call-result bindings may select an output index",
                ));
            }
            if fact.result_type_qualified_name.is_some()
                && fact.kind != crate::BindingKind::CallResult
            {
                return Err(invalid_fact(
                    &fact.id,
                    "only call-result bindings may carry an exact result type",
                ));
            }
            if fact
                .result_type_qualified_name
                .as_deref()
                .is_some_and(str::is_empty)
            {
                return Err(invalid_fact(&fact.id, "call-result type is empty"));
            }
            if (fact.receiver_binding_id.is_some() || fact.fallback_binding_id.is_some())
                && fact.kind != crate::BindingKind::CallResult
            {
                return Err(invalid_fact(
                    &fact.id,
                    "only call-result bindings may reference receiver or fallback bindings",
                ));
            }
            require_optional_reference(
                &fact.id,
                "receiver binding",
                fact.receiver_binding_id.as_deref(),
                bindings,
            )?;
            require_optional_reference(
                &fact.id,
                "fallback binding",
                fact.fallback_binding_id.as_deref(),
                bindings,
            )?;
            if let Some(receiver_id) = fact.receiver_binding_id.as_deref()
                && bindings
                    .get(receiver_id)
                    .is_some_and(|receiver| receiver.kind != crate::BindingKind::CallResult)
            {
                return Err(invalid_fact(
                    &fact.id,
                    "call-result receiver must reference another call-result binding",
                ));
            }
            if let Some(fallback_id) = fact.fallback_binding_id.as_deref()
                && bindings
                    .get(fallback_id)
                    .is_some_and(|fallback| fallback.kind == crate::BindingKind::CallResult)
            {
                return Err(invalid_fact(
                    &fact.id,
                    "call-result fallback must reference a non-call-result binding",
                ));
            }
            require_optional_reference(
                &fact.id,
                "target declaration",
                fact.target_declaration_id.as_deref(),
                declarations,
            )?;
            require_optional_reference(&fact.id, "scope", fact.scope_id.as_deref(), scopes)?;
        }
        Fact::Occurrence(fact) => {
            validate_language(&fact.id, &fact.language, adapter_language)?;
            validate_range(&fact.range, &fact.id, false)?;
            require_capability(&fact.id, fact.role.required_capability(), capabilities)?;
            if fact.spelling.is_empty() {
                return Err(invalid_fact(&fact.id, "occurrence spelling is empty"));
            }
            require_reference(
                &fact.id,
                "owner declaration",
                &fact.owner_declaration_id,
                declarations,
            )?;
            require_optional_reference(&fact.id, "scope", fact.scope_id.as_deref(), scopes)?;
        }
        Fact::Candidate(fact) => {
            validate_language(&fact.id, &fact.language, adapter_language)?;
            require_capability(&fact.id, fact.relation.required_capability(), capabilities)?;
            require_reference(
                &fact.id,
                "source declaration",
                &fact.source_declaration_id,
                declarations,
            )?;
            if fact.target_spelling.is_empty() {
                return Err(invalid_fact(
                    &fact.id,
                    &format!(
                        "candidate {:?} from declaration {:?} has an empty target spelling \
                         (qualified target {:?}, module/package {:?})",
                        fact.relation,
                        fact.source_declaration_id,
                        fact.constraints.qualified_name,
                        fact.constraints.module_or_package
                    ),
                ));
            }
            if fact.relation.requires_occurrence() && fact.occurrence_id.is_none() {
                return Err(EvidenceError::new(
                    EvidenceErrorCode::MissingOccurrence,
                    format!("candidate {:?} requires an occurrence", fact.id),
                ));
            }
            if let Some(occurrence_id) = fact.occurrence_id.as_deref() {
                require_reference(&fact.id, "occurrence", occurrence_id, occurrences)?;
                let occurrence = occurrences[occurrence_id];
                if occurrence.language != fact.language {
                    return Err(EvidenceError::new(
                        EvidenceErrorCode::LanguageMismatch,
                        format!(
                            "candidate {:?} and occurrence {:?} use different languages",
                            fact.id, occurrence_id
                        ),
                    ));
                }
            }
            require_optional_reference(&fact.id, "binding", fact.binding_id.as_deref(), bindings)?;
            require_optional_reference(
                &fact.id,
                "constraint scope",
                fact.constraints.scope_id.as_deref(),
                scopes,
            )?;
            require_optional_reference(
                &fact.id,
                "exact target declaration",
                fact.constraints.exact_target_declaration_id.as_deref(),
                declarations,
            )?;
            validate_callable_types(
                &fact.id,
                &fact.constraints.argument_types,
                fact.constraints.argument_count,
                limits,
            )?;
            if let Some(language) = fact.constraints.exact_language.as_deref()
                && language != fact.language
            {
                return Err(EvidenceError::new(
                    EvidenceErrorCode::LanguageMismatch,
                    format!(
                        "candidate {:?} constraint language {:?} differs from occurrence language",
                        fact.id, language
                    ),
                ));
            }
            if let Some(hierarchy) = fact.constraints.hierarchy.as_ref() {
                require_capability(
                    &fact.id,
                    LanguageCapability::HierarchyDispatch,
                    capabilities,
                )?;
                let occurrence = fact
                    .occurrence_id
                    .as_deref()
                    .and_then(|id| occurrences.get(id));
                match hierarchy {
                    HierarchyConstraint::DirectBase { .. }
                        if !matches!(
                            fact.relation,
                            CandidateRelation::Extends | CandidateRelation::Implements
                        ) || occurrence.is_none_or(|occurrence| {
                            occurrence.role != SemanticRole::BaseType
                                && !(fact.language == "rust"
                                    && occurrence.role == SemanticRole::TraitBound)
                        }) =>
                    {
                        return Err(invalid_fact(
                            &fact.id,
                            "direct-base hierarchy evidence requires an extends-or-implements/base-type occurrence",
                        ));
                    }
                    HierarchyConstraint::ReceiverDispatch {
                        receiver_qualified_name,
                        ..
                    } => {
                        if !matches!(
                            fact.relation,
                            CandidateRelation::Calls
                                | CandidateRelation::Constructs
                                | CandidateRelation::AccessesMember
                        ) {
                            return Err(invalid_fact(
                                &fact.id,
                                "receiver dispatch requires a call, construction, or member-access candidate",
                            ));
                        }
                        if receiver_qualified_name.is_empty() {
                            return Err(invalid_fact(
                                &fact.id,
                                "receiver dispatch identity is empty",
                            ));
                        }
                        if fact.constraints.qualified_name.is_some() {
                            return Err(invalid_fact(
                                &fact.id,
                                "receiver dispatch cannot also select a qualified target",
                            ));
                        }
                    }
                    HierarchyConstraint::RustAssociatedType {
                        receiver_declaration_id,
                        receiver_qualified_name,
                        trait_qualified_name,
                    } => {
                        if fact.language != "rust"
                            || !matches!(
                                fact.relation,
                                CandidateRelation::References
                                    | CandidateRelation::TypeOf
                                    | CandidateRelation::Returns
                            )
                            || occurrence
                                .is_none_or(|fact| fact.qualifier.as_deref() != Some("Self"))
                        {
                            return Err(invalid_fact(
                                &fact.id,
                                "Rust associated-type hierarchy evidence requires a Self-qualified type occurrence",
                            ));
                        }
                        if receiver_declaration_id.is_empty()
                            || receiver_qualified_name.is_empty()
                            || trait_qualified_name.is_empty()
                        {
                            return Err(invalid_fact(
                                &fact.id,
                                "Rust associated-type hierarchy identity is empty",
                            ));
                        }
                        require_reference(
                            &fact.id,
                            "Rust associated-type receiver declaration",
                            receiver_declaration_id,
                            declarations,
                        )?;
                        let receiver = declarations[receiver_declaration_id.as_str()];
                        if receiver.language != "rust"
                            || receiver.qualified_name != *receiver_qualified_name
                            || !matches!(receiver.kind.as_str(), "struct" | "enum" | "type_alias")
                        {
                            return Err(invalid_fact(
                                &fact.id,
                                "Rust associated-type receiver identity does not match its declaration",
                            ));
                        }
                        if fact.constraints.qualified_name.is_some()
                            || fact.constraints.exact_target_declaration_id.is_some()
                        {
                            return Err(invalid_fact(
                                &fact.id,
                                "Rust associated-type hierarchy evidence cannot also select an exact target",
                            ));
                        }
                    }
                    HierarchyConstraint::DirectBase { .. } => {}
                }
            }
            if fact.constraints.allowed_target_kinds.len()
                > limits.allowed_target_kinds_per_candidate
            {
                return Err(EvidenceError::new(
                    EvidenceErrorCode::ResourceLimit,
                    format!(
                        "candidate {:?} exceeds allowed target kind limit {}",
                        fact.id, limits.allowed_target_kinds_per_candidate
                    ),
                ));
            }
            if fact.constraints.allow_external
                && !capabilities.contains(&LanguageCapability::ExternalReferences)
            {
                return Err(EvidenceError::new(
                    EvidenceErrorCode::UndeclaredCapability,
                    format!(
                        "candidate {:?} allows external targets without external_references",
                        fact.id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn validate_binding_chains(bindings: &AHashMap<&str, &BindingFact>) -> Result<(), EvidenceError> {
    const MAX_BINDING_CHAIN_DEPTH: usize = 64;

    fn visit<'a>(
        id: &'a str,
        bindings: &AHashMap<&'a str, &'a BindingFact>,
        visiting: &mut BTreeSet<&'a str>,
        visited: &mut BTreeSet<&'a str>,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if visited.contains(id) {
            return Ok(());
        }
        if depth >= MAX_BINDING_CHAIN_DEPTH {
            return Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                format!("binding chain rooted at {id:?} exceeds depth limit"),
            ));
        }
        if !visiting.insert(id) {
            return Err(EvidenceError::new(
                EvidenceErrorCode::InvalidFact,
                format!("binding chain contains a cycle at {id:?}"),
            ));
        }
        if let Some(binding) = bindings.get(id) {
            for next in [
                binding.receiver_binding_id.as_deref(),
                binding.fallback_binding_id.as_deref(),
            ]
            .into_iter()
            .flatten()
            {
                visit(next, bindings, visiting, visited, depth.saturating_add(1))?;
            }
        }
        visiting.remove(id);
        visited.insert(id);
        Ok(())
    }

    let mut visited = BTreeSet::new();
    let mut ids = bindings.keys().copied().collect::<Vec<_>>();
    ids.sort_unstable();
    for id in ids {
        visit(id, bindings, &mut BTreeSet::new(), &mut visited, 0)?;
    }
    Ok(())
}

trait CallableTypeValue {
    fn value(&self) -> Option<&str>;
}

impl CallableTypeValue for String {
    fn value(&self) -> Option<&str> {
        Some(self)
    }
}

impl CallableTypeValue for Option<String> {
    fn value(&self) -> Option<&str> {
        self.as_deref()
    }
}

fn validate_callable_types<T: CallableTypeValue>(
    id: &str,
    types: &[T],
    count: Option<u32>,
    limits: EvidenceLimits,
) -> Result<(), EvidenceError> {
    if types.len() > limits.callable_types_per_fact {
        return Err(EvidenceError::new(
            EvidenceErrorCode::ResourceLimit,
            format!(
                "fact {id:?} exceeds callable type limit {}",
                limits.callable_types_per_fact
            ),
        ));
    }
    if !types.is_empty() && count != u32::try_from(types.len()).ok() {
        return Err(invalid_fact(
            id,
            "callable type count differs from the source-level arity",
        ));
    }
    if types.iter().any(|kind| {
        kind.value()
            .is_some_and(|kind| kind.is_empty() || kind.len() > limits.callable_type_bytes)
    }) {
        return Err(invalid_fact(
            id,
            "callable type identity is empty or too large",
        ));
    }
    Ok(())
}

fn validate_language(
    id: &str,
    language: &str,
    adapter_language: &str,
) -> Result<(), EvidenceError> {
    if language != adapter_language {
        return Err(EvidenceError::new(
            EvidenceErrorCode::LanguageMismatch,
            format!("fact {id:?} language {language:?} differs from adapter {adapter_language:?}"),
        ));
    }
    Ok(())
}

fn validate_range(
    range: &EvidenceRange,
    id: &str,
    allow_zero_width: bool,
) -> Result<(), EvidenceError> {
    let path = Path::new(&range.source_file);
    let path_is_safe = !range.source_file.is_empty()
        && !range.source_file.contains('\\')
        && !range.source_file.contains('\0')
        && !range.source_file.contains(':')
        && !path.is_absolute()
        && range
            .source_file
            .split('/')
            .all(|component| !matches!(component, "" | "." | ".."))
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !path_is_safe {
        return Err(EvidenceError::new(
            EvidenceErrorCode::InvalidPath,
            format!("fact {id:?} has unsafe source path {:?}", range.source_file),
        ));
    }
    let non_empty_position = range.start_byte < range.end_byte
        && (range.end_line != range.start_line || range.end_column > range.start_column);
    let empty_position = allow_zero_width
        && range.start_byte == range.end_byte
        && range.start_line == range.end_line
        && range.start_column == range.end_column;
    let position_is_valid = (non_empty_position || empty_position)
        && range.start_line > 0
        && range.end_line >= range.start_line;
    if !position_is_valid {
        return Err(EvidenceError::new(
            EvidenceErrorCode::InvalidRange,
            format!("fact {id:?} has a zero-width or reversed source range"),
        ));
    }
    Ok(())
}

fn require_capability(
    id: &str,
    required: LanguageCapability,
    capabilities: &BTreeSet<LanguageCapability>,
) -> Result<(), EvidenceError> {
    if !capabilities.contains(&required) {
        return Err(EvidenceError::new(
            EvidenceErrorCode::UndeclaredCapability,
            format!("fact {id:?} requires capability {required:?}"),
        ));
    }
    Ok(())
}

fn require_reference<T>(
    owner_id: &str,
    kind: &str,
    target_id: &str,
    index: &AHashMap<&str, &T>,
) -> Result<(), EvidenceError> {
    if target_id.is_empty() || !index.contains_key(target_id) {
        return Err(EvidenceError::new(
            EvidenceErrorCode::MissingReference,
            format!("fact {owner_id:?} references missing {kind} {target_id:?}"),
        ));
    }
    Ok(())
}

fn require_optional_reference<T>(
    owner_id: &str,
    kind: &str,
    target_id: Option<&str>,
    index: &AHashMap<&str, &T>,
) -> Result<(), EvidenceError> {
    if let Some(target_id) = target_id {
        require_reference(owner_id, kind, target_id, index)?;
    }
    Ok(())
}

fn invalid_fact(id: &str, detail: &str) -> EvidenceError {
    EvidenceError::new(
        EvidenceErrorCode::InvalidFact,
        format!("fact {id:?} {detail}"),
    )
}
