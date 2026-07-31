use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use super::model::{
    BindingFact, DeclarationFact, EvidenceRange, LanguageCapability, OccurrenceFact,
    RelationshipCandidate, ScopeFact, SemanticEvidenceBatch,
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

    let declarations: BTreeMap<_, _> = batch
        .declarations
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    let scopes: BTreeMap<_, _> = batch
        .scopes
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    let bindings: BTreeMap<_, _> = batch
        .bindings
        .iter()
        .map(|fact| (fact.id.as_str(), fact))
        .collect();
    let occurrences: BTreeMap<_, _> = batch
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
    declarations: &BTreeMap<&str, &DeclarationFact>,
    scopes: &BTreeMap<&str, &ScopeFact>,
    bindings: &BTreeMap<&str, &BindingFact>,
    occurrences: &BTreeMap<&str, &OccurrenceFact>,
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
    index: &BTreeMap<&str, &T>,
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
    index: &BTreeMap<&str, &T>,
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
