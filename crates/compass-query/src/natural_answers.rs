//! Best-effort operand selection for natural language. Exact structured APIs
//! deliberately retain their strict ambiguity behavior.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

use compass_model::code_graph::{EdgeKind, NodeKind};
use compass_model::query_contract::{
    CodeQueryLimits, CodeQueryOperation, CodeQueryResponse, QueryDiagnostic, QueryDiagnosticCode,
    QueryNode, SearchRequest,
};

use crate::QueryError;
use crate::code_query::{CodeQueryEngine, normalize_symbol, query_edge, query_node};
use crate::telemetry::QueryInstrumentation;

const NATURAL_ALTERNATIVES: usize = 4;
const CONNECTIVITY_PROBE: usize = 32;
const NATURAL_RANK_CANDIDATES: usize = 32;

impl CodeQueryEngine {
    pub(crate) fn select_natural_symbol(
        &self,
        operand: &str,
        question: &str,
        limits: &CodeQueryLimits,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<(Option<String>, Vec<QueryDiagnostic>, bool), QueryError> {
        self.check_deadline()?;
        if let Some(node) = self.backend.node_by_id(operand)? {
            instrumentation.work.candidates_read += 1;
            return Ok((Some(node.id), Vec::new(), false));
        }
        let normalized = normalize_symbol(operand);
        let (exact_nodes, exact_truncated) = self.backend.nodes_by_normalized_name(
            &normalized,
            usize::try_from(limits.max_candidates).unwrap_or(usize::MAX),
        )?;
        instrumentation.work.candidates_read +=
            u64::try_from(exact_nodes.len()).unwrap_or(u64::MAX);
        let has_exact = !exact_nodes.is_empty();
        let (mut candidates, mut diagnostics, mut candidate_truncated) = if has_exact {
            (
                exact_nodes
                    .iter()
                    .map(|node| (query_node(node), 1.0))
                    .collect::<Vec<_>>(),
                Vec::new(),
                exact_truncated,
            )
        } else {
            let search = self.search_instrumented(
                SearchRequest {
                    query: operand.to_owned(),
                    limits: limits.clone(),
                },
                instrumentation,
            )?;
            let candidates = search
                .results
                .iter()
                .filter_map(|hit| {
                    search
                        .nodes
                        .iter()
                        .find(|node| node.id == hit.node_id)
                        .filter(|node| natural_fuzzy_relevant(operand, node))
                        .map(|node| (node.clone(), hit.score))
                })
                .collect();
            (candidates, search.diagnostics, search.truncated)
        };
        let wants_tests = question.split(|c: char| !c.is_alphanumeric()).any(|word| {
            matches!(
                word.to_ascii_lowercase().as_str(),
                "test" | "tests" | "testing"
            )
        });
        candidates.sort_by(|(left, left_score), (right, right_score)| {
            u8::from(natural_test_source(right) == wants_tests)
                .cmp(&u8::from(natural_test_source(left) == wants_tests))
                .then_with(|| {
                    u8::from(natural_declaration(right)).cmp(&u8::from(natural_declaration(left)))
                })
                .then_with(|| right_score.total_cmp(left_score))
                .then_with(|| left.id.cmp(&right.id))
        });
        candidate_truncated |= candidates.len() > NATURAL_RANK_CANDIDATES;
        let mut ranked = Vec::new();
        // Apply source preferences before the connectivity probe cap, so a
        // request about tests can select a test behind many production matches.
        for (node, score) in candidates.iter().take(NATURAL_RANK_CANDIDATES) {
            self.check_deadline()?;
            let is_test = natural_test_source(node);
            let declaration = natural_declaration(node);
            let (incoming, _) = self.backend.matching_bounded(
                &node.id,
                true,
                NATURAL_DEPENDENCY_KINDS,
                false,
                CONNECTIVITY_PROBE,
            )?;
            let (outgoing, _) = self.backend.matching_bounded(
                &node.id,
                false,
                NATURAL_DEPENDENCY_KINDS,
                false,
                CONNECTIVITY_PROBE,
            )?;
            instrumentation.work.edges_expanded +=
                u64::try_from(incoming.len() + outgoing.len()).unwrap_or(u64::MAX);
            ranked.push((
                node,
                u8::from(is_test == wants_tests),
                u8::from(declaration),
                *score,
                incoming.len() + outgoing.len(),
            ));
        }
        ranked.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| {
                    if has_exact {
                        std::cmp::Ordering::Equal
                    } else {
                        right.3.total_cmp(&left.3)
                    }
                })
                .then_with(|| right.4.cmp(&left.4))
                .then_with(|| left.0.id.cmp(&right.0.id))
        });
        let Some((selected, ..)) = ranked.first() else {
            return Ok((None, diagnostics, candidate_truncated));
        };
        let alternatives = ranked
            .iter()
            .skip(1)
            .take(NATURAL_ALTERNATIVES)
            .map(|(node, ..)| format!("{} ({})", node.qualified_name, node.id))
            .collect::<Vec<_>>();
        if !has_exact {
            // This is a statement about operand matching, not uncertainty in
            // the witnessed edges subsequently returned by the typed query.
            diagnostics.retain(|diagnostic| diagnostic.code != QueryDiagnosticCode::NoMatch);
            diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: format!("NO EXACT MATCH for {operand:?}; approximate — selected {} by fuzzy/lexical matching{}",
                    selected.qualified_name,
                    if alternatives.is_empty() { String::new() } else { format!("; also matched: {}", alternatives.join(", ")) }),
                node_id: Some(selected.id.clone()), path: None,
            });
        } else if !alternatives.is_empty() || candidate_truncated {
            diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::AmbiguousMatch,
                message: format!("Auto-picked {} for {operand:?} using declaration, source scope and bounded connectivity; also matched: {}{}",
                    selected.qualified_name, alternatives.join(", "),
                    if candidate_truncated { "; candidate coverage incomplete" } else { "" }),
                node_id: Some(selected.id.clone()), path: None,
            });
        }
        Ok((Some(selected.id.clone()), diagnostics, candidate_truncated))
    }

    pub(crate) fn natural_dependencies(
        &self,
        symbol: &str,
        include_heuristic: bool,
        limits: CodeQueryLimits,
        instrumentation: &mut QueryInstrumentation,
    ) -> Result<CodeQueryResponse, QueryError> {
        let started = Instant::now();
        let mut response = CodeQueryResponse::empty(CodeQueryOperation::Explore, limits.clone());
        let Some(seed) = self.backend.node_by_id(symbol)? else {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::NoMatch,
                message: format!("NO EXACT MATCH for {symbol:?}"),
                node_id: None,
                path: None,
            });
            return Ok(response);
        };
        let max_nodes = usize::try_from(limits.max_nodes).unwrap_or(usize::MAX);
        let max_edges = usize::try_from(limits.max_edges).unwrap_or(usize::MAX);
        let mut nodes = BTreeMap::from([(seed.id.clone(), query_node(&seed))]);
        let mut seen = BTreeSet::new();
        let mut pending = VecDeque::from([(seed.id.clone(), 0)]);
        let mut edges = BTreeMap::new();
        while let Some((owner, depth)) = pending.pop_front() {
            self.check_deadline()?;
            if !seen.insert(owner.clone()) {
                continue;
            }
            instrumentation.work.nodes_expanded += 1;
            let remaining = max_edges.saturating_sub(edges.len());
            if remaining == 0 {
                response.truncated = true;
                break;
            }
            let (outgoing, truncated) = self.backend.matching_bounded(
                &owner,
                false,
                NATURAL_OUTGOING_KINDS,
                include_heuristic,
                remaining,
            )?;
            response.truncated |= truncated;
            instrumentation.work.edges_expanded +=
                u64::try_from(outgoing.len()).unwrap_or(u64::MAX);
            for edge in outgoing {
                let member = edge.kind == EdgeKind::Contains;
                if member && depth >= limits.max_depth {
                    response.truncated = true;
                    continue;
                }
                if !nodes.contains_key(&edge.target) {
                    if nodes.len() == max_nodes {
                        response.truncated = true;
                        continue;
                    }
                    if let Some(target) = self.backend.node_by_id(&edge.target)? {
                        nodes.insert(target.id.clone(), query_node(&target));
                    }
                }
                if member {
                    pending.push_back((edge.target.clone(), depth + 1));
                }
                edges.insert(edge.id.clone(), query_edge(&edge));
            }
        }
        response.nodes = nodes.into_values().collect();
        response.edges = edges.into_values().collect();
        if let Some(program) = &self.program {
            crate::join_program_evidence(&mut response, Some(program));
        }
        if let Some(message) = &self.partial_graph_message {
            response.diagnostics.push(QueryDiagnostic {
                code: QueryDiagnosticCode::IncompleteCoverage,
                message: message.clone(),
                node_id: None,
                path: None,
            });
        }
        instrumentation.execution += started.elapsed();
        self.finish_natural_response(response)
    }
}

const NATURAL_DEPENDENCY_KINDS: &[EdgeKind] = &[
    EdgeKind::Calls,
    EdgeKind::Imports,
    EdgeKind::References,
    EdgeKind::DependsOn,
];
const NATURAL_OUTGOING_KINDS: &[EdgeKind] = &[
    EdgeKind::Contains,
    EdgeKind::Calls,
    EdgeKind::Imports,
    EdgeKind::DependsOn,
];

fn natural_declaration(node: &QueryNode) -> bool {
    node.kind.is_callable()
        || matches!(
            node.kind,
            NodeKind::Class
                | NodeKind::Struct
                | NodeKind::Interface
                | NodeKind::Trait
                | NodeKind::Enum
        )
}

fn natural_test_source(node: &QueryNode) -> bool {
    node.source.as_ref().is_some_and(|anchor| {
        anchor.file.replace('\\', "/").split('/').any(|part| {
            matches!(part, "test" | "tests" | "__tests__" | "generated")
                || part.starts_with("test_")
                || part.ends_with("_test.py")
                || part.ends_with("_test.rs")
                || part.contains(".test.")
                || part.contains(".spec.")
        })
    })
}

// Do not turn an absent compound symbol into a confident query about the
// generic "Service" or "Repository" token. Typo variants may still match
// the complete identifier, while partial names need multiple shared terms.
pub(crate) fn natural_fuzzy_relevant(operand: &str, node: &QueryNode) -> bool {
    let terms = crate::text::search_tokens(operand);
    if terms.len() < 3 {
        return true;
    }
    let node_terms = crate::text::search_tokens(&node.qualified_name);
    if terms
        .iter()
        .filter(|term| node_terms.contains(term))
        .count()
        >= 2
    {
        return true;
    }
    let variants = crate::code_query::recall_fuzzy_term_variants(&[operand.to_owned()]);
    let name = normalize_symbol(&node.name);
    variants
        .iter()
        .any(|variant| normalize_symbol(variant) == name)
}
