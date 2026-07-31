use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;

use ahash::AHashMap;
use compass_languages::{
    CandidateRelation, DeclarationFact, EvidenceLimits, HierarchyConstraint, OccurrenceFact,
    ReceiverDispatchStrategy, RelationshipCandidate, SemanticEvidenceBatch, make_id,
    validate_evidence,
};
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
            let module_attribute = owner.kind == "file"
                && binding.language == "python"
                && matches!(
                    binding.kind,
                    compass_languages::BindingKind::Import
                        | compass_languages::BindingKind::ImportAlias
                        | compass_languages::BindingKind::Reexport
                );
            if binding.kind != compass_languages::BindingKind::Reexport && !module_attribute {
                continue;
            }
            let Some(module) = owner.module_or_package.as_ref() else {
                continue;
            };
            aliases
                .entry((
                    binding.language.clone(),
                    format!("{module}.{}", binding.spelling),
                ))
                .or_default()
                .push(binding.qualified_target.clone());
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
            strategy: ReceiverDispatchStrategy::C3AfterReceiver,
        }) = candidate.constraints.hierarchy.as_ref()
        {
            return self.resolve_c3_receiver_dispatch(language, receiver_qualified_name, candidate);
        }
        if matches!(
            candidate.constraints.hierarchy.as_ref(),
            Some(HierarchyConstraint::DirectBase { .. })
        ) {
            return self.resolve_direct_base(language, candidate);
        }
        let occurrence = self.occurrence(candidate);
        let has_unbound_qualified_receiver = occurrence
            .and_then(|occurrence| occurrence.qualifier.as_deref())
            .is_some_and(|qualifier| {
                candidate.binding_id.is_none()
                    && !matches!((language, qualifier), ("python", "self" | "cls"))
            });

        if let Some(decision) = self.resolve_explicit_binding(language, candidate) {
            return decision;
        }

        if !has_unbound_qualified_receiver
            && let Some(scope) = candidate.constraints.scope_id.as_deref()
        {
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
        ResolutionDecision::Unresolved
    }

    fn resolve_explicit_binding(
        &self,
        language: &str,
        candidate: &RelationshipCandidate,
    ) -> Option<ResolutionDecision> {
        let binding = candidate
            .binding_id
            .as_deref()
            .and_then(|id| self.bindings.get(id))?;
        if let Some(target) = binding.target_declaration_id.as_ref()
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
        let qualified = match self.follow_alias(language, &binding.qualified_target) {
            Ok(qualified) => qualified,
            Err(candidate_count) => {
                return Some(ResolutionDecision::Ambiguous { candidate_count });
            }
        };
        let key = (language.to_owned(), qualified.clone());
        if let Some(decision) = self.unique_decision(
            self.by_qualified.get(&key),
            candidate,
            ResolutionRule::ExplicitBinding,
        ) {
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
        let existing_nodes = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        let mut emitted_external = BTreeSet::new();
        let mut emitted_edges = BTreeSet::new();
        let candidate_ids = self.candidate_ids();
        profile_internal("universal candidate ordering", &mut profile_started);
        for candidate_id in candidate_ids {
            let candidate = &self.candidates[candidate_id];
            let Some(source) = self
                .declarations
                .get(&candidate.source_declaration_id)
                .map(|declaration| declaration.graph_node_id.clone())
            else {
                continue;
            };
            let (target, target_source_file, resolution_rule) = match self.resolve(candidate_id) {
                ResolutionDecision::Resolved {
                    declaration_id,
                    evidence,
                } => {
                    let Some(target) = self.declarations.get(&declaration_id) else {
                        continue;
                    };
                    (
                        target.graph_node_id.clone(),
                        Some(target.range.source_file.as_str()),
                        evidence.rule,
                    )
                }
                ResolutionDecision::ResolvedInventory {
                    graph_node_id,
                    evidence,
                } => (graph_node_id, None, evidence.rule),
                ResolutionDecision::QualifiedExternal {
                    qualified_name,
                    evidence,
                } => {
                    let site = self
                        .occurrence(candidate)
                        .map(|occurrence| &occurrence.range);
                    let binding_site = candidate
                        .binding_id
                        .as_deref()
                        .and_then(|id| self.bindings.get(id))
                        .map(|binding| &binding.range);
                    let external_site = binding_site.or(site);
                    let kind = external_kind(candidate);
                    let id = match (kind, external_site) {
                        ("import" | "export", Some(range)) => make_id(&[
                            "external",
                            &candidate.language,
                            kind,
                            &qualified_name,
                            &range.source_file,
                            &range.start_byte.to_string(),
                            &range.end_byte.to_string(),
                        ]),
                        (_, Some(range)) => make_id(&[
                            "external",
                            &candidate.language,
                            kind,
                            &qualified_name,
                            &range.source_file,
                        ]),
                        (_, None) => {
                            make_id(&["external", &candidate.language, kind, &qualified_name])
                        }
                    };
                    if !existing_nodes.contains(&id) && emitted_external.insert(id.clone()) {
                        nodes.push(external_node(
                            &id,
                            &qualified_name,
                            &candidate.language,
                            candidate,
                            external_site,
                        ));
                    }
                    (id, None, evidence.rule)
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
            );
            if !emitted_edges.insert(key) || source == target {
                continue;
            }
            edges.push(materialized_edge(MaterializedEdge {
                source,
                target,
                relation,
                candidate_relation: candidate.relation,
                range: site,
                rule: resolution_rule,
                language: &candidate.language,
                target_source_file,
                binding: candidate
                    .binding_id
                    .as_deref()
                    .and_then(|id| self.bindings.get(id)),
            }));
        }
        profile_internal("universal candidate resolution", &mut profile_started);
    }

    fn resolve_c3_receiver_dispatch(
        &self,
        language: &str,
        receiver_qualified_name: &str,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        if let Some(decision) =
            self.resolve_direct_receiver_successor(language, receiver_qualified_name, candidate)
        {
            return decision;
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

    fn declaration_allowed(&self, declaration_id: &str, candidate: &RelationshipCandidate) -> bool {
        self.declarations.get(declaration_id).is_some_and(|target| {
            target.language == candidate.language
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

fn relation_name(relation: CandidateRelation) -> &'static str {
    match relation {
        CandidateRelation::Calls | CandidateRelation::Constructs => "calls",
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
    }
}

fn external_node(
    id: &str,
    qualified_name: &str,
    language: &str,
    candidate: &RelationshipCandidate,
    site: Option<&compass_languages::EvidenceRange>,
) -> NodeRecord {
    let kind = external_kind(candidate);
    let source_file = site.map_or_else(String::new, |range| range.source_file.clone());
    let source_location = site.map_or_else(String::new, |range| format!("L{}", range.start_line));
    let mut attributes = Map::from_iter([
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
        ("source_file".to_owned(), Value::String(source_file)),
        ("source_location".to_owned(), Value::String(source_location)),
        ("language".to_owned(), Value::String(language.to_owned())),
        (
            "external_role".to_owned(),
            Value::String(relation_name(candidate.relation).to_owned()),
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
    ]);
    if let Some(range) = site {
        attributes.extend([
            ("start_byte".to_owned(), Value::from(range.start_byte)),
            ("end_byte".to_owned(), Value::from(range.end_byte)),
            ("line_start".to_owned(), Value::from(range.start_line)),
            ("line_end".to_owned(), Value::from(range.end_line)),
            ("column_start".to_owned(), Value::from(range.start_column)),
            ("column_end".to_owned(), Value::from(range.end_column)),
        ]);
    }
    NodeRecord {
        id: id.to_owned(),
        attributes,
    }
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
        CandidateRelation::Calls
        | CandidateRelation::Constructs
        | CandidateRelation::Decorates
        | CandidateRelation::References => {
            if candidate.binding_id.is_some() {
                "import"
            } else {
                "variable"
            }
        }
        CandidateRelation::Contains | CandidateRelation::Owns => "variable",
    }
}

struct MaterializedEdge<'a> {
    source: String,
    target: String,
    relation: &'a str,
    candidate_relation: CandidateRelation,
    range: &'a compass_languages::EvidenceRange,
    rule: ResolutionRule,
    language: &'a str,
    target_source_file: Option<&'a str>,
    binding: Option<&'a compass_languages::BindingFact>,
}

fn materialized_edge(edge: MaterializedEdge<'_>) -> EdgeRecord {
    let MaterializedEdge {
        source,
        target,
        relation,
        candidate_relation,
        range,
        rule,
        language,
        target_source_file,
        binding,
    } = edge;
    let context = match (relation, rule) {
        ("calls", ResolutionRule::QualifiedExternal) => "external_call",
        ("calls", _) => "call",
        ("references", _) if candidate_relation == CandidateRelation::Decorates => "decorator",
        ("imports_from", _) => "import",
        ("re_exports", _) => "export",
        ("inherits", _) => "base_type",
        ("references", _) => "type_reference",
        ("embeds", _) => "embedding",
        ("method", _) => "receiver",
        _ => "",
    };
    let confidence = if rule == ResolutionRule::QualifiedExternal {
        "INFERRED"
    } else {
        "EXTRACTED"
    };
    let mut attributes = Map::from_iter([
        ("relation".to_owned(), Value::String(relation.to_owned())),
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
            Value::String(format!("{rule:?}").to_ascii_lowercase()),
        ),
    ]);
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
