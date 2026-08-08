//! Deterministic projection of resolution decisions into graph records.

use super::*;

mod edges;
mod nodes;

pub(crate) use edges::is_replaced_relation;
use edges::*;
pub(in crate::evidence) use nodes::is_deferred_receiver;
use nodes::*;

struct PreparedTarget<'a> {
    candidate_id: &'a str,
    target: String,
    rule: ResolutionRule,
    target_kind: Option<String>,
    declaration_id: Option<String>,
    external_qualified_name: Option<String>,
    deferred_qualified_name: Option<String>,
}

impl UniversalResolutionIndex {
    pub fn materialize(&self, nodes: &mut Vec<NodeRecord>, edges: &mut Vec<EdgeRecord>) {
        let mut profile_started = Instant::now();
        let overloads = declaration_overloads(self.facts.declarations.values());
        let graph_ids = materialized_declaration_ids(self.facts.declarations.values());
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
        let candidate_ids = self.candidate_ids();
        profile_internal("universal candidate ordering", &mut profile_started);
        let decisions = candidate_ids
            .into_par_iter()
            .map(|candidate_id| {
                let decision = self.resolve(candidate_id);
                let exact_declaration_id = match &decision {
                    ResolutionDecision::Resolved { declaration_id, .. } => {
                        Some(declaration_id.clone())
                    }
                    _ => None,
                };
                let allow_possible = !matches!(decision, ResolutionDecision::Ambiguous { .. });
                let mut decisions = vec![(candidate_id, decision)];
                if allow_possible {
                    decisions.extend(
                        self.possible_receiver_dispatches(
                            candidate_id,
                            exact_declaration_id.as_deref(),
                        )
                        .into_iter()
                        .map(|(declaration_id, rule)| {
                            (
                                candidate_id,
                                ResolutionDecision::Resolved {
                                    declaration_id,
                                    evidence: ResolutionEvidence {
                                        rule,
                                        candidate_count: 1,
                                    },
                                },
                            )
                        }),
                    );
                }
                decisions
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        profile_internal("universal candidate decisions", &mut profile_started);
        let prepared_targets = decisions
            .into_par_iter()
            .map(|(candidate_id, decision)| match decision {
                ResolutionDecision::Resolved {
                    declaration_id,
                    evidence,
                } => {
                    let target = self.facts.declarations.get(&declaration_id)?;
                    Some(PreparedTarget {
                        candidate_id,
                        target: graph_ids[&target.id].clone(),
                        rule: evidence.rule,
                        target_kind: Some(target.kind.clone()),
                        declaration_id: Some(target.id.clone()),
                        external_qualified_name: None,
                        deferred_qualified_name: None,
                    })
                }
                ResolutionDecision::ResolvedInventory {
                    graph_node_id,
                    evidence,
                } => {
                    let kind = inventory_kinds.get(&graph_node_id).cloned();
                    Some(PreparedTarget {
                        candidate_id,
                        target: graph_node_id,
                        rule: evidence.rule,
                        target_kind: kind,
                        declaration_id: None,
                        external_qualified_name: None,
                        deferred_qualified_name: None,
                    })
                }
                ResolutionDecision::QualifiedExternal {
                    qualified_name,
                    evidence,
                } => {
                    let candidate = &self.facts.candidates[candidate_id];
                    let id = make_id(&["external", &candidate.language, &qualified_name]);
                    Some(PreparedTarget {
                        candidate_id,
                        target: id,
                        rule: evidence.rule,
                        target_kind: None,
                        declaration_id: None,
                        external_qualified_name: Some(qualified_name),
                        deferred_qualified_name: None,
                    })
                }
                ResolutionDecision::DeferredReceiver {
                    qualified_name,
                    evidence,
                } => {
                    let candidate = &self.facts.candidates[candidate_id];
                    let id = make_id(&["deferred", &candidate.language, &qualified_name]);
                    Some(PreparedTarget {
                        candidate_id,
                        target: id,
                        rule: evidence.rule,
                        target_kind: None,
                        declaration_id: None,
                        external_qualified_name: None,
                        deferred_qualified_name: Some(qualified_name),
                    })
                }
                ResolutionDecision::Ambiguous { .. } | ResolutionDecision::Unresolved => None,
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let mut resolved_targets = Vec::with_capacity(prepared_targets.len());
        for mut prepared in prepared_targets {
            let candidate = &self.facts.candidates[prepared.candidate_id];
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
            resolved_targets.push((
                prepared.candidate_id,
                prepared.target,
                prepared.rule,
                prepared.target_kind,
                target_site,
            ));
        }
        profile_internal("universal target projection", &mut profile_started);
        let materialized = resolved_targets
            .into_par_iter()
            .filter_map(
                |(candidate_id, target, resolution_rule, target_kind, target_site)| {
                    let candidate = &self.facts.candidates[candidate_id];
                    let owner_source = self
                        .facts
                        .declarations
                        .get(&candidate.source_declaration_id)
                        .map(|declaration| graph_ids[&declaration.id].clone())?;
                    let annotation_source = (candidate.relation == CandidateRelation::Decorates)
                        .then(|| {
                            let occurrence = self.occurrence(candidate)?;
                            self.facts
                                .declarations
                                .values()
                                .filter(|declaration| {
                                    declaration.kind == "annotation"
                                        && declaration.name == occurrence.spelling
                                        && declaration.range.source_file
                                            == occurrence.range.source_file
                                        && declaration.range.start_byte
                                            <= occurrence.range.start_byte
                                        && declaration.range.end_byte >= occurrence.range.end_byte
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
                        .and_then(|id| self.facts.declarations.get(id));
                    let relation = if candidate.relation == CandidateRelation::Decorates
                        && annotation_source.is_some()
                    {
                        "decorates"
                    } else if self.occurrence(candidate).is_some_and(|occurrence| {
                        occurrence.role == compass_languages::SemanticRole::Receiver
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
                    let target_source_file = target_site.map(|range| range.source_file.as_str());
                    let project_metadata =
                        self.typescript_project_metadata(candidate, target_source_file);
                    let binding = candidate
                        .binding_id
                        .as_deref()
                        .and_then(|binding_id| self.facts.bindings.get(binding_id));
                    let occurrence = self.occurrence(candidate);
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
                    let mut materialized = vec![edge.clone()];
                    if candidate.relation == CandidateRelation::Reexports
                        && binding.is_some_and(|binding| {
                            let target_name = binding
                                .qualified_target
                                .rsplit([':', '.'])
                                .find(|name| !name.is_empty())
                                .unwrap_or_default();
                            binding.spelling != target_name
                                || occurrence
                                    .and_then(|occurrence| occurrence.qualifier.as_deref())
                                    .is_some_and(|qualifier| qualifier != binding.spelling)
                        })
                    {
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
                                                == occurrence.range.source_file
                                            && declaration.range.start_byte
                                                <= occurrence.range.start_byte
                                            && declaration.range.end_byte
                                                >= occurrence.range.end_byte
                                    })
                                    .min_by_key(|declaration| {
                                        (declaration.range.start_byte, declaration.id.as_str())
                                    })
                                    .map(|declaration| graph_ids[&declaration.id].clone())
                            })
                        });
                        if let Some(alias_source) = alias_source {
                            let mut attributes = edge.attributes.clone();
                            attributes
                                .insert("relation".to_owned(), Value::String("aliases".to_owned()));
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
                            materialized.push(EdgeRecord {
                                source: alias_source,
                                target: edge.target.clone(),
                                attributes,
                            });
                        }
                    }
                    Some(materialized)
                },
            )
            .flatten()
            .collect::<Vec<_>>();
        edges.extend(materialized);
        profile_internal("universal edge materialization", &mut profile_started);
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
