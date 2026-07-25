use serde::Serialize;

use crate::CommitId;

pub const HISTORY_TIMELINE_SCHEMA: &str = "compass.history.timeline/1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineCommit {
    pub commit: CommitId,
    pub parents: Vec<CommitId>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at_seconds: i64,
    pub subject: String,
}
