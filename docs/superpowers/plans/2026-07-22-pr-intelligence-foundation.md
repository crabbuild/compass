# Compass PR Intelligence Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the trusted foundation for Compass PR Intelligence: shared immutable graph snapshots, exact pull-request revision capture, a versioned typed report, an immutable evidence manifest, durable report persistence, and typed CLI/MCP adapters.

**Architecture:** `compass-core` owns one shared graph-selection and immutable-snapshot interface consumed by PR Intelligence and CompassQL. `compass-prs` owns exact change-request capture, canonical report identity, foundation orchestration, and durable persistence. CLI and MCP call the same typed operation. The foundation emits no risk, impact, owner, test, overlap, or gate verdict and labels every result `foundation_preview` until the later child designs are approved and implemented.

**Tech Stack:** Rust 2024, Rust 1.97, Serde/JSON, SHA-256, workspace `url`, existing `compass-history` SQLite-backed Prolly realizations, existing `compass-core`/`compass-model` graph types, Git CLI, GitHub CLI, `wait-timeout`, and the current Compass CLI/MCP crates.

## Global Constraints

- Implement in an isolated worktree created from the standalone Compass repository root; preserve all unrelated user changes in the primary worktree.
- Treat `docs/superpowers/specs/2026-07-22-pr-intelligence-design.md` as the governing umbrella design.
- This plan implements child 1 only. It must not implement semantic delta, downstream traversal, ownership selection, test selection, overlap analysis, risk bands, policy evaluation, or deterministic gates.
- PR Intelligence remains preview-only. No foundation result may imply that a pull request is safe, low risk, policy-compliant, or sufficiently tested.
- This plan owns the shared `GraphSelection`, `SnapshotProvider`, `GraphSnapshot`, and `SnapshotIdentity` interfaces. The CompassQL integration plan must consume these types after this plan lands rather than adding duplicates.
- Reuse the current implemented `compass-history` crate and `LoadedGraph::from_document`; do not create another historical store or revision resolver.
- A pull-request analysis captures full object IDs for merge base, pull-request head, target head, and the synthetic merge tree before opening graph snapshots.
- Never fetch, checkout, or execute code implicitly while capturing pull-request metadata. Missing Git objects fail with an actionable error.
- Require support for `git merge-tree --write-tree -z --name-only`. An unsupported Git build fails with `PRS2007` and an upgrade diagnostic; do not silently substitute a mutating merge.
- Graph materialization continues to use the existing offline history worktree protections and complete-realization validation.
- Policies, CODEOWNERS, risk, owners, tests, overlap, and merge gates are absent from the foundation report rather than represented by invented clean results.
- Canonical report identity excludes timestamps, machine-specific absolute paths, process IDs, timings, and presentation formatting. It includes the exact semantic file snapshot, including the learning overlay visible to queries.
- Compass owns the command syntax, MCP tools, schemas, storage paths, environment variables, tests, and deprecation policy introduced by this plan.
- Keep Compass-specific runtime code, fixtures, schemas, migrations, and release automation inside the Compass repository. Tests assert Compass contracts directly.
- New JSON schemas reject unknown fields when deserialized by Compass-owned readers.
- Process output remains bounded at 16 MiB per stream and becomes strict UTF-8; invalid UTF-8 is an error rather than lossy evidence.
- Report files are limited to 64 MiB, written atomically, owner-only where supported, and never written through symlinks.
- Every task follows red-green-refactor, ends in an independently reviewable commit, and runs focused tests plus Clippy with warnings denied.
- Workspace lints remain `unsafe_code = "forbid"`, `unwrap_used = "deny"`, `expect_used = "deny"`, and `panic = "deny"`.

---

## Plan sequence and dependency rule

This plan lands before the snapshot portion of `docs/superpowers/plans/2026-07-22-compassql-integrations.md`. When that plan executes, it uses the `compass-core` snapshot types created here and adds query/check orchestration around them. It must not create a second `GraphSelection`, `SnapshotProvider`, `GraphSnapshot`, or `SnapshotIdentity`.

The already implemented versioned-graph history is a precondition. Verify before Task 1:

```bash
test -f crates/compass-history/src/lib.rs
test -f crates/compass-core/src/history.rs
cargo test -p compass-history
cargo test -p compass-core --test history_materialize
```

Expected: all commands exit `0`.

## File and module map

Create or extend these focused modules:

- `crates/compass-core/src/snapshot.rs` owns shared selection, identity, snapshot, and provider types.
- `crates/compass-core/src/lib.rs` exports the stable snapshot surface.
- `crates/compass-core/tests/snapshot.rs` verifies file snapshot identity and provider contracts.
- `crates/compass-prs/src/model.rs` owns reports, revisions, findings, completeness, and gates.
- `crates/compass-prs/src/canonical.rs` owns canonical bytes and report/evidence SHA-256 identities.
- `crates/compass-prs/src/source.rs` owns the change-request seam and GitHub CLI adapter.
- `crates/compass-prs/src/git.rs` owns exact local Git revision, diff, and merge-tree operations.
- `crates/compass-prs/src/operation.rs` owns foundation orchestration and the repository seam.
- `crates/compass-prs/src/report_store.rs` owns filesystem persistence.
- `crates/compass-prs/src/service.rs` owns the object-safe analysis service consumed by adapters.
- `crates/compass-prs/src/lib.rs` contains the public typed surface.
- `crates/compass-prs/tests/report_contract.rs` verifies schema, identity, and strict decoding.
- `crates/compass-prs/tests/change_capture.rs` verifies exact revisions and merge outcomes.
- `crates/compass-prs/tests/foundation_operation.rs` verifies manifest and snapshot consistency.
- `crates/compass-prs/tests/report_store.rs` verifies atomic persistence and hostile paths.
- `crates/compass-prs/tests/service.rs` verifies the adapter-facing analysis service.
- `crates/compass-cli/src/snapshot_provider.rs` owns file/history production snapshots.
- `crates/compass-cli/src/prs_commands.rs` owns the foundation preview adapter.
- `crates/compass-cli/src/lib.rs` consumes the shared selection type and dispatches preview.
- `crates/compass-cli/tests/prs_cli.rs` verifies existing Compass PR commands remain functional.
- `crates/compass-cli/tests/prs_intelligence_preview.rs` verifies exact-revision preview end to end.
- `crates/compass-mcp/src/pr_intelligence.rs` owns the typed `analyze_pr` MCP adapter.
- `crates/compass-mcp/src/lib.rs` registers the typed adapter.
- `crates/compass-mcp/tests/pr_intelligence.rs` verifies structured PR Intelligence results.
- `docs/PR_INTELLIGENCE.md` documents the preview contract, limitations, and troubleshooting.
- `scripts/benchmark_pr_foundation.sh` measures reproducible cold/warm foundation performance.

Do not add a new crate. `compass-prs` remains the domain module and `compass-core` remains the shared graph snapshot module.

## Shared public interfaces

Tasks use these exact names and shapes.

### Shared snapshots in `compass-core`

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphSelection {
    File(std::path::PathBuf),
    Commit(String),
    SyntheticMerge {
        target_head: String,
        pull_request_head: String,
        tree: String,
    },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SnapshotIdentity {
    File { sha256: String },
    Commit { commit: String, realization: String },
    SyntheticMerge {
        target_head: String,
        pull_request_head: String,
        tree: String,
        realization: String,
    },
}

pub struct GraphSnapshot {
    pub graph: compass_model::Graph,
    pub overlay:
        std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>>,
    pub schema_fingerprint: compass_model::SchemaFingerprint,
    pub identity: SnapshotIdentity,
}

pub trait SnapshotProvider: Send + Sync {
    fn load(
        &self,
        selection: &GraphSelection,
    ) -> Result<std::sync::Arc<GraphSnapshot>, SnapshotError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("graph snapshot was not found: {0}")]
    NotFound(String),
    #[error("graph snapshot is corrupt: {0}")]
    Corrupt(String),
    #[error("graph snapshot selection is unsupported by this adapter: {0}")]
    Unsupported(String),
    #[error("graph snapshot identity mismatch: expected {expected}, got {actual}")]
    IdentityMismatch { expected: String, actual: String },
    #[error("graph snapshot could not be loaded: {0}")]
    Load(String),
}
```

Implement `SnapshotIdentity` deserialization through a private raw enum plus `TryFrom`: file digests and realization IDs must be exactly 64 lowercase hexadecimal characters, and every commit/tree ID must satisfy the same full 40-or-64-character rule as `GitObjectId`. Invalid identities must fail before a provider or report can use them. Child 1 reserves `SyntheticMerge` but providers return `SnapshotError::Unsupported("synthetic_merge")`; child 2 will add non-mutating tree materialization without changing this shared contract.

`SnapshotIdentity::Display` returns:

```text
file:<64 lowercase hex>
commit:<full commit oid>:<64 lowercase realization hex>
merge:<full target oid>:<full PR oid>:<full tree oid>:<64 lowercase realization hex>
```

The `File.sha256` digest is not merely the raw `graph.json` checksum. It is the SHA-256 of a versioned canonical semantic-snapshot envelope containing the parsed `GraphDocument` and the exact learning overlay returned in `GraphSnapshot`. Recursively sort every JSON object key before serialization. This ensures that a changed overlay produces a changed identity and that the identity always describes the data actually queried.

### Change-request and revision model in `compass-prs`

```rust
pub const REPORT_SCHEMA: &str = "compass.pr_intelligence.report/1";
pub const EVIDENCE_MANIFEST_SCHEMA: &str = "compass.pr_intelligence.evidence/1";
pub const REPORT_ID_SCHEMA: &str = "cmppr-report-v1";
pub const FINDING_FINGERPRINT_SCHEMA: &str = "cmpprv1";

#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
pub enum ForgeKind {
    #[serde(rename = "github")]
    GitHub,
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryIdentityError {
    #[error("invalid GitHub host: {0}")]
    Host(String),
    #[error("invalid GitHub owner: {0}")]
    Owner(String),
    #[error("invalid GitHub repository name: {0}")]
    Name(String),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryIdentity {
    forge: ForgeKind,
    host: String,
    owner: String,
    name: String,
}

impl RepositoryIdentity {
    pub fn github(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, RepositoryIdentityError> {
        let host = normalize_github_host(host.into())?;
        let owner = validate_github_component(owner.into())
            .map_err(RepositoryIdentityError::Owner)?;
        let name = validate_github_component(name.into())
            .map_err(RepositoryIdentityError::Name)?;
        Ok(Self {
            forge: ForgeKind::GitHub,
            host,
            owner,
            name,
        })
    }

    #[must_use]
    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    #[must_use]
    pub const fn forge(&self) -> ForgeKind {
        self.forge
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn gh_repo_argument(&self) -> String {
        if self.host == "github.com" {
            self.slug()
        } else {
            format!("{}/{}", self.host, self.slug())
        }
    }
}

fn normalize_github_host(value: String) -> Result<String, RepositoryIdentityError> {
    if value.is_empty()
        || value.chars().any(char::is_control)
        || value.chars().any(char::is_whitespace)
        || value
            .chars()
            .any(|character| matches!(character, '/' | '@' | '?' | '#'))
    {
        return Err(RepositoryIdentityError::Host(value));
    }
    let parsed = url::Url::parse(&format!("https://{value}/"))
        .map_err(|_| RepositoryIdentityError::Host(value.clone()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| RepositoryIdentityError::Host(value.clone()))?
        .to_ascii_lowercase();
    Ok(match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

fn validate_github_component(value: String) -> Result<String, String> {
    let valid = !value.is_empty()
        && value.len() <= 255
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
        });
    if valid {
        Ok(value.to_ascii_lowercase())
    } else {
        Err(value)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepositoryIdentity {
    forge: ForgeKind,
    host: String,
    owner: String,
    name: String,
}

impl<'de> serde::Deserialize<'de> for RepositoryIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawRepositoryIdentity::deserialize(deserializer)?;
        match raw.forge {
            ForgeKind::GitHub => Self::github(raw.host, raw.owner, raw.name),
        }
        .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRequestSelector {
    pub number: u64,
    pub repository: Option<RepositoryIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRequestIdentity {
    pub repository: RepositoryIdentity,
    pub number: u64,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize)]
#[serde(transparent)]
pub struct GitObjectId(String);

#[derive(Debug, thiserror::Error)]
#[error("invalid full Git object ID: {0}")]
pub struct GitObjectIdError(String);

impl GitObjectId {
    pub fn parse(value: impl Into<String>) -> Result<Self, GitObjectIdError> {
        let value = value.into();
        let valid_length = matches!(value.len(), 40 | 64);
        let valid_hex = value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if valid_length && valid_hex {
            Ok(Self(value))
        } else {
            Err(GitObjectIdError(value))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum MergeOutcome {
    Clean { tree: GitObjectId },
    Conflicted {
        tree: GitObjectId,
        paths: Vec<String>,
        conflict_digest: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionSet {
    pub merge_base: GitObjectId,
    pub pull_request_head: GitObjectId,
    pub target_head: GitObjectId,
    pub merge_outcome: MergeOutcome,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct ChangedFile {
    pub kind: FileChangeKind,
    pub old_path: Option<String>,
    pub new_path: Option<String>,
}

impl ChangedFile {
    #[must_use]
    pub fn paths_are_repository_relative(&self) -> bool {
        self.old_path
            .iter()
            .chain(self.new_path.iter())
            .all(|path| is_safe_repository_path(path))
    }
}

fn is_safe_repository_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.ends_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && !path
            .split('/')
            .any(|component| component.is_empty() || matches!(component, "." | ".."))
        && !path
            .split('/')
            .next()
            .is_some_and(|component| component.ends_with(':'))
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedChangeRequest {
    pub identity: ChangeRequestIdentity,
    pub revisions: RevisionSet,
    pub base_branch: String,
    pub head_branch: String,
    pub changed_files: Vec<ChangedFile>,
    pub changed_files_digest: String,
}

pub trait ChangeRequestSource: Send + Sync {
    fn capture(
        &self,
        selector: &ChangeRequestSelector,
    ) -> Result<CapturedChangeRequest, SourceError>;
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("[PRS2001] GitHub metadata is unavailable: {message}")]
    GitHubMetadataUnavailable { message: String },
    #[error("[PRS2002] GitHub metadata is malformed: {message}")]
    MalformedGitHubMetadata { message: String },
    #[error("[PRS2003] repository identity mismatch: expected {expected}, got {actual}")]
    RepositoryMismatch { expected: String, actual: String },
    #[error("[PRS2004] required Git object is missing locally: {object}")]
    MissingGitObject { object: GitObjectId },
    #[error("[PRS2005] Git revision resolution failed: {message}")]
    RevisionResolution { message: String },
    #[error("[PRS2006] changed-file evidence is invalid: {message}")]
    InvalidChangedFiles { message: String },
    #[error("[PRS2007] synthetic merge evaluation failed: {message}")]
    MergeEvaluation { message: String },
    #[error("[PRS2008] {program} returned non-UTF-8 {stream}")]
    InvalidUtf8 {
        program: String,
        stream: &'static str,
    },
}

impl SourceError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::GitHubMetadataUnavailable { .. } => "PRS2001",
            Self::MalformedGitHubMetadata { .. } => "PRS2002",
            Self::RepositoryMismatch { .. } => "PRS2003",
            Self::MissingGitObject { .. } => "PRS2004",
            Self::RevisionResolution { .. } => "PRS2005",
            Self::InvalidChangedFiles { .. } => "PRS2006",
            Self::MergeEvaluation { .. } => "PRS2007",
            Self::InvalidUtf8 { .. } => "PRS2008",
        }
    }
}
```

`GitObjectId::parse` accepts only lowercase hexadecimal Git object IDs of exactly 40 or 64 characters. Uppercase, abbreviated, symbolic, empty, or non-hex values fail.

`RepositoryIdentity` has no public field constructor. Its constructor and custom deserializer reject unknown fields, invalid hosts, credentials, paths, whitespace, and invalid components. GitHub host, owner, and repository components normalize to lowercase so equality, hashing, ordering, report identity, CLI input, PR URLs, and local remotes use one canonical identity.

### Foundation report in `compass-prs`

```rust
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisState {
    FoundationPreview,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    LocalExact,
    DownstreamComplete,
    DownstreamPartial,
    DownstreamUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceManifest {
    pub schema: String,
    pub request: ChangeRequestIdentity,
    pub revisions: RevisionSet,
    pub changed_files_digest: String,
    pub snapshots: std::collections::BTreeMap<SnapshotRole, SnapshotEvidence>,
    pub extractor_version: String,
    pub extractor_config_digest: String,
    pub policy_pack_digest: Option<String>,
}

#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotRole {
    MergeBase,
    PullRequestHead,
    TargetHead,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEvidence {
    pub identity: compass_core::SnapshotIdentity,
    pub schema_fingerprint: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct PrIntelligenceReport {
    pub schema: String,
    pub report_id: String,
    pub state: AnalysisState,
    pub request: ChangeRequestIdentity,
    pub revisions: RevisionSet,
    pub evidence_manifest: EvidenceManifest,
    pub evidence_manifest_digest: String,
    pub snapshots: std::collections::BTreeMap<SnapshotRole, SnapshotEvidence>,
    pub completeness: Completeness,
    pub repository_evidence: Vec<RepositoryEvidence>,
    pub findings: Vec<Finding>,
    pub risk: Option<RiskAssessment>,
    pub gates: Vec<GateResult>,
    pub failures: Vec<AnalysisFailure>,
}
```

Task 2 defines `Finding`, `RiskAssessment`, `GateResult`, and `AnalysisFailure` as strict serializable types even though the foundation creates empty `findings`/`gates`, `risk: None`, and no failures on success. This reserves the approved schema without inventing verdicts.

The strict report validator requires findings sorted uniquely by fingerprint, gates sorted uniquely by `(rule_id, rule_version)`, repository evidence sorted uniquely by `(role, repository)`, risk factors sorted uniquely by kind, and failures sorted by `(code, source, message)`. Every finding fingerprint must be `cmpprv1:<64 lowercase hex>`. `FoundationPreview` requires empty findings/gates, `risk: None`, and exactly one captured local repository record; `Complete` requires a risk assessment and is reserved for later children. These ordering and state rules make canonical report bytes independent of collection insertion order and prevent preview data from masquerading as a verdict.

Reserve completeness detail without claiming downstream coverage:

```rust
#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryRole {
    Local,
    Downstream,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationOutcome {
    NotRequired,
    Allowed,
    Denied,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFreshness {
    Current,
    Stale,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryEvidenceState {
    Captured,
    Complete,
    Unavailable,
    Unauthorized,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryEvidence {
    pub repository: RepositoryIdentity,
    pub role: RepositoryRole,
    pub observed_default_head: Option<GitObjectId>,
    pub graph_revision: Option<GitObjectId>,
    pub freshness: EvidenceFreshness,
    pub authorization: AuthorizationOutcome,
    pub state: RepositoryEvidenceState,
    pub failure_code: Option<String>,
}
```

The foundation constructor emits exactly one `Local` record for the request repository, with target head as both observed default head and graph revision, `Current`, `NotRequired`, `Captured`, and no failure code. `Captured` means revision and snapshot provenance is exact; it does not claim semantic merge analysis is complete. The foundation emits no downstream records.

`EvidenceManifest` is embedded so a report remains independently verifiable after transport or storage. Its custom deserializer requires `schema == EVIDENCE_MANIFEST_SCHEMA`. `PrIntelligenceReport` uses a raw helper plus `TryFrom` during deserialization to require `schema == REPORT_SCHEMA`, validate every digest and ID, verify the duplicated request/revisions/snapshots equal the embedded manifest, recompute the manifest digest, and recompute the report ID.

### Foundation operation and persistence

```rust
pub trait ReportRepository: Send + Sync {
    fn save(&self, report: &PrIntelligenceReport) -> Result<StoredReport, ReportStoreError>;
    fn load(&self, report_id: &str) -> Result<PrIntelligenceReport, ReportStoreError>;
}

pub struct StoredReport {
    pub report_id: String,
    pub path: Option<std::path::PathBuf>,
}

pub struct FoundationOperation {
    source: std::sync::Arc<dyn ChangeRequestSource>,
    snapshots: std::sync::Arc<dyn compass_core::SnapshotProvider>,
    reports: std::sync::Arc<dyn ReportRepository>,
    extractor_version: String,
    extractor_config_digest: String,
}

impl FoundationOperation {
    pub fn new(
        source: std::sync::Arc<dyn ChangeRequestSource>,
        snapshots: std::sync::Arc<dyn compass_core::SnapshotProvider>,
        reports: std::sync::Arc<dyn ReportRepository>,
        extractor_version: String,
        extractor_config_digest: String,
    ) -> Result<Self, CanonicalError> {
        if extractor_version.is_empty()
            || extractor_version.chars().any(char::is_control)
        {
            return Err(CanonicalError::Contract(
                "extractor version must be nonempty and contain no control characters"
                    .to_owned(),
            ));
        }
        validate_sha256_digest(&extractor_config_digest)?;
        Ok(Self {
            source,
            snapshots,
            reports,
            extractor_version,
            extractor_config_digest,
        })
    }

    pub fn analyze(
        &self,
        selector: &ChangeRequestSelector,
    ) -> Result<PrIntelligenceReport, OperationError>;
}
```

The remaining shared errors have these exact contracts:

```rust
#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    #[error("canonical JSON serialization failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid lowercase SHA-256 digest: {0}")]
    InvalidDigest(String),
    #[error("invalid PR Intelligence report ID: {0}")]
    InvalidReportId(String),
    #[error("report contract mismatch: {0}")]
    Contract(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ReportStoreError {
    #[error("invalid report ID: {0}")]
    InvalidReportId(String),
    #[error("unsafe report path {path}: {reason}")]
    UnsafePath { path: std::path::PathBuf, reason: String },
    #[error("report exceeds the 64 MiB limit: {bytes} bytes")]
    TooLarge { bytes: u64 },
    #[error("report identity collision: {0}")]
    IdentityCollision(String),
    #[error("report was not found: {0}")]
    NotFound(String),
    #[error("report repository state is unavailable: {0}")]
    State(String),
    #[error("report contract is invalid: {0}")]
    Contract(#[from] CanonicalError),
    #[error("report I/O failed at {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum OperationError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error("snapshot load failed for {role:?}: {source}")]
    SnapshotLoad {
        role: SnapshotRole,
        #[source]
        source: compass_core::SnapshotError,
    },
    #[error("snapshot identity mismatch for {role:?}: expected {expected}, got {actual}")]
    SnapshotIdentity {
        role: SnapshotRole,
        expected: String,
        actual: String,
    },
    #[error(transparent)]
    Canonical(#[from] CanonicalError),
    #[error(transparent)]
    ReportStore(#[from] ReportStoreError),
}

impl OperationError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Source(source) => source.code(),
            Self::SnapshotLoad { .. } => "PRS3001",
            Self::SnapshotIdentity { .. } => "PRS3002",
            Self::Canonical(_) => "PRS3003",
            Self::ReportStore(_) => "PRS3004",
        }
    }
}
```

`ReportStoreError::IdentityCollision` carries the textual `cmppr-report-v1:` ID. Public getters expose validated string forms without exposing mutable inner strings.

## Test fixture contracts

Test helpers are fully specified local support code:

- `fixture_report(order)` constructs one fully valid `foundation_preview` report, inserting its three snapshot roles in the requested order before the production constructor canonicalizes them.
- `fixture_report_json()` serializes that valid report to a mutable `serde_json::Value` for one-field corruption tests.
- `fixture_manifest_json()` returns the valid embedded evidence manifest used by `fixture_report_json()`.
- `FileSnapshotFixture` writes a guarded `graph.json` plus optional `.compass_learning.json` and exposes owned paths without changing process-global environment.
- `GitFixture::diverged_clean_merge()` creates a temporary repository with a shared merge base, an advanced target, a divergent PR head, one modified `src/lib.rs`, and a stub `ProcessRunner` whose recorded argument vectors are exposed by `saw_arguments`.
- `captured_fixture()` returns full lowercase OIDs and a canonical changed-file digest; `FakeSource` returns it once; `FakeSnapshots` maps its three commit selections to matching immutable snapshots.
- `PreviewFixture::new()` creates a temporary repository, three complete history realizations, a fake `gh` executable on a fixture-scoped `PATH`, and helpers for invoking the built Compass binary and resolving a persisted report path.
- `QualificationFixture` wraps `PreviewFixture`, can replace the captured head/target OID returned by fake `gh`, return one mismatched snapshot identity, create a conflicted merge, remove one preferred history realization, inject one report-store failpoint, create Unicode/control-character paths, generate one-byte-over-limit process/report values, place a symlink at a named store segment, and report the exact persisted files after retry.
- `FakeAnalyzer` implements `PrIntelligenceAnalyzer`, returns one configured report or error, and exposes an atomic call count. `McpToolResultExt` decodes the typed structured report and exposes the protocol error flag without inspecting display text.
- Hostile filesystem tests use test-only failpoints in `FilesystemReportRepository`, enabled through a private constructor under `cfg(test)`; production code has no environment-variable failpoints.

## Task 1: Promote graph selection into one immutable snapshot module

**Files:**

- Create: `crates/compass-core/src/snapshot.rs`
- Modify: `crates/compass-core/src/lib.rs`
- Modify: `crates/compass-cli/src/lib.rs:3370-3646`
- Create: `crates/compass-core/tests/snapshot.rs`
- Modify: `crates/compass-core/Cargo.toml`
- Modify: `Cargo.lock`

**Interfaces:**

- Consumes: `compass_model::{Graph, GraphDocument, SchemaFingerprint}` and existing `LoadedGraph`.
- Produces: `GraphSelection`, `SnapshotIdentity`, `GraphSnapshot`, `SnapshotProvider`, `SnapshotError`, and `FileSnapshotProvider`.

- [ ] **Step 1: Write failing snapshot identity and provider tests**

Create `crates/compass-core/tests/snapshot.rs` with the exact fixture contract above and these tests:

```rust
use std::fs;

use compass_core::{
    FileSnapshotProvider, GraphSelection, SnapshotIdentity, SnapshotProvider,
};

#[test]
fn file_snapshot_identity_is_content_addressed_and_schema_bound()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    fs::write(
        &path,
        br#"{"directed":true,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"A"}],"links":[]}"#,
    )?;
    let provider = FileSnapshotProvider;
    let first = provider.load(&GraphSelection::File(path.clone()))?;
    let second = provider.load(&GraphSelection::File(path.clone()))?;
    assert_eq!(first.identity, second.identity);
    assert_eq!(first.schema_fingerprint, first.graph.schema_fingerprint());
    assert!(matches!(first.identity, SnapshotIdentity::File { .. }));

    fs::write(
        &path,
        br#"{"directed":true,"multigraph":false,"graph":{},"nodes":[{"id":"b","label":"B"}],"links":[]}"#,
    )?;
    let changed = provider.load(&GraphSelection::File(path))?;
    assert_ne!(first.identity, changed.identity);
    Ok(())
}

#[test]
fn file_snapshot_identity_includes_the_loaded_learning_overlay()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = FileSnapshotFixture::new()?;
    let provider = FileSnapshotProvider;
    let before = provider.load(&GraphSelection::File(fixture.graph_path()))?;
    fixture.write_learning_overlay("node-a", "owner-a")?;
    let after = provider.load(&GraphSelection::File(fixture.graph_path()))?;
    assert_ne!(before.identity, after.identity);
    assert_ne!(before.overlay, after.overlay);
    Ok(())
}

#[test]
fn file_provider_rejects_commit_selection() {
    let error = FileSnapshotProvider
        .load(&GraphSelection::Commit("a".repeat(40)))
        .err()
        .map(|error| error.to_string());
    assert_eq!(
        error.as_deref(),
        Some("graph snapshot selection is unsupported by this adapter: commit")
    );
    let merge_error = FileSnapshotProvider
        .load(&GraphSelection::SyntheticMerge {
            target_head: "a".repeat(40),
            pull_request_head: "b".repeat(40),
            tree: "c".repeat(40),
        })
        .err()
        .map(|error| error.to_string());
    assert_eq!(
        merge_error.as_deref(),
        Some(
            "graph snapshot selection is unsupported by this adapter: synthetic_merge"
        )
    );
}
```

- [ ] **Step 2: Run the tests and verify the shared interface is absent**

Run:

```bash
cargo test -p compass-core --test snapshot
```

Expected: compilation fails because `FileSnapshotProvider`, `GraphSelection`, `SnapshotIdentity`, and `SnapshotProvider` are not exported.

- [ ] **Step 3: Implement the shared snapshot module**

Implement the shared interfaces exactly as declared above. Keep the existing tolerant `load_learning_overlay` unchanged for current query behavior, and add `load_learning_overlay_strict` for snapshot consumers; an absent sidecar means an empty overlay, while an existing unreadable or malformed sidecar is an error. `FileSnapshotProvider::load` performs these actions in order:

1. Reject `GraphSelection::Commit`.
2. Load one owned `GraphDocument` through the existing size, extension, cache-consistency, and JSON guards.
3. Load one owned learning overlay through the strict helper.
4. Recursively sort all JSON object keys in the document and overlay, serialize a `compass.graph_snapshot/1` semantic envelope, and hash those canonical bytes with SHA-256.
5. Build `LoadedGraph::from_document`, install the already loaded overlay, and capture `graph.schema_fingerprint()`.
6. Return one `Arc<GraphSnapshot>` containing exactly the semantic data covered by the digest.

The central implementation has this shape:

```rust
use sha2::Digest as _;

#[derive(Clone, Copy, Debug, Default)]
pub struct FileSnapshotProvider;

impl SnapshotProvider for FileSnapshotProvider {
    fn load(
        &self,
        selection: &GraphSelection,
    ) -> Result<std::sync::Arc<GraphSnapshot>, SnapshotError> {
        let path = match selection {
            GraphSelection::File(path) => path,
            GraphSelection::Commit(_) => {
                return Err(SnapshotError::Unsupported("commit".to_owned()));
            }
            GraphSelection::SyntheticMerge { .. } => {
                return Err(SnapshotError::Unsupported(
                    "synthetic_merge".to_owned(),
                ));
            }
        };
        let document = compass_model::GraphDocument::load(path)
            .map_err(|error| SnapshotError::Corrupt(error.to_string()))?;
        let overlay = crate::load_learning_overlay_strict(path)
            .map_err(|error| SnapshotError::Corrupt(error.to_string()))?;
        let identity_bytes = canonical_snapshot_bytes(&document, &overlay)
            .map_err(|error| SnapshotError::Corrupt(error.to_string()))?;
        let digest = sha2::Sha256::digest(&identity_bytes);
        let mut loaded = crate::LoadedGraph::from_document(document, false)
            .map_err(|error| SnapshotError::Corrupt(error.to_string()))?;
        loaded.overlay = overlay;
        let schema_fingerprint = loaded.graph.schema_fingerprint();
        Ok(std::sync::Arc::new(GraphSnapshot {
            graph: loaded.graph,
            overlay: loaded.overlay,
            schema_fingerprint,
            identity: SnapshotIdentity::File {
                sha256: lower_hex(&digest),
            },
        }))
    }
}
```

`canonical_snapshot_bytes` converts every `serde_json::Value::Object` into lexicographic key order recursively before serialization. `lower_hex` writes exactly two lowercase characters per byte without `unwrap`, `expect`, or `unsafe`. Add a regression test proving two semantically identical input documents with different object-key orders have the same file identity.

Use these helper signatures and canonicalization:

```rust
type LearningOverlay =
    std::collections::HashMap<String, serde_json::Map<String, serde_json::Value>>;

fn load_learning_overlay_strict(
    graph_path: &std::path::Path,
) -> Result<LearningOverlay, SnapshotError>;

fn canonical_snapshot_bytes(
    document: &compass_model::GraphDocument,
    overlay: &LearningOverlay,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut value = serde_json::json!({
        "schema": "compass.graph_snapshot/1",
        "document": document,
        "overlay": overlay,
    });
    sort_json_objects(&mut value);
    serde_json::to_vec(&value)
}

fn sort_json_objects(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(values) => {
            for value in values {
                sort_json_objects(value);
            }
        }
        serde_json::Value::Object(object) => {
            let mut entries = std::mem::take(object)
                .into_iter()
                .collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            for (key, mut value) in entries {
                sort_json_objects(&mut value);
                object.insert(key, value);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}
```

Implement `lower_hex(bytes: &[u8]) -> String` with `std::fmt::Write::write_fmt`; ignoring its infallible `String` formatting result is permitted and avoids forbidden `unwrap`/`expect`.

- [ ] **Step 4: Replace the CLI-private selection enum**

Delete the enum at `crates/compass-cli/src/lib.rs:3370-3374`, import `compass_core::GraphSelection`, and preserve the documented parser behavior. Add unit assertions covering `--graph`, `--at`, duplicate selectors, mixed selectors, and `--`.

The production edit is deliberately limited to the type owner:

```rust
use compass_core::GraphSelection;

// Delete the CLI-local GraphSelection declaration. Existing parser arms
// continue constructing GraphSelection::File and GraphSelection::Commit.
```

Add an exhaustive `SyntheticMerge` arm to the existing `load_selected_graph` match that returns `"synthetic merge snapshots are not accepted by this command"`. No existing CLI parser constructs that variant.

- [ ] **Step 5: Run focused behavior and lint checks**

Run:

```bash
cargo test -p compass-core --test snapshot
cargo test -p compass-cli --test history_cli
cargo test -p compass-cli --test coverage_paths
cargo clippy -p compass-core -p compass-cli --all-targets -- -D warnings
```

Expected: all commands pass; query/path/explain `--at` behavior and existing help remain unchanged.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/compass-core crates/compass-cli/src/lib.rs crates/compass-cli/tests
git commit -m "refactor(core): share immutable graph snapshots"
```

## Task 2: Add the strict versioned PR Intelligence report contract

**Files:**

- Create: `crates/compass-prs/src/model.rs`
- Create: `crates/compass-prs/src/canonical.rs`
- Modify: `crates/compass-prs/src/lib.rs`
- Modify: `crates/compass-prs/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/compass-prs/tests/report_contract.rs`

**Interfaces:**

- Consumes: `compass_core::SnapshotIdentity`, Serde, and SHA-256.
- Produces: every report/revision type in Shared public interfaces plus `canonical_report_bytes`, `evidence_manifest_digest`, `report_id`, `validate_report_id`, `validate_sha256_digest`, and `format_sha256_digest`.

Add `compass-core = { path = "../compass-core", version = "0.1.0" }`, `serde.workspace = true`, `sha2.workspace = true`, and `url.workspace = true` to `compass-prs` dependencies.

- [ ] **Step 1: Write failing schema and canonical identity tests**

Create tests covering strict round-trip, unknown fields, stable ordering, and excluded operational data:

```rust
#[test]
fn identical_foundation_evidence_has_identical_ids_and_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let first = fixture_report(["head", "base", "target"]);
    let second = fixture_report(["target", "head", "base"]);
    assert_eq!(first.report_id, second.report_id);
    assert_eq!(
        compass_prs::canonical_report_bytes(&first)?,
        compass_prs::canonical_report_bytes(&second)?
    );
    assert!(first.findings.is_empty());
    assert!(first.risk.is_none());
    assert!(first.gates.is_empty());
    assert_eq!(first.state, AnalysisState::FoundationPreview);
    Ok(())
}

#[test]
fn report_and_manifest_reject_unknown_fields() {
    let mut report = fixture_report_json();
    report["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<PrIntelligenceReport>(report).is_err());

    let mut manifest = fixture_manifest_json();
    manifest["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<EvidenceManifest>(manifest).is_err());
}

#[test]
fn report_rejects_wrong_schema_digest_identity_and_manifest_copies() {
    let mut wrong_schema = fixture_report_json();
    wrong_schema["schema"] = serde_json::json!("compass.pr_intelligence.report/2");
    assert!(serde_json::from_value::<PrIntelligenceReport>(wrong_schema).is_err());

    let mut wrong_digest = fixture_report_json();
    wrong_digest["evidence_manifest_digest"] = serde_json::json!("c".repeat(64));
    assert!(serde_json::from_value::<PrIntelligenceReport>(wrong_digest).is_err());

    let mut wrong_id = fixture_report_json();
    wrong_id["report_id"] =
        serde_json::json!(format!("cmppr-report-v1:{}", "d".repeat(64)));
    assert!(serde_json::from_value::<PrIntelligenceReport>(wrong_id).is_err());

    let mut inconsistent_copy = fixture_report_json();
    inconsistent_copy["request"]["repository"]["name"] =
        serde_json::json!("different");
    assert!(
        serde_json::from_value::<PrIntelligenceReport>(inconsistent_copy).is_err()
    );
}
```

- [ ] **Step 2: Run and verify the report model is absent**

Run:

```bash
cargo test -p compass-prs --test report_contract
```

Expected: compilation fails because the report types and canonical functions do not exist.

- [ ] **Step 3: Implement strict foundational types**

Implement the shared types plus these exact supporting types:

```rust
#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Architecture,
    Impact,
    Owner,
    Test,
    Overlap,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Exact,
    Strong,
    Moderate,
    Weak,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidencePrecision {
    Exact,
    Inferred,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub fingerprint: String,
    pub kind: FindingKind,
    pub producer_id: String,
    pub producer_version: String,
    pub statement: String,
    pub source_entities: Vec<String>,
    pub target_entities: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub confidence: Confidence,
    pub completeness: Completeness,
    pub freshness: EvidenceFreshness,
    pub remediation: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub source: String,
    pub source_revision: String,
    pub source_digest: String,
    pub precision: EvidencePrecision,
    pub freshness: EvidenceFreshness,
    pub witness: Vec<WitnessHop>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessHop {
    pub source_id: String,
    pub relation: String,
    pub target_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskBand {
    Low,
    Moderate,
    High,
    Critical,
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum RiskFactorKind {
    ContractSeverity,
    Propagation,
    Criticality,
    VerificationGap,
    ConcurrentExposure,
    Uncertainty,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskFactor {
    pub kind: RiskFactorKind,
    pub severity: RiskBand,
    pub statement: String,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskAssessment {
    pub band: RiskBand,
    pub factors: Vec<RiskFactor>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateState {
    Pass,
    Fail,
    Indeterminate,
    Error,
    Pending,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct GateResult {
    pub rule_id: String,
    pub rule_version: String,
    pub state: GateState,
    pub finding_fingerprint: Option<String>,
    pub evidence: Vec<Evidence>,
    pub explanation: String,
    pub remediation: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisFailure {
    pub code: String,
    pub message: String,
    pub source: Option<String>,
}
```

The foundation constructor is the only public constructor in this child:

```rust
impl PrIntelligenceReport {
    pub fn foundation(
        captured: &CapturedChangeRequest,
        manifest: &EvidenceManifest,
    ) -> Result<Self, CanonicalError> {
        let evidence_manifest_digest = evidence_manifest_digest(manifest)?;
        let report_id = report_id(
            &captured.identity,
            &captured.revisions,
            &captured.changed_files_digest,
            &evidence_manifest_digest,
        )?;
        Ok(Self {
            schema: REPORT_SCHEMA.to_owned(),
            report_id,
            state: AnalysisState::FoundationPreview,
            request: captured.identity.clone(),
            revisions: captured.revisions.clone(),
            evidence_manifest: manifest.clone(),
            evidence_manifest_digest,
            snapshots: manifest.snapshots.clone(),
            completeness: Completeness::LocalExact,
            repository_evidence: vec![RepositoryEvidence {
                repository: captured.identity.repository.clone(),
                role: RepositoryRole::Local,
                observed_default_head: Some(
                    captured.revisions.target_head.clone()
                ),
                graph_revision: Some(captured.revisions.target_head.clone()),
                freshness: EvidenceFreshness::Current,
                authorization: AuthorizationOutcome::NotRequired,
                state: RepositoryEvidenceState::Captured,
                failure_code: None,
            }],
            findings: Vec::new(),
            risk: None,
            gates: Vec::new(),
            failures: Vec::new(),
        })
    }
}
```

- [ ] **Step 4: Implement canonical bytes and versioned identities**

Use only structs, vectors, enums, and `BTreeMap` in identity-bearing data. Serialize with `serde_json::to_vec`; do not serialize `HashMap` or arbitrary object-valued `serde_json::Value` into identity.

```rust
pub fn canonical_report_bytes(
    report: &PrIntelligenceReport,
) -> Result<Vec<u8>, CanonicalError> {
    serde_json::to_vec(report).map_err(CanonicalError::Json)
}

pub fn evidence_manifest_digest(
    manifest: &EvidenceManifest,
) -> Result<String, CanonicalError> {
    let bytes = serde_json::to_vec(manifest).map_err(CanonicalError::Json)?;
    Ok(lower_sha256(&bytes))
}

pub fn report_id(
    request: &ChangeRequestIdentity,
    revisions: &RevisionSet,
    changed_files_digest: &str,
    evidence_manifest_digest: &str,
) -> Result<String, CanonicalError> {
    #[derive(serde::Serialize)]
    struct Identity<'a> {
        schema: &'static str,
        request: &'a ChangeRequestIdentity,
        revisions: &'a RevisionSet,
        changed_files_digest: &'a str,
        evidence_manifest_digest: &'a str,
    }
    let bytes = serde_json::to_vec(&Identity {
        schema: REPORT_ID_SCHEMA,
        request,
        revisions,
        changed_files_digest,
        evidence_manifest_digest,
    })
    .map_err(CanonicalError::Json)?;
    Ok(format!("{REPORT_ID_SCHEMA}:{}", lower_sha256(&bytes)))
}

pub fn validate_report_id(value: &str) -> Result<(), CanonicalError> {
    let Some(digest) = value.strip_prefix("cmppr-report-v1:") else {
        return Err(CanonicalError::InvalidReportId(value.to_owned()));
    };
    validate_sha256_digest(digest)
        .map_err(|_| CanonicalError::InvalidReportId(value.to_owned()))
}

#[must_use]
pub fn format_sha256_digest(bytes: &[u8; 32]) -> String {
    lower_hex(bytes)
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(*byte >> 4)]));
        output.push(char::from(HEX[usize::from(*byte & 0x0f)]));
    }
    output
}

fn lower_sha256(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let digest: [u8; 32] = sha2::Sha256::digest(bytes).into();
    format_sha256_digest(&digest)
}

pub fn validate_sha256_digest(value: &str) -> Result<(), CanonicalError> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(CanonicalError::InvalidDigest(value.to_owned()))
    }
}
```

`GitObjectId`, every digest field, and the report ID validate length, lowercase hexadecimal encoding, and prefixes at construction and deserialization. Implement shared validators rather than repeating regular expressions. The `TryFrom` report contract validates embedded-manifest consistency and rejects a valid-looking but incorrect `evidence_manifest_digest` or `report_id`.

- [ ] **Step 5: Run tests, schema snapshot, and Clippy**

Run:

```bash
cargo test -p compass-prs --test report_contract
cargo test -p compass-prs
cargo clippy -p compass-prs --all-targets -- -D warnings
```

Expected: all pass; the checked JSON fixture uses schema `compass.pr_intelligence.report/1`, report IDs use `cmppr-report-v1:`, and the reserved finding fingerprint prefix remains `cmpprv1:`.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/compass-prs
git commit -m "feat(prs): define versioned intelligence reports"
```

## Task 3: Capture exact GitHub pull-request revisions without mutating Git

**Files:**

- Create: `crates/compass-prs/src/source.rs`
- Create: `crates/compass-prs/src/git.rs`
- Modify: `crates/compass-prs/src/lib.rs`
- Modify: `crates/compass-prs/src/model.rs`
- Modify: `crates/compass-prs/Cargo.toml`
- Create: `crates/compass-prs/tests/change_capture.rs`
- Modify: `crates/compass-prs/tests/coverage_paths.rs`

**Interfaces:**

- Consumes: existing bounded `ProcessRunner`, `gh pr view`, and local Git object database.
- Produces: `ChangeRequestSource`, `GitHubCliChangeRequestSource`, `CapturedChangeRequest`, strict Git parsing, and `SourceError` codes `PRS2001`–`PRS2008`.

- [ ] **Step 1: Write failing exact-capture tests**

Use a temporary real Git repository plus a stubbed GitHub response. Cover:

- Full base/head OIDs are captured.
- Merge base differs from an advanced target head.
- Multiple best merge bases fail explicitly instead of choosing one arbitrarily.
- Clean merge produces a tree OID.
- Conflicted merge produces sorted unique paths.
- Missing local head/base objects fail without invoking `git fetch`.
- A local checkout whose GitHub remotes do not match the PR repository fails with `PRS2003`.
- GitHub.com and GitHub Enterprise Server URLs produce distinct canonical repository identities.
- Repository identity decoding rejects invalid or unknown fields and normalizes host, owner, and repository case.
- Changed files distinguish add/modify/delete/rename.
- Abbreviated, uppercase, malformed, or invalid UTF-8 evidence fails.

The main acceptance assertion is:

```rust
#[test]
fn capture_freezes_four_revision_identities_and_changed_files()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = GitFixture::diverged_clean_merge()?;
    let runner = fixture.runner();
    let source = GitHubCliChangeRequestSource::new(
        runner,
        fixture.repository_root().to_path_buf(),
    );
    let captured = source.capture(&ChangeRequestSelector {
        number: 42,
        repository: Some(RepositoryIdentity::github(
            "github.com",
            "acme",
            "widgets",
        )?),
    })?;
    assert_eq!(captured.identity.repository.slug(), "acme/widgets");
    assert_eq!(
        captured.revisions.merge_base,
        GitObjectId::parse(fixture.merge_base())?
    );
    assert_eq!(
        captured.revisions.pull_request_head,
        GitObjectId::parse(fixture.head())?
    );
    assert_eq!(
        captured.revisions.target_head,
        GitObjectId::parse(fixture.target())?
    );
    assert!(matches!(
        captured.revisions.merge_outcome,
        MergeOutcome::Clean { .. }
    ));
    assert_eq!(
        captured.changed_files,
        vec![ChangedFile {
            kind: FileChangeKind::Modified,
            old_path: Some("src/lib.rs".to_owned()),
            new_path: Some("src/lib.rs".to_owned()),
        }]
    );
    assert!(!runner.saw_arguments(&["fetch"]));
    Ok(())
}
```

- [ ] **Step 2: Run and verify exact capture is absent**

Run:

```bash
cargo test -p compass-prs --test change_capture
```

Expected: compilation fails because `GitHubCliChangeRequestSource` and exact capture types do not exist.

- [ ] **Step 3: Make bounded process output strict UTF-8**

Replace `String::from_utf8_lossy` in `run_bounded` with strict conversion. Extend `PrsError`:

```rust
#[error("{program} returned non-UTF-8 {stream}")]
InvalidUtf8 {
    program: String,
    stream: &'static str,
}
```

Use:

```rust
let stdout = String::from_utf8(stdout).map_err(|_| PrsError::InvalidUtf8 {
    program: program.to_owned(),
    stream: "stdout",
})?;
let stderr = String::from_utf8(stderr).map_err(|_| PrsError::InvalidUtf8 {
    program: program.to_owned(),
    stream: "stderr",
})?;
```

Update existing tests to prove Unicode PR titles still pass and invalid byte sequences fail instead of being replaced.

- [ ] **Step 4: Implement exact local Git operations**

`git.rs` exposes:

```rust
pub(crate) fn verify_commit(
    runner: &dyn ProcessRunner,
    root: &std::path::Path,
    object: &GitObjectId,
) -> Result<(), SourceError>;

pub(crate) fn merge_base(
    runner: &dyn ProcessRunner,
    root: &std::path::Path,
    target: &GitObjectId,
    head: &GitObjectId,
) -> Result<GitObjectId, SourceError>;

pub(crate) fn merge_outcome(
    runner: &dyn ProcessRunner,
    root: &std::path::Path,
    target: &GitObjectId,
    head: &GitObjectId,
) -> Result<MergeOutcome, SourceError>;

pub(crate) fn changed_files(
    runner: &dyn ProcessRunner,
    root: &std::path::Path,
    base: &GitObjectId,
    head: &GitObjectId,
) -> Result<Vec<ChangedFile>, SourceError>;
```

Every command uses `git -C ROOT`, `--end-of-options` where supported, and exact OIDs. `verify_commit` runs `git cat-file -e <oid>^{commit}`. `merge_base` runs `git merge-base --all TARGET HEAD` and requires exactly one full OID; zero or multiple best bases return `PRS2005` instead of selecting ambiguous intent. `merge_outcome` runs `git merge-tree --write-tree -z --name-only TARGET HEAD` without `--messages`; exit `0` requires exactly one NUL-delimited tree-OID record, while exit `1` parses that tree OID followed only by conflicted path records. The conflicted form retains both the tree OID and a SHA-256 digest of the canonical sorted paths, so the fourth revision always has an immutable identity. Other exits are errors. Detect an unsupported option or subcommand and return `PRS2007` with the required command and installed `git --version`; do not parse localized human prose as conflict evidence.

`changed_files` runs:

```text
git -C ROOT diff --name-status -z --find-renames --find-copies BASE HEAD --
```

The parser consumes NUL-separated fields and rejects truncated records, unsupported status letters, path traversal, absolute paths, embedded NUL, and empty paths. Sort by `(new_path, old_path, kind)` before hashing.

Map status records exactly: `A` to `(None, Some(new))`, `D` to `(Some(old), None)`, `M`/`T` to the same path in both fields, and `R0..R100`/`C0..C100` to separate old/new paths. Reject unmerged `U`, combined-diff records, missing or out-of-range similarity scores, and any unexpected extra field.

Hash changed files as the canonical JSON encoding of:

```rust
#[derive(serde::Serialize)]
struct ChangedFilesIdentity<'a> {
    schema: &'static str, // "compass.pr_intelligence.changed_files/1"
    files: &'a [ChangedFile],
}
```

The digest is exactly 64 lowercase hexadecimal characters and covers the sorted, normalized records.

- [ ] **Step 5: Implement the GitHub CLI source adapter**

The adapter runs exactly one metadata request:

```text
gh pr view NUMBER --json number,baseRefName,headRefName,baseRefOid,headRefOid,url
```

Append `--repo REPOSITORY_IDENTITY.gh_repo_argument()` only when selected. Derive the canonical repository from the PR URL and require it to equal the explicit selector when present.

The adapter then:

1. Parses full target/head OIDs.
2. Verifies both objects exist locally.
3. Computes merge base.
4. Computes changed files from merge base to head.
5. Computes the synthetic merge tree from target head and PR head.
6. Hashes canonical changed-file bytes.
7. Returns one immutable `CapturedChangeRequest`.

Use this exact response boundary and adapter ownership:

```rust
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GitHubPullRequest {
    number: u64,
    base_ref_name: String,
    head_ref_name: String,
    base_ref_oid: String,
    head_ref_oid: String,
    url: String,
}

pub struct GitHubCliChangeRequestSource {
    runner: std::sync::Arc<dyn ProcessRunner + Send + Sync>,
    repository_root: std::path::PathBuf,
}

impl GitHubCliChangeRequestSource {
    pub fn new(
        runner: std::sync::Arc<dyn ProcessRunner + Send + Sync>,
        repository_root: std::path::PathBuf,
    ) -> Self {
        Self {
            runner,
            repository_root,
        }
    }
}
```

Private helper contracts are:

```rust
fn fetch_metadata(
    runner: &(dyn ProcessRunner + Send + Sync),
    selector: &ChangeRequestSelector,
) -> Result<GitHubPullRequest, SourceError>;

fn repository_from_pull_url(
    url: &str,
) -> Result<(RepositoryIdentity, u64), SourceError>;

fn require_selected_repository(
    selected: Option<&RepositoryIdentity>,
    actual: &RepositoryIdentity,
) -> Result<(), SourceError>;

fn require_local_repository(
    runner: &(dyn ProcessRunner + Send + Sync),
    repository_root: &std::path::Path,
    actual: &RepositoryIdentity,
) -> Result<(), SourceError>;

fn changed_files_digest(files: &[ChangedFile]) -> Result<String, SourceError>;
```

Before this body continues, `fetch_metadata` requires `metadata.number == selector.number`, and `repository_from_pull_url` returns both repository and URL PR number so the URL must agree as well. The implementation body is:

```rust
fn capture(
    &self,
    selector: &ChangeRequestSelector,
) -> Result<CapturedChangeRequest, SourceError> {
    let metadata = fetch_metadata(self.runner.as_ref(), selector)?;
    let (repository, url_number) = repository_from_pull_url(&metadata.url)?;
    if metadata.number != selector.number || url_number != selector.number {
        return Err(SourceError::MalformedGitHubMetadata {
            message: "selected, returned, and URL PR numbers differ".to_owned(),
        });
    }
    require_selected_repository(selector.repository.as_ref(), &repository)?;
    require_local_repository(
        self.runner.as_ref(),
        &self.repository_root,
        &repository,
    )?;
    let target = GitObjectId::parse(&metadata.base_ref_oid).map_err(|error| {
        SourceError::MalformedGitHubMetadata {
            message: error.to_string(),
        }
    })?;
    let head = GitObjectId::parse(&metadata.head_ref_oid).map_err(|error| {
        SourceError::MalformedGitHubMetadata {
            message: error.to_string(),
        }
    })?;
    verify_commit(self.runner.as_ref(), &self.repository_root, &target)?;
    verify_commit(self.runner.as_ref(), &self.repository_root, &head)?;
    let base = merge_base(
        self.runner.as_ref(),
        &self.repository_root,
        &target,
        &head,
    )?;
    let files = changed_files(
        self.runner.as_ref(),
        &self.repository_root,
        &base,
        &head,
    )?;
    let changed_files_digest = changed_files_digest(&files)?;
    let merge_outcome = merge_outcome(
        self.runner.as_ref(),
        &self.repository_root,
        &target,
        &head,
    )?;
    Ok(CapturedChangeRequest {
        identity: ChangeRequestIdentity {
            repository,
            number: metadata.number,
        },
        revisions: RevisionSet {
            merge_base: base,
            pull_request_head: head,
            target_head: target,
            merge_outcome,
        },
        base_branch: metadata.base_ref_name,
        head_branch: metadata.head_ref_name,
        changed_files: files,
        changed_files_digest,
    })
}
```

`repository_from_pull_url` uses workspace `url::Url`, accepts only `https`, no credentials/query/fragment, and exactly the four path components `<owner>/<repository>/pull/<number>`. It canonicalizes the URL hostname and optional port through `RepositoryIdentity::github`. The adapter supports GitHub.com and configured GitHub Enterprise Server hosts. It compares the canonical host/owner/name identity to an explicit selector.

`require_local_repository` runs `git -C ROOT remote`, then
`git -C ROOT remote get-url --all REMOTE` for each returned name. It parses GitHub HTTPS and SCP-like SSH URLs without invoking a shell, removes one terminal `.git` suffix from the repository path, and requires at least one remote to match the canonical PR hostname, owner, and repository identity. Missing, malformed-only, or nonmatching remotes return `PRS2003`; the diagnostic lists remote names but never credentials or full remote URLs.

Map errors to stable codes:

```text
PRS2001 GitHub metadata unavailable
PRS2002 malformed GitHub metadata
PRS2003 repository identity mismatch
PRS2004 required Git object missing locally
PRS2005 Git revision resolution failed
PRS2006 changed-file evidence invalid
PRS2007 synthetic merge evaluation failed
PRS2008 invalid UTF-8 evidence
```

- [ ] **Step 6: Run focused capture, PR command, and lint tests**

Run:

```bash
cargo test -p compass-prs --test change_capture
cargo test -p compass-prs --test coverage_paths
cargo test -p compass-cli --test prs_cli
cargo clippy -p compass-prs --all-targets -- -D warnings
```

Expected: all pass; no test observes an implicit fetch, checkout, ref update, or hook.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/compass-prs
git commit -m "feat(prs): capture exact pull request revisions"
```

## Task 4: Build one immutable evidence manifest and foundation operation

**Files:**

- Create: `crates/compass-prs/src/operation.rs`
- Modify: `crates/compass-prs/src/model.rs`
- Modify: `crates/compass-prs/src/canonical.rs`
- Modify: `crates/compass-prs/src/lib.rs`
- Create: `crates/compass-prs/tests/foundation_operation.rs`

**Interfaces:**

- Consumes: `ChangeRequestSource`, `SnapshotProvider`, the strict report model, and `ReportRepository`.
- Produces: `FoundationOperation`, `OperationError`, `MemoryReportRepository`, and immutable evidence/report construction.

- [ ] **Step 1: Write failing orchestration tests with real seams**

Create one fake source, one fake snapshot provider, and an in-memory report repository. Assert:

- Capture occurs once.
- Each of merge base, PR head, and target head loads once.
- All loads use full commit OIDs from the captured request.
- Snapshot identities must contain the requested commit.
- A changed source response after capture cannot affect the in-flight report.
- Any snapshot failure returns an operation error and persists nothing.
- Success persists exactly one foundation report with no verdict fields populated.

```rust
#[test]
fn operation_captures_once_loads_exact_snapshots_and_persists_no_verdict()
-> Result<(), Box<dyn std::error::Error>> {
    let source = Arc::new(FakeSource::new(captured_fixture()));
    let snapshots = Arc::new(FakeSnapshots::for_captured(source.current()));
    let reports = Arc::new(MemoryReportRepository::default());
    let operation = FoundationOperation::new(
        source.clone(),
        snapshots.clone(),
        reports.clone(),
        "0.1.0".to_owned(),
        "a".repeat(64),
    )?;
    let report = operation.analyze(&ChangeRequestSelector {
        number: 42,
        repository: Some(RepositoryIdentity::github(
            "github.com",
            "acme",
            "widgets",
        )?),
    })?;
    assert_eq!(source.capture_count(), 1);
    assert_eq!(
        snapshots.selections(),
        vec![
            GraphSelection::Commit(source.current().revisions.merge_base.to_string()),
            GraphSelection::Commit(
                source.current().revisions.pull_request_head.to_string()
            ),
            GraphSelection::Commit(source.current().revisions.target_head.to_string()),
        ]
    );
    assert_eq!(report.state, AnalysisState::FoundationPreview);
    assert!(report.findings.is_empty());
    assert!(report.risk.is_none());
    assert!(report.gates.is_empty());
    assert_eq!(reports.saved_ids()?, vec![report.report_id.clone()]);
    Ok(())
}
```

- [ ] **Step 2: Run and verify the operation is absent**

Run:

```bash
cargo test -p compass-prs --test foundation_operation
```

Expected: compilation fails because `FoundationOperation` and `MemoryReportRepository` do not exist.

- [ ] **Step 3: Define the report repository seam and memory adapter**

Add `ReportRepository`, `StoredReport`, and `ReportStoreError` to `operation.rs` or `report_store.rs`; export them from `lib.rs`. The in-memory adapter stores canonical bytes in a `Mutex<BTreeMap<String, Vec<u8>>>`, rejects duplicate IDs with different bytes, and deserializes through the strict report schema.

The duplicate rule is:

```rust
match reports.get(&report.report_id) {
    Some(existing) if existing == &bytes => Ok(existing result),
    Some(_) => Err(ReportStoreError::IdentityCollision(report.report_id.clone())),
    None => insert and return,
}
```

Implement the in-memory adapter with this storage boundary:

```rust
#[derive(Default)]
pub struct MemoryReportRepository {
    reports: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl MemoryReportRepository {
    pub fn saved_ids(&self) -> Result<Vec<String>, ReportStoreError> {
        self.reports
            .lock()
            .map(|reports| reports.keys().cloned().collect())
            .map_err(|_| ReportStoreError::State("report lock poisoned".to_owned()))
    }
}

impl ReportRepository for MemoryReportRepository {
    fn save(
        &self,
        report: &PrIntelligenceReport,
    ) -> Result<StoredReport, ReportStoreError> {
        let bytes = canonical_report_bytes(report)?;
        let mut reports = self.reports.lock().map_err(|_| {
            ReportStoreError::State("report lock poisoned".to_owned())
        })?;
        match reports.get(&report.report_id) {
            Some(existing) if existing == &bytes => {}
            Some(_) => {
                return Err(ReportStoreError::IdentityCollision(
                    report.report_id.clone(),
                ));
            }
            None => {
                reports.insert(report.report_id.clone(), bytes);
            }
        }
        Ok(StoredReport {
            report_id: report.report_id.clone(),
            path: None,
        })
    }

    fn load(
        &self,
        report_id: &str,
    ) -> Result<PrIntelligenceReport, ReportStoreError> {
        crate::validate_report_id(report_id)?;
        let reports = self.reports.lock().map_err(|_| {
            ReportStoreError::State("report lock poisoned".to_owned())
        })?;
        let bytes = reports
            .get(report_id)
            .ok_or_else(|| ReportStoreError::NotFound(report_id.to_owned()))?;
        serde_json::from_slice(bytes)
            .map_err(CanonicalError::Json)
            .map_err(ReportStoreError::Contract)
    }
}
```

- [ ] **Step 4: Implement foundation orchestration**

The operation:

1. Calls `source.capture` exactly once.
2. Constructs three `GraphSelection::Commit` values from the captured OIDs.
3. Loads snapshots in deterministic role order.
4. Verifies every `SnapshotIdentity::Commit.commit` equals its requested OID.
5. Creates an `EvidenceManifest` with an ordered `BTreeMap<SnapshotRole, SnapshotEvidence>` containing each validated identity and `schema_fingerprint.to_hex()`.
6. Creates `PrIntelligenceReport::foundation`.
7. Persists the report.
8. Returns the same report object.

The core helper is:

```rust
fn require_commit_identity(
    role: SnapshotRole,
    expected: &GitObjectId,
    snapshot: &compass_core::GraphSnapshot,
) -> Result<(), OperationError> {
    match &snapshot.identity {
        compass_core::SnapshotIdentity::Commit { commit, .. }
            if commit == expected.as_str() =>
        {
            Ok(())
        }
        actual => Err(OperationError::SnapshotIdentity {
            role,
            expected: expected.to_string(),
            actual: actual.to_string(),
        }),
    }
}
```

Do not load a merge-result graph in this child. Preserve the exact synthetic tree in `RevisionSet`, keep `state: foundation_preview`, and emit no risk or gate.

`FoundationOperation::new` validates `extractor_version` as nonempty UTF-8 without control characters and `extractor_config_digest` as 64 lowercase hexadecimal characters. `analyze` inserts roles in the fixed order `MergeBase`, `PullRequestHead`, `TargetHead`, creates `SnapshotEvidence { identity, schema_fingerprint: snapshot.schema_fingerprint.to_hex() }`, constructs the manifest with `policy_pack_digest: None`, and calls the repository only after all validations succeed.

- [ ] **Step 5: Run orchestration, model, and lint checks**

Run:

```bash
cargo test -p compass-prs --test foundation_operation
cargo test -p compass-prs --test report_contract
cargo clippy -p compass-prs --all-targets -- -D warnings
```

Expected: all pass; failure tests prove no partial report is persisted.

- [ ] **Step 6: Commit**

```bash
git add crates/compass-prs
git commit -m "feat(prs): orchestrate foundation evidence"
```

## Task 5: Persist canonical reports safely below the Git common directory

**Files:**

- Create: `crates/compass-prs/src/report_store.rs`
- Modify: `crates/compass-prs/src/lib.rs`
- Modify: `crates/compass-prs/src/operation.rs`
- Modify: `crates/compass-prs/Cargo.toml`
- Create: `crates/compass-prs/tests/report_store.rs`
- Modify: `crates/compass-prs/Cargo.toml` to move workspace `tempfile` from dev-only to production dependencies

**Interfaces:**

- Consumes: `ReportRepository`, `canonical_report_bytes`, and an explicit canonical Git common directory.
- Produces: `FilesystemReportRepository::{new, save, load, report_path}`.

- [ ] **Step 1: Write failing persistence and hostile-path tests**

Cover:

- Save/load round trip.
- Idempotent save of identical bytes.
- Collision rejection for same ID with different bytes.
- Owner-only directory/file mode on Unix.
- Symlink rejection at `compass`, `pr-intelligence`, `reports`, destination, and temporary path.
- Truncated, oversized, corrupt, and unknown-schema reports.
- An injected failure before no-clobber persistence leaves no visible report; retry creates one complete report.
- Paths never escape the canonical common directory.

```rust
#[test]
fn report_store_is_idempotent_atomic_and_confined()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let common = directory.path().canonicalize()?;
    let store = FilesystemReportRepository::new(&common)?;
    let report = fixture_report();
    let first = store.save(&report)?;
    let second = store.save(&report)?;
    assert_eq!(first.report_id, second.report_id);
    assert_eq!(first.path, second.path);
    assert_eq!(store.load(&report.report_id)?, report);
    let path = first.path.ok_or("missing persisted path")?;
    assert!(path.starts_with(common.join("compass/pr-intelligence/reports")));
    Ok(())
}
```

- [ ] **Step 2: Run and verify filesystem persistence is absent**

Run:

```bash
cargo test -p compass-prs --test report_store
```

Expected: compilation fails because `FilesystemReportRepository` does not exist.

- [ ] **Step 3: Implement confined owner-only storage**

Store each report at
`<git-common-dir>/compass/pr-intelligence/reports/<cmppr-report-v1-64hex>.json`.

Replace the colon in the filename with a hyphen. The JSON body retains the canonical `cmppr-report-v1:` report ID.

`new` requires the supplied Git common directory to exist, be a directory, be nonsymlinked, and canonicalize to itself. Create descendants one segment at a time, checking each with `symlink_metadata`.

On Unix, create directories with `0700` and files with `0600`. On every platform:

1. Reject symlink destinations.
2. Serialize before opening the filesystem.
3. Reject bytes above `64 * 1024 * 1024`.
4. Write to a `tempfile::NamedTempFile` created inside the validated reports directory.
5. Flush and `sync_all`.
6. Call `persist_noclobber` within the same directory so concurrent writers cannot replace an existing immutable report.
7. Sync the parent directory where supported.
8. Remove only the exact validated temporary file on failure.

Do not use `compass_files::write_bytes_atomic` here because that helper resolves destination symlinks.

The production store has this shape:

```rust
const MAX_REPORT_BYTES: u64 = 64 * 1024 * 1024;

pub struct FilesystemReportRepository {
    common_dir: std::path::PathBuf,
    reports_dir: std::path::PathBuf,
}

impl FilesystemReportRepository {
    pub fn new(common_dir: &std::path::Path) -> Result<Self, ReportStoreError> {
        reject_existing_symlink(common_dir)?;
        let canonical = common_dir.canonicalize().map_err(|source| {
            report_io(common_dir, source)
        })?;
        if canonical != common_dir {
            return Err(ReportStoreError::UnsafePath {
                path: common_dir.to_path_buf(),
                reason: "Git common directory must be canonical".to_owned(),
            });
        }
        let compass = canonical.join("compass");
        let intelligence = compass.join("pr-intelligence");
        let reports = intelligence.join("reports");
        for path in [&compass, &intelligence, &reports] {
            create_owner_directory(path)?;
        }
        Ok(Self {
            common_dir: canonical,
            reports_dir: reports,
        })
    }

    pub fn report_path(
        &self,
        report_id: &str,
    ) -> Result<std::path::PathBuf, ReportStoreError> {
        crate::validate_report_id(report_id)?;
        let file_name = format!("{}.json", report_id.replace(':', "-"));
        let path = self.reports_dir.join(file_name);
        if !path.starts_with(&self.common_dir) {
            return Err(ReportStoreError::UnsafePath {
                path,
                reason: "report path escaped Git common directory".to_owned(),
            });
        }
        Ok(path)
    }
}
```

Define `reject_existing_symlink`, `create_owner_directory`, `set_owner_file`, `report_io`, and `sync_directory` as private functions in this module. Reuse `crate::validate_report_id` from Task 2. Each filesystem helper accepts the exact path it checks and returns `ReportStoreError`; none resolves or follows a report destination symlink.

- [ ] **Step 4: Implement strict load and collision handling**

`load` validates the report ID before constructing a path, rejects files above 64 MiB, deserializes with `deny_unknown_fields`, reserializes canonically, recomputes `report_id`, and rejects any mismatch.

If `save` finds an existing destination, load and compare canonical bytes. Return success for identical bytes and `IdentityCollision` otherwise. If `persist_noclobber` loses a concurrent race, perform the same comparison against the winner; never replace it.

```rust
fn compare_existing(
    store: &FilesystemReportRepository,
    report: &PrIntelligenceReport,
    expected: &[u8],
) -> Result<StoredReport, ReportStoreError> {
    let actual = store.load(&report.report_id)?;
    let actual_bytes = canonical_report_bytes(&actual)?;
    if actual_bytes == expected {
        Ok(StoredReport {
            report_id: report.report_id.clone(),
            path: Some(store.report_path(&report.report_id)?),
        })
    } else {
        Err(ReportStoreError::IdentityCollision(
            report.report_id.clone(),
        ))
    }
}
```

The no-clobber write path is:

```rust
let bytes = canonical_report_bytes(report)?;
let byte_count = u64::try_from(bytes.len()).map_err(|_| {
    ReportStoreError::TooLarge { bytes: u64::MAX }
})?;
if byte_count > MAX_REPORT_BYTES {
    return Err(ReportStoreError::TooLarge {
        bytes: byte_count,
    });
}
let destination = self.report_path(&report.report_id)?;
if destination.exists() {
    return compare_existing(self, report, &bytes);
}
let mut temporary = tempfile::NamedTempFile::new_in(&self.reports_dir)
    .map_err(|source| report_io(&self.reports_dir, source))?;
set_owner_file(temporary.path())?;
std::io::Write::write_all(&mut temporary, &bytes)
    .map_err(|source| report_io(temporary.path(), source))?;
temporary
    .as_file()
    .sync_all()
    .map_err(|source| report_io(temporary.path(), source))?;
match temporary.persist_noclobber(&destination) {
    Ok(_) => {
        sync_directory(&self.reports_dir)?;
        Ok(StoredReport {
            report_id: report.report_id.clone(),
            path: Some(destination),
        })
    }
    Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
        compare_existing(self, report, &bytes)
    }
    Err(error) => Err(report_io(&destination, error.error)),
}
```

- [ ] **Step 5: Run persistence, Miri, and lint checks**

Run:

```bash
cargo test -p compass-prs --test report_store
cargo test -p compass-prs
cargo clippy -p compass-prs --all-targets -- -D warnings
```

On a toolchain with Miri installed, additionally run:

```bash
cargo miri test -p compass-prs --test report_store
```

Expected: all available commands pass; skipping unavailable Miri is recorded in the task handoff rather than treated as a test failure.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/compass-prs
git commit -m "feat(prs): persist canonical intelligence reports"
```

## Task 6: Add the production history snapshot adapter and preview

**Files:**

- Create: `crates/compass-cli/src/snapshot_provider.rs`
- Modify: `crates/compass-cli/src/history_commands.rs:63-83`
- Modify: `crates/compass-cli/src/lib.rs`
- Modify: `crates/compass-cli/src/prs_commands.rs`
- Modify: `crates/compass-cli/Cargo.toml`
- Create: `crates/compass-cli/tests/prs_intelligence_preview.rs`
- Modify: `crates/compass-cli/tests/prs_cli.rs`

**Interfaces:**

- Consumes: `GraphSelection`, `SnapshotProvider`, `HistoryStore`, `FoundationOperation`, `GitHubCliChangeRequestSource`, and `FilesystemReportRepository`.
- Produces: `CliSnapshotProvider` and `compass prs NUMBER --intelligence-preview --format json`.

- [ ] **Step 1: Write failing end-to-end preview tests**

Create a temporary real Git repository with diverged target/head commits and a fake `gh` executable that returns their full OIDs. Assert:

- The Compass command succeeds and emits one canonical report object.
- Report state is `foundation_preview`.
- The three snapshot commit identities match merge base/head/target.
- Synthetic merge tree is present in revisions.
- Findings, risk, and gates contain no verdict.
- The report is persisted below the Git common directory.
- Preview requires exactly one PR number and `--format json`.
- Existing Compass PR commands continue to pass their behavior tests.

```rust
#[test]
fn compass_preview_emits_exact_foundation_report_without_a_verdict()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = PreviewFixture::new()?;
    let output = fixture.run_compass(&[
        "prs",
        &fixture.pr_number().to_string(),
        "--intelligence-preview",
        "--format",
        "json",
    ])?;
    assert_eq!(output.status.code(), Some(0));
    let report: compass_prs::PrIntelligenceReport =
        serde_json::from_slice(&output.stdout)?;
    assert_eq!(report.state, compass_prs::AnalysisState::FoundationPreview);
    assert!(report.findings.is_empty());
    assert!(report.risk.is_none());
    assert!(report.gates.is_empty());
    assert!(fixture.persisted_report(&report.report_id).is_file());
    Ok(())
}
```

- [ ] **Step 2: Run and verify production wiring is absent**

Run:

```bash
cargo test -p compass-cli --test prs_intelligence_preview
```

Expected: compilation or assertion failure because the preview flag and production snapshot provider do not exist.

- [ ] **Step 3: Implement `CliSnapshotProvider`**

`File` delegates to `FileSnapshotProvider`. `Commit`:

1. Discovers the current `Repository`.
2. Resolves the revision to full `CommitId`.
3. Uses the existing configured history build options.
4. Resolves or materializes one complete preferred realization.
5. Holds one history activity guard through validation and reconstruction.
6. Builds `LoadedGraph::from_document`.
7. Returns `SnapshotIdentity::Commit { commit, realization }`.

Refactor `history_commands::load_graph_at` to call the same internal loader used by `CliSnapshotProvider`; query/path/explain and PR Intelligence must not maintain two materialization paths.

The returned identity uses `preferred.id.as_hex()` and `preferred.version.git_commit`. It never uses the caller's unresolved revision string.

Use one provider instance bound to one discovered repository and build profile:

```rust
pub(crate) struct CliSnapshotProvider {
    repository: compass_history::Repository,
    options: crate::history_build::HistoryBuildOptions,
    files: compass_core::FileSnapshotProvider,
}

impl CliSnapshotProvider {
    pub(crate) fn new(
        repository: compass_history::Repository,
        options: crate::history_build::HistoryBuildOptions,
    ) -> Self {
        Self {
            repository,
            options,
            files: compass_core::FileSnapshotProvider,
        }
    }
}

impl compass_core::SnapshotProvider for CliSnapshotProvider {
    fn load(
        &self,
        selection: &compass_core::GraphSelection,
    ) -> Result<std::sync::Arc<compass_core::GraphSnapshot>, compass_core::SnapshotError> {
        match selection {
            compass_core::GraphSelection::File(_) => self.files.load(selection),
            compass_core::GraphSelection::Commit(revision) => {
                crate::history_commands::load_snapshot_at(
                    &self.repository,
                    revision,
                    &self.options,
                    false,
                )
                .map(std::sync::Arc::new)
            }
            compass_core::GraphSelection::SyntheticMerge { .. } => {
                Err(compass_core::SnapshotError::Unsupported(
                    "synthetic_merge".to_owned(),
                ))
            }
        }
    }
}
```

`load_snapshot_at` resolves `revision` to `CommitId`, calls the existing `resolve_or_materialize`, opens one activity guard, validates and reconstructs the preferred realization under that guard, computes the schema fingerprint, and returns an `Arc<GraphSnapshot>` with an empty historical overlay and the full commit/realization identity.

Its shared internal signature is:

```rust
pub(crate) fn load_snapshot_at(
    repository: &compass_history::Repository,
    revision: &str,
    options: &crate::history_build::HistoryBuildOptions,
    force_directed: bool,
) -> Result<compass_core::GraphSnapshot, compass_core::SnapshotError>;
```

`load_graph_at` passes through its existing `force_directed` value, then moves the owned `graph`/`overlay` data into its existing `LoadedGraph` return without re-resolving or re-materializing the revision.

- [ ] **Step 4: Wire the explicit preview path**

Leave the existing `Arguments` parser unchanged. Add a separate strict parser activated only when `--intelligence-preview` is present:

```rust
struct PreviewArguments {
    number: u64,
    repository: Option<compass_prs::RepositoryIdentity>,
}

fn parse_preview_arguments(args: &[String]) -> Result<PreviewArguments, String>;
```

Accept only:

```text
compass prs NUMBER [--repo [HOST/]OWNER/REPO] --intelligence-preview --format json
```

The parser accepts `-R` and `--repo=[HOST/]OWNER/REPO` equivalents. It defaults a two-component repository to `github.com` and preserves a three-component enterprise hostname. Reject missing or multiple numbers, duplicate repository/format/preview flags, non-JSON formats, unknown arguments, `--triage`, `--worktrees`, `--conflicts`, `--wrong-base`, `--base`, and `--graph`. The preview always uses exact history snapshots. The strict preview parser is independent from other Compass PR commands.

Construct:

- `GitHubCliChangeRequestSource<SystemRunner>`
- `CliSnapshotProvider`
- `FilesystemReportRepository` rooted at `Repository::common_dir()`
- `FoundationOperation`

Use `env!("CARGO_PKG_VERSION")` for extractor version and the exact configured history `BuildProfile` digest for `extractor_config_digest`.

Serialize with `canonical_report_bytes`, convert the guaranteed JSON UTF-8 with strict `String::from_utf8`, and let `Outcome.stdout_trailing_newline` append one CLI newline. Keep progress on stderr. Operation errors exit `1`; parser errors exit `2`.

The preview dispatch remains an adapter:

```rust
let preview = parse_preview_arguments(args)?;
let current = std::env::current_dir().map_err(|error| error.to_string())?;
let repository = compass_history::Repository::discover(&current)
    .map_err(|error| error.to_string())?;
let options = configured_build_options(&repository)?;
let extractor_config_digest = compass_prs::format_sha256_digest(
    &options.profile().digest().map_err(|error| error.to_string())?,
);
let source = std::sync::Arc::new(
    compass_prs::GitHubCliChangeRequestSource::new(
        std::sync::Arc::new(compass_prs::SystemRunner),
        repository.root().to_path_buf(),
    ),
);
let snapshots = std::sync::Arc::new(CliSnapshotProvider::new(
    repository.clone(),
    options,
));
let reports = std::sync::Arc::new(
    compass_prs::FilesystemReportRepository::new(repository.common_dir())?,
);
let operation = compass_prs::FoundationOperation::new(
    source,
    snapshots,
    reports,
    env!("CARGO_PKG_VERSION").to_owned(),
    extractor_config_digest,
).map_err(|error| error.to_string())?;
let report = operation.analyze(&compass_prs::ChangeRequestSelector {
    number: preview.number,
    repository: preview.repository.clone(),
}).map_err(|error| error.to_string())?;
let output = compass_prs::canonical_report_bytes(&report)
    .map_err(|error| error.to_string())?;
let stdout = String::from_utf8(output).map_err(|error| error.to_string())?;
```

Convert each `?` through the existing `Outcome` boundary so usage errors remain exit `2` and operational errors exit `1`. Return `stdout` with `stdout_trailing_newline: true`; never use a lossy conversion.

- [ ] **Step 5: Run preview, history, PR command, and lint checks**

Run:

```bash
cargo test -p compass-cli --test prs_intelligence_preview
cargo test -p compass-cli --test prs_cli
cargo test -p compass-cli --test history_cli
cargo test -p compass-prs
cargo clippy -p compass-cli -p compass-prs -p compass-core --all-targets -- -D warnings
```

Expected: all pass; existing Compass PR commands keep their documented behavior.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock crates/compass-cli crates/compass-core crates/compass-prs
git commit -m "feat(cli): expose PR foundation preview"
```

## Task 7: Expose the typed foundation through MCP

**Files:**

- Create: `crates/compass-prs/src/service.rs`
- Modify: `crates/compass-prs/src/lib.rs`
- Create: `crates/compass-prs/tests/service.rs`
- Create: `crates/compass-mcp/src/pr_intelligence.rs`
- Modify: `crates/compass-mcp/src/lib.rs`
- Create: `crates/compass-mcp/tests/pr_intelligence.rs`
- Modify: `crates/compass-cli/src/lib.rs`

**Interfaces:**

- Consumes: `FoundationOperation`, `ChangeRequestSelector`, and `PrIntelligenceReport`.
- Produces: object-safe `PrIntelligenceAnalyzer`, typed MCP `analyze_pr`, and CLI injection of the repository-bound analyzer.

- [ ] **Step 1: Write failing service and MCP tests**

Cover:

- The service calls the shared foundation operation once.
- MCP success returns the exact canonical report as structured content.
- MCP text states that the result is a foundation preview and contains no verdict.
- MCP errors include a stable code and set the protocol error flag.
- GitHub.com and enterprise repository identities round-trip through MCP input.
- The adapter never opens a graph, invokes GitHub, computes a finding, or persists a second report.

```rust
#[test]
fn analyze_pr_returns_the_exact_typed_report()
-> Result<(), Box<dyn std::error::Error>> {
    let expected = fixture_report(["base", "head", "target"]);
    let analyzer = Arc::new(FakeAnalyzer::returning(expected.clone()));
    let server = PrIntelligenceMcp::new(analyzer.clone());
    let result = server.analyze_pr(AnalyzePrInput {
        number: 42,
        repository: Some(RepositoryIdentity::github(
            "github.example.com",
            "acme",
            "widgets",
        )?),
    })?;
    assert_eq!(result.structured_report()?, expected);
    assert!(!result.is_error());
    assert_eq!(analyzer.calls(), 1);
    Ok(())
}
```

- [ ] **Step 2: Run and verify the typed service is absent**

Run:

```bash
cargo test -p compass-prs --test service
cargo test -p compass-mcp --test pr_intelligence
```

Expected: compilation fails because `PrIntelligenceAnalyzer`, `PrIntelligenceMcp`, and `analyze_pr` do not exist.

- [ ] **Step 3: Implement the adapter-facing service**

```rust
pub trait PrIntelligenceAnalyzer: Send + Sync {
    fn analyze(
        &self,
        selector: &ChangeRequestSelector,
    ) -> Result<PrIntelligenceReport, OperationError>;
}

impl PrIntelligenceAnalyzer for FoundationOperation {
    fn analyze(
        &self,
        selector: &ChangeRequestSelector,
    ) -> Result<PrIntelligenceReport, OperationError> {
        FoundationOperation::analyze(self, selector)
    }
}
```

Add `OperationError::code()`. Source errors retain `PRS2001` through `PRS2008`; snapshot load, snapshot identity, canonical report, and report store failures use `PRS3001` through `PRS3004`.

- [ ] **Step 4: Implement the typed MCP adapter**

```rust
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyzePrInput {
    pub number: u64,
    pub repository: Option<compass_prs::RepositoryIdentity>,
}

pub struct PrIntelligenceMcp {
    analyzer: std::sync::Arc<dyn compass_prs::PrIntelligenceAnalyzer>,
}
```

Register one `analyze_pr` tool. Run the synchronous analyzer through `tokio::task::spawn_blocking`. On success, serialize the report once and use that value as MCP structured content. Add one short text content item: `Compass PR Intelligence foundation preview; no risk or gate verdict is available yet.` On failure, return structured `{ "code": CODE, "message": MESSAGE }`, set `isError: true`, and do not synthesize an empty report.

- [ ] **Step 5: Inject the same repository runtime into CLI and MCP**

The CLI startup path that serves MCP constructs the same repository, build options, source, snapshot provider, and report repository used by Task 6. It wraps one `FoundationOperation` in `Arc<dyn PrIntelligenceAnalyzer>` and passes it to the MCP server constructor.

Do not let `compass-mcp` depend on `compass-cli`; the CLI remains the composition root. Unit tests inject `FakeAnalyzer` directly.

- [ ] **Step 6: Run typed adapter and lint gates**

Run:

```bash
cargo test -p compass-prs --test service
cargo test -p compass-mcp --test pr_intelligence
cargo test -p compass-cli --test prs_intelligence_preview
cargo clippy -p compass-prs -p compass-cli -p compass-mcp --all-targets -- -D warnings
```

Expected: all pass; CLI and MCP return the same report ID and canonical structured report for the same captured evidence.

- [ ] **Step 7: Commit**

```bash
git add crates/compass-prs crates/compass-cli crates/compass-mcp
git commit -m "feat(mcp): expose typed PR intelligence preview"
```

## Task 8: Harden, benchmark, document, and qualify the foundation

**Files:**

- Create: `docs/PR_INTELLIGENCE.md`
- Create: `scripts/benchmark_pr_foundation.sh`
- Modify: `scripts/check_critical_coverage.sh`
- Modify: `fuzz/Cargo.toml`
- Create: `fuzz/fuzz_targets/pr_report.rs`
- Create: `fuzz/fuzz_targets/pr_changed_files.rs`
- Modify: `.github/workflows/compass-ci.yml`
- Modify: `.github/workflows/compass-hardening.yml`
- Modify: `crates/compass-prs/tests/report_contract.rs`
- Modify: `crates/compass-prs/tests/change_capture.rs`
- Modify: `crates/compass-prs/tests/report_store.rs`
- Modify: `crates/compass-cli/tests/prs_intelligence_preview.rs`

**Interfaces:**

- Consumes: the completed child-1 foundation.
- Produces: documented preview contract, fuzz/coverage/performance gates, cross-platform CI, and final qualification evidence.

- [ ] **Step 1: Add hostile and invariant test cases**

Add tests for:

- Report ID collision.
- Snapshot identity mismatch.
- Force-push capture producing a different report ID.
- Target-head movement producing a different report ID.
- Synthetic merge conflict without a false merge snapshot.
- Missing history realization followed by exact lazy materialization.
- Report-store interruption and retry.
- Repository names, branches, and paths containing Unicode and control characters.
- Maximum bounded process and report output.
- Symlink attacks at every report-store segment.

Name the tests exactly and make their invariant assertions explicit:

```rust
#[test]
fn force_push_or_target_movement_changes_report_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = QualificationFixture::new()?;
    let original = fixture.analyze()?;
    fixture.advance_pull_request_head()?;
    let force_pushed = fixture.analyze()?;
    assert_ne!(original.report_id, force_pushed.report_id);
    fixture.advance_target_head()?;
    let moved_target = fixture.analyze()?;
    assert_ne!(force_pushed.report_id, moved_target.report_id);
    Ok(())
}

#[test]
fn snapshot_mismatch_and_interrupted_store_never_publish_partial_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = QualificationFixture::new()?;
    fixture.return_wrong_snapshot_once();
    assert!(fixture.analyze().is_err());
    assert!(fixture.persisted_reports()?.is_empty());
    fixture.fail_store_before_persist_once();
    assert!(fixture.analyze().is_err());
    assert!(fixture.persisted_reports()?.is_empty());
    let report = fixture.analyze()?;
    assert_eq!(fixture.persisted_reports()?, vec![report.report_id]);
    Ok(())
}

#[test]
fn conflict_retains_identity_without_claiming_a_merge_snapshot()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = QualificationFixture::conflicted_merge()?;
    let report = fixture.analyze()?;
    assert!(matches!(
        report.revisions.merge_outcome,
        MergeOutcome::Conflicted { .. }
    ));
    assert_eq!(report.snapshots.len(), 3);
    assert!(report.risk.is_none());
    assert!(report.gates.is_empty());
    Ok(())
}
```

Complete the matrix with these assertions:

```rust
#[test]
fn missing_realization_materializes_exact_commit()
-> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = QualificationFixture::new()?;
    let expected = fixture.pull_request_head().to_owned();
    fixture.remove_head_realization()?;
    let report = fixture.analyze()?;
    let head = &report.snapshots[&SnapshotRole::PullRequestHead].identity;
    assert!(matches!(
        head,
        SnapshotIdentity::Commit { commit, .. } if commit == &expected
    ));
    Ok(())
}

#[test]
fn unicode_and_control_characters_round_trip_without_terminal_injection()
-> Result<(), Box<dyn std::error::Error>> {
    let report = QualificationFixture::unicode_paths()?.analyze()?;
    let bytes = canonical_report_bytes(&report)?;
    assert!(!bytes.contains(&0x1b));
    assert_eq!(serde_json::from_slice::<PrIntelligenceReport>(&bytes)?, report);
    Ok(())
}

#[test]
fn bounded_process_and_report_limits_are_enforced() {
    assert!(matches!(
        QualificationFixture::oversized_process_output(),
        Err(PrsError::OutputTooLarge { .. })
    ));
    assert!(matches!(
        QualificationFixture::oversized_report(),
        Err(ReportStoreError::TooLarge { .. })
    ));
}

#[test]
fn every_report_store_segment_rejects_symlinks()
-> Result<(), Box<dyn std::error::Error>> {
    for segment in ["compass", "pr-intelligence", "reports", "destination"] {
        let fixture = QualificationFixture::symlink_at(segment)?;
        assert!(matches!(
            fixture.save_report(),
            Err(ReportStoreError::UnsafePath { .. })
        ));
    }
    Ok(())
}
```

Run:

```bash
cargo test -p compass-prs
cargo test -p compass-cli --test prs_intelligence_preview
```

Expected: all pass.

- [ ] **Step 2: Add fuzz targets**

`pr_report` feeds arbitrary bytes to strict `PrIntelligenceReport` deserialization and, on success, asserts canonical round-trip and stable report ID verification.

`pr_changed_files` feeds arbitrary UTF-8/NUL-delimited bytes to the internal changed-file parser and asserts no panic, no absolute path, no parent traversal, deterministic sorting, and bounded output.

Add a non-default `fuzzing` feature to `compass-prs` that exposes only the parser shim. The target bodies are:

```toml
# crates/compass-prs/Cargo.toml
[features]
fuzzing = []

# fuzz/Cargo.toml dependency and targets
compass-prs = { path = "../crates/compass-prs", features = ["fuzzing"] }

[[bin]]
name = "pr_report"
path = "fuzz_targets/pr_report.rs"
test = false
doc = false
bench = false

[[bin]]
name = "pr_changed_files"
path = "fuzz_targets/pr_changed_files.rs"
test = false
doc = false
bench = false
```

```rust
// crates/compass-prs/src/lib.rs
#[cfg(feature = "fuzzing")]
pub fn fuzz_parse_changed_files(input: &str) -> Result<Vec<ChangedFile>, SourceError> {
    git::parse_changed_files(input)
}
```

```rust
// fuzz/fuzz_targets/pr_report.rs
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(report) =
        serde_json::from_slice::<compass_prs::PrIntelligenceReport>(bytes)
        && let Ok(canonical) = compass_prs::canonical_report_bytes(&report)
        && let Ok(round_trip) =
            serde_json::from_slice::<compass_prs::PrIntelligenceReport>(&canonical)
    {
        assert_eq!(report, round_trip);
        assert_eq!(
            compass_prs::canonical_report_bytes(&round_trip).ok(),
            Some(canonical)
        );
    }
});
```

```rust
// fuzz/fuzz_targets/pr_changed_files.rs
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(text) = std::str::from_utf8(bytes)
        && let Ok(first) = compass_prs::fuzz_parse_changed_files(text)
    {
        let second = compass_prs::fuzz_parse_changed_files(text);
        assert_eq!(second.as_ref().ok(), Some(&first));
        assert!(first.iter().all(|file| file.paths_are_repository_relative()));
    }
});
```

Run bounded smoke fuzzing:

```bash
cargo fuzz run pr_report -- -max_total_time=30
cargo fuzz run pr_changed_files -- -max_total_time=30
```

Expected: both complete without crashes, timeouts, or corpus-triggered assertion failures.

- [ ] **Step 3: Add critical coverage and CI gates**

Add `compass-core/src/snapshot.rs`, `compass-prs/src/{model,canonical,source,git,operation,report_store}.rs`, and the Compass preview adapter to critical coverage. Require at least 95% line coverage for identity validation, evidence-manifest construction, snapshot mismatch, report persistence, and parser error branches.

CI runs focused tests on Linux, macOS, and Windows. Hardening runs fuzz smoke tests, report-store hostile-path tests, and the small benchmark smoke tier on Linux. Full medium/large performance gates run only on the declared self-hosted reference runner so shared-runner variance cannot create meaningless regressions.

Add these workflow commands to the existing matrices rather than creating a parallel workflow:

```yaml
- name: PR Intelligence foundation
  run: |
    cargo test -p compass-core --test snapshot
    cargo test -p compass-prs
    cargo test -p compass-cli --test prs_intelligence_preview
    cargo test -p compass-mcp --test pr_intelligence
```

On Linux hardening:

```yaml
- run: cargo fuzz run pr_report -- -max_total_time=30
- run: cargo fuzz run pr_changed_files -- -max_total_time=30
- run: COMPASS_BENCH_TIER=small bash scripts/benchmark_pr_foundation.sh
```

Extend `scripts/check_critical_coverage.sh`'s checked-source array with the exact files named in this step and fail if any named file is absent, uncovered, or below 95%.

- [ ] **Step 4: Add the reproducible benchmark**

`benchmark_pr_foundation.sh` creates Git repositories and prebuilt graph history for:

```text
small:  10,000 graph nodes, 20,000 relationships
medium: 250,000 graph nodes, 750,000 relationships
large:  1,000,000 graph nodes, 3,000,000 relationships
```

`COMPASS_BENCH_TIER=small|medium|large|all` selects fixture size; the default is `small`. Reuse checked benchmark fixture manifests between runs rather than rebuilding million-node histories inside the timed interval. For cold and warm runs, record:

- Capture time.
- Three snapshot load times.
- Peak RSS.
- Canonical serialization time.
- Persistence time.
- Total wall time.
- Report bytes.

On the declared reference host, the script exits nonzero when warm p95 exceeds 5 seconds for small, 30 seconds for medium, or 90 seconds for large; when foundation memory exceeds the three loaded snapshots plus 20%; or when report persistence exceeds two seconds. On an undeclared host, it records metrics and enforces only correctness and the two-second persistence ceiling. The umbrella five-minute SLA remains reserved for completed stages 1–5.

The script accepts only these environment inputs:

```bash
COMPASS_BENCH_TIER="${COMPASS_BENCH_TIER:-small}"
COMPASS_BENCH_RUNS="${COMPASS_BENCH_RUNS:-20}"
COMPASS_BENCH_REFERENCE_HOST="${COMPASS_BENCH_REFERENCE_HOST:-}"
```

Validate the tier against `small|medium|large|all`, validate runs as an integer from 5 through 100, create all temporary repositories with `mktemp -d`, install one exit trap that removes only that exact directory, and write newline-delimited JSON measurements to stdout. Timing starts after fixture/history construction and ends after report persistence.

- [ ] **Step 5: Document exact behavior and limitations**

`docs/PR_INTELLIGENCE.md` documents:

- `foundation_preview` is provenance infrastructure, not a safety verdict.
- Exact four-revision meanings.
- Why only three commit snapshots load in child 1.
- Report and evidence schema names.
- Report storage path and permissions.
- Git/GitHub prerequisites.
- Missing-object remediation without implicit fetching.
- Preview CLI syntax and exit codes.
- No findings, risk, or gates until the relevant child designs are approved and implemented.
- Relationship to versioned graphs and CompassQL shared snapshots.
- Redaction and authorization are not implemented in the local-only foundation.

Do not add marketing copy to the README until stages 1–5 are complete.

- [ ] **Step 6: Run the full qualification matrix**

Run:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check_critical_coverage.sh
COMPASS_BENCH_TIER=small bash scripts/benchmark_pr_foundation.sh
```

Expected: every command exits `0`.

- [ ] **Step 7: Review the final diff and commit**

Verify:

```bash
git status --short
git diff --check
git log --oneline --max-count=8
```

Expected: only child-1 files are modified; no primary-worktree user files appear.

Commit:

```bash
git add .github crates/compass-core crates/compass-prs crates/compass-cli crates/compass-mcp docs/PR_INTELLIGENCE.md scripts fuzz
git commit -m "test(prs): qualify intelligence foundation"
```

## Acceptance checklist

- [ ] `GraphSelection`, `SnapshotProvider`, `GraphSnapshot`, and `SnapshotIdentity` exist exactly once in `compass-core`.
- [ ] Existing query/path/explain `--at` behavior remains green.
- [ ] Pull-request capture records merge base, PR head, target head, and synthetic merge tree as full object IDs.
- [ ] Capture performs no implicit fetch, checkout, ref mutation, hook, or code execution.
- [ ] The evidence manifest contains only immutable, identity-bearing inputs.
- [ ] Foundation reports use `compass.pr_intelligence.report/1` and `cmppr-report-v1:` report identities; future finding fingerprints retain `cmpprv1:`.
- [ ] Foundation reports contain no risk, gate, impact, owner, test, or overlap verdict.
- [ ] Snapshot identity mismatch prevents persistence.
- [ ] Reports persist atomically below the Git common directory and reject symlink traversal.
- [ ] The Compass preview emits canonical JSON and labels itself `foundation_preview`.
- [ ] CLI and MCP consume the same typed analyzer and return the same canonical report.
- [ ] Repository identity distinguishes GitHub.com and GitHub Enterprise Server hosts.
- [ ] Fuzz, critical coverage, cross-platform, and performance gates pass.
