use compass_model::query_contract::{
    CallRequest, CodeQueryLimits, CodeQueryResponse, ImpactRequest, NodeTrailRequest, SearchRequest,
};
use serde::{Deserialize, Serialize};
use std::time::Instant;

use crate::code_query::{CodeQueryEngine, validate_limits};
use crate::cql::{QueryError, QueryErrorKind};
use crate::ranking::QUERY_RANKER_PROFILE_V2;
use crate::telemetry::{ProfiledCodeQueryResponse, QueryInstrumentation};

pub const QUERY_PLANNER_PROFILE_V1: &str = "query-planner/1";
const MAX_NATURAL_QUERY_BYTES: usize = 4_096;
const AUTO_ROUTE_CONFIDENCE: u8 = 90;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NaturalQueryRequest {
    pub question: String,
    pub include_heuristic: bool,
    pub limits: CodeQueryLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NaturalQueryIntent {
    Search,
    Callers,
    Callees,
    Impact,
    NodeTrail,
    Fallback,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NaturalQueryPlan {
    profile: String,
    intent: NaturalQueryIntent,
    confidence: u8,
    operands: Vec<String>,
}

impl NaturalQueryPlan {
    #[must_use]
    pub fn profile(&self) -> &str {
        &self.profile
    }

    #[must_use]
    pub fn intent(&self) -> NaturalQueryIntent {
        self.intent
    }

    #[must_use]
    pub fn confidence(&self) -> u8 {
        self.confidence
    }

    #[must_use]
    pub fn operands(&self) -> &[String] {
        &self.operands
    }

    #[must_use]
    pub fn routes_to_typed_query(&self) -> bool {
        self.intent != NaturalQueryIntent::Fallback && self.confidence >= AUTO_ROUTE_CONFIDENCE
    }
}

impl CodeQueryEngine {
    pub fn query_natural(
        &self,
        request: NaturalQueryRequest,
    ) -> Result<CodeQueryResponse, QueryError> {
        self.execute_natural_query(request)
            .map(|(response, _)| response)
    }

    pub fn query_natural_profiled(
        &self,
        request: NaturalQueryRequest,
    ) -> Result<ProfiledCodeQueryResponse, QueryError> {
        let total_started = Instant::now();
        let (response, instrumentation) = self.execute_natural_query(request)?;
        Ok(instrumentation.finish(
            response,
            total_started.elapsed(),
            QUERY_PLANNER_PROFILE_V1,
            QUERY_RANKER_PROFILE_V2,
        ))
    }

    fn execute_natural_query(
        &self,
        request: NaturalQueryRequest,
    ) -> Result<(CodeQueryResponse, QueryInstrumentation), QueryError> {
        validate_limits(&request.limits)?;
        let mut instrumentation = QueryInstrumentation::default();
        let intent_started = Instant::now();
        let plan = plan_natural_query(&request.question)?;
        instrumentation.intent += intent_started.elapsed();
        let primary = plan.operands.first().cloned().unwrap_or_default();
        let response = match plan.intent {
            NaturalQueryIntent::Search | NaturalQueryIntent::Fallback => self.search_instrumented(
                SearchRequest {
                    query: primary,
                    limits: request.limits,
                },
                &mut instrumentation,
            ),
            NaturalQueryIntent::Callers => self.call_neighbors_instrumented(
                CallRequest {
                    symbol: primary,
                    include_heuristic: request.include_heuristic,
                    limits: request.limits,
                },
                true,
                &mut instrumentation,
            ),
            NaturalQueryIntent::Callees => self.call_neighbors_instrumented(
                CallRequest {
                    symbol: primary,
                    include_heuristic: request.include_heuristic,
                    limits: request.limits,
                },
                false,
                &mut instrumentation,
            ),
            NaturalQueryIntent::Impact => self.impact_instrumented(
                ImpactRequest {
                    symbol: primary,
                    include_heuristic: request.include_heuristic,
                    limits: request.limits,
                },
                &mut instrumentation,
            ),
            NaturalQueryIntent::NodeTrail => {
                let target = plan.operands.get(1).cloned().ok_or_else(|| {
                    QueryError::new(
                        QueryErrorKind::Internal,
                        "invalid_natural_query_plan",
                        "node-trail intent is missing its target operand",
                    )
                })?;
                self.node_trail_instrumented(
                    NodeTrailRequest {
                        source: primary,
                        target,
                        include_heuristic: request.include_heuristic,
                        limits: request.limits,
                    },
                    &mut instrumentation,
                )
            }
        }?;
        Ok((response, instrumentation))
    }
}

pub fn plan_natural_query(question: &str) -> Result<NaturalQueryPlan, QueryError> {
    if question.len() > MAX_NATURAL_QUERY_BYTES {
        return Err(QueryError::new(
            QueryErrorKind::InvalidParameter,
            "natural_query_too_large",
            format!("natural query exceeds {MAX_NATURAL_QUERY_BYTES} bytes"),
        ));
    }
    Ok(plan_validated_natural_query(question))
}

fn plan_validated_natural_query(question: &str) -> NaturalQueryPlan {
    let original = question.trim().trim_end_matches(['?', '!', '.']).trim();
    if original.is_empty() {
        return plan(NaturalQueryIntent::Fallback, 0, [String::new()]);
    }
    let lower = original.to_ascii_lowercase();

    for prefix in [
        "shortest path from ",
        "path from ",
        "route from ",
        "connection from ",
    ] {
        if let Some((source, target)) = split_operands(original, &lower, prefix, " to ") {
            return plan(NaturalQueryIntent::NodeTrail, 100, [source, target]);
        }
    }
    for prefix in ["how does ", "how can "] {
        if let Some((source, target)) = split_operands(original, &lower, prefix, " reach ") {
            return plan(NaturalQueryIntent::NodeTrail, 100, [source, target]);
        }
    }
    if let Some((source, target)) = split_operands(original, &lower, "how is ", " connected to ") {
        return plan(NaturalQueryIntent::NodeTrail, 100, [source, target]);
    }

    for prefix in [
        "find callers of ",
        "show callers of ",
        "callers of ",
        "who calls ",
        "what calls ",
        "what functions call ",
        "what methods call ",
        "which functions call ",
        "which methods call ",
    ] {
        if let Some(symbol) = operand_after_prefix(original, &lower, prefix) {
            return plan(NaturalQueryIntent::Callers, 100, [symbol]);
        }
    }
    if let Some(symbol) = operand_between(original, &lower, "where is ", " called") {
        return plan(NaturalQueryIntent::Callers, 95, [symbol]);
    }

    for prefix in [
        "find callees of ",
        "show callees of ",
        "callees of ",
        "calls made by ",
    ] {
        if let Some(symbol) = operand_after_prefix(original, &lower, prefix) {
            return plan(NaturalQueryIntent::Callees, 100, [symbol]);
        }
    }
    for prefix in ["what does ", "what functions does ", "what methods does "] {
        for suffix in [" call", " calls", " invoke", " invokes"] {
            if let Some(symbol) = operand_between(original, &lower, prefix, suffix) {
                return plan(NaturalQueryIntent::Callees, 100, [symbol]);
            }
        }
    }

    // Contradictory direction words are deliberately not resolved by choosing
    // the first cue. Search is the safe fallback and can surface candidates.
    // Explicit structural syntax is parsed first because symbol operands can
    // themselves contain words such as `caller` and `callee`.
    if (["caller", "callers"]
        .iter()
        .any(|word| contains_word(&lower, word))
        && ["callee", "callees"]
            .iter()
            .any(|word| contains_word(&lower, word)))
        || (contains_word(&lower, "incoming") && contains_word(&lower, "outgoing"))
    {
        return fallback_plan(original);
    }

    for prefix in [
        "what is impacted by ",
        "what depends on ",
        "who depends on ",
        "what breaks if ",
        "what changes if ",
        "dependents of ",
        "impact of ",
        "what is the impact of ",
        "what would break if ",
    ] {
        if let Some(mut symbol) = operand_after_prefix(original, &lower, prefix) {
            for suffix in [" changes", " changed"] {
                if symbol.to_ascii_lowercase().ends_with(suffix) {
                    symbol.truncate(symbol.len().saturating_sub(suffix.len()));
                    symbol = clean_operand(&symbol);
                }
            }
            if !symbol.is_empty() {
                return plan(NaturalQueryIntent::Impact, 100, [symbol]);
            }
        }
    }
    for suffix in [" changes, what breaks", " changes what breaks"] {
        if let Some(symbol) = operand_between(original, &lower, "if ", suffix) {
            return plan(NaturalQueryIntent::Impact, 95, [symbol]);
        }
    }

    for prefix in ["find definition of ", "definition of ", "search for "] {
        if let Some(mut query) = operand_after_prefix(original, &lower, prefix) {
            if query.to_ascii_lowercase().ends_with(" defined") {
                query.truncate(query.len().saturating_sub(" defined".len()));
                query = clean_operand(&query);
            }
            if !query.is_empty() {
                return plan(NaturalQueryIntent::Search, 90, [query]);
            }
        }
    }
    for prefix in [
        "show me ",
        "where is ",
        "where are ",
        "find ",
        "search ",
        "show ",
    ] {
        if let Some(mut query) = operand_after_prefix(original, &lower, prefix) {
            let explicitly_defined = query.to_ascii_lowercase().ends_with(" defined");
            if explicitly_defined {
                query.truncate(query.len().saturating_sub(" defined".len()));
                query = clean_operand(&query);
            }
            if !query.is_empty() {
                let confidence = if explicitly_defined || looks_like_symbol_operand(&query) {
                    90
                } else {
                    60
                };
                return plan(NaturalQueryIntent::Search, confidence, [query]);
            }
        }
    }

    fallback_plan(original)
}

fn plan(
    intent: NaturalQueryIntent,
    confidence: u8,
    operands: impl IntoIterator<Item = String>,
) -> NaturalQueryPlan {
    NaturalQueryPlan {
        profile: QUERY_PLANNER_PROFILE_V1.to_owned(),
        intent,
        confidence,
        operands: operands.into_iter().collect(),
    }
}

fn fallback_plan(query: &str) -> NaturalQueryPlan {
    plan(NaturalQueryIntent::Fallback, 0, [query.to_owned()])
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

fn looks_like_symbol_operand(value: &str) -> bool {
    !value.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::{NaturalQueryIntent, QueryError, plan_natural_query};

    fn planned(question: &str) -> Result<(NaturalQueryIntent, Vec<String>), QueryError> {
        let plan = plan_natural_query(question)?;
        Ok((plan.intent(), plan.operands().to_vec()))
    }

    #[test]
    fn planner_routes_high_confidence_structural_intents() -> Result<(), QueryError> {
        assert_eq!(
            planned("Who calls UserService.list?")?,
            (
                NaturalQueryIntent::Callers,
                vec!["UserService.list".to_owned()]
            )
        );
        assert_eq!(
            planned("What does Api.caller call?")?,
            (NaturalQueryIntent::Callees, vec!["Api.caller".to_owned()])
        );
        assert_eq!(
            planned("Find callees of Api.caller")?,
            (NaturalQueryIntent::Callees, vec!["Api.caller".to_owned()])
        );
        assert_eq!(
            planned("What breaks if Api.caller changes?")?,
            (NaturalQueryIntent::Impact, vec!["Api.caller".to_owned()])
        );
        assert_eq!(
            planned("Path from Api.caller to Store.callee")?,
            (
                NaturalQueryIntent::NodeTrail,
                vec!["Api.caller".to_owned(), "Store.callee".to_owned()]
            )
        );
        Ok(())
    }

    #[test]
    fn planner_cleans_search_scaffolding_and_preserves_unicode() -> Result<(), QueryError> {
        assert_eq!(
            planned("Where is résumé defined?")?,
            (NaturalQueryIntent::Search, vec!["résumé".to_owned()])
        );
        Ok(())
    }

    #[test]
    fn planner_falls_back_on_low_confidence_and_contradictory_direction() -> Result<(), QueryError>
    {
        for question in [
            "authentication flow",
            "show callers and callees of UserService.list",
        ] {
            assert_eq!(
                planned(question)?,
                (NaturalQueryIntent::Fallback, vec![question.to_owned()])
            );
        }
        Ok(())
    }
}
