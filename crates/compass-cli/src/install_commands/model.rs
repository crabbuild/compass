use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum InstallScope {
    Project(PathBuf),
    User(PathBuf),
}

impl InstallScope {
    pub(super) fn root(&self) -> &Path {
        match self {
            Self::Project(root) | Self::User(root) => root,
        }
    }

    pub(super) fn kind(&self) -> ScopeKind {
        match self {
            Self::Project(_) => ScopeKind::Project,
            Self::User(_) => ScopeKind::User,
        }
    }

    pub(super) fn is_project(&self) -> bool {
        matches!(self, Self::Project(_))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ScopeKind {
    Project,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SupportTier {
    SharedSkill,
    NativeSkill,
    AdapterOnly,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct InstallRequest {
    pub platforms: Vec<String>,
    pub all: bool,
    pub project: bool,
    pub user: bool,
    pub strict: bool,
    pub dry_run: bool,
    pub require_all: bool,
    pub format: OutputFormat,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InstallStatus {
    Installed,
    Updated,
    Current,
    Skipped,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct TargetResult {
    pub id: String,
    pub consumers: BTreeSet<String>,
    pub status: InstallStatus,
    pub paths: Vec<PathBuf>,
    pub reason: Option<String>,
    pub rollback: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct InstallReport {
    pub schema: u32,
    pub scope: ScopeKind,
    pub root: PathBuf,
    pub selected: Vec<String>,
    pub detected: BTreeMap<String, Vec<String>>,
    pub results: Vec<TargetResult>,
    pub graph_exists: bool,
    pub next_actions: Vec<String>,
}
