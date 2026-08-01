use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use ahash::AHashMap;
use compass_languages::{
    BindingFact, CandidateRelation, DeclarationFact, EvidenceLimits, HierarchyConstraint,
    OccurrenceFact, ReceiverDispatchStrategy, RelationshipCandidate, SemanticEvidenceBatch,
    make_id, validate_evidence,
};
use compass_model::provenance::OCCURRENCE_RULE_ATTRIBUTE;
use serde_json::{Map, Value};

use compass_languages::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UniversalResolutionLimits {
    pub declarations: usize,
    pub bindings: usize,
    pub occurrences: usize,
    pub candidates: usize,
    pub candidates_per_lookup: usize,
}

impl Default for UniversalResolutionLimits {
    fn default() -> Self {
        Self {
            declarations: 1_000_000,
            bindings: 1_000_000,
            occurrences: 5_000_000,
            candidates: 5_000_000,
            candidates_per_lookup: 256,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionRule {
    ExactSourceDeclaration,
    ExactLexicalDeclaration,
    ExplicitBinding,
    MemberBinding,
    DeferredReceiver,
    WildcardBinding,
    UniqueModuleOrPackage,
    ExactHierarchyBase,
    DirectReceiverSuccessorDispatch,
    LinearizedReceiverDispatch,
    ExactSourceInventory,
    QualifiedExternal,
}

#[derive(Clone, Debug)]
struct DirectBaseLink {
    qualified_name: Option<String>,
    source_file: String,
    start_byte: u64,
    end_byte: u64,
    candidate_id: String,
}

#[derive(Clone, Debug, Default)]
struct DirectBaseSet {
    links: Vec<DirectBaseLink>,
    complete: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolutionEvidence {
    pub rule: ResolutionRule,
    pub candidate_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ResolutionDecision {
    Resolved {
        declaration_id: String,
        evidence: ResolutionEvidence,
    },
    ResolvedInventory {
        graph_node_id: String,
        evidence: ResolutionEvidence,
    },
    QualifiedExternal {
        qualified_name: String,
        evidence: ResolutionEvidence,
    },
    DeferredReceiver {
        qualified_name: String,
        evidence: ResolutionEvidence,
    },
    Ambiguous {
        candidate_count: usize,
    },
    Unresolved,
}

pub struct UniversalResolutionIndex {
    declarations: BTreeMap<String, DeclarationFact>,
    occurrences: BTreeMap<String, OccurrenceFact>,
    bindings: BTreeMap<String, compass_languages::BindingFact>,
    candidates: BTreeMap<String, RelationshipCandidate>,
    scopes: BTreeMap<String, compass_languages::ScopeFact>,
    by_qualified: AHashMap<(String, String), Vec<String>>,
    by_module_name: AHashMap<(String, String, String), Vec<String>>,
    by_scope_name: AHashMap<(String, String, String), Vec<String>>,
    by_source_directory_name: AHashMap<(String, String, String), Vec<String>>,
    direct_bases: AHashMap<(String, String), DirectBaseSet>,
    members_by_owner: AHashMap<(String, String, String), Vec<String>>,
    inventory_by_qualified: AHashMap<(String, String), Vec<String>>,
    aliases: AHashMap<(String, String), Vec<String>>,
    wildcard_reexports_by_module: AHashMap<(String, String), Vec<String>>,
    members: AHashMap<(String, String, String), Vec<String>>,
    limits: UniversalResolutionLimits,
}

impl UniversalResolutionIndex {
    pub fn new(
        batches: &[SemanticEvidenceBatch],
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        Self::new_with_inventory(batches, &[], Path::new("."), limits)
    }

    pub fn new_with_inventory(
        batches: &[SemanticEvidenceBatch],
        inventory_nodes: &[NodeRecord],
        root: &Path,
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
        let mut profile_started = Instant::now();
        let mut declarations = BTreeMap::new();
        let mut occurrences = BTreeMap::new();
        let mut bindings = BTreeMap::new();
        let mut candidates = BTreeMap::new();
        let mut scopes = BTreeMap::new();
        for batch in batches {
            validate_evidence(batch, EvidenceLimits::default())
                .map_err(|error| format!("invalid universal evidence: {error}"))?;
            for fact in &batch.declarations {
                insert_unique(&mut declarations, fact.id.clone(), fact.clone())?;
            }
            for fact in &batch.occurrences {
                insert_unique(&mut occurrences, fact.id.clone(), fact.clone())?;
            }
            for fact in &batch.bindings {
                insert_unique(&mut bindings, fact.id.clone(), fact.clone())?;
            }
            for fact in &batch.candidates {
                insert_unique(&mut candidates, fact.id.clone(), fact.clone())?;
            }
            for fact in &batch.scopes {
                insert_unique(&mut scopes, fact.id.clone(), fact.clone())?;
            }
        }
        profile_internal(
            "universal validation and fact collection",
            &mut profile_started,
        );
        for (name, count, limit) in [
            ("declarations", declarations.len(), limits.declarations),
            ("bindings", bindings.len(), limits.bindings),
            ("occurrences", occurrences.len(), limits.occurrences),
            ("candidates", candidates.len(), limits.candidates),
        ] {
            if count > limit {
                return Err(format!(
                    "universal {name} count {count} exceeds limit {limit}"
                ));
            }
        }
        let mut by_qualified = AHashMap::<_, Vec<_>>::new();
        let mut by_module_name = AHashMap::<_, Vec<_>>::new();
        let mut by_scope_name = AHashMap::<_, Vec<_>>::new();
        let mut by_source_directory_name = AHashMap::<_, Vec<_>>::new();
        for declaration in declarations.values() {
            by_qualified
                .entry((
                    declaration.language.clone(),
                    declaration.qualified_name.clone(),
                ))
                .or_default()
                .push(declaration.id.clone());
            if let Some(module) = declaration.module_or_package.as_ref() {
                by_module_name
                    .entry((
                        declaration.language.clone(),
                        module.clone(),
                        declaration.name.clone(),
                    ))
                    .or_default()
                    .push(declaration.id.clone());
            }
            if let Some(scope) = declaration.scope_id.as_ref() {
                by_scope_name
                    .entry((
                        declaration.language.clone(),
                        scope.clone(),
                        declaration.name.clone(),
                    ))
                    .or_default()
                    .push(declaration.id.clone());
            }
            if let Some(directory) = source_directory(&declaration.range.source_file, root) {
                by_source_directory_name
                    .entry((
                        declaration.language.clone(),
                        directory,
                        declaration.name.clone(),
                    ))
                    .or_default()
                    .push(declaration.id.clone());
            }
        }
        for values in by_qualified
            .values_mut()
            .chain(by_module_name.values_mut())
            .chain(by_scope_name.values_mut())
            .chain(by_source_directory_name.values_mut())
        {
            values.sort_unstable();
            values.dedup();
            if values.len() > limits.candidates_per_lookup {
                values.truncate(limits.candidates_per_lookup);
            }
        }
        profile_internal("universal declaration indices", &mut profile_started);
        let mut inventory_by_qualified = AHashMap::<_, Vec<_>>::new();
        for node in inventory_nodes {
            if node.string("symbol_kind") != "file" || node.string("source_file").is_empty() {
                continue;
            }
            let language = node.string("language");
            let qualified = match language.as_str() {
                "python" => python_module_name(&node.string("source_file"), root),
                "go" => {
                    let package = node.string("package");
                    (!package.is_empty()).then_some(package)
                }
                _ => None,
            };
            if let Some(qualified) = qualified {
                inventory_by_qualified
                    .entry((language, qualified))
                    .or_default()
                    .push(node.id.clone());
            }
        }
        for values in inventory_by_qualified.values_mut() {
            values.sort_unstable();
            values.dedup();
            if values.len() > limits.candidates_per_lookup {
                values.truncate(limits.candidates_per_lookup);
            }
        }
        profile_internal("universal source inventory index", &mut profile_started);
        let mut aliases = AHashMap::<_, Vec<_>>::new();
        for binding in bindings.values() {
            let Some(owner) = binding
                .scope_id
                .as_deref()
                .and_then(|id| scopes.get(id))
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            else {
                continue;
            };
            for separator in [".", "::"] {
                aliases
                    .entry((
                        binding.language.clone(),
                        format!("{}{separator}{}", owner.qualified_name, binding.spelling),
                    ))
                    .or_default()
                    .push(binding.qualified_target.clone());
            }
        }
        for targets in aliases.values_mut() {
            targets.sort_unstable();
            targets.dedup();
        }
        profile_internal("universal alias index", &mut profile_started);
        let mut direct_bases = AHashMap::<(String, String), DirectBaseSet>::new();
        for candidate in candidates.values() {
            let Some(HierarchyConstraint::DirectBase { base_set_complete }) =
                candidate.constraints.hierarchy.as_ref()
            else {
                continue;
            };
            let Some(owner) = declarations.get(&candidate.source_declaration_id) else {
                continue;
            };
            let range = candidate
                .occurrence_id
                .as_deref()
                .and_then(|id| occurrences.get(id))
                .map(|occurrence| &occurrence.range);
            let entry = direct_bases
                .entry((candidate.language.clone(), owner.qualified_name.clone()))
                .or_insert_with(|| DirectBaseSet {
                    links: Vec::new(),
                    complete: true,
                });
            entry.complete &= *base_set_complete;
            if entry.links.len() <= limits.candidates_per_lookup {
                entry.links.push(DirectBaseLink {
                    qualified_name: candidate.constraints.qualified_name.clone(),
                    source_file: range.map_or_else(String::new, |range| range.source_file.clone()),
                    start_byte: range.map_or(u64::MAX, |range| range.start_byte),
                    end_byte: range.map_or(u64::MAX, |range| range.end_byte),
                    candidate_id: candidate.id.clone(),
                });
            } else {
                entry.complete = false;
            }
        }
        for bases in direct_bases.values_mut() {
            bases.links.sort_unstable_by(|left, right| {
                left.source_file
                    .cmp(&right.source_file)
                    .then_with(|| left.start_byte.cmp(&right.start_byte))
                    .then_with(|| left.end_byte.cmp(&right.end_byte))
                    .then_with(|| left.candidate_id.cmp(&right.candidate_id))
            });
            if bases.links.len() > limits.candidates_per_lookup {
                bases.complete = false;
                bases.links.truncate(limits.candidates_per_lookup);
            }
        }
        let mut members_by_owner = AHashMap::<_, Vec<_>>::new();
        for declaration in declarations.values() {
            let Some(owner) = declaration
                .scope_id
                .as_deref()
                .and_then(|id| scopes.get(id))
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            else {
                continue;
            };
            members_by_owner
                .entry((
                    declaration.language.clone(),
                    owner.qualified_name.clone(),
                    declaration.name.clone(),
                ))
                .or_default()
                .push(declaration.id.clone());
        }
        for members in members_by_owner.values_mut() {
            members.sort_unstable();
            members.dedup();
            if members.len() > limits.candidates_per_lookup {
                members.truncate(limits.candidates_per_lookup);
            }
        }
        profile_internal("universal hierarchy indices", &mut profile_started);
        let mut wildcard_reexports_by_module = AHashMap::<_, Vec<_>>::new();
        for binding in bindings.values().filter(|binding| binding.spelling == "*") {
            let Some(scope_id) = binding.scope_id.as_ref() else {
                continue;
            };
            if binding.kind == compass_languages::BindingKind::Reexport
                && let Some(owner) = scopes
                    .get(scope_id)
                    .and_then(|scope| scope.owner_declaration_id.as_deref())
                    .and_then(|id| declarations.get(id))
            {
                wildcard_reexports_by_module
                    .entry((binding.language.clone(), owner.qualified_name.clone()))
                    .or_default()
                    .push(binding.qualified_target.clone());
            }
        }
        for values in wildcard_reexports_by_module.values_mut() {
            values.sort_unstable();
            values.dedup();
            if values.len() > limits.candidates_per_lookup {
                values.truncate(limits.candidates_per_lookup);
            }
        }
        profile_internal("universal wildcard index", &mut profile_started);
        let mut members = AHashMap::<_, Vec<_>>::new();
        for binding in bindings
            .values()
            .filter(|binding| binding.kind == compass_languages::BindingKind::Member)
        {
            let Some(owner) = binding
                .scope_id
                .as_deref()
                .and_then(|id| scopes.get(id))
                .and_then(|scope| scope.owner_declaration_id.as_deref())
                .and_then(|id| declarations.get(id))
            else {
                continue;
            };
            members
                .entry((
                    binding.language.clone(),
                    owner.qualified_name.clone(),
                    binding.spelling.clone(),
                ))
                .or_default()
                .push(binding.qualified_target.clone());
        }
        for targets in members.values_mut() {
            targets.sort_unstable();
            targets.dedup();
        }
        profile_internal("universal member index", &mut profile_started);
        Ok(Self {
            declarations,
            occurrences,
            bindings,
            candidates,
            scopes,
            by_qualified,
            by_module_name,
            by_scope_name,
            by_source_directory_name,
            direct_bases,
            members_by_owner,
            inventory_by_qualified,
            aliases,
            wildcard_reexports_by_module,
            members,
            limits,
        })
    }

    #[must_use]
    pub fn candidate_ids(&self) -> Vec<&str> {
        let mut ordered = self
            .candidates
            .iter()
            .map(|(id, candidate)| {
                let range = self
                    .occurrence(candidate)
                    .map(|occurrence| &occurrence.range)
                    .or_else(|| {
                        self.declarations
                            .get(&candidate.source_declaration_id)
                            .map(|declaration| &declaration.range)
                    });
                (id.as_str(), range)
            })
            .collect::<Vec<_>>();
        ordered.sort_unstable_by(|(left_id, left_range), (right_id, right_range)| {
            left_range
                .map(|range| range.source_file.as_str())
                .unwrap_or_default()
                .cmp(
                    right_range
                        .map(|range| range.source_file.as_str())
                        .unwrap_or_default(),
                )
                .then_with(|| {
                    left_range
                        .map_or(u64::MAX, |range| range.start_byte)
                        .cmp(&right_range.map_or(u64::MAX, |range| range.start_byte))
                })
                .then_with(|| {
                    left_range
                        .map_or(u64::MAX, |range| range.end_byte)
                        .cmp(&right_range.map_or(u64::MAX, |range| range.end_byte))
                })
                .then_with(|| left_id.cmp(right_id))
        });
        ordered.into_iter().map(|(id, _)| id).collect()
    }

    #[must_use]
    pub fn resolve(&self, candidate_id: &str) -> ResolutionDecision {
        let Some(candidate) = self.candidates.get(candidate_id) else {
            return ResolutionDecision::Unresolved;
        };
        let language = candidate
            .constraints
            .exact_language
            .as_deref()
            .unwrap_or(&candidate.language);
        if let Some(target) = candidate.constraints.exact_target_declaration_id.as_ref()
            && self.declaration_allowed(target, candidate)
        {
            return ResolutionDecision::Resolved {
                declaration_id: target.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExactSourceDeclaration,
                    candidate_count: 1,
                },
            };
        }
        if let Some(HierarchyConstraint::ReceiverDispatch {
            receiver_qualified_name,
            strategy,
        }) = candidate.constraints.hierarchy.as_ref()
        {
            return self.resolve_c3_receiver_dispatch(
                language,
                receiver_qualified_name,
                *strategy,
                candidate,
            );
        }
        let occurrence = self.occurrence(candidate);
        let qualifier = occurrence.and_then(|occurrence| occurrence.qualifier.as_deref());
        let has_unbound_qualified_receiver = qualifier.is_some_and(|qualifier| {
            candidate.binding_id.is_none()
                && !matches!((language, qualifier), ("python", "self" | "cls"))
        });
        let allows_lexical_lookup = qualifier.is_none()
            || qualifier.is_some_and(|qualifier| {
                candidate.binding_id.is_none()
                    && matches!((language, qualifier), ("python", "self" | "cls"))
            });

        if let Some(decision) = self.resolve_explicit_binding(language, candidate) {
            return decision;
        }
        if matches!(
            candidate.constraints.hierarchy.as_ref(),
            Some(HierarchyConstraint::DirectBase { .. })
        ) {
            return self.resolve_direct_base(language, candidate);
        }

        if matches!(
            candidate.relation,
            CandidateRelation::Contains | CandidateRelation::Owns
        ) && let Some(qualified) = candidate.constraints.qualified_name.as_ref()
        {
            let qualified = match self.follow_alias(language, qualified) {
                Ok(qualified) => qualified,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
            if let Some(decision) = self.unique_decision(
                self.by_qualified
                    .get(&(language.to_owned(), qualified.clone())),
                candidate,
                ResolutionRule::ExplicitBinding,
            ) {
                return decision;
            }
        }

        if allows_lexical_lookup && let Some(scope) = candidate.constraints.scope_id.as_deref() {
            let mut cursor = Some(scope);
            let mut visited = BTreeSet::new();
            while let Some(scope) = cursor.filter(|scope| visited.insert((*scope).to_owned())) {
                let key = (
                    language.to_owned(),
                    scope.to_owned(),
                    candidate.target_spelling.clone(),
                );
                if let Some(decision) = self.unique_decision(
                    self.by_scope_name.get(&key),
                    candidate,
                    ResolutionRule::ExactLexicalDeclaration,
                ) {
                    return decision;
                }
                cursor = self
                    .scopes
                    .get(scope)
                    .and_then(|scope| scope.parent_scope_id.as_deref());
            }
        }

        if let Some(qualified) = candidate.constraints.qualified_name.as_ref() {
            let qualified = match self.follow_alias(language, qualified) {
                Ok(qualified) => qualified,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
            let key = (language.to_owned(), qualified.clone());
            if let Some(decision) = self.unique_decision(
                self.by_qualified.get(&key),
                candidate,
                ResolutionRule::ExplicitBinding,
            ) {
                return decision;
            }
            if let Some(decision) = self.member_decision(language, &qualified, candidate) {
                return decision;
            }
            if let Some(decision) = self.inventory_decision(language, &qualified, candidate) {
                return decision;
            }
        }

        if !has_unbound_qualified_receiver
            && let Some(module) = candidate.constraints.module_or_package.as_ref()
        {
            let key = (
                language.to_owned(),
                module.clone(),
                candidate.target_spelling.clone(),
            );
            if let Some(decision) = self.unique_decision(
                self.by_module_name.get(&key),
                candidate,
                ResolutionRule::UniqueModuleOrPackage,
            ) {
                return decision;
            }
        }

        if let Some(decision) = self.resolve_wildcard_binding(language, candidate) {
            return decision;
        }

        if candidate.constraints.allow_external
            && let Some(qualified_name) = candidate.constraints.qualified_name.clone()
        {
            return ResolutionDecision::QualifiedExternal {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::QualifiedExternal,
                    candidate_count: 0,
                },
            };
        }
        if matches!(
            candidate.relation,
            CandidateRelation::Calls | CandidateRelation::IndirectCalls | CandidateRelation::Tests
        ) && let Some(qualified_name) = candidate.constraints.qualified_name.clone()
            && qualifier.is_some_and(is_deferred_receiver)
        {
            return ResolutionDecision::DeferredReceiver {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::DeferredReceiver,
                    candidate_count: 0,
                },
            };
        }
        ResolutionDecision::Unresolved
    }

    fn resolve_wildcard_binding(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) {
            return None;
        }
        let binding = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))
            .filter(|binding| binding.spelling == "*")?;
        let qualifier = self
            .occurrence(candidate)
            .and_then(|occurrence| occurrence.qualifier.as_deref());
        let mut modules = vec![binding.qualified_target.clone()];
        let mut visited = BTreeSet::new();
        let mut declarations = BTreeSet::new();
        for _ in 0..64 {
            let Some(module) = modules.pop() else {
                break;
            };
            if !visited.insert(module.clone()) {
                continue;
            }
            if qualifier.is_none()
                && let Some(ids) = self.by_module_name.get(&(
                    language.to_owned(),
                    module.clone(),
                    candidate.target_spelling.clone(),
                ))
            {
                declarations.extend(
                    ids.iter()
                        .filter(|id| self.declaration_allowed(id, candidate))
                        .cloned(),
                );
            }
            for qualified in
                wildcard_qualified_names(&module, qualifier, &candidate.target_spelling)
            {
                let qualified = match self.follow_alias(language, &qualified) {
                    Ok(qualified) => qualified,
                    Err(candidate_count) => {
                        return Some(ResolutionDecision::Ambiguous { candidate_count });
                    }
                };
                if let Some(ids) = self
                    .by_qualified
                    .get(&(language.to_owned(), qualified.clone()))
                {
                    declarations.extend(
                        ids.iter()
                            .filter(|id| self.declaration_allowed(id, candidate))
                            .cloned(),
                    );
                }
                if let Some(ids) = self.member_declarations(language, &qualified, candidate) {
                    declarations.extend(ids);
                }
            }
            if declarations.len() > self.limits.candidates_per_lookup {
                return Some(ResolutionDecision::Ambiguous {
                    candidate_count: declarations.len(),
                });
            }
            if let Some(reexports) = self
                .wildcard_reexports_by_module
                .get(&(language.to_owned(), module))
            {
                modules.extend(reexports.iter().cloned());
            }
        }
        match declarations.into_iter().collect::<Vec<_>>().as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: only.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::WildcardBinding,
                    candidate_count: 1,
                },
            }),
            [] if !self.binding_target_is_internal(binding) => {
                Some(ResolutionDecision::QualifiedExternal {
                    qualified_name: wildcard_qualified_names(
                        &binding.qualified_target,
                        qualifier,
                        &candidate.target_spelling,
                    )
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| candidate.target_spelling.clone()),
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::QualifiedExternal,
                        candidate_count: 0,
                    },
                })
            }
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn binding_target_is_internal(&self, binding: &BindingFact) -> bool {
        let Some(owner) = binding
            .scope_id
            .as_deref()
            .and_then(|scope_id| self.scopes.get(scope_id))
            .and_then(|scope| scope.owner_declaration_id.as_deref())
            .and_then(|declaration_id| self.declarations.get(declaration_id))
        else {
            return false;
        };
        qualified_root(&owner.qualified_name) == qualified_root(&binding.qualified_target)
    }

    fn resolve_explicit_binding(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let binding = candidate
            .binding_id
            .as_deref()
            .and_then(|binding_id| self.bindings.get(binding_id))?;
        let qualified_occurrence = self
            .occurrence(candidate)
            .is_some_and(|occurrence| occurrence.qualifier.is_some());
        if !qualified_occurrence
            && let Some(target) = binding.target_declaration_id.as_ref()
            && self.declaration_allowed(target, candidate)
        {
            return Some(ResolutionDecision::Resolved {
                declaration_id: target.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExplicitBinding,
                    candidate_count: 1,
                },
            });
        }
        match self.bound_member_target(language, binding, candidate) {
            Ok(Some(qualified)) => {
                let key = (language.to_owned(), qualified.clone());
                if let Some(decision) = self.unique_decision(
                    self.by_qualified.get(&key),
                    candidate,
                    ResolutionRule::MemberBinding,
                ) {
                    return Some(decision);
                }
                return Some(ResolutionDecision::QualifiedExternal {
                    qualified_name: qualified,
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::QualifiedExternal,
                        candidate_count: 0,
                    },
                });
            }
            Ok(None) => {}
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        }
        let binding_lookup = if qualified_occurrence
            || matches!(
                candidate.relation,
                CandidateRelation::Imports | CandidateRelation::Reexports
            ) {
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .unwrap_or(&binding.qualified_target)
        } else {
            &binding.qualified_target
        };
        let qualified = match self.follow_alias(language, binding_lookup) {
            Ok(qualified) => qualified,
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        };
        let key = (language.to_owned(), qualified.clone());
        let qualified_rule = if language == "rust"
            && qualified_occurrence
            && matches!(
                candidate.relation,
                CandidateRelation::Calls
                    | CandidateRelation::IndirectCalls
                    | CandidateRelation::Constructs
                    | CandidateRelation::AccessesMember
            ) {
            ResolutionRule::MemberBinding
        } else {
            ResolutionRule::ExplicitBinding
        };
        if let Some(decision) =
            self.unique_decision(self.by_qualified.get(&key), candidate, qualified_rule)
        {
            return Some(decision);
        }
        if let Some(decision) = self.member_decision(language, &qualified, candidate) {
            return Some(decision);
        }
        if let Some(decision) = self.inventory_decision(language, &qualified, candidate) {
            return Some(decision);
        }
        let imported = self.imported_declarations(
            language,
            &binding.qualified_target,
            &candidate.target_spelling,
        );
        (!imported.is_empty())
            .then(|| {
                self.unique_decision(Some(&imported), candidate, ResolutionRule::ExplicitBinding)
            })
            .flatten()
    }

    pub fn materialize(&self, nodes: &mut Vec<NodeRecord>, edges: &mut Vec<EdgeRecord>) {
        let mut profile_started = Instant::now();
        let overloads = declaration_overloads(self.declarations.values());
        let graph_ids = materialized_declaration_ids(self.declarations.values());
        let existing_positions = nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let mut existing_nodes = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        for declaration in self.declarations.values() {
            let graph_node_id = &graph_ids[&declaration.id];
            if let Some(index) = existing_positions.get(graph_node_id) {
                project_declaration_onto_node(&mut nodes[*index], declaration, graph_node_id);
                if let Some(discriminator) = overloads.get(&declaration.id) {
                    nodes[*index].attributes.insert(
                        "overload_discriminator".to_owned(),
                        Value::String(discriminator.clone()),
                    );
                }
            } else if existing_nodes.insert(graph_node_id.clone()) {
                let mut node = declaration_node(declaration, graph_node_id);
                if let Some(discriminator) = overloads.get(&declaration.id) {
                    node.attributes.insert(
                        "overload_discriminator".to_owned(),
                        Value::String(discriminator.clone()),
                    );
                }
                nodes.push(node);
            }
        }
        profile_internal("universal declaration projection", &mut profile_started);
        let inventory_kinds = nodes
            .iter()
            .map(|node| (node.id.clone(), node.string("symbol_kind")))
            .collect::<BTreeMap<_, _>>();
        let mut external_positions = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.attributes.get("external").and_then(Value::as_bool) == Some(true)
            })
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<BTreeMap<_, _>>();
        let mut emitted_edges = BTreeSet::new();
        let candidate_ids = self.candidate_ids();
        profile_internal("universal candidate ordering", &mut profile_started);
        for candidate_id in candidate_ids {
            let candidate = &self.candidates[candidate_id];
            let Some(source) = self
                .declarations
                .get(&candidate.source_declaration_id)
                .map(|declaration| graph_ids[&declaration.id].clone())
            else {
                continue;
            };
            let (target, resolution_rule, target_kind, target_site) = match self
                .resolve(candidate_id)
            {
                ResolutionDecision::Resolved {
                    declaration_id,
                    evidence,
                } => {
                    let Some(target) = self.declarations.get(&declaration_id) else {
                        continue;
                    };
                    (
                        graph_ids[&target.id].clone(),
                        evidence.rule,
                        Some(target.kind.clone()),
                        Some(&target.range),
                    )
                }
                ResolutionDecision::ResolvedInventory {
                    graph_node_id,
                    evidence,
                } => {
                    let kind = inventory_kinds.get(&graph_node_id).cloned();
                    (graph_node_id, evidence.rule, kind, None)
                }
                ResolutionDecision::QualifiedExternal {
                    qualified_name,
                    evidence,
                } => {
                    let kind = external_kind(candidate);
                    let id = make_id(&["external", &candidate.language, &qualified_name]);
                    if let Some(position) = external_positions.get(&id).copied() {
                        merge_external_node(&mut nodes[position], candidate);
                    } else if !existing_nodes.contains(&id) {
                        let position = nodes.len();
                        nodes.push(external_node(
                            &id,
                            &qualified_name,
                            &candidate.language,
                            candidate,
                        ));
                        existing_nodes.insert(id.clone());
                        external_positions.insert(id.clone(), position);
                    }
                    let projected_kind = external_positions
                        .get(&id)
                        .map(|position| nodes[*position].string("symbol_kind"))
                        .filter(|kind| !kind.is_empty())
                        .unwrap_or_else(|| kind.to_owned());
                    (id, evidence.rule, Some(projected_kind), None)
                }
                ResolutionDecision::DeferredReceiver {
                    qualified_name,
                    evidence,
                } => {
                    let id = make_id(&["deferred", &candidate.language, &qualified_name]);
                    if existing_nodes.insert(id.clone()) {
                        nodes.push(deferred_receiver_node(
                            &id,
                            &qualified_name,
                            &candidate.language,
                            candidate,
                        ));
                    }
                    (
                        id,
                        evidence.rule,
                        Some(external_kind(candidate).to_owned()),
                        None,
                    )
                }
                ResolutionDecision::Ambiguous { .. } | ResolutionDecision::Unresolved => continue,
            };
            let (source, target) = if candidate.relation == CandidateRelation::Contains {
                (source, target)
            } else if self.occurrence(candidate).is_some_and(|occurrence| {
                occurrence.role == compass_languages::SemanticRole::Receiver
            }) {
                (target, source)
            } else {
                (source, target)
            };
            let exact_target = candidate
                .constraints
                .exact_target_declaration_id
                .as_deref()
                .and_then(|id| self.declarations.get(id));
            let relation = if self.occurrence(candidate).is_some_and(|occurrence| {
                occurrence.role == compass_languages::SemanticRole::Receiver
            }) || (candidate.relation == CandidateRelation::Contains
                && exact_target.is_some_and(|target| target.kind == "method"))
            {
                "method"
            } else {
                relation_name(candidate.relation)
            };
            let site = self
                .occurrence(candidate)
                .map(|occurrence| &occurrence.range)
                .or_else(|| exact_target.map(|target| &target.range))
                .or_else(|| {
                    matches!(
                        candidate.relation,
                        CandidateRelation::Contains | CandidateRelation::Owns
                    )
                    .then_some(target_site)
                    .flatten()
                })
                .or_else(|| {
                    self.declarations
                        .get(&candidate.source_declaration_id)
                        .map(|declaration| &declaration.range)
                });
            let Some(site) = site else { continue };
            let key = (
                source.clone(),
                target.clone(),
                relation.to_owned(),
                site.source_file.clone(),
                site.start_byte,
                site.end_byte,
                candidate.id.clone(),
            );
            if !emitted_edges.insert(key) || source == target {
                continue;
            }
            edges.push(materialized_edge(
                source,
                target,
                relation,
                candidate,
                self.occurrence(candidate),
                candidate
                    .binding_id
                    .as_deref()
                    .and_then(|binding_id| self.bindings.get(binding_id)),
                target_kind.as_deref(),
                target_site.map(|range| range.source_file.as_str()),
                site,
                resolution_rule,
                &candidate.language,
            ));
        }
        profile_internal("universal candidate resolution", &mut profile_started);
    }

    fn resolve_c3_receiver_dispatch(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        strategy: ReceiverDispatchStrategy,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        match strategy {
            ReceiverDispatchStrategy::C3FromReceiver => {
                if let Some(decision) =
                    self.resolve_exact_receiver_member(language, receiver_qualified_name, candidate)
                {
                    return decision;
                }
                // A direct member on the first base is source-proven even
                // when a later external base prevents construction of the
                // complete linearization. The same remains true through a
                // chain of single inheritance: no sibling can precede the
                // next class until the chain reaches a multiple-base fork.
                if let Some(decision) = self.resolve_source_proven_receiver_prefix(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
            }
            ReceiverDispatchStrategy::C3AfterReceiver => {
                if let Some(decision) = self.resolve_direct_receiver_successor(
                    language,
                    receiver_qualified_name,
                    candidate,
                ) {
                    return decision;
                }
            }
        }
        let mut memo = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        let Ok(linearization) = self.c3_linearization(
            language,
            receiver_qualified_name,
            &mut memo,
            &mut visiting,
            0,
        ) else {
            return ResolutionDecision::Unresolved;
        };
        if linearization.first().map(String::as_str) != Some(receiver_qualified_name) {
            return ResolutionDecision::Unresolved;
        }
        for owner in linearization.iter().skip(1) {
            let key = (
                language.to_owned(),
                owner.clone(),
                candidate.target_spelling.clone(),
            );
            let Some(members) = self.members_by_owner.get(&key) else {
                continue;
            };
            let eligible = members
                .iter()
                .filter(|id| self.declaration_allowed(id, candidate))
                .take(self.limits.candidates_per_lookup.saturating_add(1))
                .cloned()
                .collect::<Vec<_>>();
            match eligible.as_slice() {
                [only] => {
                    return ResolutionDecision::Resolved {
                        declaration_id: only.clone(),
                        evidence: ResolutionEvidence {
                            rule: ResolutionRule::LinearizedReceiverDispatch,
                            candidate_count: 1,
                        },
                    };
                }
                [] => {}
                many => {
                    return ResolutionDecision::Ambiguous {
                        candidate_count: many.len(),
                    };
                }
            }
        }
        ResolutionDecision::Unresolved
    }

    fn resolve_source_proven_receiver_prefix(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let mut receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let mut visited = BTreeSet::new();
        for _ in 0..self.limits.candidates_per_lookup {
            if !visited.insert(receiver.clone()) {
                return None;
            }
            let base_set = self
                .direct_bases
                .get(&(language.to_owned(), receiver.clone()))?;
            if !base_set.complete
                || base_set.links.is_empty()
                || base_set.links.len() > self.limits.candidates_per_lookup
            {
                return None;
            }
            let first_base = base_set.links[0]
                .qualified_name
                .as_deref()
                .and_then(|name| self.exact_hierarchy_type(language, name))?;
            if let Some(decision) =
                self.resolve_exact_receiver_member(language, &first_base, candidate)
            {
                return Some(decision);
            }
            if base_set.links.len() != 1 {
                return None;
            }
            receiver = first_base;
        }
        None
    }

    fn resolve_exact_receiver_member(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let members = self.members_by_owner.get(&(
            language.to_owned(),
            receiver,
            candidate.target_spelling.clone(),
        ))?;
        let eligible = members
            .iter()
            .filter(|id| self.declaration_allowed(id, candidate))
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .cloned()
            .collect::<Vec<_>>();
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: only.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::LinearizedReceiverDispatch,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn resolve_direct_base(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        let Some(qualified_name) = candidate.constraints.qualified_name.as_deref() else {
            return ResolutionDecision::Unresolved;
        };
        let qualified_name = match self.follow_alias(language, qualified_name) {
            Ok(qualified_name) => qualified_name,
            Err(candidate_count) => {
                return ResolutionDecision::Ambiguous { candidate_count };
            }
        };
        let key = (language.to_owned(), qualified_name.clone());
        if let Some(decision) = self.unique_decision(
            self.by_qualified.get(&key),
            candidate,
            ResolutionRule::ExactHierarchyBase,
        ) {
            return decision;
        }
        if candidate.constraints.allow_external {
            return ResolutionDecision::QualifiedExternal {
                qualified_name,
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::QualifiedExternal,
                    candidate_count: 0,
                },
            };
        }
        ResolutionDecision::Unresolved
    }

    fn resolve_direct_receiver_successor(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let receiver = self.exact_hierarchy_type(language, receiver_qualified_name)?;
        let base_set = self.direct_bases.get(&(language.to_owned(), receiver))?;
        if !base_set.complete
            || base_set.links.is_empty()
            || base_set.links.len() > self.limits.candidates_per_lookup
        {
            return None;
        }
        let direct_successor = base_set.links[0]
            .qualified_name
            .as_deref()
            .and_then(|name| self.exact_hierarchy_type(language, name))?;
        let members = self.members_by_owner.get(&(
            language.to_owned(),
            direct_successor,
            candidate.target_spelling.clone(),
        ))?;
        let eligible = members
            .iter()
            .filter(|id| self.declaration_allowed(id, candidate))
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .cloned()
            .collect::<Vec<_>>();
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: only.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::DirectReceiverSuccessorDispatch,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn c3_linearization(
        &self,
        language: &str,
        qualified_name: &str,
        memo: &mut BTreeMap<(String, String), Result<Vec<String>, ()>>,
        visiting: &mut BTreeSet<(String, String)>,
        depth: usize,
    ) -> Result<Vec<String>, ()> {
        if depth >= self.limits.candidates_per_lookup {
            return Err(());
        }
        let canonical = self
            .exact_hierarchy_type(language, qualified_name)
            .ok_or(())?;
        let key = (language.to_owned(), canonical.clone());
        if let Some(cached) = memo.get(&key) {
            return cached.clone();
        }
        if !visiting.insert(key.clone()) {
            return Err(());
        }
        let result = (|| {
            let Some(base_set) = self.direct_bases.get(&key) else {
                return Ok(vec![canonical.clone()]);
            };
            if !base_set.complete
                || base_set.links.is_empty()
                || base_set.links.len() > self.limits.candidates_per_lookup
            {
                return Err(());
            }
            let mut bases = Vec::with_capacity(base_set.links.len());
            let mut sequences = Vec::with_capacity(base_set.links.len().saturating_add(1));
            for link in &base_set.links {
                let base = link
                    .qualified_name
                    .as_deref()
                    .and_then(|name| self.exact_hierarchy_type(language, name))
                    .ok_or(())?;
                bases.push(base.clone());
                sequences.push(self.c3_linearization(
                    language,
                    &base,
                    memo,
                    visiting,
                    depth.saturating_add(1),
                )?);
            }
            sequences.push(bases);
            let mut linearization = vec![canonical.clone()];
            linearization.extend(c3_merge(sequences, self.limits.candidates_per_lookup)?);
            Ok(linearization)
        })();
        visiting.remove(&key);
        memo.insert(key, result.clone());
        result
    }

    fn exact_hierarchy_type(&self, language: &str, qualified_name: &str) -> Option<String> {
        let qualified_name = self.follow_alias(language, qualified_name).ok()?;
        let declarations = self
            .by_qualified
            .get(&(language.to_owned(), qualified_name))?;
        let eligible = declarations
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter(|declaration| declaration.kind == "class")
            .take(2)
            .collect::<Vec<_>>();
        let [declaration] = eligible.as_slice() else {
            return None;
        };
        Some(declaration.qualified_name.clone())
    }

    fn occurrence(&self, candidate: &RelationshipCandidate) -> Option<&OccurrenceFact> {
        candidate
            .occurrence_id
            .as_deref()
            .and_then(|id| self.occurrences.get(id))
    }

    fn unique_decision(
        &self,
        ids: Option<&Vec<String>>,
        candidate: &RelationshipCandidate,
        rule: ResolutionRule,
    ) -> Option<ResolutionDecision> {
        let ids = ids?;
        let eligible = ids
            .iter()
            .filter(|id| self.declaration_allowed(id, candidate))
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .cloned()
            .collect::<Vec<_>>();
        match eligible.as_slice() {
            [only] => Some(ResolutionDecision::Resolved {
                declaration_id: only.clone(),
                evidence: ResolutionEvidence {
                    rule,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn inventory_decision(
        &self,
        language: &str,
        qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        if !matches!(
            candidate.relation,
            CandidateRelation::Imports | CandidateRelation::Reexports
        ) || (!candidate.constraints.allowed_target_kinds.is_empty()
            && !candidate
                .constraints
                .allowed_target_kinds
                .iter()
                .any(|kind| matches!(kind.as_str(), "file" | "module" | "package")))
        {
            return None;
        }
        let candidates = self
            .inventory_by_qualified
            .get(&(language.to_owned(), qualified_name.to_owned()))?;
        match candidates.as_slice() {
            [graph_node_id] => Some(ResolutionDecision::ResolvedInventory {
                graph_node_id: graph_node_id.clone(),
                evidence: ResolutionEvidence {
                    rule: ResolutionRule::ExactSourceInventory,
                    candidate_count: 1,
                },
            }),
            [] => None,
            many => Some(ResolutionDecision::Ambiguous {
                candidate_count: many.len(),
            }),
        }
    }

    fn imported_declarations(
        &self,
        language: &str,
        import_path: &str,
        spelling: &str,
    ) -> Vec<String> {
        if language != "go" {
            return Vec::new();
        }
        let components = import_path
            .split('/')
            .filter(|component| !component.is_empty() && *component != ".")
            .collect::<Vec<_>>();
        let mut imported = BTreeSet::new();
        for start in 0..components.len().min(64) {
            let directory = components[start..].join("/");
            let key = (language.to_owned(), directory, spelling.to_owned());
            if let Some(candidates) = self.by_source_directory_name.get(&key) {
                imported.extend(candidates.iter().cloned());
                if imported.len() > self.limits.candidates_per_lookup {
                    break;
                }
            }
        }
        imported
            .into_iter()
            .take(self.limits.candidates_per_lookup.saturating_add(1))
            .collect()
    }

    fn bound_member_target(
        &self,
        language: &str,
        binding: &BindingFact,
        candidate: &RelationshipCandidate,
    ) -> Result<Option<String>, usize> {
        if binding.kind != compass_languages::BindingKind::LocalAlias {
            return Ok(None);
        }
        let Some(qualifier) = self.occurrence(candidate).and_then(|occurrence| {
            occurrence
                .qualifier
                .as_deref()
                .filter(|qualifier| qualifier.contains('.'))
        }) else {
            return Ok(None);
        };
        let mut parts = qualifier.split('.');
        if parts.next() != Some(binding.spelling.as_str()) {
            return Ok(None);
        }
        let mut target = binding.qualified_target.clone();
        for member in parts {
            let Some(targets) =
                self.members
                    .get(&(language.to_owned(), target.clone(), member.to_owned()))
            else {
                return Ok(None);
            };
            let [next] = targets.as_slice() else {
                return Err(targets.len());
            };
            target.clone_from(next);
        }
        Ok(Some(format!("{target}::{}", candidate.target_spelling)))
    }

    fn member_decision(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let declarations = self.member_declarations(language, qualified, candidate)?;
        self.unique_decision(
            Some(&declarations),
            candidate,
            ResolutionRule::MemberBinding,
        )
    }

    fn member_declarations(
        &self,
        language: &str,
        qualified: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<Vec<String>> {
        let (owner, spelling) = split_qualified_member(qualified)?;
        let targets =
            self.members
                .get(&(language.to_owned(), owner.to_owned(), spelling.to_owned()))?;
        let mut declarations = BTreeSet::new();
        for target in targets {
            if let Some(ids) = self
                .by_qualified
                .get(&(language.to_owned(), target.clone()))
            {
                declarations.extend(
                    ids.iter()
                        .filter(|id| self.declaration_allowed(id, candidate))
                        .cloned(),
                );
            }
        }
        Some(declarations.into_iter().collect())
    }

    fn declaration_allowed(&self, declaration_id: &str, candidate: &RelationshipCandidate) -> bool {
        self.declarations.get(declaration_id).is_some_and(|target| {
            target.language == candidate.language
                && candidate
                    .constraints
                    .argument_count
                    .is_none_or(|arguments| {
                        target.parameter_count.is_none_or(|parameters| {
                            arguments == parameters
                                || (target.variadic && arguments >= parameters.saturating_sub(1))
                        })
                    })
                && (candidate.constraints.allowed_target_kinds.is_empty()
                    || candidate
                        .constraints
                        .allowed_target_kinds
                        .contains(&target.kind))
        })
    }

    fn follow_alias(&self, language: &str, qualified: &str) -> Result<String, usize> {
        let mut current = qualified.to_owned();
        let mut seen = BTreeSet::new();
        for _ in 0..64 {
            if !seen.insert(current.clone()) {
                return Err(seen.len());
            }
            let Some(targets) = self.aliases.get(&(language.to_owned(), current.clone())) else {
                return Ok(current);
            };
            let [target] = targets.as_slice() else {
                return Err(targets.len());
            };
            if target == &current {
                return Ok(current);
            }
            current.clone_from(target);
        }
        Err(64)
    }
}

fn declaration_overloads<'a>(
    declarations: impl Iterator<Item = &'a DeclarationFact>,
) -> BTreeMap<String, String> {
    let mut groups = BTreeMap::<(String, String, String, String), Vec<&DeclarationFact>>::new();
    for declaration in declarations {
        groups
            .entry((
                declaration.language.clone(),
                declaration.range.source_file.clone(),
                declaration.kind.clone(),
                declaration.qualified_name.clone(),
            ))
            .or_default()
            .push(declaration);
    }
    let mut overloads = BTreeMap::new();
    for declarations in groups.values_mut().filter(|group| group.len() > 1) {
        declarations.sort_by_key(|declaration| (declaration.range.start_byte, &declaration.id));
        for (position, declaration) in declarations.iter().enumerate() {
            overloads.insert(declaration.id.clone(), format!("overload:{position}"));
        }
    }
    overloads
}

fn wildcard_qualified_names(module: &str, qualifier: Option<&str>, spelling: &str) -> Vec<String> {
    let separator = if module.contains("::") { "::" } else { "." };
    let mut parts = vec![module];
    if let Some(qualifier) = qualifier.filter(|qualifier| !qualifier.is_empty()) {
        parts.push(qualifier);
    }
    parts.push(spelling);
    vec![parts.join(separator)]
}

fn split_qualified_member(qualified: &str) -> Option<(&str, &str)> {
    qualified
        .rsplit_once("::")
        .or_else(|| qualified.rsplit_once('.'))
}

fn qualified_root(qualified: &str) -> &str {
    qualified
        .trim_start_matches('<')
        .split("::")
        .next()
        .unwrap_or(qualified)
}

fn materialized_declaration_ids<'a>(
    declarations: impl Iterator<Item = &'a DeclarationFact>,
) -> BTreeMap<String, String> {
    let mut groups = BTreeMap::<String, Vec<&DeclarationFact>>::new();
    for declaration in declarations {
        groups
            .entry(declaration.graph_node_id.clone())
            .or_default()
            .push(declaration);
    }
    let mut ids = BTreeMap::new();
    for (graph_node_id, declarations) in groups {
        if declarations.len() == 1 {
            ids.insert(declarations[0].id.clone(), graph_node_id);
            continue;
        }
        for declaration in declarations {
            ids.insert(
                declaration.id.clone(),
                make_id(&[&graph_node_id, &declaration.id]),
            );
        }
    }
    ids
}

fn c3_merge(mut sequences: Vec<Vec<String>>, limit: usize) -> Result<Vec<String>, ()> {
    let mut merged = Vec::new();
    loop {
        sequences.retain(|sequence| !sequence.is_empty());
        if sequences.is_empty() {
            return Ok(merged);
        }
        if merged.len() >= limit {
            return Err(());
        }
        let candidate = sequences
            .iter()
            .map(|sequence| &sequence[0])
            .find(|head| {
                sequences
                    .iter()
                    .all(|sequence| !sequence.iter().skip(1).any(|item| item == *head))
            })
            .cloned()
            .ok_or(())?;
        merged.push(candidate.clone());
        for sequence in &mut sequences {
            if sequence.first() == Some(&candidate) {
                sequence.remove(0);
            }
        }
    }
}

fn profile_internal(label: &str, started: &mut Instant) {
    if std::env::var_os("COMPASS_PROFILE_INTERNAL").is_some() {
        eprintln!(
            "[compass internal] {label}: {:.3}s",
            started.elapsed().as_secs_f64()
        );
        *started = Instant::now();
    }
}

fn python_module_name(source_file: &str, root: &Path) -> Option<String> {
    let path = Path::new(source_file);
    let relative = path.strip_prefix(root).unwrap_or(path);
    let source = relative.to_string_lossy().replace('\\', "/");
    let source = source.strip_suffix(".py")?;
    let source = source.strip_suffix("/__init__").unwrap_or(source);
    let module = source
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>()
        .join(".");
    (!module.is_empty()).then_some(module)
}

fn source_directory(source_file: &str, root: &Path) -> Option<String> {
    let path = Path::new(source_file);
    let relative = path.strip_prefix(root).unwrap_or(path);
    let directory = relative
        .parent()?
        .to_string_lossy()
        .replace('\\', "/")
        .trim_matches('/')
        .to_owned();
    (!directory.is_empty()).then_some(directory)
}

fn insert_unique<T>(map: &mut BTreeMap<String, T>, id: String, value: T) -> Result<(), String> {
    if map.insert(id.clone(), value).is_some() {
        return Err(format!("duplicate universal evidence id {id:?}"));
    }
    Ok(())
}

fn project_declaration_onto_node(
    node: &mut NodeRecord,
    declaration: &DeclarationFact,
    graph_node_id: &str,
) {
    node.attributes
        .extend(declaration_node(declaration, graph_node_id).attributes);
}

fn declaration_node(declaration: &DeclarationFact, graph_node_id: &str) -> NodeRecord {
    let label = match declaration.kind.as_str() {
        "function" => format!("{}()", declaration.name),
        "method" => format!(".{}()", declaration.name),
        _ => declaration.name.clone(),
    };
    let callable = matches!(declaration.kind.as_str(), "function" | "method");
    let mut attributes = Map::from_iter([
        ("label".to_owned(), Value::String(label)),
        (
            "qualified_name".to_owned(),
            Value::String(declaration.qualified_name.clone()),
        ),
        (
            "symbol_kind".to_owned(),
            Value::String(declaration.kind.clone()),
        ),
        ("file_type".to_owned(), Value::String("code".to_owned())),
        (
            "source_file".to_owned(),
            Value::String(declaration.range.source_file.clone()),
        ),
        (
            "source_location".to_owned(),
            Value::String(format!("L{}", declaration.range.start_line)),
        ),
        (
            "start_byte".to_owned(),
            Value::from(declaration.range.start_byte),
        ),
        (
            "end_byte".to_owned(),
            Value::from(declaration.range.end_byte),
        ),
        (
            "line_start".to_owned(),
            Value::from(declaration.range.start_line),
        ),
        (
            "line_end".to_owned(),
            Value::from(declaration.range.end_line),
        ),
        (
            "column_start".to_owned(),
            Value::from(declaration.range.start_column),
        ),
        (
            "column_end".to_owned(),
            Value::from(declaration.range.end_column),
        ),
        (
            "language".to_owned(),
            Value::String(declaration.language.clone()),
        ),
        (
            "extractor".to_owned(),
            Value::String(format!(
                "compass.languages.{}.universal",
                declaration.language
            )),
        ),
        (
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        ),
        ("_origin".to_owned(), Value::String("ast".to_owned())),
        (
            "evidence_declaration_id".to_owned(),
            Value::String(declaration.id.clone()),
        ),
    ]);
    if callable {
        attributes.insert("_callable".to_owned(), Value::Bool(true));
    }
    for (key, value) in [
        ("signature", declaration.signature.as_ref()),
        ("signature_hash", declaration.signature_hash.as_ref()),
        (
            "implementation_hash",
            declaration.implementation_hash.as_ref(),
        ),
        ("source_hash", declaration.source_hash.as_ref()),
    ] {
        if let Some(value) = value {
            attributes.insert(key.to_owned(), Value::String(value.clone()));
        }
    }
    if let Some(module_or_package) = declaration.module_or_package.as_ref() {
        attributes.insert(
            "module".to_owned(),
            Value::String(module_or_package.clone()),
        );
    }
    NodeRecord {
        id: graph_node_id.to_owned(),
        attributes,
    }
}

fn relation_name(relation: CandidateRelation) -> &'static str {
    match relation {
        CandidateRelation::Calls | CandidateRelation::Constructs => "calls",
        CandidateRelation::IndirectCalls => "indirect_call",
        CandidateRelation::Decorates => "references",
        CandidateRelation::Annotates | CandidateRelation::References => "references",
        CandidateRelation::Extends => "inherits",
        CandidateRelation::Implements => "implements",
        CandidateRelation::AccessesMember => "accesses",
        CandidateRelation::Contains => "contains",
        CandidateRelation::Owns => "owns",
        CandidateRelation::Embeds => "embeds",
        CandidateRelation::Imports => "imports_from",
        CandidateRelation::Reexports => "re_exports",
        CandidateRelation::InvokesMacro => "references",
        CandidateRelation::Tests => "tests",
    }
}

fn external_node(
    id: &str,
    qualified_name: &str,
    language: &str,
    candidate: &RelationshipCandidate,
) -> NodeRecord {
    let kind = external_kind(candidate);
    let role = relation_name(candidate.relation);
    let attributes = Map::from_iter([
        (
            "label".to_owned(),
            Value::String(
                qualified_name
                    .rsplit(['.', '/'])
                    .next()
                    .unwrap_or(qualified_name)
                    .to_owned(),
            ),
        ),
        (
            "qualified_name".to_owned(),
            Value::String(qualified_name.to_owned()),
        ),
        ("symbol_kind".to_owned(), Value::String(kind.to_owned())),
        ("file_type".to_owned(), Value::String("code".to_owned())),
        ("source_file".to_owned(), Value::String(String::new())),
        ("source_location".to_owned(), Value::String(String::new())),
        ("language".to_owned(), Value::String(language.to_owned())),
        ("external_role".to_owned(), Value::String(role.to_owned())),
        (
            "external_roles".to_owned(),
            Value::Array(vec![Value::String(role.to_owned())]),
        ),
        (
            "extractor".to_owned(),
            Value::String(format!("compass.resolve.{language}.universal")),
        ),
        (
            "confidence".to_owned(),
            Value::String(
                if candidate.binding_id.is_some() {
                    "EXTRACTED"
                } else {
                    "INFERRED"
                }
                .to_owned(),
            ),
        ),
        ("external".to_owned(), Value::Bool(true)),
        ("placeholder".to_owned(), Value::Bool(true)),
        ("_canonical_external_symbol".to_owned(), Value::Bool(true)),
    ]);
    NodeRecord {
        id: id.to_owned(),
        attributes,
    }
}

fn deferred_receiver_node(
    id: &str,
    qualified_name: &str,
    language: &str,
    candidate: &RelationshipCandidate,
) -> NodeRecord {
    let kind = external_kind(candidate);
    NodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
            (
                "label".to_owned(),
                Value::String(
                    qualified_name
                        .rsplit([':', '.'])
                        .find(|component| !component.is_empty())
                        .unwrap_or(qualified_name)
                        .to_owned(),
                ),
            ),
            (
                "qualified_name".to_owned(),
                Value::String(qualified_name.to_owned()),
            ),
            ("symbol_kind".to_owned(), Value::String(kind.to_owned())),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            ("source_file".to_owned(), Value::String(String::new())),
            ("source_location".to_owned(), Value::String(String::new())),
            ("language".to_owned(), Value::String(language.to_owned())),
            (
                "extractor".to_owned(),
                Value::String(format!("compass.resolve.{language}.universal")),
            ),
            (
                "confidence".to_owned(),
                Value::String("INFERRED".to_owned()),
            ),
            ("external".to_owned(), Value::Bool(false)),
            ("placeholder".to_owned(), Value::Bool(true)),
            ("deferred_receiver".to_owned(), Value::Bool(true)),
            (
                "deferred_role".to_owned(),
                Value::String(relation_name(candidate.relation).to_owned()),
            ),
        ]),
    }
}

fn is_deferred_receiver(qualifier: &str) -> bool {
    !qualifier.contains("::") && !qualifier.contains('/')
}

fn merge_external_node(node: &mut NodeRecord, candidate: &RelationshipCandidate) {
    let incoming_kind = external_kind(candidate);
    let current_kind = node.string("symbol_kind");
    if external_kind_rank(incoming_kind) > external_kind_rank(&current_kind) {
        node.attributes.insert(
            "symbol_kind".to_owned(),
            Value::String(incoming_kind.to_owned()),
        );
        node.attributes.insert(
            "external_role".to_owned(),
            Value::String(relation_name(candidate.relation).to_owned()),
        );
    }
    let incoming_role = relation_name(candidate.relation);
    let mut roles = node
        .attributes
        .get("external_roles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    roles.insert(incoming_role.to_owned());
    node.attributes.insert(
        "external_roles".to_owned(),
        Value::Array(roles.into_iter().map(Value::String).collect()),
    );
}

fn external_kind(candidate: &RelationshipCandidate) -> &'static str {
    match candidate.relation {
        CandidateRelation::Imports => "import",
        CandidateRelation::Reexports => "export",
        CandidateRelation::Extends | CandidateRelation::Annotates | CandidateRelation::Embeds => {
            "type_alias"
        }
        CandidateRelation::Implements => "interface",
        CandidateRelation::AccessesMember => "variable",
        CandidateRelation::Calls | CandidateRelation::IndirectCalls => "function",
        CandidateRelation::Constructs => "class",
        CandidateRelation::Decorates => "function",
        CandidateRelation::References => "variable",
        CandidateRelation::Contains | CandidateRelation::Owns => "variable",
        CandidateRelation::InvokesMacro => "macro",
        CandidateRelation::Tests => "function",
    }
}

fn external_kind_rank(kind: &str) -> u8 {
    match kind {
        "interface" => 7,
        "class" => 6,
        "type_alias" => 5,
        "function" => 4,
        "variable" => 3,
        "import" => 2,
        "export" => 1,
        _ => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn materialized_edge(
    source: String,
    target: String,
    relation: &str,
    candidate: &RelationshipCandidate,
    occurrence: Option<&OccurrenceFact>,
    binding: Option<&BindingFact>,
    target_kind: Option<&str>,
    target_source_file: Option<&str>,
    range: &compass_languages::EvidenceRange,
    resolution_rule: ResolutionRule,
    language: &str,
) -> EdgeRecord {
    let context = match (relation, resolution_rule) {
        ("calls", ResolutionRule::QualifiedExternal) => "external_call",
        ("calls", ResolutionRule::DeferredReceiver) => "deferred_receiver_call",
        ("calls", _) => "call",
        ("indirect_call", _) => occurrence
            .and_then(|occurrence| occurrence.context.as_deref())
            .unwrap_or("reference"),
        ("references", _) if candidate.relation == CandidateRelation::Decorates => "decorator",
        ("imports_from", _)
            if candidate.relation == CandidateRelation::Imports && target_kind == Some("file") =>
        {
            "submodule_import"
        }
        ("imports_from", _) => "import",
        ("re_exports", _) => "export",
        ("inherits", _) => "base_type",
        ("references", _) => "type_reference",
        ("embeds", _) => "embedding",
        ("method", _) => "receiver",
        _ => "",
    };
    let confidence = if matches!(
        resolution_rule,
        ResolutionRule::QualifiedExternal | ResolutionRule::DeferredReceiver
    ) {
        "INFERRED"
    } else {
        "EXTRACTED"
    };
    let producer_rule = format!(
        "universal-{}-{}",
        candidate_relation_name(candidate.relation),
        resolution_rule_name(resolution_rule)
    );
    let occurrence_rule = binding.map_or_else(
        || producer_rule.clone(),
        |binding| {
            // Import aliases can share one statement anchor and one resolved
            // endpoint. Keep those occurrences distinct without using the
            // candidate ID, whose absolute byte position changes when the
            // statement moves. Binding offsets within the statement remain
            // portable across checkouts and source relocation.
            format!(
                "{producer_rule}:binding:{}:{}:{}",
                binding.spelling,
                binding.range.start_byte.saturating_sub(range.start_byte),
                binding.range.end_byte.saturating_sub(range.start_byte)
            )
        },
    );
    let mut attributes = Map::from_iter([
        ("relation".to_owned(), Value::String(relation.to_owned())),
        ("_origin".to_owned(), Value::String("ast".to_owned())),
        (
            "confidence".to_owned(),
            Value::String(confidence.to_owned()),
        ),
        (
            "source_file".to_owned(),
            Value::String(range.source_file.clone()),
        ),
        (
            "source_location".to_owned(),
            Value::String(format!("L{}", range.start_line)),
        ),
        ("start_byte".to_owned(), Value::from(range.start_byte)),
        ("end_byte".to_owned(), Value::from(range.end_byte)),
        ("line_start".to_owned(), Value::from(range.start_line)),
        ("line_end".to_owned(), Value::from(range.end_line)),
        ("column_start".to_owned(), Value::from(range.start_column)),
        ("column_end".to_owned(), Value::from(range.end_column)),
        ("weight".to_owned(), Value::from(1.0)),
        ("language".to_owned(), Value::String(language.to_owned())),
        (
            "extractor".to_owned(),
            Value::String(format!("compass.resolve.{language}.universal")),
        ),
        (
            "resolution_rule".to_owned(),
            Value::String(resolution_rule_name(resolution_rule).to_owned()),
        ),
        ("rule".to_owned(), Value::String(producer_rule)),
        (
            OCCURRENCE_RULE_ATTRIBUTE.to_owned(),
            Value::String(occurrence_rule),
        ),
        (
            "evidence_candidate_id".to_owned(),
            Value::String(candidate.id.clone()),
        ),
    ]);
    if let Some(occurrence_id) = candidate.occurrence_id.as_ref() {
        attributes.insert(
            "evidence_occurrence_id".to_owned(),
            Value::String(occurrence_id.clone()),
        );
    }
    if matches!(
        candidate.relation,
        CandidateRelation::Imports | CandidateRelation::Reexports
    ) && let Some(binding) = binding
    {
        attributes.insert(
            "local_name".to_owned(),
            Value::String(binding.spelling.clone()),
        );
        attributes.insert(
            "imported_name".to_owned(),
            Value::String(candidate.target_spelling.clone()),
        );
        attributes.insert(
            "qualified_target".to_owned(),
            Value::String(binding.qualified_target.clone()),
        );
        attributes.insert(
            "binding_kind".to_owned(),
            Value::String(binding_kind_name(binding.kind).to_owned()),
        );
        if let Some(module) = candidate.constraints.module_or_package.as_ref() {
            attributes.insert(
                "module".to_owned(),
                Value::String(import_module_for_edge(language, &range.source_file, module)),
            );
        }
    }
    if !context.is_empty() {
        attributes.insert("context".to_owned(), Value::String(context.to_owned()));
    }
    if let Some(target_source_file) = target_source_file {
        attributes.insert(
            "target_file".to_owned(),
            Value::String(target_source_file.to_owned()),
        );
    }
    if let Some(binding) = binding {
        attributes.extend([
            (
                "binding_name".to_owned(),
                Value::String(binding.spelling.clone()),
            ),
            (
                "binding_qualified_target".to_owned(),
                Value::String(binding.qualified_target.clone()),
            ),
        ]);
    }
    EdgeRecord {
        source,
        target,
        attributes,
    }
}

fn binding_kind_name(kind: compass_languages::BindingKind) -> &'static str {
    match kind {
        compass_languages::BindingKind::Import => "import",
        compass_languages::BindingKind::ImportAlias => "import_alias",
        compass_languages::BindingKind::Reexport => "reexport",
        compass_languages::BindingKind::LocalAlias => "local_alias",
        compass_languages::BindingKind::Package => "package",
        compass_languages::BindingKind::Member => "member",
    }
}

fn import_module_for_edge(language: &str, source_file: &str, module: &str) -> String {
    if language != "python" {
        return module.to_owned();
    }
    let source_package = Path::new(source_file)
        .parent()
        .map(|parent| {
            parent
                .components()
                .filter_map(|component| component.as_os_str().to_str())
                .filter(|component| !component.is_empty() && *component != ".")
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let module_components = module
        .split('.')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    let shared = source_package
        .iter()
        .zip(&module_components)
        .take_while(|(left, right)| left == right)
        .count();
    if shared == 0 {
        return module.to_owned();
    }
    let upward = source_package.len().saturating_sub(shared);
    let suffix = module_components[shared..].join(".");
    format!("{}{}", ".".repeat(upward.saturating_add(1)), suffix)
}

const fn candidate_relation_name(relation: CandidateRelation) -> &'static str {
    match relation {
        CandidateRelation::Calls => "call",
        CandidateRelation::IndirectCalls => "indirect-call",
        CandidateRelation::Constructs => "construction",
        CandidateRelation::Decorates => "decorator",
        CandidateRelation::Annotates => "annotation",
        CandidateRelation::Extends => "extends",
        CandidateRelation::Implements => "implements",
        CandidateRelation::References => "reference",
        CandidateRelation::AccessesMember => "member-access",
        CandidateRelation::Contains => "contains",
        CandidateRelation::Owns => "owns",
        CandidateRelation::Embeds => "embedding",
        CandidateRelation::Imports => "import",
        CandidateRelation::Reexports => "reexport",
        CandidateRelation::InvokesMacro => "macro-invocation",
        CandidateRelation::Tests => "test-call",
    }
}

const fn resolution_rule_name(rule: ResolutionRule) -> &'static str {
    match rule {
        ResolutionRule::ExactSourceDeclaration => "exact-source-declaration",
        ResolutionRule::ExactLexicalDeclaration => "exact-lexical-declaration",
        ResolutionRule::ExplicitBinding => "explicit-binding",
        ResolutionRule::MemberBinding => "member-binding",
        ResolutionRule::DeferredReceiver => "deferred-receiver",
        ResolutionRule::WildcardBinding => "wildcard-binding",
        ResolutionRule::UniqueModuleOrPackage => "unique-module-or-package",
        ResolutionRule::ExactHierarchyBase => "exact-hierarchy-base",
        ResolutionRule::DirectReceiverSuccessorDispatch => "direct-receiver-successor-dispatch",
        ResolutionRule::LinearizedReceiverDispatch => "linearized-receiver-dispatch",
        ResolutionRule::ExactSourceInventory => "exact-source-inventory",
        ResolutionRule::QualifiedExternal => "qualified-external",
    }
}

#[must_use]
pub(crate) fn is_replaced_relation(relation: &str) -> bool {
    matches!(
        relation,
        "contains"
            | "method"
            | "calls"
            | "indirect_call"
            | "imports"
            | "imports_from"
            | "re_exports"
            | "inherits"
            | "implements"
            | "references"
            | "embeds"
            | "decorated_by"
            | "owns"
            | "accesses"
    )
}
