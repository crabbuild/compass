use std::collections::{HashMap, HashSet};

use compass_languages::{
    Extraction, FrameworkLimitError, FrameworkLimits, RawDomainFact, RawEdgeRecord,
    RawFrameworkFact, RawNodeRecord, make_id,
};
use compass_model::provenance::{ResolutionCandidate, ResolutionState, SourceAnchor};
use serde_json::{Map, Value, json};

use super::FrameworkResolutionError;

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
    let facts = extraction
        .framework_facts
        .iter()
        .filter_map(|fact| match fact {
            RawFrameworkFact::Domain(fact) => Some(fact),
            RawFrameworkFact::Route(_) => None,
        })
        .collect::<Vec<_>>();
    limits.check_facts(facts.len())?;
    facts
        .into_iter()
        .map(|fact| resolve_one(fact, extraction, limits))
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
            if let ([model], [table]) = (
                resolved.source_candidates.as_slice(),
                resolved.target_candidates.as_slice(),
            ) {
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
    extraction: &Extraction,
    limits: FrameworkLimits,
) -> Result<ResolvedDomainFact, FrameworkResolutionError> {
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
        let sources = resolve_reference(extraction, model, TargetKind::Type, limits)?;
        let targets = resolve_reference(extraction, &table, TargetKind::DatabaseTable, limits)?;
        return Ok(ResolvedDomainFact {
            fact: fact.clone(),
            state: pair_state(&sources, &targets),
            source_candidates: sources,
            target_candidates: targets,
        });
    }
    let reference = fact
        .detail
        .get("handler_reference")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let candidates = resolve_reference(extraction, reference, TargetKind::Callable, limits)?;
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
    extraction: &Extraction,
    reference: &str,
    target_kind: TargetKind,
    limits: FrameworkLimits,
) -> Result<Vec<ResolutionCandidate>, FrameworkResolutionError> {
    if reference.trim().is_empty() {
        return Ok(Vec::new());
    }
    let expected = normalize(reference);
    let expected_terminal = terminal(&expected);
    let owner = expected.rsplit_once('.').map(|(owner, _)| owner);
    let by_id = extraction
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut parents = HashMap::<&str, Vec<&RawNodeRecord>>::new();
    for edge in &extraction.edges {
        if matches!(
            edge.attributes.get("relation").and_then(Value::as_str),
            Some("contains" | "method" | "defines")
        ) && let Some(parent) = by_id.get(edge.source.as_str())
        {
            parents
                .entry(edge.target.as_str())
                .or_default()
                .push(parent);
        }
    }
    let mut matches = Vec::new();
    for node in extraction
        .nodes
        .iter()
        .filter(|node| accepts(node, target_kind))
    {
        let names = [
            node.string("qualified_name"),
            node.string("name"),
            node.label().to_owned(),
        ];
        let normalized = names.iter().map(|name| normalize(name)).collect::<Vec<_>>();
        let owner_match = owner.is_some_and(|owner| {
            parents
                .get(node.id.as_str())
                .into_iter()
                .flatten()
                .flat_map(|parent| {
                    [
                        parent.string("qualified_name"),
                        parent.string("name"),
                        parent.label().to_owned(),
                    ]
                })
                .map(|name| normalize(&name))
                .any(|name| name == owner || terminal(&name) == terminal(owner))
        });
        let score = if normalized.iter().any(|name| name == &expected) {
            100_u8
        } else if owner_match
            && normalized
                .iter()
                .any(|name| terminal(name) == expected_terminal)
        {
            97
        } else if normalized
            .iter()
            .any(|name| terminal(name) == expected_terminal)
        {
            70
        } else {
            continue;
        };
        matches.push((score, node));
    }
    let Some(best) = matches.iter().map(|(score, _)| *score).max() else {
        return Ok(Vec::new());
    };
    let mut candidates = matches
        .into_iter()
        .filter(|(score, _)| *score == best)
        .map(|(score, node)| ResolutionCandidate {
            node_id: node.id.clone(),
            reason: if score >= 97 {
                "exact domain reference".to_owned()
            } else {
                "terminal domain reference".to_owned()
            },
            confidence: if score >= 97 {
                compass_model::provenance::EvidenceConfidence::Exact
            } else {
                compass_model::provenance::EvidenceConfidence::Ambiguous
            },
            score: Some(f64::from(score) / 100.0),
            anchor: node_anchor(node),
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    candidates.dedup_by(|left, right| left.node_id == right.node_id);
    if candidates.len() > limits.max_candidates {
        return Err(FrameworkLimitError {
            limit: "max_candidates",
            maximum: limits.max_candidates,
            observed: candidates.len(),
        }
        .into());
    }
    Ok(candidates)
}

fn accepts(node: &RawNodeRecord, target: TargetKind) -> bool {
    let kind = node
        .attributes
        .get("symbol_kind")
        .or_else(|| node.attributes.get("type"))
        .and_then(Value::as_str);
    match target {
        TargetKind::Callable => matches!(kind, Some("function" | "method")),
        TargetKind::Type => matches!(
            kind,
            Some("class" | "struct" | "interface" | "trait" | "protocol" | "enum")
        ),
        TargetKind::DatabaseTable => matches!(kind, Some("database_table" | "table")),
    }
}

fn domain_node_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "event" => Some("event"),
        "message" => Some("message"),
        "topic" => Some("topic"),
        "queue" => Some("queue"),
        "job" => Some("job"),
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

fn normalize(value: &str) -> String {
    value
        .trim()
        .trim_start_matches(['&', '*'])
        .trim_end_matches("()")
        .replace(['\\', '#'], ".")
        .replace("::", ".")
}

fn terminal(value: &str) -> &str {
    value.rsplit('.').next().unwrap_or(value)
}

fn single_state(candidates: &[ResolutionCandidate]) -> ResolutionState {
    match candidates.len() {
        0 => ResolutionState::Unresolved,
        1 => ResolutionState::Exact,
        _ => ResolutionState::Ambiguous,
    }
}

fn pair_state(left: &[ResolutionCandidate], right: &[ResolutionCandidate]) -> ResolutionState {
    if left.len() == 1 && right.len() == 1 {
        ResolutionState::Exact
    } else if left.len() > 1 || right.len() > 1 {
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
