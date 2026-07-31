use std::collections::{BTreeMap, BTreeSet};

use ahash::AHashMap;
use compass_languages::{
    CandidateRelation, DeclarationFact, EvidenceLimits, OccurrenceFact, RelationshipCandidate,
    SemanticEvidenceBatch, make_id, validate_evidence,
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
    ExactLexicalDeclaration,
    ExplicitBinding,
    UniqueModuleOrPackage,
    QualifiedExternal,
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
    aliases: AHashMap<(String, String), Vec<String>>,
    limits: UniversalResolutionLimits,
}

impl UniversalResolutionIndex {
    pub fn new(
        batches: &[SemanticEvidenceBatch],
        limits: UniversalResolutionLimits,
    ) -> Result<Self, String> {
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
        }
        for values in by_qualified
            .values_mut()
            .chain(by_module_name.values_mut())
            .chain(by_scope_name.values_mut())
        {
            values.sort_unstable();
            values.dedup();
            if values.len() > limits.candidates_per_lookup {
                values.truncate(limits.candidates_per_lookup);
            }
        }
        let mut aliases = AHashMap::<_, Vec<_>>::new();
        for binding in bindings
            .values()
            .filter(|binding| binding.kind == compass_languages::BindingKind::Reexport)
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
        Ok(Self {
            declarations,
            occurrences,
            bindings,
            candidates,
            scopes,
            by_qualified,
            by_module_name,
            by_scope_name,
            aliases,
            limits,
        })
    }

    #[must_use]
    pub fn candidate_ids(&self) -> Vec<&str> {
        let mut ids = self
            .candidates
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        ids.sort_unstable_by_key(|id| {
            let candidate = &self.candidates[*id];
            let range = self
                .occurrence(candidate)
                .map(|occurrence| &occurrence.range)
                .or_else(|| {
                    self.declarations
                        .get(&candidate.source_declaration_id)
                        .map(|declaration| &declaration.range)
                });
            range.map_or_else(
                || (String::new(), u64::MAX, u64::MAX, (*id).to_owned()),
                |range| {
                    (
                        range.source_file.clone(),
                        range.start_byte,
                        range.end_byte,
                        (*id).to_owned(),
                    )
                },
            )
        });
        ids
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

        if let Some(scope) = candidate.constraints.scope_id.as_deref() {
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

        if let Some(binding_id) = candidate.binding_id.as_deref()
            && let Some(binding) = self.bindings.get(binding_id)
        {
            if let Some(target) = binding.target_declaration_id.as_ref()
                && self.declaration_allowed(target, candidate)
            {
                return ResolutionDecision::Resolved {
                    declaration_id: target.clone(),
                    evidence: ResolutionEvidence {
                        rule: ResolutionRule::ExplicitBinding,
                        candidate_count: 1,
                    },
                };
            }
            let qualified = match self.follow_alias(language, &binding.qualified_target) {
                Ok(qualified) => qualified,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
            let key = (language.to_owned(), qualified);
            if let Some(decision) = self.unique_decision(
                self.by_qualified.get(&key),
                candidate,
                ResolutionRule::ExplicitBinding,
            ) {
                return decision;
            }
            let imported = self
                .declarations
                .values()
                .filter(|declaration| {
                    let directory = std::path::Path::new(&declaration.range.source_file)
                        .parent()
                        .map(|path| path.to_string_lossy().replace('\\', "/"))
                        .unwrap_or_default();
                    declaration.language == language
                        && declaration.name == candidate.target_spelling
                        && !directory.is_empty()
                        && (binding.qualified_target == directory
                            || binding
                                .qualified_target
                                .strip_suffix(&directory)
                                .is_some_and(|prefix| prefix.ends_with('/')))
                })
                .map(|declaration| declaration.id.clone())
                .take(self.limits.candidates_per_lookup.saturating_add(1))
                .collect::<Vec<_>>();
            if !imported.is_empty()
                && let Some(decision) = self.unique_decision(
                    Some(&imported),
                    candidate,
                    ResolutionRule::ExplicitBinding,
                )
            {
                return decision;
            }
        }

        if let Some(qualified) = candidate.constraints.qualified_name.as_ref() {
            let qualified = match self.follow_alias(language, qualified) {
                Ok(qualified) => qualified,
                Err(candidate_count) => {
                    return ResolutionDecision::Ambiguous { candidate_count };
                }
            };
            let key = (language.to_owned(), qualified);
            if let Some(decision) = self.unique_decision(
                self.by_qualified.get(&key),
                candidate,
                ResolutionRule::ExplicitBinding,
            ) {
                return decision;
            }
        }

        if let Some(module) = candidate.constraints.module_or_package.as_ref() {
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

    pub fn materialize(&self, nodes: &mut Vec<NodeRecord>, edges: &mut Vec<EdgeRecord>) {
        let existing_nodes = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        let mut emitted_external = BTreeSet::new();
        let mut emitted_edges = BTreeSet::new();
        for candidate_id in self.candidate_ids() {
            let candidate = &self.candidates[candidate_id];
            let Some(source) = self
                .declarations
                .get(&candidate.source_declaration_id)
                .map(|declaration| declaration.graph_node_id.clone())
            else {
                continue;
            };
            let (target, resolution_rule) = match self.resolve(candidate_id) {
                ResolutionDecision::Resolved {
                    declaration_id,
                    evidence,
                } => {
                    let Some(target) = self.declarations.get(&declaration_id) else {
                        continue;
                    };
                    (target.graph_node_id.clone(), evidence.rule)
                }
                ResolutionDecision::QualifiedExternal {
                    qualified_name,
                    evidence,
                } => {
                    let site = self
                        .occurrence(candidate)
                        .map(|occurrence| &occurrence.range);
                    let id = site.map_or_else(
                        || make_id(&["external", &candidate.language, &qualified_name]),
                        |range| {
                            make_id(&[
                                "external",
                                &candidate.language,
                                &qualified_name,
                                &range.source_file,
                                &range.start_byte.to_string(),
                                &range.end_byte.to_string(),
                            ])
                        },
                    );
                    if !existing_nodes.contains(&id) && emitted_external.insert(id.clone()) {
                        nodes.push(external_node(
                            &id,
                            &qualified_name,
                            &candidate.language,
                            candidate.relation,
                        ));
                    }
                    (id, evidence.rule)
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
            let relation = if self.occurrence(candidate).is_some_and(|occurrence| {
                occurrence.role == compass_languages::SemanticRole::Receiver
            }) {
                "method"
            } else {
                relation_name(candidate.relation)
            };
            let site = self
                .occurrence(candidate)
                .map(|occurrence| &occurrence.range)
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
            edges.push(materialized_edge(
                source,
                target,
                relation,
                site,
                resolution_rule,
                &candidate.language,
            ));
        }
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
            current.clone_from(target);
        }
        Err(64)
    }
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
        CandidateRelation::Decorates => "decorated_by",
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
    relation: CandidateRelation,
) -> NodeRecord {
    NodeRecord {
        id: id.to_owned(),
        attributes: Map::from_iter([
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
            ("symbol_kind".to_owned(), Value::String("symbol".to_owned())),
            ("file_type".to_owned(), Value::String("code".to_owned())),
            ("source_file".to_owned(), Value::String(String::new())),
            ("source_location".to_owned(), Value::String(String::new())),
            ("language".to_owned(), Value::String(language.to_owned())),
            (
                "external_role".to_owned(),
                Value::String(relation_name(relation).to_owned()),
            ),
            (
                "extractor".to_owned(),
                Value::String(format!("compass.resolve.{language}.universal")),
            ),
        ]),
    }
}

fn materialized_edge(
    source: String,
    target: String,
    relation: &str,
    range: &compass_languages::EvidenceRange,
    rule: ResolutionRule,
    language: &str,
) -> EdgeRecord {
    let context = match (relation, rule) {
        ("calls", ResolutionRule::QualifiedExternal) => "external_call",
        ("calls", _) => "call",
        ("decorated_by", _) => "decorator",
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
