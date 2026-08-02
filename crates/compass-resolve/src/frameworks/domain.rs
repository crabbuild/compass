use ahash::AHashSet as HashSet;

use compass_languages::{
    Extraction, FrameworkLimitError, FrameworkLimits, RawDomainFact, RawEdgeRecord,
    RawFrameworkFact, RawNodeRecord, make_id,
};
use compass_model::provenance::{ResolutionCandidate, ResolutionState, SourceAnchor};
use serde_json::{Map, Value, json};

use super::FrameworkResolutionError;
use super::target_index::{FrameworkTargetIndex, TargetFamily, normalize_reference, terminal_name};

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedDomainFact {
    pub fact: RawDomainFact,
    pub state: ResolutionState,
    pub source_candidates: Vec<ResolutionCandidate>,
    pub target_candidates: Vec<ResolutionCandidate>,
}

pub fn resolve_domains(
    extraction: &Extraction,
    limits: FrameworkLimits,
) -> Result<Vec<ResolvedDomainFact>, FrameworkResolutionError> {
    let targets = FrameworkTargetIndex::new(extraction);
    resolve_domains_with_targets(extraction, limits, &targets)
}

pub(super) fn resolve_domains_with_targets(
    extraction: &Extraction,
    limits: FrameworkLimits,
    targets: &FrameworkTargetIndex<'_>,
) -> Result<Vec<ResolvedDomainFact>, FrameworkResolutionError> {
    let facts = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(fact) => Some(fact),
            RawFrameworkFact::Route(_) | RawFrameworkFact::Annotation(_) => None,
        })
        .collect::<Vec<_>>();
    limits.check_facts(facts.len())?;
    facts
        .into_iter()
        .map(|fact| resolve_one(fact, targets, limits))
        .collect()
}

pub fn resolve_and_publish_framework_domains(
    extraction: &mut Extraction,
    limits: FrameworkLimits,
) -> Result<Vec<ResolvedDomainFact>, FrameworkResolutionError> {
    let resolved = resolve_domains(extraction, limits)?;
    publish_resolved_domains(extraction, &resolved);
    Ok(resolved)
}

pub fn publish_resolved_domains(extraction: &mut Extraction, resolved: &[ResolvedDomainFact]) {
    let mut nodes = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let mut edges = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                edge.string("relation"),
            )
        })
        .collect::<HashSet<_>>();
    let mut diagnostics = Vec::new();

    for resolved in resolved {
        let fact = &resolved.fact;
        if fact.kind == "orm_mapping" {
            if resolved.state == ResolutionState::Exact
                && let ([model], [table]) = (
                    resolved.source_candidates.as_slice(),
                    resolved.target_candidates.as_slice(),
                )
            {
                push_edge(
                    extraction,
                    &mut edges,
                    &model.node_id,
                    &table.node_id,
                    "maps_to",
                    fact,
                    resolved.state,
                );
            } else {
                diagnostics.push(json!({
                    "kind": "unresolved_orm_mapping",
                    "framework": fact.framework,
                    "model": fact.name,
                    "databaseTable": fact.detail.get("database_table"),
                    "source": fact.anchor.source_file,
                    "line": fact.anchor.start_line,
                    "resolution": resolution_name(resolved.state),
                }));
            }
            continue;
        }
        if fact.kind == "framework_decoration" {
            if resolved.state == ResolutionState::Exact
                && let Some(trait_name) = fact.detail.get("trait").and_then(Value::as_str)
            {
                for candidate in &resolved.source_candidates {
                    if let Some(node) = extraction
                        .nodes
                        .iter_mut()
                        .find(|node| node.id == candidate.node_id)
                    {
                        let traits = node
                            .attributes
                            .entry("framework_traits".to_owned())
                            .or_insert_with(|| Value::Array(Vec::new()));
                        if let Some(traits) = traits.as_array_mut()
                            && !traits
                                .iter()
                                .any(|value| value.as_str() == Some(trait_name))
                        {
                            traits.push(Value::String(trait_name.to_owned()));
                            traits.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
                        }
                    }
                }
            } else {
                diagnostics.push(json!({
                    "kind": "unresolved_framework_decoration",
                    "framework": fact.framework,
                    "trait": fact.name,
                    "source": fact.anchor.source_file,
                    "line": fact.anchor.start_line,
                    "resolution": resolution_name(resolved.state),
                }));
            }
            continue;
        }
        if fact.kind == "injection" {
            if resolved.state == ResolutionState::Exact
                && let ([source], [target]) = (
                    resolved.source_candidates.as_slice(),
                    resolved.target_candidates.as_slice(),
                )
            {
                push_edge(
                    extraction,
                    &mut edges,
                    &source.node_id,
                    &target.node_id,
                    "depends_on",
                    fact,
                    resolved.state,
                );
            } else {
                diagnostics.push(json!({
                    "kind": "unresolved_injection",
                    "framework": fact.framework,
                    "source": fact.detail.get("source_reference"),
                    "target": fact.detail.get("target_reference"),
                    "sourceFile": fact.anchor.source_file,
                    "line": fact.anchor.start_line,
                    "resolution": resolution_name(resolved.state),
                }));
            }
            continue;
        }
        if fact.kind == "route_middleware" {
            let middleware_id = route_middleware_id(fact);
            if nodes.insert(middleware_id.clone()) {
                extraction.nodes.push(RawNodeRecord {
                    id: middleware_id,
                    attributes: route_middleware_attributes(fact),
                });
            }
            continue;
        }

        let Some(symbol_kind) = domain_node_kind(&fact.kind) else {
            diagnostics.push(json!({
                "kind": "unsupported_domain_fact",
                "framework": fact.framework,
                "domainKind": fact.kind,
                "name": fact.name,
            }));
            continue;
        };
        let domain_id = domain_id(fact);
        if nodes.insert(domain_id.clone()) {
            extraction.nodes.push(RawNodeRecord {
                id: domain_id.clone(),
                attributes: domain_attributes(fact, symbol_kind, resolved.state),
            });
        }
        if resolved.state != ResolutionState::Exact {
            diagnostics.push(json!({
                "kind": "unresolved_domain_handler",
                "framework": fact.framework,
                "domainKind": fact.kind,
                "name": fact.name,
                "source": fact.anchor.source_file,
                "line": fact.anchor.start_line,
                "resolution": resolution_name(resolved.state),
                "candidates": resolved.source_candidates,
            }));
            continue;
        }
        let Some(source) = resolved.source_candidates.first() else {
            continue;
        };
        if fact.kind == "job" {
            push_edge(
                extraction,
                &mut edges,
                &source.node_id,
                &domain_id,
                "schedules",
                fact,
                resolved.state,
            );
            push_edge(
                extraction,
                &mut edges,
                &domain_id,
                &source.node_id,
                "triggers",
                fact,
                resolved.state,
            );
        } else {
            let relationship = fact
                .detail
                .get("relationship")
                .and_then(Value::as_str)
                .unwrap_or("handles");
            push_edge(
                extraction,
                &mut edges,
                &source.node_id,
                &domain_id,
                relationship,
                fact,
                resolved.state,
            );
        }
    }

    if !diagnostics.is_empty()
        && let Some(values) = extraction
            .extensions
            .entry("framework_domain_diagnostics".to_owned())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
    {
        values.extend(diagnostics);
    }
}

fn resolve_one(
    fact: &RawDomainFact,
    targets: &FrameworkTargetIndex<'_>,
    limits: FrameworkLimits,
) -> Result<ResolvedDomainFact, FrameworkResolutionError> {
    if fact.kind == "route_middleware" {
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: ResolutionState::Exact,
            source_candidates: Vec::new(),
            target_candidates: Vec::new(),
        });
    }
    if fact.kind == "orm_mapping" {
        let model = fact
            .detail
            .get("model_reference")
            .and_then(Value::as_str)
            .unwrap_or(&fact.name);
        let table = fact
            .detail
            .get("database_table")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let schema = fact
            .detail
            .get("database_schema")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let table = if schema.is_empty() {
            table.to_owned()
        } else {
            format!("{schema}.{table}")
        };
        let sources = resolve_reference(
            targets,
            model,
            TargetKind::Type,
            &fact.anchor.source_file,
            limits,
        )?;
        let targets = resolve_reference(
            targets,
            &table,
            TargetKind::DatabaseTable,
            &fact.anchor.source_file,
            limits,
        )?;
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: pair_state(&sources, &targets),
            source_candidates: sources,
            target_candidates: targets,
        });
    }
    if fact.kind == "injection" {
        let source = fact
            .detail
            .get("source_reference")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let target = fact
            .detail
            .get("target_reference")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let sources = resolve_reference(
            targets,
            source,
            TargetKind::Type,
            &fact.anchor.source_file,
            limits,
        )?;
        let targets = resolve_reference(
            targets,
            target,
            TargetKind::Type,
            &fact.anchor.source_file,
            limits,
        )?;
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: pair_state(&sources, &targets),
            source_candidates: sources,
            target_candidates: targets,
        });
    }
    let signature_reference = fact
        .detail
        .get("target_signature_qualified")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let qualified_reference = fact
        .detail
        .get("target_qualified_name")
        .or_else(|| fact.detail.get("handler_reference"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut candidates = if signature_reference.is_empty() {
        Vec::new()
    } else {
        resolve_reference(
            targets,
            signature_reference,
            TargetKind::Callable,
            &fact.anchor.source_file,
            limits,
        )?
    };
    if candidates.is_empty() {
        candidates = resolve_reference(
            targets,
            qualified_reference,
            TargetKind::Callable,
            &fact.anchor.source_file,
            limits,
        )?;
    }
    if candidates.is_empty()
        && fact.kind == "bean_definition"
        && fact.detail.get("owner_kind").and_then(Value::as_str) != Some("method")
    {
        candidates = resolve_reference(
            targets,
            qualified_reference,
            TargetKind::Type,
            &fact.anchor.source_file,
            limits,
        )?;
    }
    Ok(ResolvedDomainFact {
        fact: fact.clone(),
        state: single_state(&candidates),
        source_candidates: candidates,
        target_candidates: Vec::new(),
    })
}

#[derive(Clone, Copy)]
enum TargetKind {
    Callable,
    Type,
    DatabaseTable,
}

fn resolve_reference(
    targets: &FrameworkTargetIndex<'_>,
    reference: &str,
    target_kind: TargetKind,
    declaring_source: &str,
    limits: FrameworkLimits,
) -> Result<Vec<ResolutionCandidate>, FrameworkResolutionError> {
    if reference.trim().is_empty() {
        return Ok(Vec::new());
    }
    let expected = normalize_reference(reference);
    let expected_terminal = terminal_name(&expected);
    let owner = expected.rsplit_once('.').map(|(owner, _)| owner);
    let families = [match target_kind {
        TargetKind::Callable => TargetFamily::Callable,
        TargetKind::Type => TargetFamily::Type,
        TargetKind::DatabaseTable => TargetFamily::DatabaseTable,
    }];
    let max = limits.max_candidates;
    let (mut positions, mut truncated) = targets.by_id(&expected, &families, max);
    let mut score = 100_u8;
    if positions.is_empty() {
        (positions, truncated) = targets.by_names(std::slice::from_ref(&expected), &families, max);
    }
    if positions.is_empty()
        && let Some(owner) = owner
    {
        (positions, truncated) =
            targets.by_owner_terminal(owner, expected_terminal, &families, max);
        score = 97;
    }
    if positions.is_empty() {
        (positions, truncated) =
            targets.by_source_terminal(declaring_source, expected_terminal, &families, max);
        score = 90;
    }
    if positions.is_empty() {
        (positions, truncated) = targets.by_terminal(expected_terminal, &families, max);
        score = 70;
    }
    if positions.is_empty() {
        return Ok(Vec::new());
    }
    if truncated {
        return Err(FrameworkLimitError {
            limit: "max_candidates",
            maximum: limits.max_candidates,
            observed: limits.max_candidates.saturating_add(1),
        }
        .into());
    }
    let mut candidates = positions
        .into_iter()
        .map(|position| ResolutionCandidate {
            node_id: targets.targets[position].node.id.clone(),
            reason: if score >= 90 {
                "exact domain reference".to_owned()
            } else {
                "terminal domain reference".to_owned()
            },
            confidence: if score >= 90 {
                compass_model::provenance::EvidenceConfidence::Exact
            } else {
                compass_model::provenance::EvidenceConfidence::Ambiguous
            },
            score: Some(f64::from(score) / 100.0),
            anchor: node_anchor(targets.targets[position].node),
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    candidates.dedup_by(|left, right| left.node_id == right.node_id);
    Ok(candidates)
}

fn domain_node_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "event" => Some("event"),
        "message" => Some("message"),
        "topic" => Some("topic"),
        "queue" => Some("queue"),
        "job" => Some("job"),
        "bean_definition" => Some("component"),
        _ => None,
    }
}

fn domain_id(fact: &RawDomainFact) -> String {
    make_id(&[
        "framework-domain",
        &fact.framework,
        &fact.kind,
        fact.detail
            .get("transport")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        &fact.name,
        &fact.declaring_scope,
    ])
}

fn route_middleware_id(fact: &RawDomainFact) -> String {
    make_id(&[
        "framework-route-middleware",
        &fact.framework,
        &fact.anchor.source_file,
        &fact.name,
    ])
}

fn route_middleware_attributes(fact: &RawDomainFact) -> Map<String, Value> {
    Map::from_iter([
        ("label".into(), Value::String(fact.name.clone())),
        ("name".into(), Value::String(fact.name.clone())),
        (
            "qualified_name".into(),
            Value::String(format!("{}::middleware::{}", fact.framework, fact.name)),
        ),
        ("symbol_kind".into(), Value::String("component".to_owned())),
        ("file_type".into(), Value::String("code".to_owned())),
        (
            "component_type".into(),
            Value::String("route_middleware".to_owned()),
        ),
        (
            "roles".into(),
            Value::Array(vec![Value::String("middleware".to_owned())]),
        ),
        ("framework".into(), Value::String(fact.framework.clone())),
        (
            "declaring_scope".into(),
            Value::String(fact.declaring_scope.clone()),
        ),
        (
            "source_file".into(),
            Value::String(fact.anchor.source_file.clone()),
        ),
        (
            "source_location".into(),
            Value::String(format!("L{}", fact.anchor.start_line)),
        ),
        (
            "source_anchor".into(),
            serde_json::to_value(source_anchor(fact)).unwrap_or(Value::Null),
        ),
        (
            "_origin".into(),
            Value::String(fact.origin.as_str().to_owned()),
        ),
        (
            "extractor".into(),
            Value::String(format!("compass.frameworks.{}.domain", fact.framework)),
        ),
        ("confidence".into(), Value::String("EXTRACTED".to_owned())),
        (
            "rule".into(),
            Value::String("route-middleware-file-convention".to_owned()),
        ),
    ])
}

fn domain_attributes(
    fact: &RawDomainFact,
    symbol_kind: &str,
    state: ResolutionState,
) -> Map<String, Value> {
    let mut attributes = Map::from_iter([
        ("label".into(), Value::String(fact.name.clone())),
        ("name".into(), Value::String(fact.name.clone())),
        (
            "qualified_name".into(),
            Value::String(format!("{}::{}::{}", fact.framework, fact.kind, fact.name)),
        ),
        ("symbol_kind".into(), Value::String(symbol_kind.to_owned())),
        ("file_type".into(), Value::String("code".to_owned())),
        ("framework".into(), Value::String(fact.framework.clone())),
        (
            "declaring_scope".into(),
            Value::String(fact.declaring_scope.clone()),
        ),
        (
            "source_file".into(),
            Value::String(fact.anchor.source_file.clone()),
        ),
        (
            "source_location".into(),
            Value::String(format!("L{}", fact.anchor.start_line)),
        ),
        (
            "source_anchor".into(),
            serde_json::to_value(source_anchor(fact)).unwrap_or(Value::Null),
        ),
        (
            "_origin".into(),
            Value::String(fact.origin.as_str().to_owned()),
        ),
        (
            "extractor".into(),
            Value::String(format!("compass.frameworks.{}.domain", fact.framework)),
        ),
        (
            "resolution".into(),
            Value::String(resolution_name(state).to_owned()),
        ),
    ]);
    if symbol_kind == "job" {
        for key in ["schedule", "queue"] {
            if let Some(value) = fact.detail.get(key).cloned() {
                attributes.insert(key.to_owned(), value);
            }
        }
    } else {
        attributes.insert(
            "transport".into(),
            fact.detail
                .get("transport")
                .cloned()
                .unwrap_or_else(|| Value::String(fact.framework.clone())),
        );
        attributes.insert(
            "subject".into(),
            fact.detail
                .get("subject")
                .cloned()
                .unwrap_or_else(|| Value::String(fact.name.clone())),
        );
    }
    attributes
}

fn push_edge(
    extraction: &mut Extraction,
    existing: &mut HashSet<(String, String, String)>,
    source: &str,
    target: &str,
    relation: &str,
    fact: &RawDomainFact,
    state: ResolutionState,
) {
    if !existing.insert((source.to_owned(), target.to_owned(), relation.to_owned())) {
        return;
    }
    extraction.edges.push(RawEdgeRecord {
        source: source.to_owned(),
        target: target.to_owned(),
        attributes: Map::from_iter([
            ("relation".into(), Value::String(relation.to_owned())),
            (
                "source_file".into(),
                Value::String(fact.anchor.source_file.clone()),
            ),
            (
                "source_location".into(),
                Value::String(format!("L{}", fact.anchor.start_line)),
            ),
            (
                "source_anchor".into(),
                serde_json::to_value(source_anchor(fact)).unwrap_or(Value::Null),
            ),
            (
                "_origin".into(),
                Value::String(fact.origin.as_str().to_owned()),
            ),
            (
                "extractor".into(),
                Value::String(format!("compass.frameworks.{}.domain", fact.framework)),
            ),
            (
                "confidence".into(),
                Value::String(
                    if state == ResolutionState::Exact {
                        "EXTRACTED"
                    } else {
                        "AMBIGUOUS"
                    }
                    .to_owned(),
                ),
            ),
            ("weight".into(), Value::from(1.0)),
        ]),
    });
}

fn source_anchor(fact: &RawDomainFact) -> SourceAnchor {
    SourceAnchor {
        file: fact.anchor.source_file.clone(),
        start_byte: fact.anchor.start_byte,
        end_byte: fact.anchor.end_byte,
        start_line: fact.anchor.start_line,
        start_column: fact.anchor.start_column,
        end_line: fact.anchor.end_line,
        end_column: fact.anchor.end_column,
    }
}

fn node_anchor(node: &RawNodeRecord) -> Option<SourceAnchor> {
    let file = node.attributes.get("source_file").and_then(Value::as_str)?;
    let line = node
        .attributes
        .get("line_start")
        .and_then(Value::as_u64)
        .and_then(|line| u32::try_from(line).ok())
        .unwrap_or(1);
    Some(SourceAnchor {
        file: file.to_owned(),
        start_byte: node
            .attributes
            .get("start_byte")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        end_byte: node
            .attributes
            .get("end_byte")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        start_line: line,
        start_column: 0,
        end_line: line,
        end_column: 0,
    })
}

fn single_state(candidates: &[ResolutionCandidate]) -> ResolutionState {
    match candidates {
        [] => ResolutionState::Unresolved,
        [candidate]
            if candidate.confidence == compass_model::provenance::EvidenceConfidence::Exact =>
        {
            ResolutionState::Exact
        }
        _ => ResolutionState::Ambiguous,
    }
}

fn pair_state(left: &[ResolutionCandidate], right: &[ResolutionCandidate]) -> ResolutionState {
    let left = single_state(left);
    let right = single_state(right);
    if left == ResolutionState::Exact && right == ResolutionState::Exact {
        ResolutionState::Exact
    } else if left == ResolutionState::Ambiguous || right == ResolutionState::Ambiguous {
        ResolutionState::Ambiguous
    } else {
        ResolutionState::Unresolved
    }
}

fn resolution_name(state: ResolutionState) -> &'static str {
    match state {
        ResolutionState::Exact => "exact",
        ResolutionState::Ambiguous => "ambiguous",
        ResolutionState::Unresolved => "unresolved",
    }
}
