use std::time::Duration;

use serde::Serialize;

#[derive(Clone, Debug, Default, Serialize)]
pub struct OperatorProfile {
    pub name: String,
    pub input_rows: u64,
    pub output_rows: u64,
    pub candidate_nodes: u64,
    pub expanded_relationships: u64,
    pub peak_memory_bytes: usize,
    pub elapsed: Duration,
    pub cancellation_checks: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct QueryProfile {
    pub operators: Vec<OperatorProfile>,
    pub candidate_nodes: u64,
    pub expanded_relationships: u64,
    pub peak_memory_bytes: usize,
    pub elapsed: Duration,
    pub cancellation_checks: u64,
    pub plan_cache_hit: Option<bool>,
}
