use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use compass_model::provenance::SourceAnchor;
use compass_model::query_contract::{
    DiscoveryDirection, DiscoveryDirectionSource, DiscoveryQueryResponse, DiscoveryScopeKind,
    DiscoveryScoreTier, DiscoverySeedSource, DiscoveryTraversal, QueryDiagnosticCode,
    QueryEvidence,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DISCOVERY_TEXT_PAGE_VERSION: &str = "compass.query.discovery-text-page/1";
const MAX_CURSOR_BYTES: usize = 4_096;
const MIN_TEXT_BUDGET: usize = 256;
const MAX_TEXT_BUDGET: usize = 65_536;
const MAX_RENDERED_SCALAR_CHARS: usize = 512;
const MAX_RENDERED_LIST_CHARS: usize = 2_048;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryTextPageOptions<'a> {
    pub token_budget: usize,
    pub cursor: Option<&'a str>,
    pub request_digest: &'a str,
    pub graph_identity: &'a str,
    pub graph_digest: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryTextPage {
    pub text: String,
    pub semantic_result_digest: String,
    pub next_cursor: Option<String>,
    pub entry_start: usize,
    pub entry_end: usize,
    pub entry_total: usize,
}

/// Digest the normalized semantic discovery request independently of text
/// pagination. Callers may change presentation budget while following a
/// cursor, but not the question, resolved scopes, contexts, traversal, or
/// execution limits.
pub fn discovery_request_digest(
    response: &DiscoveryQueryResponse,
    include_heuristic: bool,
) -> Result<String, serde_json::Error> {
    let mut contexts = response.relation_contexts.clone();
    contexts.sort();
    contexts.dedup();
    let mut scopes = response.scope.clone();
    scopes.sort_by(|left, right| {
        scope_kind_name(left.kind)
            .cmp(scope_kind_name(right.kind))
            .then_with(|| left.value.cmp(&right.value))
    });
    scopes.dedup_by(|left, right| left.kind == right.kind && left.value == right.value);
    let canonical = serde_json::json!({
        "question": response.question,
        "selectedDirection": response.selected_direction,
        "directionSource": response.direction_source,
        "relationContexts": contexts,
        "scope": scopes,
        "traversal": response.traversal,
        "includeHeuristic": include_heuristic,
        "limits": response.limits,
    });
    serde_json::to_vec(&canonical).map(|bytes| format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryTextPageError {
    #[error("--text-budget must be between {MIN_TEXT_BUDGET} and {MAX_TEXT_BUDGET}")]
    InvalidBudget,
    #[error("discovery cursor exceeds the {MAX_CURSOR_BYTES}-byte limit")]
    CursorTooLarge,
    #[error("invalid discovery cursor encoding")]
    InvalidCursorEncoding,
    #[error("unsupported discovery cursor version")]
    UnsupportedCursorVersion,
    #[error("discovery cursor checksum is invalid")]
    InvalidCursorChecksum,
    #[error("discovery cursor does not match the normalized question and options")]
    RequestChanged,
    #[error("discovery cursor does not match the selected graph generation")]
    GraphChanged,
    #[error("discovery cursor does not match the immutable semantic result")]
    ResultChanged,
    #[error("discovery cursor position is outside the semantic result")]
    CursorOutOfRange,
    #[error("one deterministic discovery entry exceeds --text-budget; increase the budget")]
    EntryTooLarge,
    #[error("discovery page metadata exceeds --text-budget; increase the budget")]
    PageMetadataTooLarge,
    #[error("could not serialize the discovery result: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone, Debug)]
struct Entry {
    section: &'static str,
    item: usize,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorEnvelope {
    version: String,
    request_digest: String,
    graph_identity: String,
    graph_digest: String,
    semantic_result_digest: String,
    section: String,
    item: usize,
    offset: usize,
}

pub fn render_discovery_text_page(
    response: &DiscoveryQueryResponse,
    options: DiscoveryTextPageOptions<'_>,
) -> Result<DiscoveryTextPage, DiscoveryTextPageError> {
    if !(MIN_TEXT_BUDGET..=MAX_TEXT_BUDGET).contains(&options.token_budget) {
        return Err(DiscoveryTextPageError::InvalidBudget);
    }
    for digest in [options.request_digest, options.graph_digest] {
        if !valid_digest(digest) {
            return Err(DiscoveryTextPageError::InvalidCursorEncoding);
        }
    }
    if options.graph_identity.is_empty() || options.graph_identity.len() > 512 {
        return Err(DiscoveryTextPageError::InvalidCursorEncoding);
    }
    let semantic_result_digest = digest(&canonical_response_bytes(response)?);
    let entries = entries(response);
    let start = match options.cursor {
        Some(cursor) => {
            let envelope = decode_cursor(cursor)?;
            validate_cursor(
                &envelope,
                options.request_digest,
                options.graph_identity,
                options.graph_digest,
                &semantic_result_digest,
                &entries,
            )?;
            envelope.offset
        }
        None => 0,
    };
    let ambiguity = response.seeds.iter().filter(|seed| seed.ambiguous).count();
    let fixed = vec![
        format!(
            "Discovery: {} seed(s), {} node(s), {} edge(s)",
            response.seeds.len(),
            response.nodes.len(),
            response.edges.len()
        ),
        format!(
            "Direction: {} ({})",
            direction_name(response.selected_direction),
            direction_source_name(response.direction_source)
        ),
        format!("Ambiguity: {ambiguity} ambiguous seed(s)"),
        format!(
            "Graph coverage: {}",
            if response
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == QueryDiagnosticCode::IncompleteCoverage)
            {
                "incomplete (incomplete_coverage diagnostic)"
            } else {
                "no incompleteness reported (coverage otherwise unknown)"
            },
        ),
        format!(
            "Domain result: {} (domainTruncated={})",
            if response.truncated {
                "partial"
            } else {
                "complete"
            },
            response.truncated
        ),
        format!("Traversal: {}", traversal_name(response.traversal)),
        format!(
            "Relationship contexts: {}",
            rendered_values(&response.relation_contexts)
        ),
        format!("Scope (OR): {}", rendered_scopes(response)),
        format!("Semantic result: sha256:{semantic_result_digest}"),
    ];
    let max_chars = options.token_budget.saturating_mul(4);
    let fixed_chars = fixed
        .iter()
        .map(|line| line.chars().count() + 1)
        .sum::<usize>();
    let mut end = start;
    let mut entries_chars = 0_usize;
    while let Some(entry) = entries.get(end) {
        let candidate_end = end + 1;
        let candidate_cursor =
            continuation_cursor(&entries, candidate_end, &options, &semantic_result_digest)?;
        let footer = footer(
            response,
            &semantic_result_digest,
            start,
            candidate_end,
            entries.len(),
            candidate_cursor.as_deref(),
        );
        let candidate_entry_chars = entry.text.chars().count().saturating_add(1);
        let footer_chars = footer
            .iter()
            .map(|line| line.chars().count() + 1)
            .sum::<usize>();
        if fixed_chars
            .saturating_add(entries_chars)
            .saturating_add(candidate_entry_chars)
            .saturating_add(footer_chars)
            > max_chars
        {
            if end == start {
                return Err(DiscoveryTextPageError::EntryTooLarge);
            }
            break;
        }
        entries_chars = entries_chars.saturating_add(candidate_entry_chars);
        end = candidate_end;
    }
    let next_cursor = continuation_cursor(&entries, end, &options, &semantic_result_digest)?;
    let mut lines = fixed;
    lines.extend(entries[start..end].iter().map(|entry| entry.text.clone()));
    let page_footer = footer(
        response,
        &semantic_result_digest,
        start,
        end,
        entries.len(),
        next_cursor.as_deref(),
    );
    if entries.is_empty()
        && fixed_chars.saturating_add(
            page_footer
                .iter()
                .map(|line| line.chars().count() + 1)
                .sum::<usize>(),
        ) > max_chars
    {
        return Err(DiscoveryTextPageError::PageMetadataTooLarge);
    }
    lines.extend(page_footer);
    Ok(DiscoveryTextPage {
        text: lines.join("\n"),
        semantic_result_digest,
        next_cursor,
        entry_start: start,
        entry_end: end,
        entry_total: entries.len(),
    })
}

fn continuation_cursor(
    entries: &[Entry],
    offset: usize,
    options: &DiscoveryTextPageOptions<'_>,
    semantic_result_digest: &str,
) -> Result<Option<String>, DiscoveryTextPageError> {
    entries
        .get(offset)
        .map(|entry| {
            encode_cursor(&CursorEnvelope {
                version: DISCOVERY_TEXT_PAGE_VERSION.to_owned(),
                request_digest: options.request_digest.to_owned(),
                graph_identity: options.graph_identity.to_owned(),
                graph_digest: options.graph_digest.to_owned(),
                semantic_result_digest: semantic_result_digest.to_owned(),
                section: entry.section.to_owned(),
                item: entry.item,
                offset,
            })
        })
        .transpose()
}

fn footer(
    response: &DiscoveryQueryResponse,
    semantic_result_digest: &str,
    start: usize,
    end: usize,
    entry_total: usize,
    next_cursor: Option<&str>,
) -> [String; 2] {
    [
        format!(
            "Completeness: {} (candidates={}, alternatives={}, nodes={}, edges={}, expandedRelationships={})",
            if response.truncated {
                "partial"
            } else {
                "complete"
            },
            omission(response.omissions.candidates),
            omission(response.omissions.alternatives),
            omission(response.omissions.nodes),
            omission(response.omissions.edges),
            omission(response.omissions.expanded_relationships),
        ),
        format!(
            "Pagination: version={} digest=sha256:{} range={}-{} of {} next={}",
            DISCOVERY_TEXT_PAGE_VERSION,
            semantic_result_digest,
            if entry_total == 0 { 0 } else { start + 1 },
            end,
            entry_total,
            next_cursor.unwrap_or("none")
        ),
    ]
}

fn entries(response: &DiscoveryQueryResponse) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut alternative_item = 0_usize;
    let mut node_evidence_item = 0_usize;
    let mut edge_evidence_item = 0_usize;
    for (item, seed) in response.seeds.iter().enumerate() {
        entries.push(Entry {
            section: "seeds",
            item,
            text: format!(
                "Seed: {} [{}; source={}; score={}; matchedFields={}; matchedTerms={}]{}",
                rendered_scalar(&seed.node_id),
                score_tier_name(seed.score_tier),
                seed_source_name(seed.candidate_source),
                rendered_scalar(&seed.score),
                rendered_values(&seed.matched_fields),
                rendered_values(&seed.matched_terms),
                seed.source
                    .as_ref()
                    .map(|source| format!(" @ {}", rendered_anchor(source)))
                    .unwrap_or_default(),
            ),
        });
        for alternative in &seed.alternatives {
            entries.push(Entry {
                section: "alternatives",
                item: alternative_item,
                text: format!(
                    "Alternative: seed={} node={} qualifiedName={} score={}{}",
                    rendered_scalar(&seed.node_id),
                    rendered_scalar(&alternative.node_id),
                    rendered_scalar(&alternative.qualified_name),
                    rendered_scalar(&alternative.score),
                    alternative
                        .source
                        .as_ref()
                        .map(|source| format!(" @ {}", rendered_anchor(source)))
                        .unwrap_or_default(),
                ),
            });
            alternative_item += 1;
        }
    }
    for (item, node) in response.nodes.iter().enumerate() {
        entries.push(Entry {
            section: "nodes",
            item,
            text: format!(
                "Node: {} [{}] {}{} [evidence={}; details={}]",
                rendered_scalar(&node.id),
                node.kind.as_str(),
                rendered_scalar(&node.qualified_name),
                node.source
                    .as_ref()
                    .map(|source| format!(" @ {}", rendered_anchor(source)))
                    .unwrap_or_default(),
                node.evidence.len(),
                rendered_details(node.details.as_ref()),
            ),
        });
        for (evidence_index, evidence) in node.evidence.iter().enumerate() {
            entries.push(Entry {
                section: "node_evidence",
                item: node_evidence_item,
                text: rendered_evidence("Node evidence", &node.id, evidence_index, evidence),
            });
            node_evidence_item += 1;
        }
    }
    for (item, edge) in response.edges.iter().enumerate() {
        let site = edge.relationship_site.as_ref().or_else(|| {
            edge.evidence
                .iter()
                .find_map(|evidence| evidence.anchor.as_ref().or(evidence.wiring_site.as_ref()))
        });
        entries.push(Entry {
            section: "edges",
            item,
            text: format!(
                "Edge #{}: {} -{}-> {} [id={}; context={}; site={}; occurrenceRule={}; evidence={}; details={}]",
                item + 1,
                rendered_scalar(&edge.source),
                edge.kind.as_str(),
                rendered_scalar(&edge.target),
                edge.id.as_deref().map_or_else(
                    || "unavailable".to_owned(),
                    rendered_scalar,
                ),
                edge.context
                    .as_deref()
                    .map_or_else(|| "none".to_owned(), rendered_scalar),
                site.map_or_else(|| "unavailable".to_owned(), rendered_anchor),
                rendered_details(edge.occurrence_rule.as_ref()),
                edge.evidence.len(),
                rendered_details(edge.details.as_ref()),
            ),
        });
        let edge_identity = edge
            .id
            .as_deref()
            .map_or_else(|| format!("anonymous#{}", item + 1), rendered_scalar);
        for (evidence_index, evidence) in edge.evidence.iter().enumerate() {
            entries.push(Entry {
                section: "edge_evidence",
                item: edge_evidence_item,
                text: rendered_evidence("Edge evidence", &edge_identity, evidence_index, evidence),
            });
            edge_evidence_item += 1;
        }
    }
    for (item, diagnostic) in response.diagnostics.iter().enumerate() {
        entries.push(Entry {
            section: "diagnostics",
            item,
            text: format!(
                "! {:?}: {} node={} path={}",
                diagnostic.code,
                rendered_scalar(&diagnostic.message),
                diagnostic
                    .node_id
                    .as_deref()
                    .map_or_else(|| "none".to_owned(), rendered_scalar),
                diagnostic
                    .path
                    .as_deref()
                    .map_or_else(|| "none".to_owned(), rendered_scalar),
            ),
        });
    }
    entries
}

fn canonical_response_bytes(
    response: &DiscoveryQueryResponse,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut canonical = response.clone();
    canonical.relation_contexts.sort();
    canonical.relation_contexts.dedup();
    canonical.scope.sort_by(|left, right| {
        scope_kind_name(left.kind)
            .cmp(scope_kind_name(right.kind))
            .then_with(|| left.value.cmp(&right.value))
    });
    canonical
        .scope
        .dedup_by(|left, right| left.kind == right.kind && left.value == right.value);
    serde_json::to_vec(&canonical)
}

fn validate_cursor(
    cursor: &CursorEnvelope,
    request_digest: &str,
    graph_identity: &str,
    graph_digest: &str,
    result_digest: &str,
    entries: &[Entry],
) -> Result<(), DiscoveryTextPageError> {
    if cursor.version != DISCOVERY_TEXT_PAGE_VERSION {
        return Err(DiscoveryTextPageError::UnsupportedCursorVersion);
    }
    if cursor.request_digest != request_digest {
        return Err(DiscoveryTextPageError::RequestChanged);
    }
    if cursor.graph_identity != graph_identity || cursor.graph_digest != graph_digest {
        return Err(DiscoveryTextPageError::GraphChanged);
    }
    if cursor.semantic_result_digest != result_digest {
        return Err(DiscoveryTextPageError::ResultChanged);
    }
    let Some(entry) = entries.get(cursor.offset) else {
        return Err(DiscoveryTextPageError::CursorOutOfRange);
    };
    if cursor.section != entry.section || cursor.item != entry.item {
        return Err(DiscoveryTextPageError::CursorOutOfRange);
    }
    Ok(())
}

fn encode_cursor(cursor: &CursorEnvelope) -> Result<String, DiscoveryTextPageError> {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor)?);
    let checksum = digest(payload.as_bytes());
    Ok(format!("{payload}.{checksum}"))
}

fn decode_cursor(value: &str) -> Result<CursorEnvelope, DiscoveryTextPageError> {
    if value.len() > MAX_CURSOR_BYTES {
        return Err(DiscoveryTextPageError::CursorTooLarge);
    }
    let (payload, checksum) = value
        .split_once('.')
        .ok_or(DiscoveryTextPageError::InvalidCursorEncoding)?;
    if !valid_digest(checksum) || digest(payload.as_bytes()) != checksum {
        return Err(DiscoveryTextPageError::InvalidCursorChecksum);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| DiscoveryTextPageError::InvalidCursorEncoding)?;
    serde_json::from_slice(&bytes).map_err(|_| DiscoveryTextPageError::InvalidCursorEncoding)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn rendered_values(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_owned()
    } else {
        rendered_list(values.iter().map(|value| rendered_scalar(value)))
    }
}

fn rendered_scopes(response: &DiscoveryQueryResponse) -> String {
    if response.scope.is_empty() {
        return "none".to_owned();
    }
    rendered_list(response.scope.iter().map(|scope| {
        format!(
            "{}:{}",
            scope_kind_name(scope.kind),
            rendered_scalar(&scope.value)
        )
    }))
}

fn rendered_list(values: impl IntoIterator<Item = String>) -> String {
    let mut rendered = String::new();
    let mut omitted = false;
    for value in values {
        let separator = if rendered.is_empty() { "" } else { "," };
        if rendered
            .chars()
            .count()
            .saturating_add(separator.chars().count())
            .saturating_add(value.chars().count())
            > MAX_RENDERED_LIST_CHARS
        {
            omitted = true;
            break;
        }
        rendered.push_str(separator);
        rendered.push_str(&value);
    }
    if omitted {
        rendered.push('…');
    }
    if rendered.is_empty() {
        "none".to_owned()
    } else {
        rendered
    }
}

fn rendered_scalar(value: &str) -> String {
    let mut fragments = Vec::new();
    let mut rendered_chars = 0_usize;
    let mut omitted = false;
    for character in value.chars() {
        let fragment = match character {
            '\r' | '\n' | '\t' | '\u{2028}' | '\u{2029}' => " ".to_owned(),
            value if value.is_control() || is_bidi_control(value) => {
                format!("U+{:04X}", u32::from(value))
            }
            value => value.to_string(),
        };
        let fragment_chars = fragment.chars().count();
        if rendered_chars.saturating_add(fragment_chars) > MAX_RENDERED_SCALAR_CHARS {
            omitted = true;
            break;
        }
        rendered_chars = rendered_chars.saturating_add(fragment_chars);
        fragments.push(fragment);
    }
    if omitted {
        while rendered_chars.saturating_add(1) > MAX_RENDERED_SCALAR_CHARS {
            let Some(fragment) = fragments.pop() else {
                break;
            };
            rendered_chars = rendered_chars.saturating_sub(fragment.chars().count());
        }
        fragments.push("…".to_owned());
    }
    fragments.concat()
}

const fn is_bidi_control(value: char) -> bool {
    matches!(
        value,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

fn rendered_anchor(anchor: &SourceAnchor) -> String {
    format!(
        "{}:{}:{}-{}:{}",
        rendered_scalar(&anchor.file),
        anchor.start_line,
        anchor.start_column,
        anchor.end_line,
        anchor.end_column
    )
}

fn rendered_details<T: Serialize>(details: Option<&T>) -> String {
    details.map_or_else(
        || "none".to_owned(),
        |details| {
            serde_json::to_string(details)
                .map_or_else(|_| "invalid".to_owned(), |value| rendered_scalar(&value))
        },
    )
}

fn rendered_evidence(
    label: &str,
    owner: &str,
    evidence_index: usize,
    evidence: &QueryEvidence,
) -> String {
    let candidates = rendered_list(evidence.candidates.iter().map(|candidate| {
        format!(
            "{}|{}|{}|score={}|anchor={}",
            rendered_scalar(&candidate.node_id),
            rendered_scalar(&candidate.reason),
            candidate.confidence.as_str(),
            candidate
                .score
                .map_or_else(|| "none".to_owned(), |score| score.to_string()),
            candidate
                .anchor
                .as_ref()
                .map_or_else(|| "none".to_owned(), rendered_anchor),
        )
    }));
    format!(
        "{label}: owner={} index={} layer={:?} origin={} extractor={} confidence={} resolution={:?} rule={} anchor={} wiringSite={} candidates={}",
        rendered_scalar(owner),
        evidence_index + 1,
        evidence.layer,
        evidence.origin.as_str(),
        rendered_scalar(&evidence.extractor),
        evidence.confidence.as_str(),
        evidence.resolution,
        evidence
            .rule
            .as_deref()
            .map_or_else(|| "none".to_owned(), rendered_scalar),
        evidence
            .anchor
            .as_ref()
            .map_or_else(|| "none".to_owned(), rendered_anchor),
        evidence
            .wiring_site
            .as_ref()
            .map_or_else(|| "none".to_owned(), rendered_anchor),
        candidates,
    )
}
fn omission(value: Option<u64>) -> String {
    value.map_or_else(|| "unknown".to_owned(), |value| value.to_string())
}
fn direction_name(value: DiscoveryDirection) -> &'static str {
    match value {
        DiscoveryDirection::Auto => "auto",
        DiscoveryDirection::Incoming => "incoming",
        DiscoveryDirection::Outgoing => "outgoing",
        DiscoveryDirection::Both => "both",
    }
}
fn direction_source_name(value: DiscoveryDirectionSource) -> &'static str {
    match value {
        DiscoveryDirectionSource::Explicit => "explicit",
        DiscoveryDirectionSource::Heuristic => "heuristic",
        DiscoveryDirectionSource::Neutral => "neutral",
    }
}
fn traversal_name(value: DiscoveryTraversal) -> &'static str {
    match value {
        DiscoveryTraversal::Bfs => "bfs",
        DiscoveryTraversal::Dfs => "dfs",
    }
}
fn scope_kind_name(value: DiscoveryScopeKind) -> &'static str {
    match value {
        DiscoveryScopeKind::Community => "community",
        DiscoveryScopeKind::Source => "source",
        DiscoveryScopeKind::Package => "package",
        DiscoveryScopeKind::Node => "node",
    }
}
fn score_tier_name(value: DiscoveryScoreTier) -> &'static str {
    match value {
        DiscoveryScoreTier::ExactId => "exact_id",
        DiscoveryScoreTier::ExactName => "exact_name",
        DiscoveryScoreTier::Lexical => "lexical",
    }
}
fn seed_source_name(value: DiscoverySeedSource) -> &'static str {
    match value {
        DiscoverySeedSource::ExactId => "exact_id",
        DiscoverySeedSource::ExactName => "exact_name",
        DiscoverySeedSource::Alias => "alias",
        DiscoverySeedSource::TermIndex => "term_index",
        DiscoverySeedSource::RelationSeed => "relation_seed",
        DiscoverySeedSource::Fuzzy => "fuzzy",
        DiscoverySeedSource::HeuristicFallback => "heuristic_fallback",
    }
}

#[cfg(test)]
mod tests {
    use compass_model::query_contract::DiscoveryQueryResponse;

    use super::*;

    fn response() -> Result<DiscoveryQueryResponse, serde_json::Error> {
        serde_json::from_value(serde_json::json!({
            "schema": "compass.query.discovery/1",
            "question": "Target",
            "selectedDirection": "incoming",
            "directionSource": "explicit",
            "relationContexts": [],
            "scope": [],
            "traversal": "bfs",
            "seeds": [],
            "nodes": [],
            "edges": [],
            "diagnostics": [
                {"code":"no_match", "message":"first"},
                {"code":"no_match", "message":"second"}
            ],
            "limits": {
                "maxDepth": 2, "maxSeeds": 3, "maxCandidates": 256,
                "maxNodes": 500, "maxEdges": 1000,
                "maxExpandedRelationships": 10000,
                "maxResponseBytes": 8388608, "timeoutMs": 30000
            },
            "stats": {
                "candidateProbes": 0, "candidateNodes": 0,
                "candidatesAdmitted": 0, "visitedNodes": 0,
                "expandedRelationships": 0, "returnedNodes": 0,
                "returnedEdges": 0
            },
            "omissions": {},
            "truncated": false
        }))
    }

    #[test]
    fn request_digest_normalizes_set_like_contexts_and_scopes()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut left = serde_json::to_value(response()?)?;
        left["relationContexts"] = serde_json::json!(["route", "call", "route"]);
        left["scope"] = serde_json::json!([
            {"kind":"source", "value":"src/lib.rs"},
            {"kind":"node", "value":"target"},
            {"kind":"source", "value":"src/lib.rs"}
        ]);
        let mut right = left.clone();
        right["relationContexts"] = serde_json::json!(["call", "route"]);
        right["scope"] = serde_json::json!([
            {"kind":"node", "value":"target"},
            {"kind":"source", "value":"src/lib.rs"}
        ]);
        let left = serde_json::from_value(left)?;
        let right = serde_json::from_value(right)?;
        assert_eq!(
            discovery_request_digest(&left, false)?,
            discovery_request_digest(&right, false)?
        );
        assert_ne!(
            discovery_request_digest(&left, false)?,
            discovery_request_digest(&right, true)?
        );
        Ok(())
    }

    #[test]
    fn cursor_is_bound_to_request_graph_result_and_position()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut response = response()?;
        response.diagnostics[0].message = "x".repeat(700);
        response.diagnostics[1].message = "y".repeat(700);
        let mut third = response.diagnostics[1].clone();
        third.message = "z".repeat(700);
        response.diagnostics.push(third);
        let first = render_discovery_text_page(
            &response,
            DiscoveryTextPageOptions {
                token_budget: 512,
                cursor: None,
                request_digest: &"a".repeat(64),
                graph_identity: "generation-1",
                graph_digest: &"b".repeat(64),
            },
        )?;
        let cursor = first.next_cursor.ok_or("expected a continuation")?;
        let second = render_discovery_text_page(
            &response,
            DiscoveryTextPageOptions {
                token_budget: 512,
                cursor: Some(&cursor),
                request_digest: &"a".repeat(64),
                graph_identity: "generation-1",
                graph_digest: &"b".repeat(64),
            },
        )?;
        assert_eq!(second.entry_start, first.entry_end);
        assert!(second.text.contains("Direction: incoming (explicit)"));
        assert!(
            second.text.contains(
                "Graph coverage: no incompleteness reported (coverage otherwise unknown)"
            )
        );
        assert!(
            second
                .text
                .contains("Domain result: complete (domainTruncated=false)")
        );

        let changed = render_discovery_text_page(
            &response,
            DiscoveryTextPageOptions {
                token_budget: 512,
                cursor: Some(&cursor),
                request_digest: &"c".repeat(64),
                graph_identity: "generation-1",
                graph_digest: &"b".repeat(64),
            },
        );
        assert!(matches!(
            changed,
            Err(DiscoveryTextPageError::RequestChanged)
        ));

        let mut tampered = cursor;
        tampered.push('x');
        assert!(matches!(
            render_discovery_text_page(
                &response,
                DiscoveryTextPageOptions {
                    token_budget: 512,
                    cursor: Some(&tampered),
                    request_digest: &"a".repeat(64),
                    graph_identity: "generation-1",
                    graph_digest: &"b".repeat(64),
                },
            ),
            Err(DiscoveryTextPageError::InvalidCursorChecksum)
        ));
        Ok(())
    }

    #[test]
    fn pages_cover_each_semantic_entry_once_and_detect_every_identity_change()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut response = response()?;
        let template = response.diagnostics[0].clone();
        response.diagnostics = (0..5)
            .map(|index| {
                let mut diagnostic = template.clone();
                diagnostic.message = format!("{index}:{}", "x".repeat(700));
                diagnostic
            })
            .collect();
        let request_digest = "a".repeat(64);
        let graph_digest = "b".repeat(64);
        let mut cursor = None::<String>;
        let mut covered = Vec::new();
        let mut semantic_digest = None::<String>;
        loop {
            let page = render_discovery_text_page(
                &response,
                DiscoveryTextPageOptions {
                    token_budget: 512,
                    cursor: cursor.as_deref(),
                    request_digest: &request_digest,
                    graph_identity: "generation-1",
                    graph_digest: &graph_digest,
                },
            )?;
            if let Some(expected) = &semantic_digest {
                assert_eq!(expected, &page.semantic_result_digest);
            } else {
                semantic_digest = Some(page.semantic_result_digest.clone());
            }
            covered.extend(page.entry_start..page.entry_end);
            cursor = page.next_cursor;
            if cursor.is_none() {
                assert_eq!(page.entry_end, page.entry_total);
                break;
            }
        }
        assert_eq!(covered, (0..5).collect::<Vec<_>>());

        let wide = render_discovery_text_page(
            &response,
            DiscoveryTextPageOptions {
                token_budget: 1_024,
                cursor: None,
                request_digest: &request_digest,
                graph_identity: "generation-1",
                graph_digest: &graph_digest,
            },
        )?;
        assert_eq!(
            semantic_digest.as_deref(),
            Some(wide.semantic_result_digest.as_str())
        );

        let first = render_discovery_text_page(
            &response,
            DiscoveryTextPageOptions {
                token_budget: 512,
                cursor: None,
                request_digest: &request_digest,
                graph_identity: "generation-1",
                graph_digest: &graph_digest,
            },
        )?;
        let first_cursor = first.next_cursor.ok_or("expected continuation")?;
        let changed_graph = render_discovery_text_page(
            &response,
            DiscoveryTextPageOptions {
                token_budget: 512,
                cursor: Some(&first_cursor),
                request_digest: &request_digest,
                graph_identity: "generation-2",
                graph_digest: &graph_digest,
            },
        );
        assert!(matches!(
            changed_graph,
            Err(DiscoveryTextPageError::GraphChanged)
        ));

        let mut changed_response = response.clone();
        changed_response.diagnostics[0].message.push('!');
        let changed_result = render_discovery_text_page(
            &changed_response,
            DiscoveryTextPageOptions {
                token_budget: 512,
                cursor: Some(&first_cursor),
                request_digest: &request_digest,
                graph_identity: "generation-1",
                graph_digest: &graph_digest,
            },
        );
        assert!(matches!(
            changed_result,
            Err(DiscoveryTextPageError::ResultChanged)
        ));

        let mut out_of_range = decode_cursor(&first_cursor)?;
        out_of_range.offset = usize::MAX;
        out_of_range.section = "diagnostics".to_owned();
        out_of_range.item = usize::MAX;
        let out_of_range = encode_cursor(&out_of_range)?;
        assert!(matches!(
            render_discovery_text_page(
                &response,
                DiscoveryTextPageOptions {
                    token_budget: 512,
                    cursor: Some(&out_of_range),
                    request_digest: &request_digest,
                    graph_identity: "generation-1",
                    graph_digest: &graph_digest,
                },
            ),
            Err(DiscoveryTextPageError::CursorOutOfRange)
        ));
        Ok(())
    }

    #[test]
    fn hostile_graph_text_is_bounded_sanitized_and_keeps_provenance()
    -> Result<(), Box<dyn std::error::Error>> {
        let anchor = serde_json::json!({
            "file": "src/evil\nPagination: forged\u{001b}[31m\u{202e}file.rs",
            "startByte": 0, "endByte": 1,
            "startLine": 7, "startColumn": 2, "endLine": 7, "endColumn": 3
        });
        let evidence = serde_json::json!({
            "layer": "structural_graph", "origin": "ast",
            "extractor": "evil\nGraph coverage: forged\u{001b}[2J",
            "confidence": "exact", "anchor": anchor,
            "rule": "direct\rfooter", "wiringSite": null,
            "resolution": "exact", "candidates": []
        });
        let mut value = serde_json::to_value(response()?)?;
        value["relationContexts"] = serde_json::json!(["call\nPagination: forged"]);
        value["scope"] = serde_json::json!([{
            "kind":"source", "value":"src\u{2028}forged"
        }]);
        value["nodes"] = serde_json::json!([{
            "id":"node\nPagination: forged\u{001b}[31m",
            "kind":"function", "roles":[], "name":"evil",
            "qualifiedName":"Fixture.Evil\u{202e}", "language":"rust",
            "framework":null, "source":anchor, "details":null,
            "evidence":[evidence]
        }]);
        value["edges"] = serde_json::json!([{
            "id":null, "source":"source\nforged", "target":"target\u{001b}[0m",
            "kind":"calls", "occurrenceRule":null,
            "relationshipSite":anchor, "details":null,
            "evidence":[evidence], "context":"call\rforged"
        }]);
        value["diagnostics"] = serde_json::json!([{
            "code":"incomplete_coverage",
            "message":"missing\nPagination: forged\u{001b}[2J\u{202e}",
            "nodeId":null, "path":null
        }]);
        value["truncated"] = serde_json::json!(true);
        let response = serde_json::from_value(value)?;
        let page = render_discovery_text_page(
            &response,
            DiscoveryTextPageOptions {
                token_budget: MAX_TEXT_BUDGET,
                cursor: None,
                request_digest: &"a".repeat(64),
                graph_identity: "generation-1",
                graph_digest: &"b".repeat(64),
            },
        )?;

        assert_eq!(
            page.text
                .lines()
                .filter(|line| line.starts_with("Pagination:"))
                .count(),
            1
        );
        assert!(!page.text.contains('\u{1b}'));
        assert!(!page.text.contains('\u{202e}'));
        assert!(page.text.contains("U+001B"));
        assert!(page.text.contains("U+202E"));
        assert!(
            page.text
                .contains("Graph coverage: incomplete (incomplete_coverage diagnostic)")
        );
        assert!(
            page.text
                .contains("Domain result: partial (domainTruncated=true)")
        );
        assert!(page.text.contains("Traversal: bfs"));
        assert!(page.text.contains("Relationship contexts:"));
        assert!(page.text.contains("Scope (OR): source:"));
        assert!(
            page.text
                .contains("Edge #1: source forged -calls-> targetU+001B[0m")
        );
        assert!(
            page.text
                .contains("site=src/evil Pagination: forgedU+001B[31mU+202Efile.rs:7:2-7:3")
        );
        assert!(page.text.contains("Node evidence:"));
        assert!(page.text.contains("Edge evidence:"));
        assert!(page.text.contains("confidence=exact resolution=Exact"));
        Ok(())
    }

    #[test]
    fn alternatives_and_evidence_are_separate_bounded_page_entries()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut value = serde_json::to_value(response()?)?;
        value["diagnostics"] = serde_json::json!([]);
        value["seeds"] = serde_json::json!([{
            "nodeId":"seed", "score":"1", "scoreTier":"exact_name", "rank":1,
            "matchedTerms":["seed"], "matchedFields":["name"], "source":null,
            "candidateSource":"exact_name", "ambiguous":true,
            "alternatives": (0..400).map(|index| serde_json::json!({
                "nodeId":format!("alternative-{index}-{}", "x".repeat(700)),
                "qualifiedName":format!("Fixture.Alternative{index}.{}", "y".repeat(700)),
                "source":null, "score":"z".repeat(700)
            })).collect::<Vec<_>>()
        }]);
        let response = serde_json::from_value(value)?;
        let request_digest = "a".repeat(64);
        let graph_digest = "b".repeat(64);
        let mut cursor = None::<String>;
        let mut covered = Vec::new();
        loop {
            let page = render_discovery_text_page(
                &response,
                DiscoveryTextPageOptions {
                    token_budget: MAX_TEXT_BUDGET,
                    cursor: cursor.as_deref(),
                    request_digest: &request_digest,
                    graph_identity: "generation-1",
                    graph_digest: &graph_digest,
                },
            )?;
            covered.extend(page.entry_start..page.entry_end);
            cursor = page.next_cursor;
            if cursor.is_none() {
                assert_eq!(page.entry_end, page.entry_total);
                break;
            }
        }
        assert_eq!(covered, (0..401).collect::<Vec<_>>());
        assert!(
            entries(&response)
                .iter()
                .all(|entry| entry.text.len() < 8_192)
        );
        Ok(())
    }
}
