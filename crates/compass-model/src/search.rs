use std::collections::{BTreeMap, BTreeSet};

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::code_graph::{EdgeKind, GraphDocument};
use crate::provenance::{EvidenceConfidence, EvidenceOrigin};

pub const OPERATION_ROLE_TOKENS: &[&str] = &[
    "builder", "factory", "handler", "manager", "provider", "service",
];

/// Structural relationships that can establish a source-backed dependency
/// on a named target. The structural relationship commands use the bounded
/// postings derived from these kinds so importer recall does not depend on a
/// narrower direct-call-only index.
pub const RELATIONSHIP_SEARCH_EDGE_KINDS: &[EdgeKind] = &[
    EdgeKind::Calls,
    EdgeKind::Imports,
    EdgeKind::Exports,
    EdgeKind::References,
    EdgeKind::Aliases,
    EdgeKind::RoutesTo,
    EdgeKind::DependsOn,
];

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

/// Bounded prose terms for document discovery. This is recall evidence, not
/// declaration identity or an alias. Older store indexes require a rebuild.
#[must_use]
pub fn document_search_terms(node: &crate::code_graph::NodeRecord) -> BTreeSet<String> {
    let content = match &node.details {
        Some(crate::code_graph::NodeDetails::Document(details)) => details.content.as_deref(),
        Some(crate::code_graph::NodeDetails::Resource(details)) => details.content.as_deref(),
        _ => None,
    };
    let text = content
        .unwrap_or_default()
        .chars()
        .take(8192)
        .collect::<String>();
    identifier_search_terms(&text)
        .into_iter()
        .take(512)
        .collect()
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

/// Return exact source postings for all source-backed dependency
/// relationships, not only direct calls.  Existing callers use the direct
/// call helpers for compatibility; relationship commands use this broader
/// index to recover import/reference consumers whose canonical edge points at
/// a file or module owner rather than the resolved declaration.
#[must_use]
pub fn relationship_source_identifier_postings(
    graph: &GraphDocument,
) -> BTreeMap<String, Vec<String>> {
    let mut postings = BTreeMap::<String, BTreeSet<String>>::new();
    for (concept, source_id, _) in relationship_source_identifier_targets(graph) {
        postings.entry(concept).or_default().insert(source_id);
    }
    postings
        .into_iter()
        .map(|(concept, source_ids)| (concept, source_ids.into_iter().collect()))
        .collect()
}

/// Return deterministic `(concept, source ID, target ID)` evidence for all
/// trusted source-backed dependency relationships.
#[must_use]
pub fn relationship_source_identifier_targets(
    graph: &GraphDocument,
) -> BTreeSet<(String, String, String)> {
    let nodes = graph
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let mut aliases_by_target = BTreeMap::<&str, BTreeSet<&str>>::new();
    for edge in &graph.links {
        if edge.kind == EdgeKind::Aliases
            && let Some(alias) = nodes.get(edge.source.as_str())
        {
            aliases_by_target
                .entry(edge.target.as_str())
                .or_default()
                .insert(alias.name.as_str());
        }
    }
    let mut targets = BTreeSet::new();
    for edge in &graph.links {
        if !RELATIONSHIP_SEARCH_EDGE_KINDS.contains(&edge.kind)
            || !is_exact_nonheuristic_relationship(edge)
        {
            continue;
        }
        let (Some(source), Some(target)) = (
            nodes.get(edge.source.as_str()),
            nodes.get(edge.target.as_str()),
        ) else {
            continue;
        };
        if source
            .source_file()
            .is_none_or(|source_file| source_file.is_empty())
        {
            continue;
        }
        let mut terms = identifier_search_terms(&target.name);
        terms.extend(identifier_search_terms(&target.qualified_name));
        if let Some(source_file) = target.source_file() {
            terms.extend(identifier_search_terms(source_file));
        }
        for alias in aliases_by_target
            .get(target.id.as_str())
            .into_iter()
            .flat_map(|aliases| aliases.iter())
        {
            terms.extend(identifier_search_terms(alias));
        }
        for concept in terms {
            targets.insert((concept, source.id.clone(), target.id.clone()));
        }
    }
    targets
}

/// Whether an edge is safe to use as exact relationship-search evidence.
/// Empty evidence is retained for legacy graph documents; explicit heuristic
/// evidence is excluded from the exact postings.
#[must_use]
pub fn is_exact_nonheuristic_relationship(edge: &crate::code_graph::EdgeRecord) -> bool {
    edge.evidence.iter().all(|evidence| {
        evidence.origin != EvidenceOrigin::Heuristic
            && evidence.confidence == EvidenceConfidence::Exact
    })
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
        identifier_search_terms, relationship_source_identifier_postings,
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

        let mut aliased_importer =
            source("aliased-importer", NodeKind::Variable, Some("src/app.ts"));
        aliased_importer.name = "pkg_import".to_owned();
        let mut alias_edge = edge("alias-edge", "aliased-importer", None);
        alias_edge.target = "target".to_owned();
        alias_edge.kind = EdgeKind::Aliases;
        graph.nodes.push(aliased_importer);
        graph.links.push(alias_edge);
        let relationship_postings = relationship_source_identifier_postings(&graph);
        assert!(
            relationship_postings
                .get("namespace")
                .is_some_and(|sources| sources.contains(&"caller".to_owned()))
        );
        assert!(
            relationship_postings
                .get("pkg")
                .is_some_and(|sources| sources.contains(&"aliased-importer".to_owned()))
        );
    }
}
