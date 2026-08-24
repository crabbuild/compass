//! Project-wide publication for generic framework relation facts.
//!
//! Packs only provide syntax-backed identity hints. This module proves both
//! endpoints against the shared target index and publishes no edge when a
//! target is missing or ambiguous.

use std::path::Path;

use ahash::AHashSet as HashSet;
use compass_languages::{
    Extraction, FrameworkLimits, RawEdgeRecord, RawFrameworkAnchor, RawFrameworkFact,
};
use compass_model::provenance::{EvidenceConfidence, ResolutionCandidate, ResolutionState};
use serde_json::{Map, Value, json};

use super::target_index::{
    FrameworkTargetIndex, TargetFamily, normalize_reference, source_key, terminal_name,
};

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum FrameworkRelationResolutionError {
    #[error("framework relation limit exceeded: {0}")]
    Limit(#[from] compass_languages::FrameworkLimitError),
}

pub fn resolve_and_publish(
    extraction: &mut Extraction,
    limits: FrameworkLimits,
    root: Option<&Path>,
) -> Result<(), FrameworkRelationResolutionError> {
    let relation_count = extraction
        .framework_facts
        .iter()
        .filter(|fact| matches!(fact, RawFrameworkFact::Relation(_)))
        .count();
    limits.check_relation_facts(relation_count)?;
    if relation_count == 0 {
        return Ok(());
    }
    let target_extraction = super::materialize_universal_framework_targets(extraction);
    let targets = FrameworkTargetIndex::new_with_root(&target_extraction, root);
    let mut edges = extraction
        .edges
        .iter()
        .map(|edge| {
            (
                edge.source.clone(),
                edge.target.clone(),
                edge.string("relation"),
                edge_anchor_key(edge),
            )
        })
        .collect::<HashSet<_>>();
    let mut diagnostics = Vec::new();
    let mut additions = Vec::new();
    for fact in extraction.framework_facts.iter().filter_map(|fact| {
        if let RawFrameworkFact::Relation(fact) = fact {
            Some(fact)
        } else {
            None
        }
    }) {
        let relation = normalize_relation(&fact.relation);
        let Some(relation) = relation else {
            diagnostics.push(json!({
                "kind": "unsupported_framework_relation",
                "framework": fact.framework,
                "relation": fact.relation,
                "sourceFile": fact.anchor.source_file,
                "line": fact.anchor.start_line,
            }));
            continue;
        };
        let (source_candidates, source_truncated) = resolve_hint(
            fact.source_reference.as_deref(),
            &fact.anchor,
            &targets,
            limits.max_candidates,
            root,
        );
        let (target_candidates, target_truncated) = resolve_hint(
            fact.target_hint.as_deref(),
            fact.target_anchor.as_ref().unwrap_or(&fact.anchor),
            &targets,
            limits.max_candidates,
            root,
        );
        let source_state = candidate_state(&source_candidates, source_truncated);
        let target_state = candidate_state(&target_candidates, target_truncated);
        if source_state != ResolutionState::Exact || target_state != ResolutionState::Exact {
            diagnostics.push(json!({
                "kind": "unresolved_framework_relation",
                "framework": fact.framework,
                "relation": relation,
                "source": fact.source_reference,
                "target": fact.target_hint,
                "sourceResolution": resolution_name(source_state),
                "targetResolution": resolution_name(target_state),
                "sourceCandidatesTruncated": source_truncated,
                "targetCandidatesTruncated": target_truncated,
                "sourceCandidates": source_candidates,
                "targetCandidates": target_candidates,
                "sourceFile": fact.anchor.source_file,
                "line": fact.anchor.start_line,
            }));
            continue;
        }
        let (Some(source), Some(target)) = (source_candidates.first(), target_candidates.first())
        else {
            continue;
        };
        let source_anchor = source_anchor_value(&fact.anchor);
        let key = (
            source.node_id.clone(),
            target.node_id.clone(),
            relation.to_owned(),
            anchor_key(&source_anchor),
        );
        if !edges.insert(key) {
            continue;
        }
        let mut attributes = Map::from_iter([
            ("relation".to_owned(), Value::String(relation.to_owned())),
            (
                "framework".to_owned(),
                Value::String(fact.framework.clone()),
            ),
            (
                "source_file".to_owned(),
                Value::String(fact.anchor.source_file.clone()),
            ),
            (
                "source_location".to_owned(),
                Value::String(format!("L{}", fact.anchor.start_line)),
            ),
            ("source_anchor".to_owned(), source_anchor.clone()),
            (
                "confidence".to_owned(),
                Value::String("EXTRACTED".to_owned()),
            ),
            (
                "_origin".to_owned(),
                Value::String(fact.origin.as_str().to_owned()),
            ),
            (
                "ambiguity_policy".to_owned(),
                Value::String(fact.ambiguity_policy.clone()),
            ),
            (
                "rule".to_owned(),
                Value::String(format!("framework-relation:{}", fact.relation)),
            ),
        ]);
        if let Some(target_anchor) = fact.target_anchor.as_ref() {
            attributes.insert(
                "target_anchor".to_owned(),
                source_anchor_value(target_anchor),
            );
        }
        additions.push(RawEdgeRecord {
            source: source.node_id.clone(),
            target: target.node_id.clone(),
            attributes,
        });
    }
    extraction.edges.extend(additions);
    if !diagnostics.is_empty() {
        extraction.extensions.insert(
            "framework_relation_diagnostics".to_owned(),
            Value::Array(diagnostics),
        );
    }
    Ok(())
}

fn edge_anchor_key(edge: &RawEdgeRecord) -> String {
    edge.attributes
        .get("source_anchor")
        .map(anchor_key)
        .or_else(|| {
            let file = edge
                .attributes
                .get("source_file")
                .or_else(|| edge.attributes.get("sourceFile"))
                .and_then(Value::as_str)?;
            let start = edge
                .attributes
                .get("start_byte")
                .or_else(|| edge.attributes.get("startByte"))
                .and_then(Value::as_u64)?;
            let end = edge
                .attributes
                .get("end_byte")
                .or_else(|| edge.attributes.get("endByte"))
                .and_then(Value::as_u64)?;
            Some(format!("{file}:{start}:{end}"))
        })
        .unwrap_or_default()
}

fn anchor_key(value: &Value) -> String {
    if let Value::Object(anchor) = value {
        let file = anchor
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let start = anchor
            .get("start_byte")
            .or_else(|| anchor.get("startByte"))
            .and_then(Value::as_u64)
            .unwrap_or_default();
        let end = anchor
            .get("end_byte")
            .or_else(|| anchor.get("endByte"))
            .and_then(Value::as_u64)
            .unwrap_or_default();
        return format!("{file}:{start}:{end}");
    }
    value.to_string()
}

fn normalize_relation(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "decorates" | "decorated_by" => Some("decorates"),
        "routes_to" | "routes" => Some("routes_to"),
        "registers" => Some("registers"),
        "handles" => Some("handles"),
        "publishes" => Some("publishes"),
        "subscribes" => Some("subscribes"),
        "produces" => Some("produces"),
        "consumes" => Some("consumes"),
        "schedules" => Some("schedules"),
        "triggers" => Some("triggers"),
        "depends_on" | "depends-on" => Some("depends_on"),
        "maps_to" | "maps-to" => Some("maps_to"),
        "renders" => Some("renders"),
        _ => None,
    }
}

fn resolve_hint(
    hint: Option<&str>,
    anchor: &RawFrameworkAnchor,
    targets: &FrameworkTargetIndex<'_>,
    limit: usize,
    root: Option<&Path>,
) -> (Vec<ResolutionCandidate>, bool) {
    let families = [
        TargetFamily::Route,
        TargetFamily::Callable,
        TargetFamily::Type,
        TargetFamily::DatabaseTable,
    ];
    let Some(hint) = hint.map(str::trim).filter(|hint| !hint.is_empty()) else {
        let (nodes, truncated) = targets.by_source_file(&anchor.source_file, limit);
        return (
            nodes
                .into_iter()
                .map(|node| candidate(node.id.clone(), node, "source-file anchor"))
                .collect(),
            truncated,
        );
    };
    let normalized = normalize_reference(hint);
    let (mut positions, mut truncated) = targets.by_id(&normalized, &families, limit);
    if positions.is_empty() {
        let (next_positions, next_truncated) =
            targets.by_names(std::slice::from_ref(&normalized), &families, limit);
        positions = next_positions;
        truncated = next_truncated;
    }
    if positions.is_empty() {
        let terminal = terminal_name(&normalized).to_owned();
        let (next_positions, next_truncated) =
            targets.by_source_terminal(&anchor.source_file, &terminal, &families, limit);
        positions = next_positions;
        truncated = next_truncated;
    }
    if positions.is_empty() {
        let (next_positions, next_truncated) =
            targets.by_terminal(terminal_name(&normalized), &families, limit);
        positions = next_positions;
        truncated = next_truncated;
    }
    if positions.len() > 1 {
        let anchored = positions
            .iter()
            .copied()
            .filter(|position| {
                let Some(node) = targets.targets.get(*position).map(|target| target.node) else {
                    return false;
                };
                let Some(file) = node.attributes.get("source_file").and_then(Value::as_str) else {
                    return false;
                };
                let start = node
                    .attributes
                    .get("start_byte")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX);
                let end = node
                    .attributes
                    .get("end_byte")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                source_key(file, root) == source_key(&anchor.source_file, root)
                    && start <= anchor.start_byte
                    && end >= anchor.end_byte
            })
            .collect::<Vec<_>>();
        if anchored.len() == 1 {
            positions = anchored;
        }
    }
    let mut output = positions
        .into_iter()
        .filter_map(|position| targets.targets.get(position))
        .map(|target| {
            candidate(
                target.node.id.clone(),
                target.node,
                "framework relation hint",
            )
        })
        .collect::<Vec<_>>();
    output.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    output.dedup_by(|left, right| left.node_id == right.node_id);
    (output, truncated)
}

fn candidate(
    id: String,
    node: &compass_languages::RawNodeRecord,
    reason: &str,
) -> ResolutionCandidate {
    ResolutionCandidate {
        node_id: id,
        reason: reason.to_owned(),
        confidence: EvidenceConfidence::Exact,
        score: Some(1.0),
        anchor: node_anchor(node),
    }
}

fn candidate_state(candidates: &[ResolutionCandidate], truncated: bool) -> ResolutionState {
    if truncated {
        return ResolutionState::Ambiguous;
    }
    match candidates {
        [candidate] if candidate.confidence == EvidenceConfidence::Exact => ResolutionState::Exact,
        [] => ResolutionState::Unresolved,
        [_] => ResolutionState::Unresolved,
        _ => ResolutionState::Ambiguous,
    }
}

fn node_anchor(
    node: &compass_languages::RawNodeRecord,
) -> Option<compass_model::provenance::SourceAnchor> {
    let file = node.attributes.get("source_file").and_then(Value::as_str)?;
    Some(compass_model::provenance::SourceAnchor {
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
        start_line: node
            .attributes
            .get("line_start")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(1),
        start_column: node
            .attributes
            .get("column_start")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        end_line: node
            .attributes
            .get("line_end")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(1),
        end_column: node
            .attributes
            .get("column_end")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
    })
}

fn source_anchor_value(anchor: &RawFrameworkAnchor) -> Value {
    serde_json::to_value(compass_model::provenance::SourceAnchor {
        file: anchor.source_file.clone(),
        start_byte: anchor.start_byte,
        end_byte: anchor.end_byte,
        start_line: anchor.start_line,
        start_column: anchor.start_column,
        end_line: anchor.end_line,
        end_column: anchor.end_column,
    })
    .unwrap_or(Value::Null)
}

fn resolution_name(state: ResolutionState) -> &'static str {
    match state {
        ResolutionState::Exact => "exact",
        ResolutionState::Ambiguous => "ambiguous",
        ResolutionState::Unresolved => "unresolved",
    }
}

#[cfg(test)]
mod tests {
    use compass_model::provenance::{EvidenceConfidence, ResolutionCandidate, ResolutionState};

    use super::candidate_state;

    #[test]
    fn truncated_candidate_sets_never_publish_as_exact() {
        let candidate = ResolutionCandidate {
            node_id: "node".to_owned(),
            reason: "bounded lookup".to_owned(),
            confidence: EvidenceConfidence::Exact,
            score: Some(1.0),
            anchor: None,
        };
        assert_eq!(
            candidate_state(&[candidate], true),
            ResolutionState::Ambiguous
        );
    }
}
