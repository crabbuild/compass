//! Dart framework-pack adapters.
//!
//! The established convention edges are projected by this framework-owned
//! bridge; these universal adapters intentionally return no
//! untyped facts. Their descriptors provide the frozen pack IDs and evidence
//! activation contract, while structural Dart relationships remain owned by
//! the universal producer.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use super::{RawFrameworkFact, UniversalDetectionContext};
use crate::{
    Extraction, ProjectEvidence, RawNodeRecord as NodeRecord, SemanticEvidenceBatch, SemanticRole,
    make_id,
};

pub(super) fn detect_flutter_navigation(
    context: &UniversalDetectionContext<'_, '_>,
) -> Vec<RawFrameworkFact> {
    let has_flutter_import = context.evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Import
            && occurrence.spelling.to_ascii_lowercase().contains("flutter")
    });
    let has_navigation_call = context.evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Call
            && matches!(
                occurrence.spelling.as_str(),
                "go" | "push" | "goNamed" | "pushNamed" | "replace" | "replaceNamed"
            )
    });
    if has_flutter_import && has_navigation_call {
        // The convention bridge emits the established anchored relation. An
        // empty typed-fact set here prevents a second, guessed framework edge.
        Vec::new()
    } else {
        Vec::new()
    }
}

pub(super) fn detect_bloc(context: &UniversalDetectionContext<'_, '_>) -> Vec<RawFrameworkFact> {
    let _activated = context.evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Call
            && matches!(occurrence.spelling.as_str(), "on" | "emit" | "add")
    });
    Vec::new()
}

pub(super) fn detect_riverpod(
    context: &UniversalDetectionContext<'_, '_>,
) -> Vec<RawFrameworkFact> {
    let _activated = context.evidence.occurrences.iter().any(|occurrence| {
        occurrence.role == SemanticRole::Call
            && matches!(occurrence.spelling.as_str(), "watch" | "read" | "listen")
    });
    Vec::new()
}

/// Preserve Dart's framework/domain conventions while the structural Dart
/// graph is hard-cut to universal evidence. These are deliberately limited to
/// convention-context edges (navigation, BLoC/Riverpod, and resource export)
/// and are marked as convention-origin facts; no legacy declaration or raw
/// call graph is copied into the universal extraction.
pub(super) fn append_convention_facts(
    path: &Path,
    source: &[u8],
    project: Option<&ProjectEvidence>,
    extraction: &mut Extraction,
) {
    let conventions = crate::dart_framework::extract(path, source);
    let Some(evidence) = extraction.semantic_evidence.as_ref() else {
        return;
    };
    let evidence_source_file = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "file")
        .map(|declaration| declaration.range.source_file.clone())
        .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"));
    let file_id = evidence
        .declarations
        .iter()
        .find(|declaration| declaration.kind == "file")
        .map(|declaration| declaration.graph_node_id.clone());
    let legacy_file_id = make_id(&[&path.to_string_lossy()]);
    let mut labels = HashMap::<String, Vec<String>>::new();
    for declaration in &evidence.declarations {
        labels
            .entry(declaration.name.clone())
            .or_default()
            .push(declaration.graph_node_id.clone());
        let terminal = declaration
            .qualified_name
            .rsplit(['.', ':'])
            .next()
            .unwrap_or_default();
        if terminal != declaration.name {
            labels
                .entry(terminal.to_owned())
                .or_default()
                .push(declaration.graph_node_id.clone());
        }
    }
    let framework_nodes = conventions
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let mut endpoint_ids = HashSet::new();
    let mut contextual_endpoint_ids = HashSet::new();
    let mut projected_edges = Vec::new();
    for mut edge in conventions.edges {
        let relation = edge.string("relation");
        let contextual = !edge.string("context").is_empty();
        if !contextual && relation != "exports" {
            continue;
        }
        if contextual && !contextual_fact_is_activated(&edge, evidence, project) {
            continue;
        }
        let source_id = project_dart_endpoint(
            &edge.source,
            &framework_nodes,
            &labels,
            file_id.as_deref(),
            &legacy_file_id,
        );
        let target_id = project_dart_endpoint(
            &edge.target,
            &framework_nodes,
            &labels,
            file_id.as_deref(),
            &legacy_file_id,
        );
        edge.source = source_id;
        edge.target = target_id;
        if contextual {
            contextual_endpoint_ids.insert(edge.source.clone());
            contextual_endpoint_ids.insert(edge.target.clone());
        }
        edge.attributes.insert(
            "source_file".to_owned(),
            Value::String(evidence_source_file.clone()),
        );
        edge.attributes.insert(
            "rule".to_owned(),
            Value::String(if contextual {
                format!("dart-{}", edge.string("context"))
            } else {
                "dart-resource-export".to_owned()
            }),
        );
        edge.attributes
            .insert("_origin".to_owned(), Value::String("convention".to_owned()));
        edge.attributes.insert(
            "extractor".to_owned(),
            Value::String("compass.languages.dart.framework".to_owned()),
        );
        endpoint_ids.insert(edge.source.clone());
        endpoint_ids.insert(edge.target.clone());
        projected_edges.push(edge);
    }
    projected_edges.sort_unstable_by(|left, right| {
        left.source
            .cmp(&right.source)
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| left.string("relation").cmp(&right.string("relation")))
            .then_with(|| {
                left.attributes
                    .get("start_byte")
                    .and_then(Value::as_u64)
                    .cmp(&right.attributes.get("start_byte").and_then(Value::as_u64))
            })
    });
    for edge in projected_edges {
        let duplicate = extraction.edges.iter().any(|existing| {
            existing.source == edge.source
                && existing.target == edge.target
                && existing.string("relation") == edge.string("relation")
                && existing.string("context") == edge.string("context")
                && existing.attributes.get("start_byte") == edge.attributes.get("start_byte")
                && existing.attributes.get("end_byte") == edge.attributes.get("end_byte")
        });
        if !duplicate {
            extraction.edges.push(edge);
        }
    }
    let existing_nodes = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    for node in conventions.nodes {
        if !endpoint_ids.contains(&node.id) || existing_nodes.contains(&node.id) {
            continue;
        }
        let mut node = node;
        if contextual_endpoint_ids.contains(&node.id) {
            node.attributes
                .insert("_origin".to_owned(), Value::String("convention".to_owned()));
            node.attributes.insert(
                "extractor".to_owned(),
                Value::String("compass.languages.dart.framework".to_owned()),
            );
        }
        extraction.nodes.push(node);
    }
}

fn contextual_fact_is_activated(
    edge: &crate::RawEdgeRecord,
    evidence: &SemanticEvidenceBatch,
    project: Option<&ProjectEvidence>,
) -> bool {
    let context = edge.string("context");
    let has_dependency =
        |markers: &[&str]| project.is_some_and(|project| project.has_any_dependency(markers));
    let has_import = |needles: &[&str]| {
        let occurrence_import = evidence.occurrences.iter().any(|occurrence| {
            occurrence.role == SemanticRole::Import
                && needles
                    .iter()
                    .any(|needle| occurrence.spelling.eq_ignore_ascii_case(needle))
        });
        let candidate_import = evidence.candidates.iter().any(|candidate| {
            matches!(
                candidate.relation,
                crate::evidence::CandidateRelation::Imports
                    | crate::evidence::CandidateRelation::Reexports
            ) && candidate
                .constraints
                .qualified_name
                .as_deref()
                .is_some_and(|target| {
                    needles
                        .iter()
                        .any(|needle| import_target_matches(target, needle))
                })
        });
        occurrence_import || candidate_import
    };
    let has_call = |names: &[&str]| {
        evidence.occurrences.iter().any(|occurrence| {
            occurrence.role == SemanticRole::Call && names.contains(&occurrence.spelling.as_str())
        })
    };
    match context.as_str() {
        "route_path" | "route_const" | "route_object" => {
            (has_dependency(&["flutter"]) || has_import(&["flutter"]))
                && has_call(&[
                    "go",
                    "push",
                    "goNamed",
                    "pushNamed",
                    "replace",
                    "replaceNamed",
                ])
        }
        "bloc_event" | "emit_state" | "bloc_add_event" | "bloc_widget_binding" | "bloc_lookup" => {
            (has_dependency(&["bloc", "flutter_bloc"]) || has_import(&["bloc", "flutter_bloc"]))
                && (has_call(&["on", "emit", "add"])
                    || evidence.occurrences.iter().any(|occurrence| {
                        matches!(
                            occurrence.role,
                            SemanticRole::TypeReference | SemanticRole::MemberAccess
                        ) && [
                            "BlocBuilder",
                            "BlocListener",
                            "BlocConsumer",
                            "BlocProvider",
                        ]
                        .contains(&occurrence.spelling.as_str())
                    }))
        }
        "riverpod_reference" => {
            (has_dependency(&["riverpod", "hooks_riverpod"])
                || has_import(&["riverpod", "hooks_riverpod"]))
                && has_call(&["watch", "read", "listen"])
        }
        _ => false,
    }
}

fn import_target_matches(target: &str, marker: &str) -> bool {
    let target = target.to_ascii_lowercase();
    let marker = marker.to_ascii_lowercase();
    target == marker
        || target.starts_with(&format!("{marker}/"))
        || target.starts_with(&format!("package:{marker}/"))
}

fn project_dart_endpoint(
    endpoint: &str,
    framework_nodes: &HashMap<&str, &NodeRecord>,
    labels: &HashMap<String, Vec<String>>,
    file_id: Option<&str>,
    legacy_file_id: &str,
) -> String {
    if let Some(file_id) = file_id
        && endpoint == legacy_file_id
    {
        return file_id.to_owned();
    }
    let Some(node) = framework_nodes.get(endpoint) else {
        return endpoint.to_owned();
    };
    let label = node.label().trim_matches(['.', '(', ')']).to_owned();
    labels
        .get(&label)
        .filter(|ids| ids.len() == 1)
        .and_then(|ids| ids.first())
        .cloned()
        .unwrap_or_else(|| endpoint.to_owned())
}
