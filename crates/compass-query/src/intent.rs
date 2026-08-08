use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryResponse, ImpactRequest, NodeTrailRequest, SearchRequest,
};

use crate::code_query::{CodeQueryEngine, validate_limits};
use crate::cql::{QueryError, QueryErrorKind};

pub const QUERY_PLANNER_PROFILE_V1: &str = "query-planner/1";
const MAX_NATURAL_QUERY_BYTES: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NaturalQueryRequest {
    pub question: String,
    pub include_heuristic: bool,
    pub limits: CodeQueryLimits,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum NaturalQueryPlan {
    Search { query: String },
    Callers { symbol: String },
    Callees { symbol: String },
    Impact { symbol: String },
    NodeTrail { source: String, target: String },
}

impl CodeQueryEngine {
    pub fn query_natural(
        &self,
        request: NaturalQueryRequest,
    ) -> Result<CodeQueryResponse, QueryError> {
        validate_limits(&request.limits)?;
        validate_question(&request.question)?;
        match plan_natural_query(&request.question) {
            NaturalQueryPlan::Search { query } => self.search(SearchRequest {
                query,
                limits: request.limits,
            }),
            NaturalQueryPlan::Callers { symbol } => self.callers(CallRequest {
                symbol,
                include_heuristic: request.include_heuristic,
                limits: request.limits,
            }),
            NaturalQueryPlan::Callees { symbol } => self.callees(CallRequest {
                symbol,
                include_heuristic: request.include_heuristic,
                limits: request.limits,
            }),
            NaturalQueryPlan::Impact { symbol } => self.impact(ImpactRequest {
                symbol,
                include_heuristic: request.include_heuristic,
                limits: request.limits,
            }),
            NaturalQueryPlan::NodeTrail { source, target } => self.node_trail(NodeTrailRequest {
                source,
                target,
                include_heuristic: request.include_heuristic,
                limits: request.limits,
            }),
        }
    }
}

fn validate_question(question: &str) -> Result<(), QueryError> {
    if question.len() > MAX_NATURAL_QUERY_BYTES {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "natural_query_too_large",
            format!("natural query exceeds {MAX_NATURAL_QUERY_BYTES} bytes"),
        ));
    }
    Ok(())
}

fn plan_natural_query(question: &str) -> NaturalQueryPlan {
    let original = question.trim().trim_end_matches(['?', '!', '.']).trim();
    if original.is_empty() {
        return NaturalQueryPlan::Search {
            query: String::new(),
        };
    }
    let lower = original.to_ascii_lowercase();

    // Contradictory direction words are deliberately not resolved by choosing
    // the first cue. Search is the safe fallback and can surface candidates.
    if contains_word(&lower, "callers") && contains_word(&lower, "callees") {
        return search_plan(original);
    }

    for prefix in ["shortest path from ", "path from "] {
        if let Some((source, target)) = split_operands(original, &lower, prefix, " to ") {
            return NaturalQueryPlan::NodeTrail { source, target };
        }
    }
    for prefix in ["how does ", "how can "] {
        if let Some((source, target)) = split_operands(original, &lower, prefix, " reach ") {
            return NaturalQueryPlan::NodeTrail { source, target };
        }
    }
    if let Some((source, target)) = split_operands(original, &lower, "how is ", " connected to ") {
        return NaturalQueryPlan::NodeTrail { source, target };
    }

    for prefix in [
        "find callers of ",
        "show callers of ",
        "callers of ",
        "who calls ",
        "what calls ",
    ] {
        if let Some(symbol) = operand_after_prefix(original, &lower, prefix) {
            return NaturalQueryPlan::Callers { symbol };
        }
    }

    for prefix in [
        "find callees of ",
        "show callees of ",
        "callees of ",
        "calls made by ",
    ] {
        if let Some(symbol) = operand_after_prefix(original, &lower, prefix) {
            return NaturalQueryPlan::Callees { symbol };
        }
    }
    for suffix in [" call", " calls", " invoke", " invokes"] {
        if let Some(symbol) = operand_between(original, &lower, "what does ", suffix) {
            return NaturalQueryPlan::Callees { symbol };
        }
    }

    for prefix in [
        "what is impacted by ",
        "what depends on ",
        "who depends on ",
        "what breaks if ",
        "what changes if ",
        "dependents of ",
        "impact of ",
    ] {
        if let Some(mut symbol) = operand_after_prefix(original, &lower, prefix) {
            for suffix in [" changes", " changed"] {
                if symbol.to_ascii_lowercase().ends_with(suffix) {
                    symbol.truncate(symbol.len().saturating_sub(suffix.len()));
                    symbol = clean_operand(&symbol);
                }
            }
            if !symbol.is_empty() {
                return NaturalQueryPlan::Impact { symbol };
            }
        }
    }

    for prefix in [
        "find definition of ",
        "definition of ",
        "search for ",
        "show me ",
        "where is ",
        "where are ",
        "find ",
        "search ",
        "show ",
    ] {
        if let Some(mut query) = operand_after_prefix(original, &lower, prefix) {
            if query.to_ascii_lowercase().ends_with(" defined") {
                query.truncate(query.len().saturating_sub(" defined".len()));
                query = clean_operand(&query);
            }
            if !query.is_empty() {
                return NaturalQueryPlan::Search { query };
            }
        }
    }

    search_plan(original)
}

fn search_plan(query: &str) -> NaturalQueryPlan {
    NaturalQueryPlan::Search {
        query: query.to_owned(),
    }
}

fn operand_after_prefix(original: &str, lower: &str, prefix: &str) -> Option<String> {
    lower
        .starts_with(prefix)
        .then(|| clean_operand(original.get(prefix.len()..).unwrap_or_default()))
        .filter(|operand| !operand.is_empty())
}

fn operand_between(original: &str, lower: &str, prefix: &str, suffix: &str) -> Option<String> {
    if !lower.starts_with(prefix) || !lower.ends_with(suffix) {
        return None;
    }
    let end = original.len().saturating_sub(suffix.len());
    (prefix.len() < end)
        .then(|| clean_operand(original.get(prefix.len()..end).unwrap_or_default()))
        .filter(|operand| !operand.is_empty())
}

fn split_operands(
    original: &str,
    lower: &str,
    prefix: &str,
    separator: &str,
) -> Option<(String, String)> {
    if !lower.starts_with(prefix) {
        return None;
    }
    let remainder = lower.get(prefix.len()..)?;
    let separator_offset = remainder.find(separator)?;
    let split = prefix.len().saturating_add(separator_offset);
    let source = clean_operand(original.get(prefix.len()..split)?);
    let target = clean_operand(original.get(split.saturating_add(separator.len())..)?);
    (!source.is_empty() && !target.is_empty()).then_some((source, target))
}

fn clean_operand(value: &str) -> String {
    value
        .trim()
        .trim_matches(['"', '\'', '`'])
        .trim()
        .to_owned()
}

fn contains_word(value: &str, expected: &str) -> bool {
    value
        .split(|character: char| !character.is_alphanumeric())
        .any(|word| word == expected)
}

#[cfg(test)]
mod tests {
    use super::{NaturalQueryPlan, plan_natural_query};

    #[test]
    fn planner_routes_high_confidence_structural_intents() {
        assert_eq!(
            plan_natural_query("Who calls UserService.list?"),
            NaturalQueryPlan::Callers {
                symbol: "UserService.list".to_owned()
            }
        );
        assert_eq!(
            plan_natural_query("What does Api.caller call?"),
            NaturalQueryPlan::Callees {
                symbol: "Api.caller".to_owned()
            }
        );
        assert_eq!(
            plan_natural_query("What breaks if Api.caller changes?"),
            NaturalQueryPlan::Impact {
                symbol: "Api.caller".to_owned()
            }
        );
        assert_eq!(
            plan_natural_query("Path from Api.caller to Store.callee"),
            NaturalQueryPlan::NodeTrail {
                source: "Api.caller".to_owned(),
                target: "Store.callee".to_owned()
            }
        );
    }

    #[test]
    fn planner_cleans_search_scaffolding_and_preserves_unicode() {
        assert_eq!(
            plan_natural_query("Where is résumé defined?"),
            NaturalQueryPlan::Search {
                query: "résumé".to_owned()
            }
        );
    }

    #[test]
    fn planner_falls_back_on_low_confidence_and_contradictory_direction() {
        for question in [
            "authentication flow",
            "show callers and callees of UserService.list",
        ] {
            assert_eq!(
                plan_natural_query(question),
                NaturalQueryPlan::Search {
                    query: question.to_owned()
                }
            );
        }
    }
}
