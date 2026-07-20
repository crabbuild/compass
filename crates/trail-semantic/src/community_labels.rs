//! Batched, failure-tolerant community naming compatible with Graphify.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;

use crate::PlainTextResponse;

const LABEL_TOP_K: usize = 12;
const LABEL_MAX_LENGTH: usize = 60;
const LABEL_BATCH_SIZE: usize = 100;
const LABEL_MAX_RETRY_DEPTH: usize = 3;

static LABEL_PAIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""?(-?\d+)"?\s*:\s*"([^"\\]*(?:\\.[^"\\]*)*)""#)
        .unwrap_or_else(|_| std::process::abort())
});

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommunityLabelOptions {
    pub backend_name: String,
    pub batch_size: usize,
    pub max_concurrency: usize,
}

impl CommunityLabelOptions {
    #[must_use]
    pub fn new(backend_name: impl Into<String>) -> Self {
        Self {
            backend_name: backend_name.into(),
            batch_size: LABEL_BATCH_SIZE,
            max_concurrency: 4,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommunityLabelResult {
    pub labels: BTreeMap<usize, String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub warnings: Vec<String>,
    pub used_backend: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommunityLabelCallError {
    pub message: String,
    pub retry_as_parse_failure: bool,
}

impl CommunityLabelCallError {
    #[must_use]
    pub fn fatal(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry_as_parse_failure: false,
        }
    }

    #[must_use]
    pub fn retryable_parse_failure(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retry_as_parse_failure: true,
        }
    }
}

#[derive(Debug)]
struct BatchResult {
    index: usize,
    size: usize,
    labels: Result<BTreeMap<usize, String>, String>,
    input_tokens: u64,
    output_tokens: u64,
    warnings: Vec<String>,
}

/// Generate community names through a caller-supplied lightweight provider.
///
/// The callback is transport-agnostic so built-in and custom providers share
/// one batching, parsing, retry, and accounting contract.
pub fn label_communities_with<F>(
    node_labels: &BTreeMap<String, String>,
    communities: &BTreeMap<usize, Vec<String>>,
    god_ids: &HashSet<String>,
    options: &CommunityLabelOptions,
    call: &F,
) -> CommunityLabelResult
where
    F: Fn(&str, usize) -> Result<PlainTextResponse, String> + Sync,
{
    label_communities_with_errors(
        node_labels,
        communities,
        god_ids,
        options,
        &|prompt, max_tokens| call(prompt, max_tokens).map_err(CommunityLabelCallError::fatal),
    )
}

pub fn label_communities_with_errors<F>(
    node_labels: &BTreeMap<String, String>,
    communities: &BTreeMap<usize, Vec<String>>,
    god_ids: &HashSet<String>,
    options: &CommunityLabelOptions,
    call: &F,
) -> CommunityLabelResult
where
    F: Fn(&str, usize) -> Result<PlainTextResponse, CommunityLabelCallError> + Sync,
{
    let mut result = CommunityLabelResult {
        labels: placeholder_labels(communities),
        used_backend: true,
        ..CommunityLabelResult::default()
    };
    let (lines, community_ids) = community_label_lines(node_labels, communities, god_ids);
    if lines.is_empty() {
        return result;
    }
    let batch_size = options.batch_size.max(1);
    let batch_count = community_ids.len().div_ceil(batch_size);
    let force_serial = (options.backend_name == "ollama"
        && std::env::var("GRAPHIFY_OLLAMA_PARALLEL").as_deref() != Ok("1"))
        || (options.backend_name == "claude-cli"
            && std::env::var("GRAPHIFY_CLAUDE_CLI_PARALLEL").as_deref() != Ok("1"));
    let worker_count = if force_serial {
        1
    } else {
        options.max_concurrency.max(1).min(batch_count)
    };

    let mut batches = if worker_count == 1 {
        (0..batch_count)
            .map(|index| run_batch(index, batch_size, &community_ids, &lines, call))
            .collect::<Vec<_>>()
    } else {
        let completed = std::thread::scope(|scope| {
            let community_ids = community_ids.as_slice();
            let lines = lines.as_slice();
            let next = Arc::new(AtomicUsize::new(0));
            let completed = Arc::new(Mutex::new(Vec::with_capacity(batch_count)));
            for _ in 0..worker_count {
                let next = Arc::clone(&next);
                let completed = Arc::clone(&completed);
                scope.spawn(move || {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index >= batch_count {
                            break;
                        }
                        let result =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                run_batch(index, batch_size, community_ids, lines, call)
                            }))
                            .unwrap_or_else(|_| BatchResult {
                                index,
                                size: community_ids
                                    .len()
                                    .saturating_sub(index.saturating_mul(batch_size))
                                    .min(batch_size),
                                labels: Err(
                                    "community-label worker terminated unexpectedly".to_owned()
                                ),
                                input_tokens: 0,
                                output_tokens: 0,
                                warnings: Vec::new(),
                            });
                        completed
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(result);
                    }
                });
            }
            completed
        });
        match Arc::try_unwrap(completed) {
            Ok(completed) => completed
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Err(completed) => {
                let mut completed = completed
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                std::mem::take(&mut *completed)
            }
        }
    };
    batches.sort_by_key(|batch| batch.index);
    let mut written = 0_usize;
    let mut first_error = None;
    for batch in batches {
        result.input_tokens = result.input_tokens.saturating_add(batch.input_tokens);
        result.output_tokens = result.output_tokens.saturating_add(batch.output_tokens);
        result.warnings.extend(batch.warnings);
        match batch.labels {
            Ok(labels) => {
                written = written.saturating_add(labels.len());
                result.labels.extend(labels);
            }
            Err(error) => {
                first_error.get_or_insert_with(|| error.clone());
                result.warnings.push(format!(
                    "[graphify label] batch {}/{} ({} communities) failed: {error}",
                    batch.index + 1,
                    batch_count,
                    batch.size
                ));
            }
        }
    }
    if written == 0
        && let Some(error) = first_error
    {
        result.warnings.push(format!(
            "[graphify label] warning: community labeling failed ({error}); using Community N placeholders."
        ));
    }
    result
}

fn run_batch<F>(
    index: usize,
    batch_size: usize,
    community_ids: &[usize],
    lines: &[String],
    call: &F,
) -> BatchResult
where
    F: Fn(&str, usize) -> Result<PlainTextResponse, CommunityLabelCallError> + Sync,
{
    let start = index.saturating_mul(batch_size);
    let end = start.saturating_add(batch_size).min(community_ids.len());
    let ids = &community_ids[start..end];
    let batch_lines = &lines[start..end];
    let mut input_tokens = 0_u64;
    let mut output_tokens = 0_u64;
    let mut warnings = Vec::new();
    let labels = label_batch_with_retry(
        ids,
        batch_lines,
        0,
        call,
        &mut input_tokens,
        &mut output_tokens,
        &mut warnings,
    );
    BatchResult {
        index,
        size: ids.len(),
        labels,
        input_tokens,
        output_tokens,
        warnings,
    }
}

#[allow(clippy::too_many_arguments)]
fn label_batch_with_retry<F>(
    community_ids: &[usize],
    lines: &[String],
    depth: usize,
    call: &F,
    input_tokens: &mut u64,
    output_tokens: &mut u64,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<usize, String>, String>
where
    F: Fn(&str, usize) -> Result<PlainTextResponse, CommunityLabelCallError> + Sync,
{
    let prompt = format!(
        "You are naming clusters in a knowledge graph. For each community below, return a concise 2-5 word plain-language name describing what it is about (e.g. \"Order Management\", \"Payment Flow\", \"Auth Middleware\"). Respond ONLY with a JSON object mapping the community id (as a string) to its name - no prose, no markdown fences.\n\n{}",
        lines.join("\n")
    );
    let max_tokens = 256_usize
        .saturating_add(48_usize.saturating_mul(community_ids.len()))
        .min(8_192);
    let error = match call(&prompt, max_tokens) {
        Ok(response) => {
            *input_tokens = input_tokens.saturating_add(response.input_tokens);
            *output_tokens = output_tokens.saturating_add(response.output_tokens);
            match parse_label_response(&response.text, community_ids) {
                Ok(labels) => return Ok(labels),
                Err(error) => error,
            }
        }
        Err(error) if error.retry_as_parse_failure => error.message,
        Err(error) => return Err(error.message),
    };
    if community_ids.len() > 1 && depth < LABEL_MAX_RETRY_DEPTH {
        let middle = community_ids.len() / 2;
        let mut left = label_batch_with_retry(
            &community_ids[..middle],
            &lines[..middle],
            depth + 1,
            call,
            input_tokens,
            output_tokens,
            warnings,
        )?;
        left.extend(label_batch_with_retry(
            &community_ids[middle..],
            &lines[middle..],
            depth + 1,
            call,
            input_tokens,
            output_tokens,
            warnings,
        )?);
        Ok(left)
    } else {
        let preview = community_ids
            .iter()
            .take(5)
            .map(usize::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if community_ids.len() > 5 { "..." } else { "" };
        warnings.push(format!(
                "[graphify label] batch of {} still unparseable at depth {depth} (cids=[{preview}]{suffix}): {error}",
                community_ids.len()
            ));
        Err(error)
    }
}

fn community_label_lines(
    node_labels: &BTreeMap<String, String>,
    communities: &BTreeMap<usize, Vec<String>>,
    god_ids: &HashSet<String>,
) -> (Vec<String>, Vec<usize>) {
    let mut ordered = communities.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|(community, members)| (std::cmp::Reverse(members.len()), **community));
    let mut lines = Vec::with_capacity(ordered.len());
    let mut community_ids = Vec::with_capacity(ordered.len());
    for (community, members) in ordered {
        let ranked = members
            .iter()
            .filter(|member| god_ids.contains(member.as_str()))
            .chain(
                members
                    .iter()
                    .filter(|member| !god_ids.contains(member.as_str())),
            );
        let mut names = Vec::new();
        let mut seen = HashSet::new();
        for member in ranked {
            let label = node_labels
                .get(member)
                .map_or(member.as_str(), String::as_str);
            let label = label.trim().trim_matches(['(', ')']);
            let label = label.chars().take(LABEL_MAX_LENGTH).collect::<String>();
            if !label.is_empty() && seen.insert(label.to_lowercase()) {
                names.push(label);
            }
            if names.len() >= LABEL_TOP_K {
                break;
            }
        }
        if !names.is_empty() {
            lines.push(format!("Community {community}: {}", names.join(", ")));
            community_ids.push(*community);
        }
    }
    (lines, community_ids)
}

fn parse_label_response(
    response: &str,
    community_ids: &[usize],
) -> Result<BTreeMap<usize, String>, String> {
    let mut cleaned = response.trim();
    if let Some(without_fence) = cleaned.strip_prefix("```json") {
        cleaned = without_fence.trim_start();
    } else if let Some(without_fence) = cleaned.strip_prefix("```") {
        cleaned = without_fence.trim_start();
    }
    if let Some(without_fence) = cleaned.strip_suffix("```") {
        cleaned = without_fence.trim_end();
    }
    if !cleaned.starts_with('{')
        && let (Some(start), Some(end)) = (cleaned.find('{'), cleaned.rfind('}'))
        && end > start
    {
        cleaned = &cleaned[start..=end];
    }
    let parsed = serde_json::from_str::<serde_json::Value>(cleaned)
        .ok()
        .and_then(|value| value.as_object().cloned());
    let values = if let Some(parsed) = parsed {
        parsed
            .into_iter()
            .filter_map(|(key, value)| {
                Some((key.parse::<usize>().ok()?, value.as_str()?.to_owned()))
            })
            .collect::<BTreeMap<_, _>>()
    } else {
        let pairs = LABEL_PAIR
            .captures_iter(cleaned)
            .filter_map(|captures| {
                let community = captures.get(1)?.as_str().parse::<usize>().ok()?;
                let encoded = format!("\"{}\"", captures.get(2)?.as_str());
                let name = serde_json::from_str::<String>(&encoded).ok()?;
                Some((community, name))
            })
            .collect::<BTreeMap<_, _>>();
        if pairs.is_empty() {
            return Err(format!(
                "label response is not parseable JSON: {:?}",
                response.chars().take(120).collect::<String>()
            ));
        }
        pairs
    };
    Ok(community_ids
        .iter()
        .filter_map(|community| {
            values
                .get(community)
                .map(|name| name.trim())
                .filter(|name| !name.is_empty())
                .map(|name| (*community, name.to_owned()))
        })
        .collect())
}

#[must_use]
pub fn placeholder_labels(communities: &BTreeMap<usize, Vec<String>>) -> BTreeMap<usize, String> {
    communities
        .keys()
        .map(|community| (*community, format!("Community {community}")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn graph(count: usize) -> (BTreeMap<String, String>, BTreeMap<usize, Vec<String>>) {
        let nodes = (0..count)
            .map(|index| (format!("node_{index}"), format!("Symbol {index}")))
            .collect::<BTreeMap<_, _>>();
        let communities = (0..count)
            .map(|index| (index, vec![format!("node_{index}")]))
            .collect();
        (nodes, communities)
    }

    #[test]
    fn batching_parsing_salvage_and_usage_match_python_contract() {
        let (node_labels, communities) = graph(3);
        let mut options = CommunityLabelOptions::new("gemini");
        options.batch_size = 2;
        options.max_concurrency = 1;
        let result = label_communities_with(
            &node_labels,
            &communities,
            &HashSet::new(),
            &options,
            &|prompt, _| {
                let ids = prompt
                    .lines()
                    .filter_map(|line| line.strip_prefix("Community "))
                    .filter_map(|line| line.split_once(':'))
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>();
                let text = if ids.len() == 2 {
                    format!(
                        "```json\n{{\"{}\":\"First\",\"{}\":\"Second\"}}\n```",
                        ids[0], ids[1]
                    )
                } else {
                    format!("{{\"{}\": \"Third\", \"999\":", ids[0])
                };
                Ok(PlainTextResponse {
                    text,
                    input_tokens: 10,
                    output_tokens: 2,
                    model: "fixture".to_owned(),
                })
            },
        );
        assert_eq!(result.labels.get(&0).map(String::as_str), Some("First"));
        assert_eq!(result.labels.get(&1).map(String::as_str), Some("Second"));
        assert_eq!(result.labels.get(&2).map(String::as_str), Some("Third"));
        assert_eq!((result.input_tokens, result.output_tokens), (20, 4));
    }

    #[test]
    fn total_failure_degrades_to_placeholders() {
        let (node_labels, communities) = graph(1);
        let options = CommunityLabelOptions::new("gemini");
        let result = label_communities_with(
            &node_labels,
            &communities,
            &HashSet::new(),
            &options,
            &|_, _| {
                Ok(PlainTextResponse {
                    text: "not json".to_owned(),
                    input_tokens: 5,
                    output_tokens: 1,
                    model: "fixture".to_owned(),
                })
            },
        );
        assert_eq!(
            result.labels.get(&0).map(String::as_str),
            Some("Community 0")
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("using Community N placeholders"))
        );
        assert_eq!((result.input_tokens, result.output_tokens), (5, 1));
    }

    #[test]
    fn max_concurrency_is_a_hard_bound() {
        let (node_labels, communities) = graph(8);
        let mut options = CommunityLabelOptions::new("gemini");
        options.batch_size = 1;
        options.max_concurrency = 2;
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let result = label_communities_with(
            &node_labels,
            &communities,
            &HashSet::new(),
            &options,
            &|prompt, _| {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(current, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(10));
                active.fetch_sub(1, Ordering::SeqCst);
                let id = prompt
                    .lines()
                    .find_map(|line| line.strip_prefix("Community "))
                    .and_then(|line| line.split_once(':'))
                    .map_or("0", |(id, _)| id);
                Ok(PlainTextResponse {
                    text: format!(r#"{{"{id}":"Named"}}"#),
                    input_tokens: 0,
                    output_tokens: 0,
                    model: "fixture".to_owned(),
                })
            },
        );
        assert_eq!(result.labels.len(), 8);
        assert!(peak.load(Ordering::SeqCst) <= 2);
    }
}
