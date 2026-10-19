//! Source-owned fielded retrieval for broad implementation questions.
//! Candidate reads are quota-bounded before fusion. Scores are retrieval
//! evidence, never a structural assertion or a calibrated probability.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use compass_model::code_graph::{EdgeKind, NodeKind, NodeRecord, NodeRole};
use compass_model::provenance::{EvidenceConfidence, EvidenceOrigin, SourceAnchor};
use compass_model::query_contract::{
    CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, ConceptMatch, ConceptMatchMethod,
    DISCOVERY_QUERY_SCHEMA_V1, DiscoveryDirection, DiscoveryOmissions, DiscoveryQueryRequest,
    DiscoveryQueryResponse, DiscoveryScope, DiscoveryScoreTier, DiscoverySeed, DiscoverySeedSource,
    DiscoveryStats, MAX_INDEXED_CANDIDATE_NODES_READ, MAX_INDEXED_CANDIDATE_PROBES,
    QueryDiagnostic, QueryDiagnosticCode, QueryEdge, QueryNode, SearchHit, SearchRequest,
    discovery_scope_matches,
};
use compass_model::search::{
    SearchDocument, identifier_search_terms, is_primary_search_kind, search_document,
    search_document_with_aliases,
};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::code_query::{
    PinnedDiscoveryBackend, normalize_symbol, query_edge, query_node, search_query_terms,
};
use crate::intent::{NaturalQueryIntent, plan_natural_query};
use crate::{CodeQueryEngine, QueryError, QueryErrorKind};

mod excerpts;
pub use excerpts::{RankedExcerptKind, RankedSourceExcerpt};
mod call_context;
mod compounds;
mod ownership;
mod relationships;

const POSTING_QUOTA: usize = 256;
// At most three public routing checks each perform eight bounded exact-name
// lookups (nine decoded records including lookahead). Reserve that work from
// the shared hard ceiling; search-phase counters describe the work below.
const FIELD_READ_CEILING: u64 = MAX_INDEXED_CANDIDATE_NODES_READ - 216;
const FIELD_PROBE_CEILING: u64 = MAX_INDEXED_CANDIDATE_PROBES - 24;
// An ordinal implementation-inspection preference, below exact name/ID tiers.
// It is not additional confidence or evidence of execution.
const IMPLEMENTATION_ACTION_PREFERENCE: f64 = 10_000.0;

/// Automatic navigation must not amplify weak lexical roots. Explicit callers
/// may inspect all witnessed paths without changing a candidate's relevance.
#[derive(Clone, Copy)]
pub(crate) enum CallContextRoots {
    HighRelevance,
    AllWitnessed,
}

fn is_retrieval_action(term: &str) -> bool {
    crate::ranking::canonical_predicate_token(term).is_some()
        || matches!(term, "encode" | "serialize")
}

// Admission roles are deliberately separate from ranking action equivalence.
// A decision verb cannot corroborate the topic of another action, but this
// classification adds no synonym, operation root, or inspection preference.
fn is_decision_predicate(term: &str) -> bool {
    static FORMS: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    FORMS
        .get_or_init(|| {
            [
                "reject",
                "rejects",
                "rejected",
                "rejecting",
                "accept",
                "accepts",
                "accepted",
                "accepting",
                "deny",
                "denies",
                "denied",
                "denying",
                "allow",
                "allows",
                "allowed",
                "allowing",
                "block",
                "blocks",
                "blocked",
                "blocking",
                "permit",
                "permits",
                "permitted",
                "permitting",
                "refuse",
                "refuses",
                "refused",
                "refusing",
            ]
            .into_iter()
            .map(|word| compass_model::canonical_code_token(word.to_owned()))
            .collect()
        })
        .contains(term)
}

fn is_independent_topic(
    term: &str,
    predicates: &BTreeSet<String>,
    exceptions: &BTreeSet<String>,
) -> bool {
    // These qualifiers already cannot corroborate literal/compound topics.
    // Apply that boundary to ordinary and implementation topics too, so a
    // load-only phrase cannot use only as independent evidence for load.
    exceptions.contains(term)
        || (!predicates.contains(term) && !matches!(term, "all" | "only" | "new" | "made"))
}

fn possible_noun_spelling(word: &str) -> bool {
    matches!(word, "block" | "blocks" | "permit" | "permits")
}

fn declared_topic(node: &NodeRecord, term: &str) -> bool {
    relationships::exact_ast(node)
        && [&node.name, &node.qualified_name].into_iter().any(|name| {
            normalize_symbol(name)
                .replace("::", ".")
                .split('.')
                .any(|part| part == term)
        })
}

// Only the actual name of an exact native nominal declaration may supply this
// noun pointer. A qualified namespace, callable, path, alias or prose cannot.
// This does not inherit the noun into methods or establish requested behavior.
fn nominal_topic(node: &NodeRecord, term: &str) -> bool {
    matches!(node.kind, NodeKind::Class | NodeKind::Struct)
        && relationships::exact_ast(node)
        && compass_model::canonical_code_token(normalize_symbol(&node.name)) == term
}

fn test_context(node: &NodeRecord) -> bool {
    if node.roles.contains(&NodeRole::Test)
        || node.roles.contains(&NodeRole::Fixture)
        || node.name.starts_with("test_")
    {
        return true;
    }
    let mut parts = node.source_file().unwrap_or_default().rsplit(['/', '\\']);
    let file = parts.next().unwrap_or_default().to_ascii_lowercase();
    file.starts_with("test_")
        || file.ends_with("_test.go")
        || file.ends_with("_test.py")
        || file.ends_with("_test.rs")
        || file.contains(".test.")
        || file.contains(".spec.")
        || parts.any(|part| {
            matches!(
                part.to_ascii_lowercase().as_str(),
                "test" | "tests" | "fixtures" | "__tests__"
            )
        })
}

fn is_query_scaffolding(word: &str) -> bool {
    matches!(
        word.to_ascii_lowercase().as_str(),
        "where"
            | "what"
            | "which"
            | "when"
            | "who"
            | "how"
            | "does"
            | "can"
            | "find"
            | "show"
            | "explain"
            | "please"
            | "locate"
            | "list"
            | "describe"
            | "identify"
            | "tell"
            | "give"
            | "not"
            | "never"
            | "without"
            | "and"
            | "or"
    )
}

fn explicit_identifiers(question: &str) -> BTreeSet<String> {
    let mut contexts = BTreeSet::new();
    let content = terms(question);
    for word in question.split_whitespace() {
        let word = word.trim_matches(|character: char| {
            !character.is_alphanumeric() && !matches!(character, '_' | '/' | '\\')
        });
        let qualified = word.contains("::") || word.contains(['.', '/', '\\']);
        let named_casing = word.chars().filter(|c| c.is_uppercase()).count() > 1
            && plural_acronym_stem(word).is_none()
            && (word.chars().any(char::is_lowercase)
                || content.contains(&compass_model::canonical_code_token(word.to_lowercase())))
            && !is_query_scaffolding(word);
        if (qualified || word.contains('_') || named_casing)
            && word.chars().all(|character| {
                character.is_alphanumeric() || matches!(character, '_' | ':' | '.' | '/' | '\\')
            })
            && word.chars().any(char::is_alphabetic)
            && word
                .split([':', '.', '/', '\\'])
                .any(|part| part.len() >= 2)
        {
            contexts.insert(word.to_owned());
        }
    }
    for delimiter in ['`', '"'] {
        for quoted in question.split(delimiter).skip(1).step_by(2) {
            if quoted.split_whitespace().count() == 1 {
                contexts.insert(quoted.trim().to_owned());
            }
        }
    }
    contexts
}

fn named_contexts(question: &str, content: &[String]) -> BTreeSet<String> {
    question
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .filter(|word| word.len() >= 3 && word.chars().next().is_some_and(char::is_uppercase))
        .filter(|word| content.contains(&compass_model::canonical_code_token(word.to_lowercase())))
        .filter(|word| !is_query_scaffolding(word))
        .filter(|word| plural_acronym_stem(word).is_none())
        .map(str::to_owned)
        .chain(explicit_identifiers(question))
        .flat_map(|word| compass_model::identifier_tokens(&word.replace('_', " ")))
        .map(compass_model::canonical_code_token)
        .collect()
}

fn has_explicit_identity(node: &NodeRecord, identifier: &str) -> bool {
    let identifier = normalize_symbol(identifier).replace("::", ".");
    [&node.name, &node.qualified_name].into_iter().any(|name| {
        let name = normalize_symbol(name).replace("::", ".");
        name == identifier
            || name.starts_with(&format!("{identifier}."))
            || name.ends_with(&format!(".{identifier}"))
            || name.contains(&format!(".{identifier}."))
    }) || node.source_file().is_some_and(|path| {
        let path = path.replace('\\', "/");
        let identifier = identifier.replace('\\', "/");
        path == identifier || path.ends_with(&format!("/{identifier}"))
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelevanceBand {
    High,
    Medium,
    Low,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedCandidate {
    pub node: QueryNode,
    pub score: f64,
    pub relevance: RelevanceBand,
    pub matched_terms: Vec<String>,
    pub matched_fields: Vec<String>,
    pub reasons: Vec<String>,
    pub supporting_sources: Vec<SourceAnchor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excerpts: Vec<RankedSourceExcerpt>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedDiscoveryResponse {
    pub discovery: DiscoveryQueryResponse,
    pub candidates: Vec<RankedCandidate>,
    pub eligible_count: Option<u64>,
    pub bounded: bool,
}

/// Developer evidence for the earliest missing stage, not a relevance oracle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedRetrievalTrace {
    pub schema: String,
    pub graph_present: bool,
    pub admitted: bool,
    pub rank: Option<usize>,
    pub seed: bool,
    pub first_page: bool,
    pub bounded: bool,
    pub target_details: Option<RankedTargetTrace>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RankedFilterReason {
    MissingNamedContext,
    MissingSubjectEvidence,
    NoMatchedConcept,
    InsufficientSupportedCoverage,
    RepresentedOwnedDocument,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedTermTrace {
    pub term: String,
    pub variants: Vec<String>,
    pub document_frequency: u64,
    pub inverse_document_frequency: f64,
    pub matched_fields: Vec<String>,
    pub action: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedTargetTrace {
    pub terms: Vec<RankedTermTrace>,
    pub field_lengths: BTreeMap<String, usize>,
    pub supported_concepts: usize,
    pub supported_coverage: f64,
    pub exact_id: bool,
    pub exact_name: bool,
    pub semantic_score: f64,
    pub score: Option<f64>,
    pub filter_reason: Option<RankedFilterReason>,
}

struct Retrieval {
    ranked: Vec<RankedCandidate>,
    admitted: BTreeSet<String>,
    bounded: bool,
    eligible: usize,
    reads: u64,
    probes: u64,
    missing_subjects: Vec<String>,
    negative_qualification: bool,
    temporal_qualification: BTreeSet<String>,
    legacy: bool,
    relationship_legacy: bool,
    parameter_roles_unavailable: bool,
    parameter_roles_partial: bool,
    ranks: BTreeMap<String, usize>,
    recall_time: Duration,
    ranking_time: Duration,
    concept_matches: Vec<ConceptMatch>,
    witness_nodes: Vec<QueryNode>,
    witness_edges: Vec<QueryEdge>,
    target_details: Option<RankedTargetTrace>,
}

pub(crate) fn is_broad_question(query: &str) -> bool {
    query.split_whitespace().count() >= 3
        && search_query_terms(query).is_ok_and(|terms| !terms.is_empty())
        && plan_natural_query(query).is_ok_and(|plan| {
            matches!(
                plan.intent(),
                NaturalQueryIntent::Search | NaturalQueryIntent::Fallback
            )
        })
}

fn alternatives(term: &str) -> Vec<String> {
    let related: &[&str] = match term {
        "error" | "exception" | "diagnostic" => &["error", "exception", "diagnostic"],
        "configuration" | "config" => &["configuration", "config", "setting"],
        "save" | "persist" | "store" => &["save", "persist", "store"],
        "render" => &["render"],
        "server" => &["server", "serve"],
        "start" => &["start", "begin", "run"],
        "enforc" | "enforce" => &["enforc", "enforce"],
        _ => &[],
    };
    std::iter::once(term.to_owned())
        .chain(related.iter().map(|s| (*s).to_owned()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

// Forward spellings are recall hints, not adjective/behavior equivalence.
// Keep literal identifiers untouched and never reverse-strip an unknown word.
fn comparative_spellings(terms: &[String]) -> BTreeMap<String, BTreeSet<String>> {
    let mut variants = BTreeMap::<String, BTreeSet<String>>::new();
    for word in terms {
        if !(4..=32).contains(&word.len()) || !word.bytes().all(|b| b.is_ascii_lowercase()) {
            continue;
        }
        let suffixes = if word.ends_with('e') {
            ["r", "st"]
        } else {
            ["er", "est"]
        };
        let group = variants.entry(word.clone()).or_default();
        for suffix in suffixes {
            group.insert(compass_model::canonical_code_token(format!(
                "{word}{suffix}"
            )));
        }
    }
    variants
}

fn plural_acronym_stem(word: &str) -> Option<&str> {
    let stem = word.strip_suffix('s')?;
    (stem.len() >= 2 && stem.bytes().all(|byte| byte.is_ascii_uppercase())).then_some(stem)
}

/// Lexical matches cannot prove ordering. Keep that condition in the original
/// request and diagnostics, rather than treating the connective as a subject.
/// Quoted and qualified identities retain their literal search terms.
fn retrieval_content(question: &str) -> (String, BTreeSet<String>) {
    let identifier_character = |character: char| {
        character.is_alphanumeric() || matches!(character, '_' | ':' | '.' | '/' | '\\' | '-')
    };
    let mut content = String::with_capacity(question.len());
    let mut conditions = BTreeSet::new();
    let mut quoted = None;
    for part in question.split_inclusive(|character: char| !identifier_character(character)) {
        let word = part.trim_end_matches(|character: char| !identifier_character(character));
        let lower = word.to_ascii_lowercase();
        if quoted.is_none() && matches!(lower.as_str(), "before" | "after" | "during" | "until") {
            conditions.insert(lower);
            content.push(' ');
        } else {
            content.push_str(word);
        }
        let suffix = &part[word.len()..];
        content.push_str(suffix);
        for delimiter in suffix
            .chars()
            .filter(|character| matches!(character, '`' | '"'))
        {
            if quoted == Some(delimiter) {
                quoted = None;
            } else if quoted.is_none() {
                quoted = Some(delimiter);
            }
        }
    }
    (content, conditions)
}

fn terms(question: &str) -> Vec<String> {
    // Natural prose uses plural acronyms such as APIs and TTLs. Code's camel
    // case splitter would otherwise turn TTLs into `tt` and `ls`. Explicit
    // quoted/qualified identities are still checked against the original text.
    let normalized = question
        .split_inclusive(|character: char| !character.is_alphanumeric())
        .map(|part| {
            let word = part.trim_end_matches(|character: char| !character.is_alphanumeric());
            plural_acronym_stem(word).map_or_else(
                || part.to_owned(),
                |stem| format!("{}{}", stem.to_ascii_lowercase(), &part[word.len()..]),
            )
        })
        .collect::<String>();
    crate::query_terms(&normalized)
        .into_iter()
        .filter(|term| {
            !matches!(
                term.as_str(),
                "builtin"
                    | "built"
                    | "cli"
                    | "output"
                    | "code"
                    | "implement"
                    | "implementation"
                    | "define"
                    | "defin"
                    | "defined"
                    | "definition"
                    | "declaration"
                    | "declar"
                    | "located"
                    | "locat"
                    | "location"
                    | "happen"
                    | "please"
            )
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn frequency(
    engine: &CodeQueryEngine,
    backend: &PinnedDiscoveryBackend<'_>,
    term: &str,
) -> Result<Option<(u64, u64)>, QueryError> {
    let frequency = match backend {
        PinnedDiscoveryBackend::Store(reader) => reader
            .fielded_document_frequency(term)
            .map(|frequency| frequency.map(|df| (reader.manifest().node_count, df)))
            .map_err(crate::code_query::snapshot_error),
        PinnedDiscoveryBackend::Materialized { graph, .. } => {
            let connection = engine.connection.as_ref().ok_or_else(|| {
                QueryError::new(
                    QueryErrorKind::Internal,
                    "query_index_missing",
                    "fielded search requires the native search index",
                )
            })?;
            let df = connection
                .query_row(
                    "SELECT frequency FROM fielded_stats WHERE term=?1",
                    params![term],
                    |row| row.get::<_, u64>(0),
                )
                .optional()
                .map_err(crate::code_query::sql_error)?
                .unwrap_or(0);
            if df > graph.nodes.len() as u64 {
                return Err(QueryError::new(
                    QueryErrorKind::CorruptArtifact,
                    "invalid_fielded_frequency",
                    "Fielded document frequency exceeds the graph node count; rebuild the query cache",
                ));
            }
            Ok(Some((graph.nodes.len() as u64, df)))
        }
    }?;
    frequency
        .map(|(n, df)| {
            let extra = engine
                .source_query
                .as_ref()
                .and_then(|index| index.frequencies.get(term))
                .copied()
                .unwrap_or(0);
            let df = df.checked_add(extra).filter(|df| *df <= n).ok_or_else(|| {
                QueryError::new(
                    QueryErrorKind::CorruptArtifact,
                    "invalid_source_document_frequency",
                    "Combined fielded frequency exceeds graph document count",
                )
            })?;
            Ok((n, df))
        })
        .transpose()
}

fn matching(document: &SearchDocument, variants: &[String]) -> Vec<&'static str> {
    document
        .fields
        .iter()
        .filter(|(_, field)| variants.iter().any(|term| field.contains(term)))
        .map(|(name, _)| *name)
        .collect()
}

fn field_weight(name: &str) -> f64 {
    match name {
        "name" => 4.0,
        "qualified_name" => 1.5,
        "signature" => 2.0,
        "declaring_type" => 1.0,
        "document" => 1.0,
        "document_link" => 0.35,
        "alias" => 1.5,
        _ => 0.5,
    }
}

fn enforce_ranked_response_budget(response: &RankedDiscoveryResponse) -> Result<(), QueryError> {
    struct ByteBudget {
        bytes: usize,
        maximum: usize,
        exceeded: bool,
    }
    impl std::io::Write for ByteBudget {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.bytes = self.bytes.saturating_add(buffer.len());
            if self.bytes > self.maximum {
                self.exceeded = true;
                return Err(std::io::Error::other("ranked response byte limit"));
            }
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut budget = ByteBudget {
        bytes: 0,
        maximum: response.discovery.limits.max_response_bytes as usize,
        exceeded: false,
    };
    let encoded = serde_json::to_writer(&mut budget, response);
    if budget.exceeded {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "ranked_response_limit",
            "Ranked evidence or context exceeds the response byte bound; narrow scope, reduce max-nodes, or omit context",
        ));
    }
    encoded.map_err(|error| {
        QueryError::new(
            QueryErrorKind::Internal,
            "ranked_serialization",
            error.to_string(),
        )
    })
}

impl CodeQueryEngine {
    pub(crate) fn is_fielded_question(&self, question: &str) -> Result<bool, QueryError> {
        if !is_broad_question(question) {
            return Ok(false);
        }
        let backend = self.backend.pin_discovery()?;
        let subject = plan_natural_query(question)
            .ok()
            .filter(|plan| {
                plan.intent() == NaturalQueryIntent::Search
                    && plan.routes_to_typed_query()
                    && plan.operands().len() == 1
            })
            .map(|plan| crate::ranking::normalize_symbol_name(&plan.operands()[0]));
        let mut seen = BTreeSet::new();
        for literal in subject
            .iter()
            .cloned()
            .chain(crate::discovery::literal_identifier_terms(question))
            .filter(|literal| seen.insert(literal.clone()))
            .take(8)
        {
            self.check_deadline()?;
            let (nodes, truncated) = backend.nodes_by_normalized_name(&literal, 8)?;
            if truncated {
                return Ok(false);
            }
            if nodes.iter().any(|node| {
                let allowed_kind = if subject.as_deref() == Some(literal.as_str()) {
                    node.kind != NodeKind::Resource
                } else {
                    !matches!(
                        node.kind,
                        NodeKind::Resource
                            | NodeKind::File
                            | NodeKind::Module
                            | NodeKind::Package
                            | NodeKind::Namespace
                    )
                };
                allowed_kind
                    && (crate::ranking::normalize_symbol_name(&node.name) == literal
                        || crate::ranking::normalize_symbol_name(&node.qualified_name) == literal)
            }) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn retrieve_broad(
        &self,
        request: &SearchRequest,
        scopes: &[DiscoveryScope],
        check: &dyn Fn() -> Result<(), QueryError>,
        admit: &dyn Fn(&NodeRecord) -> bool,
        trace_target: Option<&str>,
        retain_excerpts: bool,
    ) -> Result<Retrieval, QueryError> {
        let started = Instant::now();
        crate::code_query::validate_limits(&request.limits)?;
        let _ = search_query_terms(&request.query)?;
        let negative_qualification = request
            .query
            .split(|character: char| !character.is_alphanumeric() && character != '_')
            .any(|word| {
                matches!(
                    word.to_ascii_lowercase().as_str(),
                    "not" | "never" | "without"
                )
            });
        check()?;
        let backend = self.backend.pin_discovery()?;
        let (content_question, temporal_qualification) = retrieval_content(&request.query);
        let original_terms = terms(&content_question);
        let named_contexts = named_contexts(&request.query, &original_terms);
        let explicit_identifiers = explicit_identifiers(&request.query);
        let compounds = if explicit_identifiers.is_empty() {
            compounds::Compounds::parse(&content_question)
        } else {
            compounds::Compounds::default()
        };
        if original_terms.len() > compass_model::query_contract::MAX_INDEXED_QUERY_TERMS {
            return Err(QueryError::new(
                QueryErrorKind::ExpansionLimit,
                "broad_concept_limit",
                "Broad retrieval supports at most 32 normalized content concepts; narrow the question rather than silently dropping its meaning",
            ));
        }
        let wants_documents = original_terms.iter().any(|term| {
            matches!(
                term.as_str(),
                "document" | "documentation" | "design" | "rationale" | "readme" | "comment"
            )
        });
        let mut action_terms = original_terms
            .iter()
            .filter(|term| is_retrieval_action(term))
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut semantic_groups = BTreeMap::<String, BTreeSet<String>>::new();
        let mut covered = BTreeSet::new();
        let expansions = crate::concepts::expansions(&request.query);
        for (phrase, alternative) in &expansions {
            let phrase_terms = terms(phrase);
            if phrase_terms.is_empty()
                || !phrase_terms
                    .iter()
                    .all(|term| original_terms.contains(term))
            {
                continue;
            }
            covered.extend(phrase_terms.iter().cloned());
            let entry = semantic_groups.entry(phrase.clone()).or_default();
            entry.extend(phrase_terms);
            entry.extend(
                identifier_search_terms(alternative)
                    .into_iter()
                    .map(compass_model::canonical_code_token),
            );
        }
        for term in &original_terms {
            if !covered.contains(term) {
                semantic_groups.insert(term.clone(), alternatives(term).into_iter().collect());
            }
        }
        // Conservative spelling alternatives preserve silent-e verbs and
        // nouns such as cookies without rewriting strict/historical tokens.
        for raw in identifier_search_terms(&request.query) {
            if raw.is_ascii() && raw.len() > 4 {
                let canonical = compass_model::canonical_code_token(raw.clone());
                if let Some(variants) = semantic_groups.get_mut(&canonical) {
                    if let Some(stem) = raw.strip_suffix('s') {
                        variants.insert(compass_model::canonical_code_token(stem.to_owned()));
                    }
                    if let Some(stem) = raw.strip_suffix("ed").or_else(|| raw.strip_suffix("ing")) {
                        let alternate = format!("{stem}e");
                        if is_retrieval_action(&alternate) {
                            action_terms.insert(canonical);
                        }
                        variants.insert(alternate);
                    }
                }
            }
        }
        // Preserve all query groups and literal spellings. Only the admission
        // checks for independent topic evidence use this larger role set.
        let mut topic_predicates = action_terms.clone();
        topic_predicates.extend(
            original_terms
                .iter()
                .filter(|term| is_decision_predicate(term))
                .cloned(),
        );
        // Preserve raw occurrences: blocks/blocked may collapse to one group,
        // but a noun witness cannot establish the separate requested predicate.
        let decision_spellings = compass_model::identifier_tokens(&request.query)
            .into_iter()
            .filter(|raw| is_decision_predicate(&compass_model::canonical_code_token(raw.clone())))
            .collect::<Vec<_>>();
        let possible_nouns = decision_spellings
            .iter()
            .filter(|raw| possible_noun_spelling(raw))
            .cloned()
            .map(compass_model::canonical_code_token)
            .collect::<BTreeSet<_>>();
        // A distinct decision spelling still requests a separate predicate:
        // blocks/rejected and permits/allowed must not disappear into a vacuous
        // facet listing simply because neither is an existing ranking action.
        let separate_predicate = decision_spellings
            .iter()
            .any(|raw| !possible_noun_spelling(raw));
        let noun_subject_requested =
            !possible_nouns.is_empty() && (!action_terms.is_empty() || separate_predicate);
        let comparative_variants = comparative_spellings(&original_terms);
        let mut groups = Vec::new();
        let mut vocabulary_fallbacks = BTreeMap::<String, (Vec<String>, Vec<String>)>::new();
        let mut literal_supported_terms = BTreeSet::new();
        let mut missing_subjects = Vec::new();
        let mut legacy = false;
        for (term, variants) in semantic_groups {
            check()?;
            let mut variants = variants.into_iter().collect::<Vec<_>>();
            let mut df = 0;
            let mut count = 1;
            let mut group_legacy = false;
            for variant in &variants {
                if let Some((n, frequency)) = frequency(self, &backend, variant)? {
                    count = n.max(1);
                    df = df.max(frequency);
                } else {
                    legacy = true;
                    group_legacy = true;
                }
            }
            // Only corpus-supported terms can justify an implementation. An
            // absent concrete subject is not replaced by a ubiquitous verb.
            if df == 0 && !legacy && !action_terms.contains(&term) {
                missing_subjects.push(term.clone());
            }
            if df > 0 {
                literal_supported_terms.insert(term.clone());
            }
            // Absent-word paraphrases require verified zero frequency.
            // Conventional code spellings remain useful when indexed elsewhere;
            // explicit identities and unknown statistics stay literal.
            let candidate_aliases = crate::concepts::code_abbreviation_alternatives(&term)
                .iter()
                .copied()
                .chain(
                    crate::concepts::operation_vocabulary_alternatives(&term)
                        .iter()
                        .copied(),
                )
                .chain(if df == 0 {
                    crate::concepts::unindexed_vocabulary_alternatives(&term)
                        .iter()
                        .copied()
                } else {
                    [].iter().copied()
                })
                .chain(
                    comparative_variants
                        .get(&term)
                        .into_iter()
                        .flatten()
                        .map(String::as_str),
                );
            if !group_legacy
                && !wants_documents
                && named_contexts.is_empty()
                && explicit_identifiers.is_empty()
            {
                let literal = variants.clone();
                let mut aliases = Vec::new();
                let mut alias_df = 0;
                for alias in candidate_aliases {
                    check()?;
                    match frequency(self, &backend, alias)? {
                        Some((n, frequency)) if frequency > 0 => {
                            count = n.max(1);
                            alias_df = alias_df.max(frequency);
                            aliases.push(alias.to_owned());
                        }
                        Some(_) => {}
                        None => group_legacy = true,
                    }
                }
                legacy |= group_legacy;
                if !group_legacy && !aliases.is_empty() {
                    variants.extend(aliases.iter().cloned());
                    variants.sort();
                    variants.dedup();
                    df = df.max(alias_df);
                    vocabulary_fallbacks.insert(term.clone(), (literal, aliases));
                }
            }
            let idf = ((count as f64 + 1.0) / (df as f64 + 1.0)).ln() + 1.0;
            groups.push((term, variants, idf, df));
        }
        let mut missing_named_context = false;
        for named in &named_contexts {
            check()?;
            if frequency(self, &backend, named)?.is_some_and(|(_, df)| df == 0) {
                missing_subjects.push(named.clone());
                missing_named_context = true;
            }
        }
        missing_subjects.sort();
        missing_subjects.dedup();
        groups.sort_by(|a, b| a.3.cmp(&b.3).then_with(|| a.0.cmp(&b.0)));
        let mut nodes = BTreeMap::<String, NodeRecord>::new();
        let mut reads = 0_u64;
        let mut probes = 2_u64; // exact ID and exact normalized-name lookups
        let mut bounded = false;
        // Exact identity remains authoritative even for a phrase-shaped name.
        if let Some(node) = backend.node_by_id(&request.query)? {
            reads += 1;
            if discovery_scope_matches(&node, scopes) {
                nodes.insert(node.id.clone(), node);
            }
        }
        let (exact, exact_bounded) =
            backend.nodes_by_normalized_name(&normalize_symbol(&request.query), POSTING_QUOTA)?;
        bounded |= exact_bounded;
        reads = reads.saturating_add(exact.len() as u64 + u64::from(exact_bounded));
        for node in exact {
            if discovery_scope_matches(&node, scopes) {
                nodes.insert(node.id.clone(), node);
            }
        }
        if let Some(index) = &self.source_query {
            bounded |= index.bounded;
            let mut source_queries = Vec::new();
            for offset in 0..groups.iter().map(|g| g.1.len()).max().unwrap_or(0) {
                for (_, variants, _, _) in &groups {
                    if let Some(term) = variants.get(offset) {
                        source_queries.push(term);
                    }
                }
            }
            bounded |= source_queries.len() > 8;
            let mut hydrated = BTreeSet::new();
            for term in source_queries.into_iter().take(8) {
                check()?;
                probes += 1;
                let (owners, limited) = index.candidates(term, 64);
                bounded |= limited;
                reads += owners.len() as u64;
                for owner in owners {
                    check()?;
                    if hydrated.contains(&owner.node_id) {
                        continue;
                    }
                    if hydrated.len() >= 64 {
                        bounded = true;
                        break;
                    }
                    hydrated.insert(owner.node_id.clone());
                    reads += 1;
                    let node = backend.node_by_id(&owner.node_id)?.ok_or_else(|| {
                        QueryError::new(
                            QueryErrorKind::CorruptArtifact,
                            "source_query_owner_missing",
                            "Source-body owner is absent from its bound graph",
                        )
                    })?;
                    if !node.kind.is_callable()
                        || node.source.as_ref() != Some(&owner.source)
                        || !relationships::exact_ast(&node)
                    {
                        return Err(QueryError::new(
                            QueryErrorKind::CorruptArtifact,
                            "source_query_owner_mismatch",
                            "Source-body owner does not match its exact AST graph record",
                        ));
                    }
                    if discovery_scope_matches(&node, scopes) && admit(&node) {
                        nodes.entry(node.id.clone()).or_insert(node);
                    }
                }
            }
        }
        let mut relationships = relationships::recall(&backend, &groups, scopes, check, admit)?;
        reads = reads.saturating_add(relationships.reads);
        probes = probes.saturating_add(relationships.probes);
        bounded |= relationships.bounded;
        nodes.append(&mut relationships.nodes);
        let mut queries = Vec::new();
        // Reserve independent quotas for primary declarations and prose/other
        // evidence, rarest concepts first. Conjunction coverage is checked on
        // source-owned fields after fusion, not on incompatible SQL/store
        // posting intersections. Neither backend scans the entire corpus.
        for (_, variants, _, _) in &groups {
            for variant in variants {
                queries.push(vec![format!("__compass_primary_v1_{variant}")]);
            }
            for variant in variants {
                queries.push(vec![variant.clone()]);
            }
        }
        let mut ownership_candidates = BTreeMap::new();
        for query in queries {
            check()?;
            if probes + 8 + ownership::PROBE_RESERVE >= FIELD_PROBE_CEILING
                || reads.saturating_add(POSTING_QUOTA as u64 * 4 + ownership::READ_RESERVE)
                    > FIELD_READ_CEILING
            {
                bounded = true;
                break;
            }
            let read = self.discovery_term_candidates(&backend, &query, POSTING_QUOTA)?;
            reads = reads.saturating_add(read.node_ids_decoded);
            probes += 1 + read.chunks_decoded;
            bounded |= read.truncated;
            for node in read.nodes {
                // Context may lie outside a node-only answer scope. It can
                // nominate only a uniquely witnessed, in-scope callable.
                if ownership::is_local(&node) {
                    ownership_candidates
                        .entry(node.id.clone())
                        .or_insert_with(|| node.clone());
                }
                if discovery_scope_matches(&node, scopes) {
                    nodes.entry(node.id.clone()).or_insert(node);
                }
            }
        }
        let mut ownership = ownership::recall(
            &backend,
            &ownership_candidates,
            &groups,
            scopes,
            check,
            admit,
        )?;
        reads = reads.saturating_add(ownership.reads);
        probes = probes.saturating_add(ownership.probes);
        bounded |= ownership.bounded;
        nodes.append(&mut ownership.nodes);
        let mut owned_passages = BTreeMap::<String, Vec<(SourceAnchor, BTreeSet<String>)>>::new();
        let mut owned_excerpts = BTreeMap::<String, Vec<excerpts::SelectedExcerpt>>::new();
        let mut excerpt_terms = groups
            .iter()
            .filter(|group| !action_terms.contains(&group.0))
            .flat_map(|group| group.1.iter().cloned())
            .collect::<BTreeSet<_>>();
        if excerpt_terms.is_empty() {
            excerpt_terms.extend(groups.iter().flat_map(|group| group.1.iter().cloned()));
        }
        let mut owned_document_ids = BTreeMap::<String, BTreeSet<String>>::new();
        let mut linked_passages = BTreeMap::<String, BTreeSet<String>>::new();
        let mut linked_sources = BTreeMap::<String, BTreeSet<SourceAnchor>>::new();
        let mut concept_matches = Vec::new();
        let mut witness_nodes = relationships
            .targets
            .into_iter()
            .map(|(id, node)| (id, query_node(&node)))
            .collect::<BTreeMap<_, _>>();
        let mut witness_edges = relationships
            .witnesses
            .into_iter()
            .map(|(id, edge)| (id, query_edge(&edge)))
            .collect::<BTreeMap<_, _>>();
        witness_nodes.extend(
            ownership
                .children
                .into_iter()
                .map(|(id, node)| (id, query_node(&node))),
        );
        witness_edges.extend(
            ownership
                .witnesses
                .into_iter()
                .map(|(id, edge)| (id, query_edge(&edge))),
        );
        let mut document_ids = nodes
            .values()
            .filter(|node| search_document(node).fields.contains_key("document"))
            .map(|node| {
                let document = search_document(node);
                let relevance = groups
                    .iter()
                    .filter(|(_, variants, _, _)| !matching(&document, variants).is_empty())
                    .map(|(_, _, idf, _)| *idf)
                    .sum::<f64>();
                (node.id.clone(), relevance)
            })
            .collect::<Vec<_>>();
        document_ids.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        bounded |= document_ids.len() > 16;
        for (id, _) in document_ids.into_iter().take(16) {
            check()?;
            if reads.saturating_add(16) > FIELD_READ_CEILING {
                bounded = true;
                break;
            }
            let Some(passage) = nodes.get(&id).cloned() else {
                continue;
            };
            let Some(anchor) = passage.source.clone() else {
                continue;
            };
            // A published Markdown mention may suggest an implementation,
            // but is never an ownership assertion. Require both the subject
            // and the target's actual name in the source-backed passage.
            let passage_document = search_document(&passage);
            let passage_terms = passage_document
                .fields
                .get("document")
                .cloned()
                .unwrap_or_default();
            if groups
                .iter()
                .filter(|(term, _, _, _)| !action_terms.contains(term))
                .all(|(_, variants, _, _)| variants.iter().any(|term| passage_terms.contains(term)))
            {
                let mut pending =
                    VecDeque::from([(passage.clone(), vec![id.clone()], Vec::new(), 0_usize)]);
                let mut visited = BTreeSet::new();
                while let Some((current, via, path_edges, depth)) = pending.pop_front() {
                    check()?;
                    if !visited.insert(current.id.clone()) {
                        continue;
                    }
                    if visited.len() > 16
                        || reads.saturating_add(81) > FIELD_READ_CEILING
                        || probes >= FIELD_PROBE_CEILING
                    {
                        bounded = true;
                        break;
                    }
                    probes += 1;
                    let (outgoing, limited) = backend.matching_bounded(
                        &current.id,
                        false,
                        &[
                            EdgeKind::Contains,
                            EdgeKind::Documents,
                            EdgeKind::References,
                        ],
                        true,
                        16,
                    )?;
                    bounded |= limited;
                    reads = reads.saturating_add(outgoing.len() as u64 + u64::from(limited));
                    for edge in outgoing {
                        if crate::code_query::is_heuristic(&edge) {
                            continue;
                        }
                        reads += 1;
                        let Some(target) = backend.node_by_id(&edge.target)? else {
                            continue;
                        };
                        let target_document = search_document(&target);
                        if edge.kind == EdgeKind::Contains
                            && target_document.fields.contains_key("document")
                        {
                            if depth < 2 {
                                let mut next_via = via.clone();
                                next_via.push(target.id.clone());
                                let mut next_edges = path_edges.clone();
                                next_edges.push(edge);
                                pending.push_back((target, next_via, next_edges, depth + 1));
                            } else {
                                bounded = true;
                            }
                        } else if edge.kind != EdgeKind::Contains
                            && !target_document.fields.contains_key("document")
                            && discovery_scope_matches(&target, scopes)
                            && admit(&target)
                        {
                            let name_terms = identifier_search_terms(&target.name)
                                .into_iter()
                                .map(compass_model::canonical_code_token)
                                .collect::<BTreeSet<_>>();
                            if name_terms.is_empty() || !name_terms.is_subset(&passage_terms) {
                                continue;
                            }
                            linked_passages
                                .entry(target.id.clone())
                                .or_default()
                                .extend(passage_terms.iter().cloned());
                            linked_sources
                                .entry(target.id.clone())
                                .or_default()
                                .insert(anchor.clone());
                            concept_matches.push(ConceptMatch {
                                node_id: target.id.clone(),
                                method: ConceptMatchMethod::DocumentLink,
                                query: request.query.clone(),
                                matched: target.qualified_name.clone(),
                                via: via.clone(),
                            });
                            for record in [passage.clone(), current.clone()] {
                                witness_nodes.insert(record.id.clone(), query_node(&record));
                            }
                            // Intermediate document endpoints are looked up
                            // from the published graph, never synthesized.
                            for step in &via {
                                reads += 1;
                                if let Some(record) = backend.node_by_id(step)? {
                                    witness_nodes.insert(step.clone(), query_node(&record));
                                }
                            }
                            for record in path_edges.iter().chain(std::iter::once(&edge)) {
                                witness_edges.insert(record.id.clone(), query_edge(record));
                            }
                            nodes.entry(target.id.clone()).or_insert(target);
                        }
                    }
                }
            }
            check()?;
            if reads.saturating_add(17) > FIELD_READ_CEILING || probes >= FIELD_PROBE_CEILING {
                bounded = true;
                break;
            }
            probes += 1;
            let (edges, limited) =
                backend.matching_bounded(&id, false, &[EdgeKind::Documents], true, 8)?;
            bounded |= limited;
            reads = reads.saturating_add(edges.len() as u64 + u64::from(limited));
            for edge in edges {
                if !compass_model::search::is_exact_nonheuristic_relationship(&edge) {
                    continue;
                }
                if !edge.evidence.iter().any(|evidence| {
                    evidence.origin == EvidenceOrigin::Ast
                        && evidence.confidence == EvidenceConfidence::Exact
                }) {
                    continue;
                }
                reads += 1;
                let Some(owner) = backend.node_by_id(&edge.target)? else {
                    continue;
                };
                let Some(source) = &owner.source else {
                    continue;
                };
                // Enclosed docstrings keep their existing ownership rule.
                // Leading excerpts require the versioned two-anchor AST proof;
                // a generic document link or matching name cannot substitute.
                let enclosed = source.file == anchor.file
                    && source.start_line <= anchor.start_line
                    && source.end_line >= anchor.end_line
                    && source.start_byte <= anchor.start_byte
                    && source.end_byte >= anchor.end_byte;
                if !is_primary_search_kind(owner.kind)
                    || !(enclosed
                        || compass_model::provenance::is_proven_leading_documentation(
                            &passage, &owner, &edge,
                        ))
                {
                    continue;
                }
                // Only the raw-retention leading-doc producer profile proves a
                // verbatim excerpt. Generic enclosed/normalized docstrings keep
                // their retrieval pointer but cannot substitute for this proof.
                if retain_excerpts
                    && compass_model::provenance::is_proven_leading_documentation(
                        &passage, &owner, &edge,
                    )
                    && let Some(compass_model::code_graph::NodeDetails::Resource(details)) =
                        &passage.details
                    && let Some(text) = &details.content
                    && let Some(excerpt) = excerpts::select_ranked(
                        RankedExcerptKind::AstDocumentation,
                        &anchor,
                        text,
                        &excerpt_terms,
                        512,
                    )
                {
                    let quotes = owned_excerpts.entry(owner.id.clone()).or_default();
                    if quotes.len() < 2 && !quotes.contains(&excerpt) {
                        quotes.push(excerpt);
                    }
                }
                let entry = owned_passages.entry(owner.id.clone()).or_default();
                if entry.len() < 2 {
                    owned_document_ids
                        .entry(owner.id.clone())
                        .or_default()
                        .insert(id.clone());
                    entry.push((
                        anchor.clone(),
                        search_document(&passage)
                            .fields
                            .get("document")
                            .cloned()
                            .unwrap_or_default(),
                    ));
                } else {
                    bounded = true;
                }
                if discovery_scope_matches(&owner, scopes) {
                    nodes.entry(owner.id.clone()).or_insert(owner);
                }
            }
        }
        let mut semantic = BTreeMap::new();
        if self.semantic_search && !missing_named_context {
            for (node, similarity) in self.semantic_candidates_guarded(
                &request.query,
                request.limits.max_candidates,
                check,
            )? {
                check()?;
                if discovery_scope_matches(&node, scopes)
                    && similarity.is_finite()
                    && similarity > 0.0
                {
                    semantic.insert(node.id.clone(), similarity);
                    nodes.entry(node.id.clone()).or_insert(node);
                }
            }
        }
        nodes.retain(|_, node| admit(node));
        let admitted = nodes.keys().cloned().collect();
        let recall_time = started.elapsed();
        let ranking_started = Instant::now();
        let wants_tests = original_terms
            .iter()
            .any(|term| matches!(term.as_str(), "test" | "fixture"));
        let supported_groups = groups
            .iter()
            .filter(|(_, _, _, df)| legacy || *df > 0)
            .count();
        let literal_supported_groups = if legacy {
            groups.len()
        } else {
            literal_supported_terms.len()
        };
        let mut target_details = None;
        let mut parameter_roles_unavailable = noun_subject_requested
            && self
                .source_query
                .as_ref()
                .is_none_or(|index| !index.supports_parameter_bindings());
        let mut parameter_roles_partial = false;
        let mut ranked = Vec::new();
        for node in nodes.into_values() {
            check()?;
            let mut document = search_document(&node);
            // Only inspect aliases when a recalled concept has no explanation
            // in the plain projection. This keeps enrichment inside the same
            // global read envelope as candidate recall.
            if groups
                .iter()
                .any(|(_, variants, _, _)| matching(&document, variants).is_empty())
            {
                if reads.saturating_add(20) <= FIELD_READ_CEILING && probes < FIELD_PROBE_CEILING {
                    probes += 1;
                    let (aliases, limited) =
                        backend.matching_bounded(&node.id, true, &[EdgeKind::Aliases], true, 9)?;
                    bounded |= limited;
                    reads = reads.saturating_add(aliases.len() as u64 + u64::from(limited));
                    let mut names = Vec::new();
                    for alias in aliases {
                        if !alias.evidence.is_empty()
                            && compass_model::search::is_exact_nonheuristic_relationship(&alias)
                        {
                            reads += 1;
                            names.push(
                                backend
                                    .node_by_id(&alias.source)?
                                    .map(|source| source.name)
                                    .unwrap_or_default(),
                            );
                        } else {
                            names.push(String::new());
                        }
                    }
                    document =
                        search_document_with_aliases(&node, names.iter().map(String::as_str));
                } else {
                    bounded = true;
                }
            }
            let passages = owned_passages.remove(&node.id).unwrap_or_default();
            if let Some(linked) = linked_passages.get(&node.id) {
                document.fields.insert("document_link", linked.clone());
            }
            if !passages.is_empty() {
                document.fields.insert(
                    "owned_document",
                    passages
                        .iter()
                        .flat_map(|(_, terms)| terms.iter().cloned())
                        .collect(),
                );
            }
            // Attribute dependency at its original enrichment stage using literal
            // variants. A hinted local name must not mask a literal available
            // only through relationship, owned-identifier or body evidence.
            // Hint-only matches stay Low through vocabulary_needed below.
            // Only terminal names on verified outgoing code relationships,
            // never the target's namespace, path or documentation.
            let relationship_terms = relationships.terms.remove(&node.id).unwrap_or_default();
            let relationship_needed = groups.iter().any(|(term, variants, _, _)| {
                let variants = vocabulary_fallbacks
                    .get(term)
                    .map_or(variants, |(literal, _)| literal);
                matching(&document, variants).is_empty()
                    && variants
                        .iter()
                        .any(|term| relationship_terms.contains(term))
            });
            if !relationship_terms.is_empty() {
                document.fields.insert("code_reference", relationship_terms);
            }
            let identifier_terms = ownership.terms.remove(&node.id).unwrap_or_default();
            let identifier_needed = groups.iter().any(|(term, variants, _, _)| {
                let variants = vocabulary_fallbacks
                    .get(term)
                    .map_or(variants, |(literal, _)| literal);
                matching(&document, variants).is_empty()
                    && variants.iter().any(|term| identifier_terms.contains(term))
            });
            if !identifier_terms.is_empty() {
                document.fields.insert("owned_identifier", identifier_terms);
            }
            let mut body_needed = false;
            let mut code_needed = false;
            let mut preference_truncated = document.truncated;
            let mut body_sources = BTreeSet::new();
            let source_index = self.source_query.as_ref();
            let source_owner = source_index.and_then(|index| index.owner(&node.id));
            if let (Some(index), Some(owner)) = (source_index, source_owner) {
                if !node.kind.is_callable()
                    || node.source.as_ref() != Some(&owner.source)
                    || !relationships::exact_ast(&node)
                {
                    return Err(QueryError::new(
                        QueryErrorKind::CorruptArtifact,
                        "source_query_owner_mismatch",
                        "Source evidence no longer matches the pinned graph",
                    ));
                }
                let body = index.document(owner)?;
                preference_truncated |= index.preference_truncated(owner)?;
                code_needed = groups.iter().any(|(term, variants, _, _)| {
                    let variants = vocabulary_fallbacks
                        .get(term)
                        .map_or(variants, |(literal, _)| literal);
                    matching(&document, variants)
                        .iter()
                        .all(|field| *field == "signature")
                        && matching(&body, variants).contains(&"body_code")
                });
                body_needed = groups.iter().any(|(term, variants, _, _)| {
                    let variants = vocabulary_fallbacks
                        .get(term)
                        .map_or(variants, |(literal, _)| literal);
                    // Some existing header projections include leading body
                    // comments. Repeated signature/body terms are not independent
                    // corroboration of the requested behavior.
                    matching(&document, variants)
                        .iter()
                        .all(|field| *field == "signature")
                        && !matching(&body, variants).is_empty()
                });
                document.truncated |= body.truncated;
                document.fields.extend(body.fields);
                let terms = groups
                    .iter()
                    .flat_map(|g| g.1.iter().cloned())
                    .collect::<BTreeSet<_>>();
                let (sources, limited) =
                    crate::source_query::matching_sources(owner, &terms, code_needed);
                body_sources = sources;
                bounded |= limited;
            }
            bounded |= document.truncated;
            let inspection_signature = compounds::inspection_signature(match &node.details {
                Some(compass_model::code_graph::NodeDetails::Symbol(details)) => {
                    details.signature.as_deref()
                }
                _ => None,
            });
            let compound_matches = if preference_truncated {
                compounds::Matches::default()
            } else {
                compounds.matches(
                    &node.name,
                    inspection_signature,
                    source_owner.map_or(&[], |owner| owner.identifiers.as_slice()),
                )
            };
            let exact_id = node.id == request.query;
            let exact_name = normalize_symbol(&node.name) == normalize_symbol(&request.query)
                || normalize_symbol(&node.qualified_name) == normalize_symbol(&request.query);
            // A literal match keeps its literal field provenance. In particular,
            // read in a namespace/signature/prose must not let hinted get in the
            // local name masquerade as a literal implementation operation.
            // Clone only affected, already bounded groups; no additional reads.
            let mut literal_groups = None;
            for (index, (term, variants, _, _)) in groups.iter().enumerate() {
                if let Some((literal, _)) = vocabulary_fallbacks.get(term)
                    && variants != literal
                    && !matching(&document, literal).is_empty()
                {
                    let selected = literal_groups.get_or_insert_with(|| groups.clone());
                    selected[index].1.clone_from(literal);
                }
            }
            let groups = literal_groups.as_ref().unwrap_or(&groups);
            let mut vocabulary_matches = Vec::new();
            for (term, (literal, aliases)) in &vocabulary_fallbacks {
                if matching(&document, literal).is_empty() {
                    for alias in aliases {
                        if !matching(&document, std::slice::from_ref(alias)).is_empty() {
                            vocabulary_matches.push((term.clone(), alias.clone()));
                        }
                    }
                }
            }
            let vocabulary_needed = !exact_id && !exact_name && !vocabulary_matches.is_empty();
            // Read only typed sites on the already validated exact owner.
            // Legacy generic identifiers/signatures cannot prove a binding.
            // This lookup adds no posting, document-frequency term or score.
            if noun_subject_requested && !exact_id && !exact_name && node.kind.is_callable() {
                match source_owner.and_then(|owner| owner.parameter_bindings.as_ref()) {
                    Some(bindings)
                        if bindings.status
                            == compass_model::source_query::ParameterBindingStatus::Complete => {}
                    Some(_) => parameter_roles_partial = true,
                    None => parameter_roles_unavailable = true,
                }
            }
            let mut noun_sources = BTreeMap::new();
            if let Some(bindings) = source_owner.and_then(|owner| owner.parameter_bindings.as_ref())
            {
                for site in &bindings.sites {
                    for word in identifier_search_terms(&site.text) {
                        let term = compass_model::canonical_code_token(word);
                        if possible_nouns.contains(&term) {
                            noun_sources
                                .entry(term)
                                .or_insert_with(|| site.source.clone());
                        }
                    }
                }
            }
            let noun_topics = noun_sources.keys().cloned().collect::<BTreeSet<_>>();
            body_sources.extend(noun_sources.into_values());
            let nominal_topics = possible_nouns
                .iter()
                .filter(|term| nominal_topic(&node, term))
                .cloned()
                .collect::<BTreeSet<_>>();
            if !nominal_topics.is_empty() {
                body_sources.extend(node.source.iter().cloned());
            }
            let mut topic_exceptions = noun_topics.clone();
            topic_exceptions.extend(nominal_topics.iter().cloned());
            topic_exceptions.extend(
                named_contexts
                    .iter()
                    .filter(|term| {
                        (topic_predicates.contains(*term)
                            || matches!(term.as_str(), "all" | "only" | "new" | "made"))
                            && declared_topic(&node, term)
                    })
                    .cloned(),
            );
            let role_uncertain = !exact_id && !exact_name && !topic_exceptions.is_empty();
            let literal_topic = groups.iter().any(|(term, variants, _, _)| {
                is_independent_topic(term, &topic_predicates, &topic_exceptions)
                    && matching(
                        &document,
                        vocabulary_fallbacks
                            .get(term)
                            .map_or(variants, |(literal, _)| literal),
                    )
                    .iter()
                    .any(|field| {
                        matches!(
                            *field,
                            "name"
                                | "signature"
                                | "declaring_type"
                                | "owned_document"
                                | "body_identifier"
                                | "body_comment"
                                | "body_code"
                                | "code_reference"
                                | "owned_identifier"
                        )
                    })
            });
            // Observe one requested target using already hydrated, bounded
            // fields. Normal queries allocate no trace and perform no extra
            // lookups; tracing does not affect admission, scores or ordering.
            let trace = (trace_target == Some(node.id.as_str())).then(|| {
                let terms = groups
                    .iter()
                    .map(|(term, variants, idf, df)| RankedTermTrace {
                        term: term.clone(),
                        variants: variants.clone(),
                        document_frequency: *df,
                        inverse_document_frequency: *idf,
                        matched_fields: matching(&document, variants)
                            .into_iter()
                            .map(str::to_owned)
                            .collect(),
                        action: action_terms.contains(term),
                    })
                    .collect::<Vec<_>>();
                let matched = terms
                    .iter()
                    .filter(|term| !term.matched_fields.is_empty())
                    .count();
                RankedTargetTrace {
                    terms,
                    field_lengths: document
                        .fields
                        .iter()
                        .map(|(field, terms)| ((*field).to_owned(), terms.len()))
                        .collect(),
                    supported_concepts: supported_groups,
                    supported_coverage: matched as f64 / supported_groups.max(1) as f64,
                    exact_id,
                    exact_name,
                    semantic_score: semantic.get(&node.id).copied().unwrap_or(0.0),
                    score: None,
                    filter_reason: None,
                }
            });
            // Unknown ordinary words may be a paraphrase of indexed behavior.
            // Retain tentative candidates and disclose their missing context;
            // a missing explicitly named subject remains a retrieval veto.
            if !exact_id && !exact_name && missing_named_context {
                if let Some(mut details) = trace {
                    details.filter_reason = Some(RankedFilterReason::MissingNamedContext);
                    target_details = Some(details);
                }
                continue;
            }
            let mut matched_terms = Vec::new();
            let mut matched_fields = BTreeSet::new();
            let mut score = 0.0;
            let mut action_matched = false;
            let mut subject_matched = false;
            let mut implementation_action = false;
            let mut implementation_subject = false;
            let mut compound_subject = false;
            for (term, variants, idf, _) in groups {
                let fields = matching(&document, variants);
                if fields.is_empty() {
                    continue;
                }
                matched_terms.push(term.clone());
                // Prose, signatures and namespaces can mention an operation
                // without identifying its implementation. Prefer a lexical
                // operation signal in the callable's own name, identifiers or
                // exact witnessed outgoing references, with separate topic
                // evidence. Comments alone never provide this preference.
                // Callable identifier postings include parameters. Retained
                // signature terms therefore cannot independently supply the
                // code signal; missing/clipped signatures keep ordinary ranking.
                implementation_action |= action_terms.contains(term)
                    && (fields.contains(&"name")
                        || (!preference_truncated
                            && inspection_signature.is_some()
                            && !fields.contains(&"signature")
                            && fields.iter().any(|field| {
                                matches!(*field, "body_identifier" | "code_reference")
                            })));
                implementation_subject |=
                    is_independent_topic(term, &topic_predicates, &topic_exceptions)
                        && fields.iter().any(|field| {
                            matches!(
                                *field,
                                "name"
                                    | "signature"
                                    | "declaring_type"
                                    | "owned_document"
                                    | "body_identifier"
                                    | "body_comment"
                                    | "code_reference"
                                    | "owned_identifier"
                            )
                        });
                compound_subject |= !compound_matches.phrases.is_empty()
                    && is_independent_topic(term, &topic_predicates, &topic_exceptions)
                    && !compounds.contains_component_group(term)
                    && fields.iter().any(|field| {
                        matches!(
                            *field,
                            "name"
                                | "signature"
                                | "declaring_type"
                                | "owned_document"
                                | "body_identifier"
                                | "body_comment"
                                | "code_reference"
                                | "owned_identifier"
                        )
                    });
                subject_matched |= is_independent_topic(term, &topic_predicates, &topic_exceptions)
                    && fields
                        .iter()
                        .any(|field| !matches!(*field, "kind" | "language"));
                let field_score = fields
                    .iter()
                    .map(|field| {
                        let length =
                            document.fields.get(field).map_or(1, BTreeSet::len).max(1) as f64;
                        // BM25F-style saturation/length normalization uses corpus
                        // DF and bounded per-field unique-token frequencies.
                        field_weight(field) / (0.75 + 0.25 * length / 16.0)
                    })
                    .sum::<f64>();
                score += idf * field_score * 2.2 / (1.2 + field_score);
                action_matched |= action_terms.contains(term) && fields.contains(&"name");
                matched_fields.extend(fields);
            }
            let semantic_score = semantic.get(&node.id).copied().unwrap_or(0.0);
            // Nonsemantic vocabulary hints need a separate literal topic.
            // The existing opt-in semantic channel can independently nominate
            // a candidate; hint dependency still forces Low and no preference.
            if vocabulary_needed && !literal_topic && semantic_score == 0.0 {
                if let Some(mut details) = trace {
                    details.filter_reason = Some(RankedFilterReason::MissingSubjectEvidence);
                    target_details = Some(details);
                }
                continue;
            }
            // A pure kind/language listing needs no topic. Once the question
            // asks for any other subject, a facet cannot stand in for it.
            // Keep unfamiliar wording tentative only when this candidate has
            // real subject evidence outside those two descriptive fields.
            let facet_only_subjects = !noun_subject_requested
                && groups
                    .iter()
                    .filter(|(term, _, _, _)| {
                        is_independent_topic(term, &topic_predicates, &topic_exceptions)
                    })
                    .all(|(_, variants, _, _)| {
                        matching(&document, variants)
                            .iter()
                            .any(|field| matches!(*field, "kind" | "language"))
                    });
            if !subject_matched
                && !facet_only_subjects
                && (noun_subject_requested
                    || groups.iter().any(|(term, _, _, _)| {
                        is_independent_topic(term, &topic_predicates, &topic_exceptions)
                    }))
                && !exact_id
                && !exact_name
                && semantic_score == 0.0
            {
                if let Some(mut details) = trace {
                    details.filter_reason = Some(RankedFilterReason::MissingSubjectEvidence);
                    target_details = Some(details);
                }
                continue;
            }
            if matched_terms.is_empty() && !exact_id && !exact_name && semantic_score == 0.0 {
                if let Some(mut details) = trace {
                    details.filter_reason = Some(RankedFilterReason::NoMatchedConcept);
                    target_details = Some(details);
                }
                continue;
            }
            let coverage = matched_terms.len() as f64 / groups.len().max(1) as f64;
            // Unknown paraphrase words still lower score and relevance. They
            // must not also veto otherwise well-supported inspection pointers.
            // Legacy statistics are unknown, so retain the full denominator.
            let supported_coverage = matched_terms.len() as f64 / supported_groups.max(1) as f64;
            // Additional alias frequencies must not veto an unchanged literal
            // candidate that passed the original corpus-supported screen.
            let preserves_literal_support = !vocabulary_needed
                && matched_terms.len() as f64 / literal_supported_groups.max(1) as f64 >= 0.4;
            if supported_coverage < 0.4
                && !preserves_literal_support
                && !exact_id
                && !exact_name
                && semantic_score == 0.0
            {
                if let Some(mut details) = trace {
                    details.filter_reason = Some(RankedFilterReason::InsufficientSupportedCoverage);
                    target_details = Some(details);
                }
                continue;
            }
            score *= coverage * coverage;
            if semantic_score > 0.0 {
                score += semantic_score.min(1.0) * 0.25;
                matched_fields.insert("semantic_lsa");
            }
            if action_matched && node.kind.is_callable() {
                score *= 1.5;
            }
            if !passages.is_empty() {
                score *= 1.25;
            }
            if matches!(
                node.kind,
                NodeKind::Parameter | NodeKind::Field | NodeKind::Property | NodeKind::Variable
            ) {
                score *= 0.35;
            }
            if document.fields.contains_key("document") && !wants_documents {
                // Prose remains a fallback candidate and a supporting passage.
                // For implementation questions it should not displace the
                // declaration that owns or is explicitly named by that prose.
                score *= 0.35;
            }
            if test_context(&node) != wants_tests {
                score *= 0.45;
            }
            let preferred_implementation = implementation_action
                && implementation_subject
                && !vocabulary_needed
                && !code_needed
                && !role_uncertain
                // A lexical preference cannot establish the relationship
                // between multiple actions (e.g. stopping new task starts).
                && action_terms.len() == 1
                && !negative_qualification
                && temporal_qualification.is_empty()
                && node.kind.is_callable()
                && !wants_documents
                && test_context(&node) == wants_tests;
            let preferred_compound = !compound_matches.phrases.is_empty()
                && compound_subject
                && !vocabulary_needed
                && !code_needed
                && !role_uncertain
                && node.kind.is_callable()
                && !negative_qualification
                && temporal_qualification.is_empty()
                && !wants_documents
                && test_context(&node) == wants_tests;
            if preferred_compound {
                // Bounded lexical cohesion outranks an isolated operation word,
                // while exact identity keeps its existing higher tier.
                score += 20_000.0 + compound_matches.phrases.len() as f64 * 100.0;
                let mut selected = compound_matches.sources.clone();
                for source in &body_sources {
                    if selected.len() >= 8 {
                        bounded |= !selected.contains(source);
                    } else {
                        selected.insert(source.clone());
                    }
                }
                body_sources = selected;
            } else if preferred_implementation {
                score += IMPLEMENTATION_ACTION_PREFERENCE;
            }
            if exact_id {
                score += 1_000_000.0;
                matched_fields.insert("exact_id");
            }
            if exact_name {
                score += 500_000.0;
                matched_fields.insert("exact_name");
            }
            let mut missing_context = groups
                .iter()
                .filter(|(term, variants, _, _)| {
                    !action_terms.contains(term) && matching(&document, variants).is_empty()
                })
                .map(|(term, _, _, _)| term.clone())
                .collect::<BTreeSet<_>>();
            let evidence_terms = document.terms();
            missing_context.extend(
                named_contexts
                    .iter()
                    .filter(|term| !evidence_terms.contains(*term))
                    .cloned(),
            );
            missing_context.extend(
                explicit_identifiers
                    .iter()
                    .filter(|identifier| !has_explicit_identity(&node, identifier))
                    .map(|identifier| format!("explicit identity `{identifier}`")),
            );
            let relevance = if vocabulary_needed
                || role_uncertain
                || (preferred_compound && !exact_id && !exact_name)
                || negative_qualification
                || !temporal_qualification.is_empty()
                || ((relationship_needed || identifier_needed || body_needed)
                    && !exact_id
                    && !exact_name)
            {
                RelevanceBand::Low
            } else if exact_id
                || exact_name
                || (missing_context.is_empty()
                    && coverage >= 0.8
                    && action_matched
                    && node.kind.is_callable())
            {
                RelevanceBand::High
            } else if missing_context.is_empty() && coverage >= 0.5 {
                RelevanceBand::Medium
            } else {
                RelevanceBand::Low
            };
            let mut reasons = Vec::new();
            if role_uncertain {
                if !noun_topics.is_empty() {
                    reasons.push("A possible predicate also has a typed AST-owned parameter binding. Retained as a Low topic inspection lead; this does not establish the requested predicate or behavior".to_owned());
                }
                if !nominal_topics.is_empty() {
                    reasons.push("A possible predicate also names an exact AST-owned class or struct declaration. Retained as a Low lexical type/declaration pointer; this does not establish the requested predicate or behavior, or recover its methods".to_owned());
                }
                if topic_exceptions
                    .iter()
                    .any(|term| !noun_topics.contains(term) && !nominal_topics.contains(term))
                {
                    reasons.push("A possible predicate/qualifier also has a source-backed explicitly named declaration. Retained as a Low topic inspection lead; this does not establish the requested predicate, qualifier, or behavior".to_owned());
                }
                if !noun_topics.is_empty() {
                    let raw = decision_spellings
                        .iter()
                        .filter(|raw| {
                            noun_topics
                                .contains(&compass_model::canonical_code_token((*raw).clone()))
                        })
                        .collect::<Vec<_>>();
                    reasons.push(format!("Ambiguous noun/predicate spelling: {} ({} raw query occurrences; showing at most 8). A parameter-binding match does not verify any separate predicate occurrence", raw.iter().take(8).map(|word| word.as_str()).collect::<Vec<_>>().join(", "), raw.len()));
                }
            }
            if vocabulary_needed {
                reasons.push(format!("Vocabulary fallback for ordinary words or code abbreviations: {}. These are inspection alternatives; equivalence and behavior remain unverified", vocabulary_matches.iter().map(|(term, alias)| format!("{term} -> {alias}")).collect::<Vec<_>>().join(", ")));
            }
            if preferred_compound {
                reasons.push(format!("Compound identifiers match requested words together: {}. Separate owner-topic evidence gives an inspection preference; equivalence and behavior remain unverified", compound_matches.phrases.into_iter().collect::<Vec<_>>().join(", ")));
            } else if preferred_implementation {
                reasons.push("Implementation-local operation identifiers and separate topic evidence receive an inspection preference; this does not establish behavior or execution".to_owned());
            }
            if negative_qualification {
                reasons.push("Negative qualification is not established by lexical evidence. These are inspection candidates, not confirmation of the requested condition".to_owned());
            }
            if !temporal_qualification.is_empty() {
                reasons.push(format!(
                    "Temporal qualification ({}) is not established by lexical evidence. Inspect the candidate source to verify the requested ordering or condition",
                    temporal_qualification.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            }
            reasons.push(format!(
                "Indexed {} evidence matches {} of {} query concepts",
                matched_fields
                    .iter()
                    .copied()
                    .collect::<Vec<_>>()
                    .join(", "),
                matched_terms.len(),
                groups.len()
            ));
            if !missing_context.is_empty() && !exact_id && !exact_name {
                reasons.push(format!("Partial indexed evidence; missing owner-specific context: {}. The requested behavior is not established by this candidate", missing_context.into_iter().collect::<Vec<_>>().join(", ")));
            }
            if document.fields.contains_key("document_link") {
                reasons.push("Source-backed prose names this candidate through a published link; this does not establish ownership or implementation equivalence".to_owned());
            }
            if matched_fields.contains("code_reference") {
                reasons.push("Exact AST call/reference sites inside this declaration mention matching identifiers. This supports inspection, not confirmation of the requested behavior".to_owned());
            }
            if matched_fields.contains("owned_identifier") {
                reasons.push("Exact AST containment inside this declaration owns matching local identifiers. This supports inspection, not confirmation of the requested behavior".to_owned());
            }
            if matched_fields.contains("body_identifier") || matched_fields.contains("body_comment")
            {
                reasons.push("Source-bound AST lexical evidence inside this callable matches query terms. Identifier/comment occurrence supports inspection; it does not establish the requested behavior".to_owned());
            }
            if matched_fields.contains("body_code") {
                reasons.push("Bounded AST-owned code text matches query terms, including literal strings. Text occurrence supports inspection only; it does not establish behavior or execution".to_owned());
            }
            if vocabulary_needed {
                for (term, alias) in &vocabulary_matches {
                    concept_matches.push(ConceptMatch {
                        node_id: node.id.clone(),
                        method: ConceptMatchMethod::Synonym,
                        query: term.clone(),
                        matched: alias.clone(),
                        via: Vec::new(),
                    });
                }
            }
            for (concept, alternative) in &expansions {
                let alternative_terms = identifier_search_terms(alternative)
                    .into_iter()
                    .map(compass_model::canonical_code_token)
                    .collect::<BTreeSet<_>>();
                if !alternative_terms.is_empty() && alternative_terms.is_subset(&document.terms()) {
                    concept_matches.push(ConceptMatch {
                        node_id: node.id.clone(),
                        method: ConceptMatchMethod::Synonym,
                        query: concept.clone(),
                        matched: alternative.clone(),
                        via: Vec::new(),
                    });
                }
            }
            if let Some(mut details) = trace {
                details.score = Some(score);
                target_details = Some(details);
            }
            ranked.push(RankedCandidate {
                node: query_node(&node),
                excerpts: Vec::new(),
                score,
                relevance,
                matched_terms,
                matched_fields: matched_fields.into_iter().map(str::to_owned).collect(),
                reasons,
                supporting_sources: passages
                    .into_iter()
                    .map(|(source, _)| source)
                    .chain(linked_sources.remove(&node.id).unwrap_or_default())
                    .chain(relationships.sources.remove(&node.id).unwrap_or_default())
                    .chain(ownership.sources.remove(&node.id).unwrap_or_default())
                    .chain(body_sources)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            });
        }
        if !wants_documents {
            let ranked_ids = ranked
                .iter()
                .map(|candidate| candidate.node.id.as_str())
                .collect::<BTreeSet<_>>();
            let represented_documents = owned_document_ids
                .iter()
                .filter(|(owner, _)| ranked_ids.contains(owner.as_str()))
                .flat_map(|(_, documents)| documents.iter().cloned())
                .collect::<BTreeSet<_>>();
            if trace_target.is_some_and(|id| represented_documents.contains(id))
                && let Some(details) = &mut target_details
            {
                details.filter_reason = Some(RankedFilterReason::RepresentedOwnedDocument);
            }
            ranked.retain(|candidate| !represented_documents.contains(&candidate.node.id));
        }
        ranked.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then_with(|| a.node.id.cmp(&b.node.id))
        });
        let eligible = ranked.len();
        let ranks = ranked
            .iter()
            .enumerate()
            .map(|(at, candidate)| (candidate.node.id.clone(), at + 1))
            .collect();
        let cap = request.limits.max_candidates.min(request.limits.max_nodes) as usize;
        bounded |= eligible > cap;
        ranked.truncate(cap);
        let quotation_groups = groups
            .iter()
            .map(|group| group.1.clone())
            .collect::<Vec<_>>();
        let quotation_statistics = groups
            .iter()
            .map(|(_, variants, _, frequency)| excerpts::QueryTermGroup {
                variants,
                document_frequency: (!legacy).then_some(*frequency),
            })
            .collect::<Vec<_>>();
        let quotation_actions = groups
            .iter()
            .filter(|group| action_terms.contains(&group.0))
            .map(|group| group.1.clone())
            .collect::<Vec<_>>();
        let quotation_terms = quotation_groups
            .iter()
            .flat_map(|group| group.iter().cloned())
            .collect::<BTreeSet<_>>();
        // Quotation selection never feeds admission, score or candidate order.
        // Only the displayed shortlist retains at most 1024 bytes per owner.
        for candidate in ranked.iter_mut().take(if retain_excerpts { 5 } else { 0 }) {
            check()?;
            let mut quotes = owned_excerpts
                .remove(&candidate.node.id)
                .unwrap_or_default();
            let owner = self
                .source_query
                .as_ref()
                .and_then(|index| index.owner(&candidate.node.id));
            if let Some(owner) = owner {
                // The owner was already checked against the pinned exact-AST
                // declaration during scoring. Never read current source here.
                for comment in &owner.comments {
                    if let Some(excerpt) = excerpts::select_ranked(
                        RankedExcerptKind::CallableComment,
                        &comment.source,
                        &comment.text,
                        &quotation_terms,
                        512,
                    ) {
                        quotes.push(excerpt);
                    }
                }
                if owner.omitted_code_windows > 0 {
                    candidate.reasons.push("Code inspection windows were omitted by extraction/publication quotas; retained text is not the complete implementation".to_owned());
                }
                for window in &owner.code_windows {
                    if let Some(excerpt) = excerpts::select_code(
                        RankedExcerptKind::CallableCode,
                        &window.source,
                        &window.text,
                        &quotation_terms,
                        512,
                        &owner.comments,
                    ) {
                        quotes.push(excerpt);
                    }
                }
                if owner.omitted_nested_contexts > 0
                    || owner
                        .nested_contexts
                        .iter()
                        .any(|context| context.omitted_code_windows > 0)
                {
                    candidate.reasons.insert(0,"Nested lexical code context was omitted by shared inspection limits; no execution is inferred".to_owned());
                }
                for context in &owner.nested_contexts {
                    for window in &context.code_windows {
                        if let Some(mut excerpt) = excerpts::select_code(
                            RankedExcerptKind::NestedCallableCode,
                            &window.source,
                            &window.text,
                            &quotation_terms,
                            512,
                            &owner.comments,
                        ) {
                            excerpt.excerpt.nested_scope = Some(context.source.clone());
                            quotes.push(excerpt);
                        }
                    }
                }
            }
            quotes.sort_by_cached_key(|quote| {
                (
                    std::cmp::Reverse(excerpts::coverage_terms(&quote.terms, &quotation_actions)),
                    excerpts::rarest_document_frequency(&quote.terms, &quotation_statistics),
                    std::cmp::Reverse(excerpts::coverage_terms(&quote.terms, &quotation_groups)),
                    std::cmp::Reverse(matches!(
                        quote.excerpt.kind,
                        RankedExcerptKind::CallableCode | RankedExcerptKind::NestedCallableCode
                    )),
                    quote.excerpt.source.clone(),
                )
            });
            quotes.dedup();
            candidate.excerpts = quotes
                .into_iter()
                .take(2)
                .map(|quote| quote.excerpt)
                .collect();
        }
        Ok(Retrieval {
            ranked,
            admitted,
            bounded,
            eligible,
            reads,
            probes,
            missing_subjects,
            negative_qualification,
            temporal_qualification,
            legacy,
            relationship_legacy: relationships.legacy,
            parameter_roles_unavailable,
            parameter_roles_partial,
            ranks,
            recall_time,
            ranking_time: ranking_started.elapsed(),
            concept_matches,
            witness_nodes: witness_nodes.into_values().collect(),
            witness_edges: witness_edges.into_values().collect(),
            target_details,
        })
    }

    pub(crate) fn broad_search(
        &self,
        request: SearchRequest,
        instrumentation: &mut crate::telemetry::QueryInstrumentation,
        admit: &dyn Fn(&NodeRecord) -> bool,
    ) -> Result<CodeQueryResponse, QueryError> {
        let deadline = Instant::now() + Duration::from_secs(30);
        let check = || {
            self.check_deadline()?;
            if Instant::now() >= deadline {
                return Err(QueryError::new(
                    QueryErrorKind::Timeout,
                    "broad_query_timeout",
                    "Broad retrieval exceeded its timeout; narrow the question or scope",
                ));
            }
            Ok(())
        };
        let retrieval = self.retrieve_broad(&request, &[], &check, admit, None, false)?;
        instrumentation.recall += retrieval.recall_time;
        instrumentation.ranking += retrieval.ranking_time;
        instrumentation.work.candidates_read += retrieval.reads;
        instrumentation.work.postings_decoded += retrieval.reads;
        let mut response = CodeQueryResponse::empty(CodeQueryOperation::Search, request.limits);
        response.truncated = retrieval.bounded;
        response.nodes = retrieval
            .ranked
            .iter()
            .map(|candidate| candidate.node.clone())
            .collect();
        response.results = retrieval
            .ranked
            .iter()
            .map(|candidate| SearchHit {
                node_id: candidate.node.id.clone(),
                score: candidate.score,
                matched_fields: candidate.matched_fields.clone(),
            })
            .collect();
        response.diagnostics = self.broad_diagnostics(&retrieval);
        let mut retained = response
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect::<BTreeSet<_>>();
        for node in &retrieval.witness_nodes {
            if retained.len() < response.limits.max_nodes as usize {
                if retained.insert(node.id.clone()) {
                    response.nodes.push(node.clone());
                }
            } else if !retained.contains(&node.id) {
                response.truncated = true;
            }
        }
        let mut witness_edges = retrieval
            .witness_edges
            .into_iter()
            .filter(|edge| retained.contains(&edge.source) && retained.contains(&edge.target))
            .collect::<Vec<_>>();
        response.truncated |= witness_edges.len() > response.limits.max_edges as usize;
        witness_edges.truncate(response.limits.max_edges as usize);
        response.edges = witness_edges;
        response.concept_matches = retrieval
            .concept_matches
            .into_iter()
            .filter(|item| {
                retained.contains(&item.node_id) && item.via.iter().all(|id| retained.contains(id))
            })
            .collect();
        response.sort_stable();
        crate::code_query::enforce_response_size(&mut response)?;
        Ok(response)
    }

    fn broad_diagnostics(&self, retrieval: &Retrieval) -> Vec<QueryDiagnostic> {
        let mut diagnostics = Vec::new();
        let mut push = |code, message| {
            diagnostics.push(QueryDiagnostic {
                code,
                message,
                node_id: None,
                path: None,
            })
        };
        // Put the actionable recovery before generic truncation. Bounded text
        // projections may have room for only the first diagnostic.
        if retrieval.legacy {
            push(QueryDiagnosticCode::IncompleteCoverage, "This immutable index predates fielded evidence statistics. Run compass update on the current checkout for signature/prose recall; historical snapshots remain unchanged".to_owned());
        }
        if retrieval.relationship_legacy {
            push(QueryDiagnosticCode::IncompleteCoverage, "This immutable index predates broad relationship postings; method recall through relationships is limited to direct calls. Run compass update on the current checkout; historical snapshots remain unchanged".to_owned());
        }
        if retrieval.parameter_roles_unavailable {
            push(QueryDiagnosticCode::IncompleteCoverage, "Typed parameter-binding evidence is unavailable for some query candidates. Ambiguous noun/verb inspection may miss valid leads. Run compass update on the current checkout for supported bindings or inspect source for unsupported syntax; historical snapshots remain unchanged".to_owned());
        }
        if retrieval.parameter_roles_partial {
            push(QueryDiagnosticCode::IncompleteCoverage, "Some recalled callables have partial or unsupported parameter-binding evidence. Retained binding sites remain inspection witnesses; missing sites do not prove absence. Inspect source for omitted or unsupported bindings".to_owned());
        }
        if retrieval.negative_qualification {
            push(QueryDiagnosticCode::IncompleteCoverage, "Negative qualification is not established by lexical retrieval. Inspect the candidate source or use a structural query to verify the requested condition".to_owned());
        }
        if !retrieval.temporal_qualification.is_empty() {
            push(
                QueryDiagnosticCode::IncompleteCoverage,
                format!(
                    "Temporal qualification ({}) is not established by lexical retrieval. Inspect the candidate source to verify the requested ordering or condition",
                    retrieval
                        .temporal_qualification
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
        if retrieval
            .ranked
            .iter()
            .any(|candidate| candidate.relevance == RelevanceBand::Low)
        {
            let missing = if retrieval.missing_subjects.is_empty() {
                "Some candidates lack owner-specific query context".to_owned()
            } else {
                format!(
                    "Missing indexed query context: {}",
                    retrieval.missing_subjects.join(", ")
                )
            };
            push(
                QueryDiagnosticCode::IncompleteCoverage,
                format!(
                    "These are inspection candidates; the requested behavior is not established. {missing}. Use ranked view for per-candidate relevance and reasons"
                ),
            );
        }
        if retrieval.ranked.is_empty() {
            push(
                QueryDiagnosticCode::NoMatch,
                format!(
                    "No supporting indexed evidence found. Missing subject evidence: {}. Narrow the question or check indexed scope; absence is not proof the behavior does not exist.",
                    retrieval.missing_subjects.join(", ")
                ),
            );
        }
        if retrieval.bounded {
            push(QueryDiagnosticCode::BoundedTruncation, "Fielded retrieval reached a candidate, source-field, or output bound; available candidates are not a complete repository census".to_owned());
        }
        if self.source_query.is_none() {
            push(QueryDiagnosticCode::IncompleteCoverage, "This graph has no immutable callable-body index. Broad recall uses published names, signatures and graph evidence; run compass update on the current checkout for body identifiers/comments. Historical snapshots remain unchanged".to_owned());
        } else if let Some(message) = self
            .source_query
            .as_ref()
            .and_then(|index| index.coverage_message.as_ref())
        {
            push(QueryDiagnosticCode::IncompleteCoverage, message.clone());
        }
        if let Some(message) = &self.partial_graph_message {
            push(QueryDiagnosticCode::IncompleteCoverage, message.clone());
        }
        diagnostics
    }

    /// Ranked source evidence independent of the three-seed graph expansion.
    pub fn discover_ranked(
        &self,
        request: DiscoveryQueryRequest,
    ) -> Result<RankedDiscoveryResponse, QueryError> {
        crate::discovery::validate_request(&request)?;
        let guard =
            crate::discovery::DiscoveryGuard::new(request.limits.timeout_ms, None, self.deadline);
        self.discover_ranked_guarded(request, &guard, true)
    }

    fn discover_ranked_guarded(
        &self,
        request: DiscoveryQueryRequest,
        guard: &crate::discovery::DiscoveryGuard<'_>,
        enforce_ranked_budget: bool,
    ) -> Result<RankedDiscoveryResponse, QueryError> {
        guard.check()?;
        let fielded = self.is_fielded_question(&request.question)?;
        guard.check()?;
        if !fielded
            || request.direction != DiscoveryDirection::Auto
            || !request.relation_contexts.is_empty()
        {
            let exact_subject = request.question.trim().to_owned();
            let discovery = self.discover_guarded(request, guard)?;
            let has_exact_id = discovery
                .seeds
                .iter()
                .any(|seed| seed.node_id == exact_subject);
            // A literal existing ID identifies the inspection target. Lexical
            // matches for an ID prefix belong to neither its shortlist nor its
            // structural evidence; retain raw discovery/context separately.
            let candidates = discovery.seeds.iter().filter(|seed| !has_exact_id || seed.node_id == exact_subject).filter_map(|seed| discovery.nodes.iter().find(|node| node.id == seed.node_id).map(|node| RankedCandidate {
                node: node.clone(), score: seed.score.parse().unwrap_or(0.0), relevance: RelevanceBand::Low,
                matched_terms: seed.matched_terms.clone(), matched_fields: seed.matched_fields.clone(), reasons: vec!["Structural query seed; relevance is not inferred from graph confidence".to_owned()],
                supporting_sources: Vec::new(), excerpts: Vec::new(),
            })).collect();
            let response = RankedDiscoveryResponse {
                eligible_count: None,
                bounded: discovery.truncated,
                discovery,
                candidates,
            };
            if enforce_ranked_budget {
                enforce_ranked_response_budget(&response)?;
            }
            guard.check()?;
            return Ok(response);
        }
        let search = SearchRequest {
            query: request.question.clone(),
            limits: CodeQueryLimits {
                max_candidates: request.limits.max_candidates,
                max_nodes: request.limits.max_nodes,
                max_response_bytes: request.limits.max_response_bytes,
                ..CodeQueryLimits::default()
            },
        };
        let scopes = self.ranked_scopes(&request, guard)?;
        let retrieval =
            self.retrieve_broad(&search, &scopes, &|| guard.check(), &|_| true, None, true)?;
        guard.check()?;
        let seeds = retrieval
            .ranked
            .iter()
            .take(request.limits.max_seeds as usize)
            .enumerate()
            .map(|(rank, candidate)| DiscoverySeed {
                node_id: candidate.node.id.clone(),
                score: format!("{:.6}", candidate.score),
                score_tier: DiscoveryScoreTier::Lexical,
                rank: rank as u32 + 1,
                matched_terms: candidate.matched_terms.clone(),
                matched_fields: candidate.matched_fields.clone(),
                source: candidate.node.source.clone(),
                candidate_source: DiscoverySeedSource::TermIndex,
                alternatives: Vec::new(),
                ambiguous: false,
            })
            .collect();
        let (selected_direction, direction_source) =
            crate::discovery::infer_discovery_direction(&request.question);
        let discovery = DiscoveryQueryResponse {
            schema: DISCOVERY_QUERY_SCHEMA_V1.to_owned(),
            question: request.question,
            selected_direction,
            direction_source,
            relation_contexts: request.relation_contexts,
            scope: scopes,
            traversal: request.traversal,
            seeds,
            nodes: retrieval
                .ranked
                .iter()
                .map(|candidate| candidate.node.clone())
                .collect(),
            edges: Vec::new(),
            diagnostics: self.broad_diagnostics(&retrieval),
            limits: request.limits,
            stats: DiscoveryStats {
                candidate_probes: retrieval.probes,
                candidate_nodes: retrieval.reads,
                candidates_admitted: retrieval.ranked.len() as u64,
                returned_nodes: retrieval.ranked.len() as u64,
                ..DiscoveryStats::default()
            },
            omissions: DiscoveryOmissions::default(),
            truncated: retrieval.bounded,
        };
        let response = RankedDiscoveryResponse {
            discovery,
            eligible_count: (!retrieval.bounded).then_some(retrieval.eligible as u64),
            bounded: retrieval.bounded,
            candidates: retrieval.ranked,
        };
        guard.check()?;
        if enforce_ranked_budget {
            enforce_ranked_response_budget(&response)?;
        }
        guard.check()?;
        Ok(response)
    }

    /// Optional witnessed context, continuing exact outgoing calls within the
    /// requested depth. Candidate rank is unaffected by
    /// adjacency degree and no context node becomes an implementation answer.
    pub fn discover_ranked_with_context(
        &self,
        request: DiscoveryQueryRequest,
        include_context: bool,
    ) -> Result<RankedDiscoveryResponse, QueryError> {
        crate::discovery::validate_request(&request)?;
        let guard =
            crate::discovery::DiscoveryGuard::new(request.limits.timeout_ms, None, self.deadline);
        self.discover_ranked_with_context_guarded(
            request,
            include_context,
            &guard,
            true,
            CallContextRoots::AllWitnessed,
        )
    }

    pub(crate) fn discover_ranked_with_context_guarded(
        &self,
        request: DiscoveryQueryRequest,
        include_context: bool,
        guard: &crate::discovery::DiscoveryGuard<'_>,
        enforce_ranked_budget: bool,
        call_roots: CallContextRoots,
    ) -> Result<RankedDiscoveryResponse, QueryError> {
        guard.check()?;
        let mut response =
            self.discover_ranked_guarded(request.clone(), guard, enforce_ranked_budget)?;
        guard.check()?;
        if !include_context {
            return Ok(response);
        }
        let backend = self.backend.pin_discovery()?;
        let fielded = self.is_fielded_question(&request.question)?
            && request.direction == DiscoveryDirection::Auto
            && request.relation_contexts.is_empty();
        // Structural discovery already owns its full neighborhood and work
        // meter. Optional presentation must not spend a second allowance or
        // replace the recorded cost with another traversal's counter.
        if !fielded {
            guard.check()?;
            return Ok(response);
        }
        let context_roots = response
            .candidates
            .iter()
            .take(5)
            .map(|candidate| candidate.node.clone())
            .collect::<Vec<_>>();
        let mut nodes = context_roots
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let mut edges = response
            .discovery
            .edges
            .iter()
            .cloned()
            .enumerate()
            .map(|(at, edge)| {
                (
                    edge.id.clone().unwrap_or_else(|| format!("anonymous:{at}")),
                    edge,
                )
            })
            .collect::<BTreeMap<_, _>>();
        let per_seed =
            (request.limits.max_edges as usize / response.discovery.seeds.len().max(1)).max(1);
        let mut bounded = false;
        let mut expanded = response.discovery.stats.expanded_relationships;
        for seed in &response.discovery.seeds {
            guard.check()?;
            let remaining_work = request
                .limits
                .max_expanded_relationships
                .saturating_sub(expanded);
            if remaining_work == 0 {
                bounded = true;
                break;
            }
            let (incident, limited, examined) = backend.incident_bounded_counted(
                &seed.node_id,
                request.include_heuristic,
                per_seed.min(remaining_work.saturating_sub(1) as usize),
            )?;
            expanded = expanded.saturating_add(examined as u64);
            bounded |= limited;
            for edge in incident {
                guard.check()?;
                if !crate::discovery::edge_matches_context(
                    &edge,
                    &response.discovery.relation_contexts,
                ) {
                    continue;
                }
                if (response.discovery.selected_direction == DiscoveryDirection::Incoming
                    && edge.target != seed.node_id)
                    || (response.discovery.selected_direction == DiscoveryDirection::Outgoing
                        && edge.source != seed.node_id)
                {
                    continue;
                }
                if edges.len() >= request.limits.max_edges as usize {
                    bounded = true;
                    break;
                }
                let neighbor = if edge.source == seed.node_id {
                    &edge.target
                } else {
                    &edge.source
                };
                if !nodes.contains_key(neighbor) {
                    if nodes.len() >= request.limits.max_nodes as usize {
                        bounded = true;
                        continue;
                    }
                    let Some(node) = backend.node_by_id(neighbor)? else {
                        continue;
                    };
                    if !discovery_scope_matches(&node, &response.discovery.scope) {
                        continue;
                    }
                    nodes.insert(node.id.clone(), query_node(&node));
                }
                edges.insert(edge.id.clone(), crate::discovery::discovery_edge(&edge));
            }
        }
        if fielded && response.discovery.selected_direction != DiscoveryDirection::Incoming {
            let continuation_seeds = response
                .discovery
                .seeds
                .iter()
                .filter(|seed| {
                    matches!(call_roots, CallContextRoots::AllWitnessed)
                        || response.candidates.iter().any(|candidate| {
                            candidate.node.id == seed.node_id
                                && candidate.relevance == RelevanceBand::High
                        })
                })
                .cloned()
                .collect::<Vec<_>>();
            bounded |= call_context::Context {
                scopes: &response.discovery.scope,
                nodes: &mut nodes,
                edges: &mut edges,
                seeds: &continuation_seeds,
                expanded: &mut expanded,
            }
            .extend(&backend, &request, &|| guard.check())?;
        }
        response.discovery.nodes = nodes.into_values().collect();
        response.discovery.edges = edges.into_values().collect();
        response.discovery.stats.returned_nodes = response.discovery.nodes.len() as u64;
        response.discovery.stats.returned_edges = response.discovery.edges.len() as u64;
        response.discovery.stats.expanded_relationships = expanded;
        response.discovery.stats.visited_nodes = response.discovery.nodes.len() as u64;
        response.discovery.truncated |= bounded;
        if bounded {
            response.discovery.diagnostics.push(QueryDiagnostic { code: QueryDiagnosticCode::BoundedTruncation, message: "Ranked context reached a node, edge or work bound; shortlist ranking is unchanged".to_owned(), node_id: None, path: None });
        }
        guard.check()?;
        if enforce_ranked_budget {
            enforce_ranked_response_budget(&response)?;
        }
        guard.check()?;
        Ok(response)
    }

    pub fn trace_ranked(
        &self,
        request: DiscoveryQueryRequest,
        expected_id: &str,
    ) -> Result<RankedRetrievalTrace, QueryError> {
        crate::discovery::validate_request(&request)?;
        let guard =
            crate::discovery::DiscoveryGuard::new(request.limits.timeout_ms, None, self.deadline);
        guard.check()?;
        let graph_present = self.backend.node_by_id(expected_id)?.is_some();
        let scopes = self.ranked_scopes(&request, &guard)?;
        let retrieval = self.retrieve_broad(
            &SearchRequest {
                query: request.question,
                limits: CodeQueryLimits {
                    max_candidates: request.limits.max_candidates,
                    max_nodes: request.limits.max_nodes,
                    ..CodeQueryLimits::default()
                },
            },
            &scopes,
            &|| guard.check(),
            &|_| true,
            Some(expected_id),
            false,
        )?;
        guard.check()?;
        let rank = retrieval.ranks.get(expected_id).copied();
        Ok(RankedRetrievalTrace {
            schema: "compass.query.retrieval-trace/1".to_owned(),
            graph_present,
            admitted: retrieval.admitted.contains(expected_id),
            rank,
            seed: rank.is_some_and(|rank| rank <= request.limits.max_seeds as usize),
            first_page: rank.is_some_and(|rank| rank <= 5),
            bounded: retrieval.bounded,
            target_details: retrieval.target_details,
        })
    }
}

#[cfg(test)]
mod comparative_tests {
    use super::*;

    #[test]
    fn canonical_once_concepts_do_not_share_spelling_groups() {
        let booking = compass_model::canonical_code_token("bookings".to_owned());
        assert_eq!(booking, "booking");
        let expected = BTreeMap::from([
            (
                "book".to_owned(),
                BTreeSet::from(["booker".to_owned(), "bookest".to_owned()]),
            ),
            (
                booking.clone(),
                BTreeSet::from(["bookinger".to_owned(), "bookingest".to_owned()]),
            ),
        ]);
        for input in [
            vec!["book".to_owned(), booking.clone()],
            vec![booking.clone(), "book".to_owned()],
        ] {
            assert_eq!(comparative_spellings(&input), expected);
        }
        assert_eq!(comparative_spellings(&[booking]).len(), 1);
    }

    #[test]
    fn spelling_generation_retains_fixed_per_concept_and_byte_bounds() {
        for (word, count) in [
            ("big".to_owned(), 0),
            ("fast".to_owned(), 2),
            ("a".repeat(32), 2),
            ("a".repeat(33), 0),
            ("Fast".to_owned(), 0),
            ("fast2".to_owned(), 0),
            ("café".to_owned(), 0),
        ] {
            assert_eq!(
                comparative_spellings(&[word])
                    .values()
                    .map(BTreeSet::len)
                    .sum::<usize>(),
                count
            );
        }
        let terms = (0..32)
            .map(|at| {
                format!(
                    "word{}{}",
                    char::from(b'a' + at / 26),
                    char::from(b'a' + at % 26)
                )
            })
            .collect::<Vec<_>>();
        let generated = comparative_spellings(&terms);
        assert_eq!(generated.len(), 32);
        assert!(generated.values().all(|v| v.len() <= 2));
        assert_eq!(generated.values().map(BTreeSet::len).sum::<usize>(), 64);
    }
}
