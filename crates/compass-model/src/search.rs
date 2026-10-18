use std::collections::{BTreeMap, BTreeSet};

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::code_graph::{EdgeKind, GraphDocument, NodeKind, NodeRecord};
use crate::provenance::{EvidenceConfidence, EvidenceOrigin};

pub const OPERATION_ROLE_TOKENS: &[&str] = &[
    "builder", "factory", "handler", "manager", "provider", "service",
];
pub const SEARCH_FIELD_MAX_BYTES: usize = 8192;
pub const SEARCH_FIELD_MAX_TERMS: usize = 512;
pub const SEARCH_MAX_ALIASES: usize = 8;

/// Bounded source-evidence projection shared by native query backends.
/// Fields describe recall evidence, never declaration aliases or graph edges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchDocument {
    pub fields: BTreeMap<&'static str, BTreeSet<String>>,
    pub truncated: bool,
}

impl SearchDocument {
    #[must_use]
    pub fn terms(&self) -> BTreeSet<String> {
        self.fields
            .values()
            .flat_map(|terms| terms.iter().cloned())
            .collect()
    }

    /// Reserved implementation channel, using exactly the same enriched
    /// projection as document-frequency statistics and relevance scoring.
    #[must_use]
    pub fn primary_terms(&self, kind: NodeKind) -> BTreeSet<String> {
        if is_primary_search_kind(kind) {
            self.terms()
                .into_iter()
                .map(|term| format!("__compass_primary_v1_{term}"))
                .collect()
        } else {
            BTreeSet::new()
        }
    }
}

/// Declaration kinds that can own implementation-search evidence. This is a
/// retrieval category and never changes graph kind or structural confidence.
#[must_use]
pub fn is_primary_search_kind(kind: NodeKind) -> bool {
    kind.is_callable()
        || matches!(
            kind,
            NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::Protocol
                | NodeKind::Enum
                | NodeKind::TypeAlias
                | NodeKind::ConfigKey
        )
}

/// Project only metadata already published with its source owner. General
/// comments are deliberately not associated by proximity at query time.
#[must_use]
pub fn search_document(node: &NodeRecord) -> SearchDocument {
    search_document_with_aliases(node, std::iter::empty())
}

/// Enrich a declaration with names from its incoming alias relationships.
/// Read at most nine entries. More than eight aliases or 8 KiB of alias text
/// omits the whole alias field and reports truncation, rather than selecting
/// an order-dependent subset. Duplicate alias relationships count toward the
/// work bound; callers must preserve their multiplicity on both backends.
#[must_use]
pub fn search_document_with_aliases<'a>(
    node: &NodeRecord,
    aliases: impl IntoIterator<Item = &'a str>,
) -> SearchDocument {
    use crate::code_graph::NodeDetails;
    let mut document = SearchDocument::default();
    let mut field = |name, text: &str| {
        let mut end = text.len().min(SEARCH_FIELD_MAX_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let bounded = &text[..end];
        let terms = identifier_search_terms(bounded)
            .into_iter()
            .map(crate::canonical_code_token)
            .collect::<BTreeSet<_>>();
        document.truncated |= end < text.len() || terms.len() > SEARCH_FIELD_MAX_TERMS;
        document.fields.insert(
            name,
            terms.into_iter().take(SEARCH_FIELD_MAX_TERMS).collect(),
        );
    };
    field("name", &node.name);
    field("qualified_name", &node.qualified_name);
    field("path", node.source_file().unwrap_or_default());
    field("kind", node.kind.as_str());
    field("language", node.language.as_deref().unwrap_or_default());
    let roles = node
        .roles
        .iter()
        .map(|role| format!("{role:?}"))
        .collect::<BTreeSet<_>>();
    field("roles", &roles.into_iter().collect::<Vec<_>>().join(" "));
    match &node.details {
        Some(NodeDetails::Symbol(details)) => {
            field(
                "signature",
                details.signature.as_deref().unwrap_or_default(),
            );
            field(
                "declaring_type",
                details.declaring_type.as_deref().unwrap_or_default(),
            );
        }
        Some(NodeDetails::Document(details)) => {
            field("document", details.content.as_deref().unwrap_or_default())
        }
        Some(NodeDetails::Resource(details)) => {
            field("document", details.content.as_deref().unwrap_or_default())
        }
        _ => {}
    }
    let aliases = aliases
        .into_iter()
        .take(SEARCH_MAX_ALIASES + 1)
        .collect::<Vec<_>>();
    let alias_bytes = aliases
        .iter()
        .fold(0_usize, |total, alias| total.saturating_add(alias.len()));
    if aliases.len() > SEARCH_MAX_ALIASES || alias_bytes > SEARCH_FIELD_MAX_BYTES {
        document.truncated = true;
    } else if !aliases.is_empty() {
        // Sorting before normalization makes the projection independent of
        // graph insertion order and of alias occurrence ordering.
        let aliases = aliases.into_iter().collect::<BTreeSet<_>>();
        let terms = aliases
            .into_iter()
            .flat_map(identifier_search_terms)
            .map(crate::canonical_code_token)
            .collect::<BTreeSet<_>>();
        document.truncated |= terms.len() > SEARCH_FIELD_MAX_TERMS;
        document.fields.insert(
            "alias",
            terms.into_iter().take(SEARCH_FIELD_MAX_TERMS).collect(),
        );
    }
    document
}

/// Reserved native recall channel for implementations and type declarations.
/// A common term's parameter leaves cannot exhaust this posting quota.
#[must_use]
pub fn primary_evidence_terms(node: &NodeRecord) -> BTreeSet<String> {
    search_document(node).primary_terms(node.kind)
}

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
        SEARCH_FIELD_MAX_BYTES, SEARCH_FIELD_MAX_TERMS, direct_call_source_identifier_postings,
        direct_call_source_identifier_targets, identifier_search_terms,
        relationship_source_identifier_postings, search_document_with_aliases,
    };

    fn projection_node() -> NodeRecord {
        NodeRecord {
            id: "target".to_owned(),
            kind: NodeKind::Function,
            roles: vec![
                crate::code_graph::NodeRole::RouteHandler,
                crate::code_graph::NodeRole::Service,
            ],
            name: "execute".to_owned(),
            qualified_name: "app::execute".to_owned(),
            language: None,
            framework: None,
            source: None,
            details: None,
            evidence: Vec::new(),
            coverage: Vec::new(),
            diagnostics: Vec::new(),
            community: None,
        }
    }

    #[test]
    fn fielded_projection_preserves_alias_origin_kind_roles_and_canonical_order() {
        let mut node = projection_node();
        let a = search_document_with_aliases(&node, ["RenewalGateway", "PaymentEndpoint"]);
        node.roles.reverse();
        let b = search_document_with_aliases(&node, ["PaymentEndpoint", "RenewalGateway"]);
        assert_eq!(a, b);
        assert!(!a.truncated);
        assert!(a.fields["alias"].contains("renewal"));
        assert!(a.fields["kind"].contains("function"));
        assert!(a.fields["roles"].contains("route"));
        assert!(a.fields["roles"].contains("handler"));
        assert!(!a.fields["name"].contains("renewal"));
        assert!(
            a.primary_terms(NodeKind::Function)
                .contains("__compass_primary_v1_renewal")
        );
        assert!(a.primary_terms(NodeKind::Parameter).is_empty());
    }

    #[test]
    fn alias_projection_omits_overflow_instead_of_selecting_an_arbitrary_subset() {
        let node = projection_node();
        let aliases = (0..9)
            .map(|index| format!("Alias{index}"))
            .collect::<Vec<_>>();
        let a = search_document_with_aliases(&node, aliases.iter().map(String::as_str));
        let b = search_document_with_aliases(&node, aliases.iter().rev().map(String::as_str));
        assert_eq!(a, b);
        assert!(a.truncated);
        assert!(!a.fields.contains_key("alias"));
        let oversized = "x".repeat(SEARCH_FIELD_MAX_BYTES + 1);
        let too_large = search_document_with_aliases(&node, [oversized.as_str()]);
        assert!(too_large.truncated);
        assert!(!too_large.fields.contains_key("alias"));
        let read_count = std::cell::Cell::new(0);
        let unbounded =
            std::iter::repeat("alias").inspect(|_| read_count.set(read_count.get() + 1));
        assert!(search_document_with_aliases(&node, unbounded).truncated);
        assert_eq!(read_count.get(), 9);
    }

    #[test]
    fn source_fields_bound_utf8_bytes_and_unique_terms_independently() {
        let mut node = projection_node();
        node.name = format!("{}🙂beyondlimit", "é".repeat(4095));
        let document = super::search_document(&node);
        assert!(document.truncated);
        assert!(!document.fields["name"].contains("beyondlimit"));
        let many_terms = (0..600)
            .map(|index| format!("token{index:03}"))
            .collect::<Vec<_>>()
            .join(" ");
        let document = search_document_with_aliases(&projection_node(), [many_terms.as_str()]);
        assert!(document.truncated);
        assert_eq!(document.fields["alias"].len(), SEARCH_FIELD_MAX_TERMS);
    }

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
