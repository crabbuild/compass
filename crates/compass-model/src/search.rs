use std::collections::{BTreeMap, BTreeSet};

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::code_graph::{EdgeKind, GraphDocument};
use crate::provenance::{EvidenceConfidence, EvidenceOrigin};

/// Return deterministic normalized full and identifier-subword terms.
///
/// The full tokens preserve compatibility with existing search indexes while
/// the subwords make `OpenRepository`, `session_state`, and acronym-bearing
/// identifiers discoverable by their constituent words.
#[must_use]
pub fn identifier_search_terms(value: &str) -> BTreeSet<String> {
    let normalized = value
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .collect::<String>();
    let mut terms = normalized
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect::<BTreeSet<_>>();

    for word in split_identifier_words(&normalized)
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
    {
        terms.insert(word.to_lowercase());
    }
    terms
}

/// Build exact identifier-concept postings for trusted direct callers.
///
/// Each concept maps to source-backed callable IDs that directly call a
/// target whose terminal symbol name contains that identifier concept.
/// Qualified-name owner and namespace terms are intentionally excluded so
/// callers do not inherit unrelated concepts from the target's container.
/// Parallel call occurrences collapse to one source ID per concept.
#[must_use]
pub fn direct_call_source_identifier_postings(
    graph: &GraphDocument,
) -> BTreeMap<String, Vec<String>> {
    let mut postings = BTreeMap::<String, BTreeSet<String>>::new();
    for (concept, source_id, _) in direct_call_source_identifier_targets(graph) {
        postings.entry(concept).or_default().insert(source_id);
    }
    postings
        .into_iter()
        .map(|(concept, source_ids)| (concept, source_ids.into_iter().collect()))
        .collect()
}

/// Return deterministic trusted `(concept, source ID, target ID)` evidence.
///
/// Parallel calls and a target name that emits the same normalized concept
/// more than once collapse to one triple. Consumers can therefore count
/// distinct supporting callees without inflating evidence multiplicity.
#[must_use]
pub fn direct_call_source_identifier_targets(
    graph: &GraphDocument,
) -> BTreeSet<(String, String, String)> {
    let nodes = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut targets = BTreeSet::new();
    for edge in &graph.links {
        if !is_exact_nonheuristic_direct_call(edge) {
            continue;
        }
        let (Some(source), Some(target)) = (
            nodes.get(edge.source.as_str()),
            nodes.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if !source.kind.is_callable() || source.source_file().is_none_or(|source| source.is_empty())
        {
            continue;
        }
        for concept in identifier_search_terms(&target.name) {
            targets.insert((concept, source.id.clone(), target.id.clone()));
        }
    }
    targets
}

/// Whether an occurrence is trusted as an exact direct call for relationship
/// discovery. Empty evidence remains accepted for legacy structural graphs;
/// any explicit evidence must be exact and nonheuristic.
#[must_use]
pub fn is_exact_nonheuristic_direct_call(edge: &crate::code_graph::EdgeRecord) -> bool {
    edge.kind == EdgeKind::Calls
        && edge.evidence.iter().all(|evidence| {
            evidence.origin != EvidenceOrigin::Heuristic
                && evidence.confidence == EvidenceConfidence::Exact
        })
}

fn split_identifier_words(value: &str) -> String {
    let characters = value.chars().collect::<Vec<_>>();
    let mut words = String::with_capacity(value.len());
    for (index, &character) in characters.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|at| characters.get(at));
        let next = characters.get(index + 1);
        let boundary = character.is_uppercase()
            && previous.is_some_and(|value| {
                value.is_lowercase()
                    || value.is_numeric()
                    || (value.is_uppercase() && next.is_some_and(|next| next.is_lowercase()))
            });
        if boundary {
            words.push(' ');
        }
        words.push(character);
    }
    words
}

#[cfg(test)]
mod tests {
    use crate::code_graph::{
        BuildMetadata, EdgeKind, EdgeRecord, GraphDocument, NodeKind, NodeRecord,
    };
    use crate::provenance::{EvidenceConfidence, EvidenceOrigin, Provenance, SourceAnchor};

    use super::{
        direct_call_source_identifier_postings, direct_call_source_identifier_targets,
        identifier_search_terms,
    };

    #[test]
    fn preserves_full_tokens_and_adds_identifier_subwords() {
        assert_eq!(
            identifier_search_terms("HTTPCheckpoint_session_state"),
            [
                "checkpoint",
                "http",
                "httpcheckpoint_session_state",
                "session",
                "state",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
    }

    #[test]
    fn direct_call_postings_dedupe_parallel_edges_and_reject_untrusted_sources() {
        let source = |id: &str, kind: NodeKind, file: Option<&str>| NodeRecord {
            id: id.to_owned(),
            kind,
            roles: Vec::new(),
            name: id.to_owned(),
            qualified_name: format!("fixture::{id}"),
            language: Some("rust".to_owned()),
            framework: None,
            source: file.map(|file| SourceAnchor {
                file: file.to_owned(),
                start_byte: 0,
                end_byte: 1,
                start_line: 1,
                start_column: 0,
                end_line: 1,
                end_column: 1,
            }),
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        };
        let edge = |id: &str, source: &str, confidence: Option<EvidenceConfidence>| EdgeRecord {
            id: id.to_owned(),
            key: id.to_owned(),
            source: source.to_owned(),
            target: "target".to_owned(),
            kind: EdgeKind::Calls,
            occurrence_rule: None,
            relationship_site: None,
            details: None,
            evidence: confidence
                .map(|confidence| Provenance {
                    origin: EvidenceOrigin::Ast,
                    extractor: "test".to_owned(),
                    confidence,
                    rule: None,
                    anchors: Vec::new(),
                    wiring_site: None,
                    score: None,
                    candidates: Vec::new(),
                })
                .into_iter()
                .collect(),
            weight: None,
            context: None,
            deferred: false,
            diagnostics: Vec::new(),
        };
        let mut graph = GraphDocument::empty_v1(BuildMetadata {
            builder_version: "test".to_owned(),
            schema_fingerprint: "schema".to_owned(),
            source_tree_digest: "tree".to_owned(),
            configuration_digest: "config".to_owned(),
            generation_id: "generation".to_owned(),
            source_commit: None,
        });
        graph.nodes = vec![
            source("caller", NodeKind::Function, Some("src/lib.rs")),
            source("inferred", NodeKind::Function, Some("src/lib.rs")),
            source("ambiguous", NodeKind::Function, Some("src/lib.rs")),
            source("heuristic", NodeKind::Function, Some("src/lib.rs")),
            source("mixed", NodeKind::Function, Some("src/lib.rs")),
            source("noncallable", NodeKind::Class, Some("src/lib.rs")),
            source("sourceless", NodeKind::Function, None),
            source("target", NodeKind::Function, Some("src/lib.rs")),
        ];
        graph.nodes[7].name = "CreateRepositoryState".to_owned();
        graph.nodes[7].qualified_name =
            "namespace::CheckpointOwner::CreateRepositoryState".to_owned();
        let mut heuristic = edge("heuristic", "heuristic", Some(EvidenceConfidence::Exact));
        heuristic.evidence[0].origin = EvidenceOrigin::Heuristic;
        let mut mixed = edge("mixed", "mixed", Some(EvidenceConfidence::Exact));
        mixed.evidence.extend(
            edge(
                "mixed-inferred",
                "mixed",
                Some(EvidenceConfidence::Inferred),
            )
            .evidence,
        );
        graph.links = vec![
            edge("exact-a", "caller", Some(EvidenceConfidence::Exact)),
            edge("exact-b", "caller", None),
            edge("inferred", "inferred", Some(EvidenceConfidence::Inferred)),
            edge(
                "ambiguous",
                "ambiguous",
                Some(EvidenceConfidence::Ambiguous),
            ),
            heuristic,
            mixed,
            edge("noncallable", "noncallable", None),
            edge("sourceless", "sourceless", None),
        ];

        let postings = direct_call_source_identifier_postings(&graph);
        for concept in ["create", "repository", "state"] {
            assert_eq!(postings.get(concept), Some(&vec!["caller".to_owned()]));
        }
        for namespace_only in ["namespace", "checkpoint", "owner"] {
            assert!(
                !postings.contains_key(namespace_only),
                "qualified-name-only term {namespace_only:?} must not become caller evidence"
            );
        }
        let targets = direct_call_source_identifier_targets(&graph);
        assert_eq!(
            targets
                .iter()
                .filter(|(term, source, _)| term == "create" && source == "caller")
                .count(),
            1,
            "parallel calls must not duplicate supporting target identity"
        );
    }
}
