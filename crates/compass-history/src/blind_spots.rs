use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::HistoryError;

pub const BLIND_SPOT_HISTORY_SCHEMA: &str = "compass.graph-blind-spot-history/1";
const MAX_TREND_ITEMS: usize = 4_096;
const MAX_REPORT_ID_BYTES: usize = 4_096;
const MAX_CANDIDATE_PAIRS: usize = 200_000;
const MAX_COMMUNITY_GAPS: usize = 3;
const MAX_WITNESSES: usize = 8;
const MAX_COMPONENTS: usize = 32;
const MAX_COMPONENT_MEMBERS: usize = 64;

#[derive(Clone, Debug)]
pub struct BlindSpotObservation {
    pub commit: String,
    pub authored_at_seconds: i64,
    pub report: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindSpotTrend {
    pub schema: String,
    pub observation_count: usize,
    pub observations_with_graph_insights: usize,
    pub first_commit: Option<String>,
    pub last_commit: Option<String>,
    pub active: Vec<BlindSpotTrendItem>,
    pub resolved: Vec<BlindSpotTrendItem>,
    pub omissions: BlindSpotTrendOmissions,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindSpotTrendItem {
    pub id: String,
    pub kind: String,
    pub first_commit: String,
    pub last_commit: String,
    pub first_authored_at_seconds: i64,
    pub last_authored_at_seconds: i64,
    pub observation_count: usize,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlindSpotTrendOmissions {
    pub items: usize,
    pub observations_without_graph_insights: usize,
}

#[derive(Clone, Debug)]
struct TrendAccumulator {
    kind: String,
    first_commit: String,
    last_commit: String,
    first_authored_at_seconds: i64,
    last_authored_at_seconds: i64,
    observation_count: usize,
}

/// Summarize exact blind-spot IDs across an ordered, bounded history slice.
/// Missing graph-insights sidecars are expected for older realizations and are
/// counted explicitly rather than being treated as an empty observation.
pub fn summarize_blind_spots(
    observations: &[BlindSpotObservation],
) -> Result<BlindSpotTrend, HistoryError> {
    let mut items = BTreeMap::<String, TrendAccumulator>::new();
    let mut observations_with_graph_insights = 0_usize;
    let mut observations_without_graph_insights = 0_usize;
    for observation in observations {
        let Some(report) = observation.report.as_ref() else {
            observations_without_graph_insights =
                observations_without_graph_insights.saturating_add(1);
            continue;
        };
        let ids = parse_report_ids(report)?;
        observations_with_graph_insights = observations_with_graph_insights.saturating_add(1);
        for (id, kind) in ids {
            if let Some(existing) = items.get(&id)
                && existing.kind != kind
            {
                return Err(HistoryError::InvalidArtifacts(format!(
                    "blind-spot ID {id} changed kind from {} to {kind}",
                    existing.kind
                )));
            }
            let entry = items.entry(id).or_insert_with(|| TrendAccumulator {
                kind: kind.clone(),
                first_commit: observation.commit.clone(),
                last_commit: observation.commit.clone(),
                first_authored_at_seconds: observation.authored_at_seconds,
                last_authored_at_seconds: observation.authored_at_seconds,
                observation_count: 0,
            });
            entry.last_commit.clone_from(&observation.commit);
            entry.last_authored_at_seconds = observation.authored_at_seconds;
            entry.observation_count = entry.observation_count.saturating_add(1);
        }
    }
    // A realization may predate the graph-insights sidecar. In that case the
    // latest observation with a valid sidecar is the newest graph we can
    // compare; an older realization must not be marked resolved merely
    // because a newer observation has no sidecar.
    let latest_graph_insights_commit = observations.iter().rev().find_map(|observation| {
        observation
            .report
            .as_ref()
            .map(|_| observation.commit.as_str())
    });
    let total_items = items.len();
    let mut active = Vec::new();
    let mut resolved = Vec::new();
    for (id, item) in items.into_iter().take(MAX_TREND_ITEMS) {
        let is_active = latest_graph_insights_commit == Some(item.last_commit.as_str());
        let trend = BlindSpotTrendItem {
            id,
            kind: item.kind,
            first_commit: item.first_commit,
            last_commit: item.last_commit,
            first_authored_at_seconds: item.first_authored_at_seconds,
            last_authored_at_seconds: item.last_authored_at_seconds,
            observation_count: item.observation_count,
            active: is_active,
        };
        if is_active {
            active.push(trend);
        } else {
            resolved.push(trend);
        }
    }
    active.sort_by(|left, right| left.id.cmp(&right.id));
    resolved.sort_by(|left, right| {
        right
            .last_authored_at_seconds
            .cmp(&left.last_authored_at_seconds)
            .then_with(|| left.id.cmp(&right.id))
    });
    let omitted_items = total_items.saturating_sub(MAX_TREND_ITEMS);
    Ok(BlindSpotTrend {
        schema: BLIND_SPOT_HISTORY_SCHEMA.to_owned(),
        observation_count: observations.len(),
        observations_with_graph_insights,
        first_commit: observations
            .first()
            .map(|observation| observation.commit.clone()),
        last_commit: observations
            .last()
            .map(|observation| observation.commit.clone()),
        active,
        resolved,
        omissions: BlindSpotTrendOmissions {
            items: omitted_items,
            observations_without_graph_insights,
        },
    })
}

fn parse_report_ids(report: &Value) -> Result<BTreeSet<(String, String)>, HistoryError> {
    let schema = report
        .get("schema")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            HistoryError::InvalidArtifacts("blind-spot report has no schema".to_owned())
        })?;
    if schema != "compass.graph-insights/1" {
        return Err(HistoryError::InvalidArtifacts(format!(
            "unsupported blind-spot report schema {schema}"
        )));
    }
    let limits = report
        .get("limits")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            HistoryError::InvalidArtifacts("blind-spot report has no limits".to_owned())
        })?;
    for (field, expected) in [
        ("maxCandidatePairs", MAX_CANDIDATE_PAIRS),
        ("maxCommunityGaps", MAX_COMMUNITY_GAPS),
        ("maxSharedIntermediaries", MAX_WITNESSES),
        ("maxDirectTopicalEdges", MAX_WITNESSES),
        ("maxDisconnectedComponents", MAX_COMPONENTS),
        ("maxComponentMembers", MAX_COMPONENT_MEMBERS),
    ] {
        let actual = limits.get(field).and_then(Value::as_u64).ok_or_else(|| {
            HistoryError::InvalidArtifacts(format!("blind-spot limits has no {field}"))
        })?;
        if actual != expected as u64 {
            return Err(HistoryError::InvalidArtifacts(format!(
                "blind-spot limit {field} is {actual}, expected {expected}"
            )));
        }
    }
    let mut ids = BTreeSet::new();
    for (field, kind) in [
        ("communityGaps", "community_gap"),
        ("disconnectedComponents", "disconnected_component"),
    ] {
        let values = report.get(field).and_then(Value::as_array).ok_or_else(|| {
            HistoryError::InvalidArtifacts(format!("blind-spot report has no {field} array"))
        })?;
        let maximum = if field == "communityGaps" {
            MAX_COMMUNITY_GAPS
        } else {
            MAX_COMPONENTS
        };
        if values.len() > maximum {
            return Err(HistoryError::InvalidArtifacts(format!(
                "blind-spot {field} exceeds limit {maximum}"
            )));
        }
        for value in values {
            let id = value
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| {
                    HistoryError::InvalidArtifacts(format!("blind-spot {kind} has no ID"))
                })?;
            if id.len() > MAX_REPORT_ID_BYTES {
                return Err(HistoryError::InvalidArtifacts(format!(
                    "blind-spot {kind} ID exceeds byte limit"
                )));
            }
            ids.insert((id.to_owned(), kind.to_owned()));
        }
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn observation(commit: &str, report: Option<Value>) -> BlindSpotObservation {
        BlindSpotObservation {
            commit: commit.to_owned(),
            authored_at_seconds: commit.as_bytes()[0] as i64,
            report,
        }
    }

    fn report(gaps: &[&str], components: &[&str]) -> Value {
        json!({
            "schema": "compass.graph-insights/1",
            "communityGaps": gaps.iter().map(|id| json!({"id": id})).collect::<Vec<_>>(),
            "disconnectedComponents": components.iter().map(|id| json!({"id": id})).collect::<Vec<_>>(),
            "limits": {
                "maxCandidatePairs": 200000,
                "maxCommunityGaps": 3,
                "maxSharedIntermediaries": 8,
                "maxDirectTopicalEdges": 8,
                "maxDisconnectedComponents": 32,
                "maxComponentMembers": 64,
            },
        })
    }

    #[test]
    fn summarizes_active_and_resolved_ids_deterministically() {
        let result = summarize_blind_spots(&[
            observation("a", Some(report(&["gap-a"], &[]))),
            observation("b", Some(report(&["gap-a"], &["component:b"]))),
            observation("c", Some(report(&[], &["component:b"]))),
        ]);
        assert!(result.is_ok(), "valid trend: {:?}", result.as_ref().err());
        let Some(trend) = result.ok() else {
            return;
        };

        assert_eq!(trend.observation_count, 3);
        assert_eq!(trend.observations_with_graph_insights, 3);
        assert_eq!(trend.active.len(), 1);
        assert_eq!(trend.active[0].id, "component:b");
        assert_eq!(trend.resolved.len(), 1);
        assert_eq!(trend.resolved[0].id, "gap-a");
        assert!(!trend.resolved[0].active);
        assert_eq!(trend.resolved[0].observation_count, 2);
    }

    #[test]
    fn missing_latest_sidecar_does_not_fake_resolution() {
        let result = summarize_blind_spots(&[
            observation("a", Some(report(&["gap-a"], &[]))),
            observation("b", None),
        ]);
        assert!(result.is_ok(), "valid trend: {:?}", result.as_ref().err());
        let Some(trend) = result.ok() else {
            return;
        };

        assert_eq!(trend.omissions.observations_without_graph_insights, 1);
        assert_eq!(
            trend
                .active
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["gap-a"]
        );
        assert!(trend.resolved.is_empty());
    }

    #[test]
    fn rejects_unsupported_schema_and_kind_changes() {
        let mut unsupported = report(&["gap-a"], &[]);
        unsupported["schema"] = json!("compass.graph-insights/2");
        assert!(summarize_blind_spots(&[observation("a", Some(unsupported))]).is_err());

        let mut mismatch = report(&["same"], &[]);
        mismatch["disconnectedComponents"] = json!([{"id": "same"}]);
        assert!(summarize_blind_spots(&[observation("a", Some(mismatch))]).is_err());
    }
}
