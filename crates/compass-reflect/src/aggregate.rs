use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

use compass_model::GraphDocument;
use serde_json::Value;
use time::format_description::well_known::Rfc3339;
use time::{Date, Month, OffsetDateTime, PrimitiveDateTime, Time};

use crate::MemoryDoc;

const UNCATEGORIZED: &str = "Uncategorized";
const MAX_SIDECAR_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    pub useful: usize,
    pub dead_end: usize,
    pub corrected: usize,
    pub unmarked: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceScore {
    pub node: String,
    pub n: usize,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContestedSource {
    pub node: String,
    pub pos: usize,
    pub neg: usize,
    pub score: f64,
    pub verdict: String,
    pub last: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadEnd {
    pub question: String,
    pub nodes: Vec<String>,
    pub date: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Correction {
    pub question: String,
    pub correction: String,
    pub date: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvenanceEvent {
    pub date: String,
    pub question: String,
    pub outcome: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LessonBucket {
    pub counts: Counts,
    pub preferred: Vec<SourceScore>,
    pub tentative: Vec<SourceScore>,
    pub contested: Vec<ContestedSource>,
    pub dead_ends: Vec<DeadEnd>,
    pub corrections: Vec<Correction>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Aggregate {
    pub total: usize,
    pub counts: Counts,
    pub min_corroboration: usize,
    pub preferred: Vec<SourceScore>,
    pub tentative: Vec<SourceScore>,
    pub contested: Vec<ContestedSource>,
    pub dead_ends: Vec<DeadEnd>,
    pub corrections: Vec<Correction>,
    pub by_community: BTreeMap<String, LessonBucket>,
    pub provenance: HashMap<String, Vec<ProvenanceEvent>>,
}

#[derive(Default)]
struct WorkingBucket {
    counts: Counts,
    node_score: HashMap<String, f64>,
    node_pos: HashMap<String, usize>,
    node_neg: HashMap<String, usize>,
    node_last: HashMap<String, String>,
    provenance: HashMap<String, Vec<ProvenanceEvent>>,
    dead_ends: Vec<DeadEnd>,
    corrections: Vec<Correction>,
}

pub(crate) struct GraphContext {
    pub node_community: Option<HashMap<String, String>>,
    pub known_nodes: Option<HashSet<String>>,
}

pub(crate) fn load_graph_context(graph: &Path, analysis: &Path, labels: &Path) -> GraphContext {
    GraphContext {
        node_community: load_node_community(graph, analysis, labels),
        known_nodes: load_known_nodes(graph),
    }
}

#[must_use]
pub fn aggregate_lessons(
    docs: &[MemoryDoc],
    node_community: Option<&HashMap<String, String>>,
    known_nodes: Option<&HashSet<String>>,
    now: OffsetDateTime,
    half_life_days: f64,
    min_corroboration: usize,
) -> Aggregate {
    let mut overall = WorkingBucket::default();
    let mut communities = HashMap::<String, WorkingBucket>::new();
    for doc in docs {
        let mut seen = HashSet::new();
        let nodes = doc
            .source_nodes
            .iter()
            .filter(|node| known_nodes.is_none_or(|known| known.contains(*node)))
            .filter(|node| seen.insert((*node).clone()))
            .cloned()
            .collect::<Vec<_>>();
        let community = document_community(&nodes, node_community);
        let bucket = communities.entry(community).or_default();
        let sign = match doc.outcome.as_str() {
            "useful" => 1,
            "dead_end" | "corrected" => -1,
            _ => 0,
        };
        let weight = if sign == 0 {
            0.0
        } else {
            decay(&doc.date, now, half_life_days)
        };
        for target in [&mut overall, bucket] {
            record_count(&mut target.counts, &doc.outcome);
            if sign != 0 {
                for node in &nodes {
                    record_node(target, node, sign, weight, doc);
                }
            }
            match doc.outcome.as_str() {
                "dead_end" => target.dead_ends.push(DeadEnd {
                    question: doc.question.clone(),
                    nodes: nodes.clone(),
                    date: doc.date.clone(),
                }),
                "corrected" => target.corrections.push(Correction {
                    question: doc.question.clone(),
                    correction: doc.correction.clone(),
                    date: doc.date.clone(),
                }),
                _ => {}
            }
        }
    }
    let (preferred, tentative, contested) = finalize_sources(&overall, min_corroboration);
    let by_community = if node_community.is_some_and(|mapping| !mapping.is_empty()) {
        communities
            .into_iter()
            .map(|(label, bucket)| {
                let (preferred, tentative, contested) =
                    finalize_sources(&bucket, min_corroboration);
                (
                    label,
                    LessonBucket {
                        counts: bucket.counts,
                        preferred,
                        tentative,
                        contested,
                        dead_ends: dedupe_dead_ends(bucket.dead_ends),
                        corrections: dedupe_corrections(bucket.corrections),
                    },
                )
            })
            .collect()
    } else {
        BTreeMap::new()
    };
    Aggregate {
        total: docs.len(),
        counts: overall.counts,
        min_corroboration,
        preferred,
        tentative,
        contested,
        dead_ends: dedupe_dead_ends(overall.dead_ends),
        corrections: dedupe_corrections(overall.corrections),
        by_community,
        provenance: overall.provenance,
    }
}

fn record_count(counts: &mut Counts, outcome: &str) {
    match outcome {
        "useful" => counts.useful += 1,
        "dead_end" => counts.dead_end += 1,
        "corrected" => counts.corrected += 1,
        _ => counts.unmarked += 1,
    }
}

fn record_node(bucket: &mut WorkingBucket, node: &str, sign: i8, weight: f64, doc: &MemoryDoc) {
    *bucket.node_score.entry(node.to_owned()).or_default() += f64::from(sign) * weight;
    if sign > 0 {
        *bucket.node_pos.entry(node.to_owned()).or_default() += 1;
    } else {
        *bucket.node_neg.entry(node.to_owned()).or_default() += 1;
    }
    let last = bucket.node_last.entry(node.to_owned()).or_default();
    if doc.date > *last {
        last.clone_from(&doc.date);
    }
    if matches!(doc.outcome.as_str(), "useful" | "corrected") {
        bucket
            .provenance
            .entry(node.to_owned())
            .or_default()
            .push(ProvenanceEvent {
                date: doc.date.clone(),
                question: doc.question.clone(),
                outcome: doc.outcome.clone(),
            });
    }
}

fn finalize_sources(
    bucket: &WorkingBucket,
    min_corroboration: usize,
) -> (Vec<SourceScore>, Vec<SourceScore>, Vec<ContestedSource>) {
    let mut preferred = Vec::new();
    let mut tentative = Vec::new();
    let mut contested = Vec::new();
    for (node, raw_score) in &bucket.node_score {
        let pos = bucket.node_pos.get(node).copied().unwrap_or_default();
        let neg = bucket.node_neg.get(node).copied().unwrap_or_default();
        let score = round_score(*raw_score);
        if pos > 0 && neg > 0 {
            contested.push(ContestedSource {
                node: node.clone(),
                pos,
                neg,
                score,
                verdict: if score > 0.0 {
                    "useful"
                } else if score < 0.0 {
                    "dead end"
                } else {
                    "even"
                }
                .to_owned(),
                last: bucket.node_last.get(node).cloned().unwrap_or_default(),
            });
        } else if pos > 0 {
            let entry = SourceScore {
                node: node.clone(),
                n: pos,
                score,
            };
            if pos >= min_corroboration {
                preferred.push(entry);
            } else {
                tentative.push(entry);
            }
        }
    }
    let sort = |left: &SourceScore, right: &SourceScore| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.node.cmp(&right.node))
    };
    preferred.sort_by(sort);
    tentative.sort_by(sort);
    contested.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.node.cmp(&right.node))
    });
    (preferred, tentative, contested)
}

fn round_score(score: f64) -> f64 {
    (score * 1_000_000_000.0).round() / 1_000_000_000.0
}

fn dedupe_dead_ends(items: Vec<DeadEnd>) -> Vec<DeadEnd> {
    let mut latest = HashMap::new();
    for item in items {
        latest.insert(item.question.clone(), item);
    }
    let mut output = latest.into_values().collect::<Vec<_>>();
    output.sort_by(|left, right| (&left.date, &left.question).cmp(&(&right.date, &right.question)));
    output
}

fn dedupe_corrections(items: Vec<Correction>) -> Vec<Correction> {
    let mut latest = HashMap::new();
    for item in items {
        latest.insert(item.question.clone(), item);
    }
    let mut output = latest.into_values().collect::<Vec<_>>();
    output.sort_by(|left, right| (&left.date, &left.question).cmp(&(&right.date, &right.question)));
    output
}

fn document_community(
    nodes: &[String],
    node_community: Option<&HashMap<String, String>>,
) -> String {
    let Some(mapping) = node_community.filter(|mapping| !mapping.is_empty()) else {
        return UNCATEGORIZED.to_owned();
    };
    let mut counts = HashMap::<&str, usize>::new();
    for label in nodes.iter().filter_map(|node| mapping.get(node)) {
        *counts.entry(label).or_default() += 1;
    }
    counts
        .into_iter()
        .min_by(|(left_label, left_count), (right_label, right_count)| {
            right_count
                .cmp(left_count)
                .then_with(|| left_label.cmp(right_label))
        })
        .map_or_else(|| UNCATEGORIZED.to_owned(), |(label, _)| label.to_owned())
}

fn decay(date: &str, now: OffsetDateTime, half_life_days: f64) -> f64 {
    let Some(then) = parse_datetime(date) else {
        return 1.0;
    };
    if half_life_days <= 0.0 {
        return 1.0;
    }
    let duration = now - then;
    let age_days = (duration.as_seconds_f64() / 86_400.0).max(0.0);
    0.5_f64.powf(age_days / half_life_days)
}

fn parse_datetime(value: &str) -> Option<OffsetDateTime> {
    if value.is_empty() {
        return None;
    }
    if let Ok(parsed) = OffsetDateTime::parse(value, &Rfc3339) {
        return Some(parsed);
    }
    if value.len() == 10 {
        let year = value.get(0..4)?.parse().ok()?;
        let month = Month::try_from(value.get(5..7)?.parse::<u8>().ok()?).ok()?;
        let day = value.get(8..10)?.parse().ok()?;
        return Date::from_calendar_date(year, month, day)
            .ok()
            .map(|date| PrimitiveDateTime::new(date, Time::MIDNIGHT).assume_utc());
    }
    OffsetDateTime::parse(&format!("{value}Z"), &Rfc3339).ok()
}

fn load_node_community(
    graph_path: &Path,
    analysis_path: &Path,
    labels_path: &Path,
) -> Option<HashMap<String, String>> {
    if !graph_path.exists() || !analysis_path.exists() {
        return None;
    }
    let analysis = read_bounded_json(analysis_path)?;
    let communities = analysis.get("communities")?.as_object()?;
    if communities.is_empty() {
        return None;
    }
    let labels = read_bounded_json(labels_path)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let graph = GraphDocument::load(graph_path).ok();
    let id_to_label = graph
        .as_ref()
        .map(|graph| {
            graph
                .nodes
                .iter()
                .map(|node| (node.id.clone(), node.label().to_owned()))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut community_ids = communities.keys().collect::<Vec<_>>();
    community_ids.sort();
    let mut mapping = HashMap::new();
    for community_id in community_ids {
        let label = labels
            .get(community_id)
            .and_then(value_as_string)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("Community {community_id}"));
        for node in communities
            .get(community_id)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(value_as_string)
        {
            mapping.entry(node.clone()).or_insert_with(|| label.clone());
            if let Some(node_label) = id_to_label.get(&node) {
                mapping
                    .entry(node_label.clone())
                    .or_insert_with(|| label.clone());
            }
        }
    }
    Some(mapping)
}

fn load_known_nodes(graph_path: &Path) -> Option<HashSet<String>> {
    let graph = GraphDocument::load(graph_path).ok()?;
    let known = graph
        .nodes
        .iter()
        .flat_map(|node| [node.id.clone(), node.label().to_owned()])
        .collect::<HashSet<_>>();
    (!known.is_empty()).then_some(known)
}

fn read_bounded_json(path: &Path) -> Option<Value> {
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SIDECAR_BYTES {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(if *value { "True" } else { "False" }.to_owned()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => Some(value.to_string()),
    }
}

pub fn render_lessons_markdown(aggregate: &Aggregate) -> String {
    let counts = &aggregate.counts;
    let memories = if aggregate.total == 1 {
        "memory"
    } else {
        "memories"
    };
    let mut output = vec![
        "# Lessons".to_owned(),
        String::new(),
        format!(
            "_Auto-generated by `graphify reflect` from {} session {memories} in graphify-out/memory/. Deterministic; no LLM. Use for orientation — verify before relying, and revisit dead ends if the code has changed since._",
            aggregate.total
        ),
        String::new(),
        "## Summary".to_owned(),
        String::new(),
        format!(
            "- {} useful · {} dead ends · {} corrected · {} unmarked",
            counts.useful, counts.dead_end, counts.corrected, counts.unmarked
        ),
        String::new(),
        "## Lessons".to_owned(),
        String::new(),
    ];
    render_bucket(
        &mut output,
        &aggregate.preferred,
        &aggregate.tentative,
        &aggregate.contested,
        &aggregate.dead_ends,
        &aggregate.corrections,
        aggregate.min_corroboration,
    );
    if !aggregate.by_community.is_empty() {
        output.extend(["## By topic".to_owned(), String::new()]);
        let mut labels = aggregate.by_community.keys().collect::<Vec<_>>();
        labels.sort_by(|left, right| {
            let left_key = (usize::from(left.as_str() == UNCATEGORIZED), left.as_str());
            let right_key = (usize::from(right.as_str() == UNCATEGORIZED), right.as_str());
            left_key.cmp(&right_key)
        });
        for label in labels {
            output.extend([format!("### {label}"), String::new()]);
            let bucket = &aggregate.by_community[label];
            render_bucket(
                &mut output,
                &bucket.preferred,
                &bucket.tentative,
                &bucket.contested,
                &bucket.dead_ends,
                &bucket.corrections,
                aggregate.min_corroboration,
            );
        }
    }
    format!("{}\n", output.join("\n").trim_end_matches('\n'))
}

#[allow(clippy::too_many_arguments)]
fn render_bucket(
    output: &mut Vec<String>,
    preferred: &[SourceScore],
    tentative: &[SourceScore],
    contested: &[ContestedSource],
    dead_ends: &[DeadEnd],
    corrections: &[Correction],
    min_corroboration: usize,
) {
    if !preferred.is_empty() {
        output.extend([
            format!(
                "**Preferred sources** — corroborated by ≥{min_corroboration} useful results; start here."
            ),
            String::new(),
        ]);
        output.extend(
            preferred
                .iter()
                .map(|entry| format!("- `{}` ({}× useful)", entry.node, entry.n)),
        );
        output.push(String::new());
    }
    if !tentative.is_empty() {
        output.extend([
            format!(
                "**Tentative** — useful in fewer than {min_corroboration} results; verify before relying."
            ),
            String::new(),
        ]);
        output.extend(
            tentative
                .iter()
                .map(|entry| format!("- `{}` ({}× useful)", entry.node, entry.n)),
        );
        output.push(String::new());
    }
    if !contested.is_empty() {
        output.extend([
            "**Contested** — mixed signals; recency decides.".to_owned(),
            String::new(),
        ]);
        for entry in contested {
            let verdict = if entry.verdict == "even" {
                "evenly split".to_owned()
            } else {
                format!("recency leans **{}**", entry.verdict)
            };
            let latest = entry
                .last
                .get(..10)
                .filter(|_| entry.last.len() >= 10)
                .map_or_else(String::new, |day| format!(" (latest {day})"));
            output.push(format!(
                "- `{}` — {}× useful, {}× dead end/corrected → {verdict}{latest}",
                entry.node, entry.pos, entry.neg
            ));
        }
        output.push(String::new());
    }
    if !dead_ends.is_empty() {
        output.extend([
            "**Known dead ends** — led nowhere; don't re-derive.".to_owned(),
            String::new(),
        ]);
        for dead_end in dead_ends {
            let nodes = dead_end
                .nodes
                .iter()
                .map(|node| format!("`{node}`"))
                .collect::<Vec<_>>()
                .join(", ");
            output.push(if nodes.is_empty() {
                format!("- \"{}\"", dead_end.question)
            } else {
                format!("- \"{}\" — {nodes}", dead_end.question)
            });
        }
        output.push(String::new());
    }
    if !corrections.is_empty() {
        output.extend([
            "**Corrections** — do these differently.".to_owned(),
            String::new(),
        ]);
        output.extend(corrections.iter().map(|correction| {
            format!("- \"{}\" → {}", correction.question, correction.correction)
        }));
        output.push(String::new());
    }
    if preferred.is_empty()
        && tentative.is_empty()
        && contested.is_empty()
        && dead_ends.is_empty()
        && corrections.is_empty()
    {
        output.extend(["_No marked outcomes yet._".to_owned(), String::new()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(outcome: &str, date: &str, nodes: &[&str]) -> MemoryDoc {
        MemoryDoc {
            outcome: outcome.to_owned(),
            date: date.to_owned(),
            question: format!("{outcome}?"),
            source_nodes: nodes.iter().map(|node| (*node).to_owned()).collect(),
            ..MemoryDoc::default()
        }
    }

    #[test]
    fn scores_corroboration_contested_and_dead_ends() {
        let docs = vec![
            doc("useful", "2026-06-01T00:00:00+00:00", &["A", "A"]),
            doc("useful", "2026-06-01T00:00:00+00:00", &["A", "B"]),
            doc("dead_end", "2026-06-01T00:00:00+00:00", &["B"]),
        ];
        let now = OffsetDateTime::parse("2026-06-01T00:00:00Z", &Rfc3339)
            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
        let aggregate = aggregate_lessons(&docs, None, None, now, 30.0, 2);
        assert_eq!(aggregate.preferred[0].node, "A");
        assert_eq!(aggregate.contested[0].node, "B");
        assert!(render_lessons_markdown(&aggregate).contains("Known dead ends"));
    }
}
