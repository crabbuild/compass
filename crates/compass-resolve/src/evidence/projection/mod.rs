//! Deterministic projection of resolution decisions into graph records.

use super::*;
use crate::ResolutionAdmission;

mod edges;
mod nodes;

pub(crate) use edges::is_replaced_relation;
use edges::*;
pub(in crate::evidence) use nodes::is_deferred_receiver;
use nodes::*;

struct PreparedTarget {
    candidate_slot: CandidateSlot,
    candidate_id_override: Option<String>,
    relation_override: Option<CandidateRelation>,
    target: String,
    rule: ResolutionRule,
    target_kind: Option<String>,
    declaration_id: Option<String>,
    external_qualified_name: Option<String>,
    deferred_qualified_name: Option<String>,
    emit_edge: bool,
}

struct PendingDecision {
    candidate_slot: CandidateSlot,
    candidate_id_override: Option<String>,
    relation_override: Option<CandidateRelation>,
    decision: ResolutionDecision,
}

const EDGE_MATERIALIZATION_BATCH_SIZE: usize = 4_096;

fn next_edge_materialization_batch<T>(values: &mut impl Iterator<Item = T>) -> Option<Vec<T>> {
    let batch = values
        .take(EDGE_MATERIALIZATION_BATCH_SIZE)
        .collect::<Vec<_>>();
    (!batch.is_empty()).then_some(batch)
}

impl UniversalResolutionIndex {
    pub fn materialize(&self, nodes: &mut Vec<NodeRecord>, edges: &mut Vec<EdgeRecord>) {
        self.materialize_inner(nodes, edges, ResolutionAdmission::Max, false);
    }

    pub(crate) fn materialize_relationships_at_inference(
        &self,
        nodes: &mut Vec<NodeRecord>,
        edges: &mut Vec<EdgeRecord>,
        admission: ResolutionAdmission,
    ) {
        self.materialize_inner_with_declarations(nodes, edges, admission, true, false);
    }

    fn materialize_inner(
        &self,
        nodes: &mut Vec<NodeRecord>,
        edges: &mut Vec<EdgeRecord>,
        admission: ResolutionAdmission,
        release_resolution_indexes: bool,
    ) {
        self.materialize_inner_with_declarations(
            nodes,
            edges,
            admission,
            release_resolution_indexes,
            true,
        );
    }

    fn materialize_inner_with_declarations(
        &self,
        nodes: &mut Vec<NodeRecord>,
        edges: &mut Vec<EdgeRecord>,
        admission: ResolutionAdmission,
        release_resolution_indexes: bool,
        project_declarations: bool,
    ) {
        let mut profile_started = Instant::now();
        let graph_ids = materialized_declaration_ids(self.facts.declarations.values());
        if project_declarations {
            let overloads = declaration_overloads(self.facts.declarations.values());
            let existing_positions = nodes
                .iter()
                .enumerate()
                .map(|(index, node)| (node.id.clone(), index))
                .collect::<AHashMap<_, _>>();
            let mut existing_nodes = nodes
                .iter()
                .map(|node| node.id.clone())
                .collect::<AHashSet<_>>();
            let mut declarations = self.facts.declarations.values().collect::<Vec<_>>();
            declarations.sort_unstable_by(|left, right| left.id.cmp(&right.id));
            const DECLARATION_BATCH_SIZE: usize = 8_192;
            for declaration_batch in declarations.chunks(DECLARATION_BATCH_SIZE) {
                let prepared = declaration_batch
                    .par_iter()
                    .map(|declaration| {
                        let graph_node_id = &graph_ids[&declaration.id];
                        let definition_range = self.facts.definition_ranges.get(&declaration.id);
                        let node = declaration_node(declaration, definition_range, graph_node_id);
                        let discriminator = overloads.get(&declaration.id).cloned();
                        (node, discriminator)
                    })
                    .collect::<Vec<_>>();
                for (mut node, discriminator) in prepared {
                    if let Some(index) = existing_positions.get(&node.id) {
                        nodes[*index].attributes.extend(node.attributes);
                        if let Some(discriminator) = discriminator {
                            nodes[*index].attributes.insert(
                                "overload_discriminator".to_owned(),
                                Value::String(discriminator),
                            );
                        }
                    } else if existing_nodes.insert(node.id.clone()) {
                        if let Some(discriminator) = discriminator {
                            node.attributes.insert(
                                "overload_discriminator".to_owned(),
                                Value::String(discriminator),
                            );
                        }
                        nodes.push(node);
                    }
                }
            }
        }
        profile_internal("universal declaration projection", &mut profile_started);
        let inventory_kinds = nodes
            .iter()
            .map(|node| (node.id.clone(), node.string("symbol_kind")))
            .collect::<AHashMap<_, _>>();
        let mut external_positions = nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.attributes.get("external").and_then(Value::as_bool) == Some(true)
            })
            .map(|(index, node)| (node.id.clone(), index))
            .collect::<AHashMap<_, _>>();
        let candidates = self.ordered_candidates();
        profile_internal("universal candidate ordering", &mut profile_started);
        let resolution_indexes = self
            .indexes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let db = ResolutionDb::new(self, &resolution_indexes);
        let decision_batches = candidates
            .into_par_iter()
            .filter_map(|candidate_slot| {
                let candidate = self.facts.candidates.at(candidate_slot)?;
                let candidate_id = candidate.id.as_str();
                let decision = db.resolve_candidate(&candidate, admission);
                let exact_declaration_id = match &decision {
                    ResolutionDecision::Resolved { declaration_id, .. } => {
                        Some(declaration_id.clone())
                    }
                    _ => None,
                };
                let mut test_source_id = (admission == ResolutionAdmission::Low
                    && candidate.relation == CandidateRelation::Tests
                    && !matches!(
                        decision,
                        ResolutionDecision::Ambiguous { .. } | ResolutionDecision::Unresolved
                    ))
                .then(|| graph_ids.get(&candidate.source_declaration_id).cloned())
                .flatten();
                let allow_primary = decision_is_needed(&candidate, &decision, admission);
                let allow_possible = admission.admits_source_backed_inference()
                    && !matches!(decision, ResolutionDecision::Ambiguous { .. });
                let aliased_tests = self.low_test_aliases.get(candidate_id);
                let mut decisions = Vec::with_capacity(
                    1 + usize::from(allow_possible) + aliased_tests.map_or(0, Vec::len),
                );
                for test_id in aliased_tests.into_iter().flatten() {
                    let mut test_candidate = candidate.clone();
                    test_candidate.id.clone_from(test_id);
                    test_candidate.relation = CandidateRelation::Tests;
                    let test_decision = db.resolve_candidate(&test_candidate, admission);
                    if !matches!(
                        test_decision,
                        ResolutionDecision::Ambiguous { .. } | ResolutionDecision::Unresolved
                    ) {
                        test_source_id = graph_ids
                            .get(&test_candidate.source_declaration_id)
                            .cloned();
                    }
                    if decision_is_needed(&test_candidate, &test_decision, admission) {
                        decisions.push(PendingDecision {
                            candidate_slot,
                            candidate_id_override: Some(test_id.clone()),
                            relation_override: Some(CandidateRelation::Tests),
                            decision: test_decision,
                        });
                    }
                }
                if allow_primary {
                    decisions.push(PendingDecision {
                        candidate_slot,
                        candidate_id_override: None,
                        relation_override: None,
                        decision,
                    });
                }
                if allow_possible {
                    decisions.extend(
                        db.possible_receiver_dispatches(
                            candidate_id,
                            exact_declaration_id.as_deref(),
                        )
                        .into_iter()
                        .map(|(declaration_id, rule)| PendingDecision {
                            candidate_slot,
                            candidate_id_override: None,
                            relation_override: None,
                            decision: ResolutionDecision::Resolved {
                                declaration_id,
                                evidence: ResolutionEvidence {
                                    rule,
                                    candidate_count: 1,
                                },
                            },
                        }),
                    );
                }
                Some((test_source_id, decisions))
            })
            .collect::<Vec<_>>();
        let mut test_source_ids = AHashSet::new();
        let mut decisions = Vec::new();
        for (test_source_id, mut batch) in decision_batches {
            if let Some(test_source_id) = test_source_id {
                test_source_ids.insert(test_source_id);
            }
            decisions.append(&mut batch);
        }
        if admission == ResolutionAdmission::Low {
            mark_test_roles(nodes, &test_source_ids);
        }
        profile_internal("universal candidate decisions", &mut profile_started);
        // Target and edge projection only reads primary facts plus project
        // metadata. Release the large name/member/hierarchy lookup indexes as
        // soon as every resolution decision is fixed so they do not overlap
        // the materialized graph records at the cold-build high-water mark.
        drop(resolution_indexes);
        if release_resolution_indexes {
            let released = {
                let mut indexes = self
                    .indexes
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                std::mem::take(&mut *indexes)
            };
            drop(released);
        }
        // Projection reads only facts and project metadata. Keep an explicit
        // empty index set so the shared read-only helper cannot accidentally
        // regain a dependency on the released resolution indexes.
        let projection_indexes = ResolutionIndexes::default();
        let db = ResolutionDb::new(self, &projection_indexes);
        let prepared_targets = decisions
            .into_par_iter()
            .map(|pending| match pending.decision {
                ResolutionDecision::Resolved {
                    declaration_id,
                    evidence,
                } => {
                    let target = self.facts.declarations.get(&declaration_id)?;
                    Some(PreparedTarget {
                        candidate_slot: pending.candidate_slot,
                        candidate_id_override: pending.candidate_id_override,
                        relation_override: pending.relation_override,
                        target: graph_ids[&target.id].clone(),
                        rule: evidence.rule,
                        target_kind: Some(target.kind.clone()),
                        declaration_id: Some(target.id.clone()),
                        external_qualified_name: None,
                        deferred_qualified_name: None,
                        emit_edge: true,
                    })
                }
                ResolutionDecision::ResolvedInventory {
                    graph_node_id,
                    evidence,
                } => {
                    let kind = inventory_kinds.get(&graph_node_id).cloned();
                    Some(PreparedTarget {
                        candidate_slot: pending.candidate_slot,
                        candidate_id_override: pending.candidate_id_override,
                        relation_override: pending.relation_override,
                        target: graph_node_id,
                        rule: evidence.rule,
                        target_kind: kind,
                        declaration_id: None,
                        external_qualified_name: None,
                        deferred_qualified_name: None,
                        emit_edge: true,
                    })
                }
                ResolutionDecision::QualifiedExternal {
                    qualified_name,
                    evidence,
                } => {
                    if !admission.admits_qualified_external() {
                        return None;
                    }
                    let candidate = self.facts.candidates.at(pending.candidate_slot)?;
                    let id = make_id(&["external", &candidate.language, &qualified_name]);
                    Some(PreparedTarget {
                        candidate_slot: pending.candidate_slot,
                        candidate_id_override: pending.candidate_id_override,
                        relation_override: pending.relation_override,
                        target: id,
                        rule: evidence.rule,
                        target_kind: None,
                        declaration_id: None,
                        external_qualified_name: Some(qualified_name),
                        deferred_qualified_name: None,
                        emit_edge: true,
                    })
                }
                ResolutionDecision::DeferredReceiver {
                    qualified_name,
                    evidence,
                } => {
                    let candidate = self.facts.candidates.at(pending.candidate_slot)?;
                    let id = make_id(&["deferred", &candidate.language, &qualified_name]);
                    Some(PreparedTarget {
                        candidate_slot: pending.candidate_slot,
                        candidate_id_override: pending.candidate_id_override,
                        relation_override: pending.relation_override,
                        target: id,
                        rule: evidence.rule,
                        target_kind: None,
                        declaration_id: None,
                        external_qualified_name: None,
                        deferred_qualified_name: Some(qualified_name),
                        emit_edge: admission.admits_deferred_receiver(),
                    })
                }
                ResolutionDecision::Ambiguous { .. } | ResolutionDecision::Unresolved => None,
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut resolved_targets = Vec::with_capacity(prepared_targets.len());
        let mut existing_nodes = nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<AHashSet<_>>();
        for mut prepared in prepared_targets {
            let Some(original_candidate) = self.facts.candidates.at(prepared.candidate_slot) else {
                continue;
            };
            let overridden_candidate = prepared.candidate_id_override.as_ref().map(|id| {
                let mut candidate = original_candidate.clone();
                candidate.id.clone_from(id);
                if let Some(relation) = prepared.relation_override {
                    candidate.relation = relation;
                }
                candidate
            });
            let candidate = overridden_candidate.as_ref().unwrap_or(&original_candidate);
            if let Some(qualified_name) = prepared.external_qualified_name.take() {
                if let Some(position) = external_positions.get(&prepared.target).copied() {
                    merge_external_node(&mut nodes[position], candidate);
                } else if !existing_nodes.contains(&prepared.target) {
                    let position = nodes.len();
                    nodes.push(external_node(
                        &prepared.target,
                        &qualified_name,
                        &candidate.language,
                        candidate,
                    ));
                    existing_nodes.insert(prepared.target.clone());
                    external_positions.insert(prepared.target.clone(), position);
                }
                let fallback = external_kind(candidate).to_owned();
                prepared.target_kind = Some(
                    external_positions
                        .get(&prepared.target)
                        .map(|position| nodes[*position].string("symbol_kind"))
                        .filter(|kind| !kind.is_empty())
                        .unwrap_or(fallback),
                );
            } else if let Some(qualified_name) = prepared.deferred_qualified_name.take() {
                if existing_nodes.insert(prepared.target.clone()) {
                    nodes.push(deferred_receiver_node(
                        &prepared.target,
                        &qualified_name,
                        &candidate.language,
                        candidate,
                    ));
                }
                prepared.target_kind = Some(external_kind(candidate).to_owned());
            }
            let target_site = prepared
                .declaration_id
                .as_deref()
                .and_then(|id| self.facts.declarations.get(id))
                .map(|declaration| &declaration.range);
            if prepared.emit_edge {
                resolved_targets.push((
                    prepared.candidate_slot,
                    prepared.candidate_id_override,
                    prepared.relation_override,
                    prepared.target,
                    prepared.rule,
                    prepared.target_kind,
                    target_site,
                ));
            }
        }
        profile_internal("universal target projection", &mut profile_started);
        // Edge records carry a comparatively large provenance map. Building
        // every record in one parallel collection temporarily duplicates the
        // complete resolved edge set immediately before it is moved into the
        // graph. Keep the indexed parallel ordering, but bound that overlap to
        // one deterministic batch at a time.
        let mut resolved_targets = resolved_targets.into_iter();
        while let Some(target_batch) = next_edge_materialization_batch(&mut resolved_targets) {
            let materialized = target_batch
                .into_par_iter()
                .filter_map(
                    |(
                        candidate_slot,
                        candidate_id_override,
                        relation_override,
                        target,
                        resolution_rule,
                        target_kind,
                        target_site,
                    )| {
                        let original_candidate = self.facts.candidates.at(candidate_slot)?;
                        let overridden_candidate = candidate_id_override.map(|id| {
                            let mut candidate = original_candidate.clone();
                            candidate.id = id;
                            if let Some(relation) = relation_override {
                                candidate.relation = relation;
                            }
                            candidate
                        });
                        let candidate =
                            overridden_candidate.as_ref().unwrap_or(&original_candidate);
                        let owner_source = self
                            .facts
                            .declarations
                            .get(&candidate.source_declaration_id)
                            .map(|declaration| graph_ids[&declaration.id].clone())?;
                        let annotation_source = (candidate.relation
                            == CandidateRelation::Decorates)
                            .then(|| {
                                let occurrence = db.occurrence(candidate)?;
                                self.facts
                                    .declarations
                                    .values()
                                    .filter(|declaration| {
                                        declaration.kind == "annotation"
                                            && declaration.name == occurrence.spelling()
                                            && declaration.range.source_file
                                                == occurrence.range().source_file
                                            && declaration.range.start_byte
                                                <= occurrence.range().start_byte
                                            && declaration.range.end_byte
                                                >= occurrence.range().end_byte
                                    })
                                    .min_by_key(|declaration| {
                                        (declaration.range.start_byte, declaration.id.as_str())
                                    })
                                    .map(|declaration| graph_ids[&declaration.id].clone())
                            })
                            .flatten();
                        let source = annotation_source
                            .clone()
                            .unwrap_or_else(|| owner_source.clone());
                        let (source, target) = if candidate.relation == CandidateRelation::Contains
                        {
                            (source, target)
                        } else if db.occurrence(candidate).is_some_and(|occurrence| {
                            occurrence.role() == compass_languages::SemanticRole::Receiver
                        }) {
                            (target, source)
                        } else {
                            (source, target)
                        };
                        let exact_target = candidate
                            .constraints
                            .exact_target_declaration_id
                            .as_deref()
                            .and_then(|id| self.facts.declarations.get(id));
                        let relation = if candidate.relation == CandidateRelation::Decorates
                            && annotation_source.is_some()
                        {
                            "decorates"
                        } else if db.occurrence(candidate).is_some_and(|occurrence| {
                            occurrence.role() == compass_languages::SemanticRole::Receiver
                        }) {
                            "method"
                        } else if candidate.language == "go"
                            && candidate.relation == CandidateRelation::Calls
                            && target_kind.as_deref().is_some_and(|kind| {
                                matches!(kind, "struct" | "interface" | "type_alias")
                            })
                        {
                            "references"
                        } else {
                            relation_name(candidate.relation)
                        };
                        let site = db
                            .occurrence(candidate)
                            .map(OccurrenceRef::range)
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
                                self.facts
                                    .declarations
                                    .get(&candidate.source_declaration_id)
                                    .map(|declaration| &declaration.range)
                            });
                        let site = site?;
                        // Exact resolution and bounded possible dispatches can project
                        // more than one target for a candidate. Downstream publication
                        // performs contract-level semantic edge coalescing.
                        if source == target && relation != "calls" {
                            return None;
                        }
                        let target_source_file =
                            target_site.map(|range| range.source_file.as_str());
                        let project_metadata =
                            db.typescript_project_metadata(candidate, target_source_file);
                        let binding = candidate
                            .binding_id
                            .as_deref()
                            .and_then(|binding_id| self.facts.bindings.get(binding_id));
                        let occurrence = db.occurrence(candidate);
                        let edge = materialized_edge(
                            source,
                            target,
                            relation,
                            candidate,
                            occurrence,
                            binding,
                            target_kind.as_deref(),
                            target_source_file,
                            site,
                            resolution_rule,
                            &candidate.language,
                            project_metadata.as_ref(),
                        );
                        let alias_edge = if candidate.relation == CandidateRelation::Reexports
                            && binding.is_some_and(|binding| {
                                let target_name = binding
                                    .qualified_target
                                    .rsplit([':', '.'])
                                    .find(|name| !name.is_empty())
                                    .unwrap_or_default();
                                binding.spelling != target_name
                                    || occurrence
                                        .and_then(OccurrenceRef::qualifier)
                                        .is_some_and(|qualifier| qualifier != binding.spelling)
                            }) {
                            let export_name = binding.map(|binding| binding.spelling.as_str());
                            let alias_source = export_name.and_then(|export_name| {
                                occurrence.and_then(|occurrence| {
                                    self.facts
                                        .declarations
                                        .values()
                                        .filter(|declaration| {
                                            declaration.kind == "export"
                                                && declaration.name == export_name
                                                && declaration.range.source_file
                                                    == occurrence.range().source_file
                                                && declaration.range.start_byte
                                                    <= occurrence.range().start_byte
                                                && declaration.range.end_byte
                                                    >= occurrence.range().end_byte
                                        })
                                        .min_by_key(|declaration| {
                                            (declaration.range.start_byte, declaration.id.as_str())
                                        })
                                        .map(|declaration| graph_ids[&declaration.id].clone())
                                })
                            });
                            alias_source.map(|alias_source| {
                                let mut attributes = edge.attributes.clone();
                                attributes.insert(
                                    "relation".to_owned(),
                                    Value::String("aliases".to_owned()),
                                );
                                attributes.insert(
                                    "context".to_owned(),
                                    Value::String("export_alias".to_owned()),
                                );
                                attributes.insert(
                                    "rule".to_owned(),
                                    Value::String(format!(
                                        "universal-reexport-alias-{}",
                                        resolution_rule_name(resolution_rule)
                                    )),
                                );
                                EdgeRecord {
                                    source: alias_source,
                                    target: edge.target.clone(),
                                    attributes,
                                }
                            })
                        } else {
                            None
                        };
                        let mut materialized =
                            Vec::with_capacity(1 + usize::from(alias_edge.is_some()));
                        materialized.push(edge);
                        materialized.extend(alias_edge);
                        Some(materialized)
                    },
                )
                .flatten()
                .collect::<Vec<_>>();
            edges.extend(materialized);
        }
        profile_internal("universal edge materialization", &mut profile_started);
    }
}

pub(super) fn project_declaration_batches(
    batches: &[SemanticEvidenceBatch],
    nodes: &mut Vec<NodeRecord>,
) -> AHashMap<String, String> {
    let mut declarations = batches
        .iter()
        .flat_map(|batch| &batch.declarations)
        .collect::<Vec<_>>();
    let overloads = declaration_overloads(declarations.iter().copied());
    let graph_ids = materialized_declaration_ids(declarations.iter().copied());
    let definition_ranges = batch_definition_ranges(batches, &declarations);
    let existing_positions = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.clone(), index))
        .collect::<AHashMap<_, _>>();
    let mut existing_nodes = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<AHashSet<_>>();
    declarations.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    for declaration in declarations {
        let mut node = declaration_node(
            declaration,
            definition_ranges.get(&declaration.id),
            &graph_ids[&declaration.id],
        );
        if let Some(discriminator) = overloads.get(&declaration.id) {
            node.attributes.insert(
                "overload_discriminator".to_owned(),
                Value::String(discriminator.clone()),
            );
        }
        if let Some(index) = existing_positions.get(&node.id) {
            nodes[*index].attributes.extend(node.attributes);
        } else if existing_nodes.insert(node.id.clone()) {
            nodes.push(node);
        }
    }
    graph_ids
}

fn batch_definition_ranges(
    batches: &[SemanticEvidenceBatch],
    declarations: &[&DeclarationFact],
) -> BTreeMap<String, EvidenceRange> {
    let declarations = declarations
        .iter()
        .map(|declaration| (declaration.id.as_str(), *declaration))
        .collect::<BTreeMap<_, _>>();
    let mut ranges = BTreeMap::new();
    let mut ambiguous = BTreeSet::new();
    for scope in batches.iter().flat_map(|batch| &batch.scopes) {
        let Some(owner_id) = scope.owner_declaration_id.as_ref() else {
            continue;
        };
        let Some(declaration) = declarations.get(owner_id.as_str()) else {
            continue;
        };
        if !range_contains(&scope.range, &declaration.range) || ambiguous.contains(owner_id) {
            continue;
        }
        if ranges
            .insert(owner_id.clone(), scope.range.clone())
            .is_some()
        {
            ranges.remove(owner_id);
            ambiguous.insert(owner_id.clone());
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::{EDGE_MATERIALIZATION_BATCH_SIZE, next_edge_materialization_batch};

    #[test]
    fn edge_materialization_batches_are_bounded_and_ordered() {
        let mut values = 0..EDGE_MATERIALIZATION_BATCH_SIZE + 1;

        let first = next_edge_materialization_batch(&mut values);
        let second = next_edge_materialization_batch(&mut values);

        assert_eq!(
            first.as_ref().map(Vec::len),
            Some(EDGE_MATERIALIZATION_BATCH_SIZE)
        );
        assert_eq!(first.and_then(|batch| batch.last().copied()), Some(4_095));
        assert_eq!(second, Some(vec![EDGE_MATERIALIZATION_BATCH_SIZE]));
        assert_eq!(next_edge_materialization_batch(&mut values), None);
    }
}

fn mark_test_roles(nodes: &mut [NodeRecord], test_source_ids: &AHashSet<String>) {
    for node in nodes
        .iter_mut()
        .filter(|node| test_source_ids.contains(&node.id))
    {
        let roles = node
            .attributes
            .entry("roles".to_owned())
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(roles) = roles.as_array_mut()
            && !roles.iter().any(|role| role.as_str() == Some("test"))
        {
            roles.push(Value::String("test".to_owned()));
        }
    }
}

fn decision_is_needed(
    candidate: &RelationshipCandidate,
    decision: &ResolutionDecision,
    admission: ResolutionAdmission,
) -> bool {
    match decision {
        ResolutionDecision::Resolved { evidence, .. }
        | ResolutionDecision::ResolvedInventory { evidence, .. } => {
            resolution_rule_is_admitted(evidence.rule, admission)
        }
        ResolutionDecision::QualifiedExternal { .. } => {
            admission.admits_qualified_external() || candidate.binding_id.is_some()
        }
        ResolutionDecision::DeferredReceiver { .. } => admission.admits_deferred_receiver(),
        ResolutionDecision::Ambiguous { .. } | ResolutionDecision::Unresolved => false,
    }
}

fn resolution_rule_is_admitted(rule: ResolutionRule, admission: ResolutionAdmission) -> bool {
    match rule {
        ResolutionRule::ClosedWorldReceiverDispatch
        | ResolutionRule::IncompleteHierarchyReceiverDispatch => {
            admission.admits_source_backed_inference()
        }
        ResolutionRule::QualifiedExternal => admission.admits_qualified_external(),
        ResolutionRule::DeferredReceiver => admission.admits_deferred_receiver(),
        _ => true,
    }
}

fn materialized_declaration_ids<'a>(
    declarations: impl Iterator<Item = &'a DeclarationFact>,
) -> AHashMap<String, String> {
    let mut groups = AHashMap::<String, Vec<&DeclarationFact>>::new();
    for declaration in declarations {
        groups
            .entry(declaration.graph_node_id.clone())
            .or_default()
            .push(declaration);
    }
    let mut ids = AHashMap::new();
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
