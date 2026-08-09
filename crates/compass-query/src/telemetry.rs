use std::time::Duration;

use compass_model::query_contract::CodeQueryResponse;
use serde::{Deserialize, Serialize};

pub const QUERY_EXECUTION_PROFILE_V1: &str = "compass.query-execution-profile/1";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkCounts {
    pub candidates_read: u64,
    pub postings_decoded: u64,
    pub nodes_expanded: u64,
    pub edges_expanded: u64,
    pub response_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryStageTimings {
    pub intent_micros: u64,
    pub recall_micros: u64,
    pub ranking_micros: u64,
    pub execution_micros: u64,
    pub total_micros: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryExecutionProfile {
    pub schema: String,
    pub planner_profile: String,
    pub ranker_profile: String,
    pub timings: QueryStageTimings,
    pub work: WorkCounts,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfiledCodeQueryResponse {
    pub response: CodeQueryResponse,
    pub profile: QueryExecutionProfile,
}

#[derive(Debug, Default)]
pub(crate) struct QueryInstrumentation {
    pub(crate) intent: Duration,
    pub(crate) recall: Duration,
    pub(crate) ranking: Duration,
    pub(crate) execution: Duration,
    pub(crate) work: WorkCounts,
}

impl QueryInstrumentation {
    pub(crate) fn finish(
        mut self,
        response: CodeQueryResponse,
        total: Duration,
        planner_profile: &str,
        ranker_profile: &str,
    ) -> ProfiledCodeQueryResponse {
        self.work.response_bytes = serde_json::to_vec(&response)
            .ok()
            .and_then(|bytes| u64::try_from(bytes.len()).ok())
            .unwrap_or(u64::MAX);
        ProfiledCodeQueryResponse {
            response,
            profile: QueryExecutionProfile {
                schema: QUERY_EXECUTION_PROFILE_V1.to_owned(),
                planner_profile: planner_profile.to_owned(),
                ranker_profile: ranker_profile.to_owned(),
                timings: QueryStageTimings {
                    intent_micros: micros(self.intent),
                    recall_micros: micros(self.recall),
                    ranking_micros: micros(self.ranking),
                    execution_micros: micros(self.execution),
                    total_micros: micros(total),
                },
                work: self.work,
            },
        }
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
