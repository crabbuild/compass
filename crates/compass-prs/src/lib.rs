//! Native GitHub PR dashboard and graph-impact analysis for Compass.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use compass_model::GraphDocument;
use regex::Regex;
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use wait_timeout::ChildExt;

const GH_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_LIMIT: usize = 50;
const MAX_PROCESS_OUTPUT: usize = 16 * 1024 * 1024;
const STALE_DAYS: i64 = 14;
const STATUS_ORDER: &[&str] = &[
    "WRONG-BASE",
    "CI-FAIL",
    "CHANGES-REQ",
    "DRAFT",
    "STALE",
    "PENDING",
    "APPROVED",
    "READY",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub branch: String,
    pub base_branch: String,
    pub author: String,
    pub is_draft: bool,
    pub review_decision: String,
    pub ci_status: String,
    pub updated_at: OffsetDateTime,
    pub expected_base: String,
    pub worktree_path: Option<String>,
    pub communities_touched: Vec<i64>,
    pub nodes_affected: usize,
    pub files_changed: Vec<String>,
}

impl PrInfo {
    #[must_use]
    pub fn status(&self, now: OffsetDateTime) -> &'static str {
        classify(self, &self.expected_base, now)
    }

    #[must_use]
    pub fn days_old(&self, now: OffsetDateTime) -> i64 {
        (now - self.updated_at).whole_seconds().div_euclid(86_400)
    }

    #[must_use]
    pub fn blast_radius(&self) -> String {
        if self.nodes_affected == 0 {
            return String::new();
        }
        let nodes = if self.nodes_affected == 1 {
            "node"
        } else {
            "nodes"
        };
        let count = self.communities_touched.len();
        let communities = if count == 1 {
            "community"
        } else {
            "communities"
        };
        format!("{} {nodes} / {count} {communities}", self.nodes_affected)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcessOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PrsError {
    #[error("could not start {program}: {source}")]
    Start {
        program: String,
        source: std::io::Error,
    },
    #[error("{program} timed out after {seconds}s")]
    Timeout { program: String, seconds: u64 },
    #[error("could not wait for {program}: {source}")]
    Wait {
        program: String,
        source: std::io::Error,
    },
    #[error("{program} produced more than {limit} bytes of output")]
    OutputTooLarge { program: String, limit: usize },
    #[error("GitHub returned invalid JSON: {0}")]
    InvalidJson(serde_json::Error),
    #[error("GitHub returned an invalid PR record: {0}")]
    InvalidRecord(String),
    #[error("gh CLI not found or not authenticated. Run: gh auth login")]
    GithubUnavailable,
}

pub trait ProcessRunner: Sync {
    fn run(
        &self,
        program: &str,
        arguments: &[String],
        timeout: Duration,
    ) -> Result<ProcessOutput, PrsError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRunner;

impl ProcessRunner for SystemRunner {
    fn run(
        &self,
        program: &str,
        arguments: &[String],
        timeout: Duration,
    ) -> Result<ProcessOutput, PrsError> {
        run_bounded(program, arguments, timeout)
    }
}

fn run_bounded(
    program: &str,
    arguments: &[String],
    timeout: Duration,
) -> Result<ProcessOutput, PrsError> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| PrsError::Start {
            program: program.to_owned(),
            source,
        })?;
    let stdout = child
        .stdout
        .take()
        .map(|stream| std::thread::spawn(move || drain(stream)));
    let stderr = child
        .stderr
        .take()
        .map(|stream| std::thread::spawn(move || drain(stream)));
    let status = match child.wait_timeout(timeout) {
        Ok(Some(status)) => status,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_output(stdout);
            let _ = join_output(stderr);
            return Err(PrsError::Timeout {
                program: program.to_owned(),
                seconds: timeout.as_secs(),
            });
        }
        Err(source) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = join_output(stdout);
            let _ = join_output(stderr);
            return Err(PrsError::Wait {
                program: program.to_owned(),
                source,
            });
        }
    };
    let (stdout, stdout_truncated) = join_output(stdout);
    let (stderr, stderr_truncated) = join_output(stderr);
    if stdout_truncated || stderr_truncated {
        return Err(PrsError::OutputTooLarge {
            program: program.to_owned(),
            limit: MAX_PROCESS_OUTPUT,
        });
    }
    Ok(ProcessOutput {
        code: status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

fn drain(mut stream: impl Read) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 16 * 1024];
    while let Ok(read) = stream.read(&mut buffer) {
        if read == 0 {
            break;
        }
        let remaining = MAX_PROCESS_OUTPUT.saturating_sub(kept.len());
        let copy = remaining.min(read);
        kept.extend_from_slice(&buffer[..copy]);
        truncated |= copy < read;
    }
    (kept, truncated)
}

fn join_output(handle: Option<std::thread::JoinHandle<(Vec<u8>, bool)>>) -> (Vec<u8>, bool) {
    handle
        .and_then(|handle| handle.join().ok())
        .unwrap_or_else(|| (Vec::new(), false))
}

#[must_use]
pub fn classify(pr: &PrInfo, base: &str, now: OffsetDateTime) -> &'static str {
    if pr.base_branch != base {
        "WRONG-BASE"
    } else if pr.ci_status == "FAILURE" {
        "CI-FAIL"
    } else if pr.review_decision == "CHANGES_REQUESTED" {
        "CHANGES-REQ"
    } else if pr.is_draft {
        "DRAFT"
    } else if pr.days_old(now) >= STALE_DAYS {
        "STALE"
    } else if pr.review_decision == "APPROVED" {
        "APPROVED"
    } else if pr.ci_status == "PENDING" {
        "PENDING"
    } else {
        "READY"
    }
}

#[must_use]
pub fn parse_ci(rollup: &[Value]) -> &'static str {
    if rollup.is_empty() {
        return "NONE";
    }
    let failure = [
        "FAILURE",
        "CANCELLED",
        "TIMED_OUT",
        "ACTION_REQUIRED",
        "STARTUP_FAILURE",
    ];
    if rollup.iter().any(|entry| {
        entry
            .get("conclusion")
            .and_then(Value::as_str)
            .is_some_and(|conclusion| failure.contains(&conclusion))
    }) {
        return "FAILURE";
    }
    if rollup.iter().any(|entry| {
        matches!(
            entry.get("status").and_then(Value::as_str),
            Some("IN_PROGRESS" | "QUEUED")
        )
    }) {
        return "PENDING";
    }
    if rollup
        .iter()
        .any(|entry| entry.get("conclusion").and_then(Value::as_str) == Some("SUCCESS"))
    {
        "SUCCESS"
    } else {
        "NONE"
    }
}

pub fn detect_default_branch(runner: &impl ProcessRunner, repo: Option<&str>) -> String {
    let mut arguments = vec![
        "repo".to_owned(),
        "view".to_owned(),
        "--json".to_owned(),
        "defaultBranchRef".to_owned(),
    ];
    if let Some(repo) = repo {
        arguments.extend(["--repo".to_owned(), repo.to_owned()]);
    }
    if let Ok(output) = runner.run("gh", &arguments, GH_TIMEOUT)
        && output.code == 0
        && let Ok(value) = serde_json::from_str::<Value>(&output.stdout)
        && let Some(branch) = value
            .pointer("/defaultBranchRef/name")
            .and_then(Value::as_str)
        && !branch.is_empty()
    {
        return branch.to_owned();
    }
    let git = [
        "symbolic-ref".to_owned(),
        "refs/remotes/origin/HEAD".to_owned(),
    ];
    runner
        .run("git", &git, Duration::from_secs(5))
        .ok()
        .filter(|output| output.code == 0)
        .and_then(|output| output.stdout.trim().rsplit('/').next().map(str::to_owned))
        .filter(|branch| !branch.is_empty())
        .unwrap_or_else(|| "main".to_owned())
}

pub fn fetch_prs(
    runner: &impl ProcessRunner,
    repo: Option<&str>,
    base: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<PrInfo>, PrsError> {
    let resolved_base = base.map_or_else(|| detect_default_branch(runner, repo), str::to_owned);
    let mut arguments = vec![
        "pr".to_owned(),
        "list".to_owned(),
        "--state".to_owned(),
        "open".to_owned(),
        "--limit".to_owned(),
        limit.unwrap_or(DEFAULT_LIMIT).to_string(),
        "--json".to_owned(),
        "number,title,headRefName,baseRefName,author,isDraft,reviewDecision,statusCheckRollup,updatedAt".to_owned(),
    ];
    if let Some(repo) = repo {
        arguments.extend(["--repo".to_owned(), repo.to_owned()]);
    }
    let output = runner
        .run("gh", &arguments, GH_TIMEOUT)
        .map_err(|_| PrsError::GithubUnavailable)?;
    if output.code != 0 {
        return Err(PrsError::GithubUnavailable);
    }
    let raw = serde_json::from_str::<Value>(&output.stdout).map_err(PrsError::InvalidJson)?;
    let items = raw
        .as_array()
        .ok_or_else(|| PrsError::InvalidRecord("PR list is not an array".to_owned()))?;
    items
        .iter()
        .map(|item| parse_pr(item, &resolved_base))
        .collect()
}

fn parse_pr(item: &Value, expected_base: &str) -> Result<PrInfo, PrsError> {
    let required = |key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| PrsError::InvalidRecord(format!("missing {key}")))
    };
    let updated = OffsetDateTime::parse(required("updatedAt")?, &Rfc3339)
        .map_err(|error| PrsError::InvalidRecord(format!("invalid updatedAt: {error}")))?;
    Ok(PrInfo {
        number: item
            .get("number")
            .and_then(Value::as_u64)
            .ok_or_else(|| PrsError::InvalidRecord("missing number".to_owned()))?,
        title: required("title")?.to_owned(),
        branch: required("headRefName")?.to_owned(),
        base_branch: required("baseRefName")?.to_owned(),
        author: item
            .pointer("/author/login")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_owned(),
        is_draft: item
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        review_decision: item
            .get("reviewDecision")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        ci_status: parse_ci(
            item.get("statusCheckRollup")
                .and_then(Value::as_array)
                .map_or(&[], Vec::as_slice),
        )
        .to_owned(),
        updated_at: updated,
        expected_base: expected_base.to_owned(),
        worktree_path: None,
        communities_touched: Vec::new(),
        nodes_affected: 0,
        files_changed: Vec::new(),
    })
}

pub fn fetch_pr_files(runner: &impl ProcessRunner, number: u64, repo: Option<&str>) -> Vec<String> {
    let mut arguments = vec![
        "pr".to_owned(),
        "diff".to_owned(),
        number.to_string(),
        "--name-only".to_owned(),
    ];
    if let Some(repo) = repo {
        arguments.extend(["--repo".to_owned(), repo.to_owned()]);
    }
    runner
        .run("gh", &arguments, GH_TIMEOUT)
        .ok()
        .filter(|output| output.code == 0)
        .map(|output| {
            output
                .stdout
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub fn fetch_worktrees(runner: &impl ProcessRunner) -> BTreeMap<String, String> {
    let arguments = [
        "worktree".to_owned(),
        "list".to_owned(),
        "--porcelain".to_owned(),
    ];
    let Some(output) = runner
        .run("git", &arguments, GIT_TIMEOUT)
        .ok()
        .filter(|output| output.code == 0)
    else {
        return BTreeMap::new();
    };
    let mut mapping = BTreeMap::new();
    let mut current_path = None;
    for line in output.stdout.lines() {
        if line.is_empty() {
            current_path = None;
        } else if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(path.to_owned());
        } else if let (Some(branch), Some(path)) = (
            line.strip_prefix("branch refs/heads/"),
            current_path.as_ref(),
        ) {
            mapping.insert(branch.to_owned(), path.clone());
        }
    }
    mapping
}

#[must_use]
pub fn path_match(graph_source: &str, pr_file: &str) -> bool {
    graph_source == pr_file
        || graph_source.ends_with(&format!("/{pr_file}"))
        || pr_file.ends_with(&format!("/{graph_source}"))
}

#[must_use]
pub fn compute_pr_impact(files: &[String], document: &GraphDocument) -> (Vec<i64>, usize) {
    let (file_communities, file_counts) = graph_file_index(document);
    impact_from_index(files, &file_communities, &file_counts)
}

fn graph_file_index(
    document: &GraphDocument,
) -> (HashMap<String, BTreeSet<i64>>, HashMap<String, usize>) {
    let mut communities = HashMap::<String, BTreeSet<i64>>::new();
    let mut counts = HashMap::<String, usize>::new();
    for node in &document.nodes {
        let source = node.string("source_file");
        if source.is_empty() {
            continue;
        }
        communities.entry(source.clone()).or_default();
        *counts.entry(source.clone()).or_default() += 1;
        if let Some(community) = node
            .attributes
            .get("community")
            .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        {
            communities.entry(source).or_default().insert(community);
        }
    }
    (communities, counts)
}

fn impact_from_index(
    files: &[String],
    file_communities: &HashMap<String, BTreeSet<i64>>,
    file_counts: &HashMap<String, usize>,
) -> (Vec<i64>, usize) {
    let mut communities = BTreeSet::new();
    let mut nodes = 0_usize;
    let mut matched = HashSet::new();
    for file in files {
        for (source, source_communities) in file_communities {
            if !matched.contains(source) && path_match(source, file) {
                communities.extend(source_communities);
                nodes = nodes.saturating_add(file_counts.get(source).copied().unwrap_or_default());
                matched.insert(source.clone());
            }
        }
    }
    (communities.into_iter().collect(), nodes)
}

#[must_use]
pub fn build_community_labels(
    document: &GraphDocument,
    top_n: usize,
) -> BTreeMap<i64, Vec<String>> {
    let mut labels = BTreeMap::<i64, Vec<String>>::new();
    for node in &document.nodes {
        let Some(community) = node
            .attributes
            .get("community")
            .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        else {
            continue;
        };
        let label = node
            .attributes
            .get("label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty())
            .unwrap_or(&node.id);
        if !label.is_empty() && labels.entry(community).or_default().len() < top_n {
            labels.entry(community).or_default().push(label.to_owned());
        }
    }
    labels
}

pub fn attach_graph_impact(
    runner: &impl ProcessRunner,
    prs: &mut [PrInfo],
    graph_path: &Path,
    repo: Option<&str>,
) -> BTreeMap<i64, Vec<String>> {
    if GraphDocument::size_cap_exceeded(graph_path).is_some() {
        return BTreeMap::new();
    }
    // `graphify prs --graph` accepts any filename, not only a `.json`
    // extension, while retaining the same graph-size guard.
    let Ok(document) = GraphDocument::load_for_recluster_compatibility(graph_path) else {
        return BTreeMap::new();
    };
    let (file_communities, file_counts) = graph_file_index(&document);
    let actionable = prs
        .iter()
        .enumerate()
        .filter(|(_, pr)| pr.status(OffsetDateTime::now_utc()) != "WRONG-BASE")
        .map(|(index, pr)| (index, pr.number))
        .collect::<Vec<_>>();
    let workers = actionable.len().clamp(1, 8);
    let next = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(Mutex::new(Vec::<(usize, Vec<String>)>::new()));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let next = Arc::clone(&next);
            let completed = Arc::clone(&completed);
            let actionable = actionable.as_slice();
            scope.spawn(move || {
                loop {
                    let position = next.fetch_add(1, Ordering::Relaxed);
                    let Some((index, number)) = actionable.get(position).copied() else {
                        break;
                    };
                    let files = fetch_pr_files(runner, number, repo);
                    completed
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push((index, files));
                }
            });
        }
    });
    let mut completed = completed
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    completed.sort_by_key(|(index, _)| *index);
    for (index, files) in completed.drain(..) {
        let (communities, nodes) = impact_from_index(&files, &file_communities, &file_counts);
        prs[index].files_changed = files;
        prs[index].communities_touched = communities;
        prs[index].nodes_affected = nodes;
    }
    build_community_labels(&document, 4)
}

#[must_use]
pub fn format_prs_text(prs: &[PrInfo], base: &str, now: OffsetDateTime) -> String {
    let mut actionable = prs
        .iter()
        .filter(|pr| pr.base_branch == base)
        .collect::<Vec<_>>();
    let wrong = prs.len().saturating_sub(actionable.len());
    actionable.sort_by_key(|pr| (status_index(pr.status(now)), pr.days_old(now)));
    let mut sections = vec![format!(
        "Open PRs targeting {base}: {}  ({wrong} on wrong base, not shown)\n",
        actionable.len()
    )];
    sections.extend(actionable.into_iter().map(|pr| {
        let impact = if pr.blast_radius().is_empty() {
            String::new()
        } else {
            format!("  blast_radius={}", pr.blast_radius())
        };
        format!(
            "#{} [{}] CI={} review={} age={}d author={}{}\n  {}",
            pr.number,
            pr.status(now),
            pr.ci_status,
            if pr.review_decision.is_empty() {
                "none"
            } else {
                &pr.review_decision
            },
            pr.days_old(now),
            pr.author,
            impact,
            pr.title
        )
    }));
    sections.join("\n\n")
}

fn status_index(status: &str) -> usize {
    STATUS_ORDER
        .iter()
        .position(|candidate| *candidate == status)
        .unwrap_or(99)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderOptions {
    pub color: bool,
    pub command_name: &'static str,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            color: false,
            command_name: "graphify prs",
        }
    }
}

impl RenderOptions {
    fn paint(self, code: &str, text: impl AsRef<str>) -> String {
        if self.color {
            format!("\u{1b}[{code}m{}\u{1b}[0m", text.as_ref())
        } else {
            text.as_ref().to_owned()
        }
    }

    fn green(self, text: impl AsRef<str>) -> String {
        self.paint("32", text)
    }

    fn red(self, text: impl AsRef<str>) -> String {
        self.paint("31", text)
    }

    fn yellow(self, text: impl AsRef<str>) -> String {
        self.paint("33", text)
    }

    fn cyan(self, text: impl AsRef<str>) -> String {
        self.paint("36", text)
    }

    fn bold(self, text: impl AsRef<str>) -> String {
        self.paint("1", text)
    }

    fn dim(self, text: impl AsRef<str>) -> String {
        self.paint("2", text)
    }

    fn status(self, status: &str) -> String {
        match status {
            "READY" => self.green(status),
            "APPROVED" => self.bold(self.green(status)),
            "CI-FAIL" | "CHANGES-REQ" => self.red(status),
            "WRONG-BASE" | "STALE" => self.dim(status),
            "DRAFT" | "PENDING" => self.yellow(status),
            _ => status.to_owned(),
        }
    }

    fn ci(self, status: &str) -> String {
        match status {
            "SUCCESS" => self.green("✓"),
            "FAILURE" => self.red("✗"),
            "PENDING" => self.yellow("…"),
            "NONE" => self.dim("–"),
            _ => "?".to_owned(),
        }
    }
}

fn visible_width(text: &str) -> usize {
    static ANSI: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new("\\x1b\\[[0-9;]*m").unwrap_or_else(|_| std::process::abort())
    });
    ANSI.replace_all(text, "").chars().count()
}

fn pad(text: String, width: usize) -> String {
    let spaces = width.saturating_sub(visible_width(&text));
    format!("{text}{}", " ".repeat(spaces))
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

#[must_use]
pub fn render_dashboard(
    prs: &[PrInfo],
    base: &str,
    show_wrong_base: bool,
    now: OffsetDateTime,
    render: RenderOptions,
) -> String {
    let mut actionable = prs
        .iter()
        .filter(|pr| pr.base_branch == base)
        .collect::<Vec<_>>();
    let wrong_base = prs
        .iter()
        .filter(|pr| pr.base_branch != base)
        .collect::<Vec<_>>();
    actionable.sort_by_key(|pr| (status_index(pr.status(now)), pr.days_old(now)));
    let mut lines = vec![
        String::new(),
        render.bold(format!(
            "  {}  ·  base: {base}  ·  {} PRs",
            render.command_name,
            actionable.len()
        )),
        String::new(),
    ];
    if actionable.is_empty() {
        lines.push(render.dim("  No open PRs targeting this base branch."));
    } else {
        lines.extend([
            format!(
                "  {:>4}  {:2}  {:13}  {:8}  {:22}  TITLE",
                "#", "CI", "STATUS", "UPDATED", "IMPACT"
            ),
            format!(
                "  {}  {}  {}  {}  {}  {}",
                "─".repeat(4),
                "─".repeat(2),
                "─".repeat(13),
                "─".repeat(8),
                "─".repeat(22),
                "─".repeat(40)
            ),
        ]);
        for pr in &actionable {
            let status = pad(render.status(pr.status(now)), 13);
            let age = if pr.days_old(now) > 0 {
                format!("{}d", pr.days_old(now))
            } else {
                "today".to_owned()
            };
            let impact = if pr.blast_radius().is_empty() {
                pad(render.dim("–"), 22)
            } else {
                pad(render.dim(truncate(&pr.blast_radius(), 22)), 22)
            };
            let worktree = if pr.worktree_path.is_some() {
                format!(" {}", render.cyan("⬡"))
            } else {
                "  ".to_owned()
            };
            let draft = if pr.is_draft {
                render.dim(" [draft]")
            } else {
                String::new()
            };
            lines.push(format!(
                "  {}{}  {}  {}  {:>6}   {}  {}{}",
                pad(render.bold(format!("#{}", pr.number)), 6),
                worktree,
                render.ci(&pr.ci_status),
                status,
                age,
                impact,
                truncate(&pr.title, 52),
                draft
            ));
        }
    }
    let mut counts = HashMap::<&str, usize>::new();
    for pr in &actionable {
        *counts.entry(pr.status(now)).or_default() += 1;
    }
    let mut parts = Vec::new();
    if let Some(count) = counts.get("READY") {
        parts.push(render.green(format!("{count} ready")));
    }
    if let Some(count) = counts.get("APPROVED") {
        parts.push(render.bold(render.green(format!("{count} approved"))));
    }
    if let Some(count) = counts.get("PENDING") {
        parts.push(render.yellow(format!("{count} pending CI")));
    }
    if let Some(count) = counts.get("CI-FAIL") {
        parts.push(render.red(format!("{count} CI failing")));
    }
    if let Some(count) = counts.get("CHANGES-REQ") {
        parts.push(render.red(format!("{count} changes requested")));
    }
    if let Some(count) = counts.get("DRAFT") {
        parts.push(render.yellow(format!("{count} draft")));
    }
    if let Some(count) = counts.get("STALE") {
        parts.push(render.dim(format!("{count} stale")));
    }
    if !wrong_base.is_empty() {
        parts.push(render.dim(format!("{} wrong base", wrong_base.len())));
    }
    lines.extend([
        String::new(),
        format!("  {}", parts.join(" · ")),
        String::new(),
    ]);
    if !wrong_base.is_empty() && show_wrong_base {
        lines.push(render.dim(format!(
            "  ── {} PRs targeting wrong base ──",
            wrong_base.len()
        )));
        let mut wrong = wrong_base;
        wrong.sort_by_key(|pr| std::cmp::Reverse(pr.number));
        for pr in wrong {
            lines.push(render.dim(format!(
                "  #{:4}  base={:12}  {}",
                pr.number,
                pr.base_branch,
                truncate(&pr.title, 60)
            )));
        }
        lines.push(String::new());
    }
    format!("{}\n", lines.join("\n"))
}

#[must_use]
pub fn render_worktrees(
    prs: &[PrInfo],
    worktrees: &BTreeMap<String, String>,
    now: OffsetDateTime,
    render: RenderOptions,
) -> String {
    let mut lines = vec![String::new(), render.bold("  Worktrees"), String::new()];
    if worktrees.is_empty() {
        lines.extend([render.dim("  No active worktrees found."), String::new()]);
        return format!("{}\n", lines.join("\n"));
    }
    let by_branch = prs
        .iter()
        .map(|pr| (pr.branch.as_str(), pr))
        .collect::<HashMap<_, _>>();
    for (branch, path) in worktrees {
        lines.push(format!("  {}", render.cyan(path)));
        if let Some(pr) = by_branch.get(branch.as_str()) {
            lines.push(format!(
                "    {} {branch}  ->  PR {}  [{}]  {}",
                render.dim("branch:"),
                render.bold(format!("#{}", pr.number)),
                render.status(pr.status(now)),
                truncate(&pr.title, 50)
            ));
        } else {
            lines.push(format!(
                "    {} {branch}  {}",
                render.dim("branch:"),
                render.dim("(no open PR)")
            ));
        }
        lines.push(String::new());
    }
    format!("{}\n", lines.join("\n"))
}

#[must_use]
pub fn render_conflicts(
    prs: &[PrInfo],
    base: &str,
    community_labels: &BTreeMap<i64, Vec<String>>,
    now: OffsetDateTime,
    render: RenderOptions,
) -> String {
    let actionable = prs
        .iter()
        .filter(|pr| pr.base_branch == base && !pr.communities_touched.is_empty())
        .collect::<Vec<_>>();
    if actionable.is_empty() {
        return format!(
            "{}\n",
            render.dim(
                "\n  No graph impact data - run with a valid graph.json to detect conflicts.\n"
            )
        );
    }
    let mut by_community = BTreeMap::<i64, Vec<&PrInfo>>::new();
    for pr in actionable {
        for community in &pr.communities_touched {
            by_community.entry(*community).or_default().push(pr);
        }
    }
    let mut conflicts = by_community
        .into_iter()
        .filter(|(_, prs)| prs.len() > 1)
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        return format!(
            "{}\n",
            render
                .green("\n  No community overlap between open PRs - safe to merge in any order.\n")
        );
    }
    conflicts.sort_by_key(|(_, prs)| std::cmp::Reverse(prs.len()));
    let mut lines = vec![
        String::new(),
        render.bold("  Community conflicts (PRs sharing the same graph community)"),
        String::new(),
    ];
    for (community, prs) in conflicts {
        let labels = community_labels
            .get(&community)
            .filter(|labels| !labels.is_empty())
            .map(|labels| render.dim(format!("  — {}", labels.join(", "))))
            .unwrap_or_default();
        lines.push(format!(
            "  {}{}  ({} PRs overlap)",
            render.yellow(format!("Community {community}")),
            labels,
            prs.len()
        ));
        for pr in prs {
            lines.push(format!(
                "    #{:4}  {}  {}",
                pr.number,
                pad(render.status(pr.status(now)), 13),
                truncate(&pr.title, 55)
            ));
        }
        lines.push(String::new());
    }
    format!("{}\n", lines.join("\n"))
}

#[must_use]
pub fn render_pr_detail(pr: &PrInfo, now: OffsetDateTime, render: RenderOptions) -> String {
    let mut lines = vec![
        String::new(),
        render.bold(format!(
            "  PR #{}  ·  {}",
            pr.number,
            render.status(pr.status(now))
        )),
        format!("  {}", pr.title),
        String::new(),
        format!(
            "  {}  {}  ->  {}",
            render.dim("branch:"),
            pr.branch,
            pr.base_branch
        ),
        format!("  {}  {}", render.dim("author:"), pr.author),
        format!("  {} {}d ago", render.dim("updated:"), pr.days_old(now)),
        format!(
            "  {}      {} {}",
            render.dim("CI:"),
            render.ci(&pr.ci_status),
            pr.ci_status
        ),
    ];
    if !pr.review_decision.is_empty() {
        lines.push(format!(
            "  {} {}",
            render.dim("review:"),
            pr.review_decision
        ));
    }
    if let Some(worktree) = &pr.worktree_path {
        lines.push(format!(
            "  {} {}",
            render.dim("worktree:"),
            render.cyan(worktree)
        ));
    }
    if !pr.blast_radius().is_empty() {
        lines.extend([
            String::new(),
            format!("  {}  {}", render.bold("Graph impact:"), pr.blast_radius()),
            format!(
                "  {} {:?}",
                render.dim("communities:"),
                pr.communities_touched
            ),
        ]);
        if !pr.files_changed.is_empty() {
            lines.push(format!(
                "  {} {}",
                render.dim("files changed:"),
                pr.files_changed.len()
            ));
            lines.extend(
                pr.files_changed
                    .iter()
                    .take(10)
                    .map(|file| format!("    {}", render.dim(file))),
            );
            if pr.files_changed.len() > 10 {
                lines.push(render.dim(format!("    … and {} more", pr.files_changed.len() - 10)));
            }
        }
    }
    lines.push(String::new());
    format!("{}\n", lines.join("\n"))
}

#[must_use]
pub fn triage_prompt(prs: &[PrInfo], base: &str, now: OffsetDateTime) -> Option<String> {
    let candidates = prs
        .iter()
        .filter(|pr| pr.base_branch == base && !matches!(pr.status(now), "WRONG-BASE" | "STALE"))
        .map(|pr| {
            let impact = if pr.blast_radius().is_empty() {
                String::new()
            } else {
                format!(", blast_radius={}", pr.blast_radius())
            };
            format!(
                "PR #{} [{}] CI={} review={} age={}d author={}{}\n  title: {}",
                pr.number,
                pr.status(now),
                pr.ci_status,
                if pr.review_decision.is_empty() {
                    "none"
                } else {
                    &pr.review_decision
                },
                pr.days_old(now),
                pr.author,
                impact,
                pr.title
            )
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    Some(format!(
        "You are a senior engineer helping triage a PR review queue. Given these open PRs, rank them by review priority for the repo maintainer. For each PR give: priority number, one sentence on what action to take and why. Be direct and specific. Format each as: #<number> — <action>.\n\n{}",
        candidates.join("\n\n")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct StubRunner {
        outputs: BTreeMap<String, ProcessOutput>,
    }

    impl StubRunner {
        fn with(mut self, program: &str, arguments: &[&str], stdout: &str) -> Self {
            self.outputs.insert(
                format!("{program} {}", arguments.join(" ")),
                ProcessOutput {
                    code: 0,
                    stdout: stdout.to_owned(),
                    stderr: String::new(),
                },
            );
            self
        }
    }

    impl ProcessRunner for StubRunner {
        fn run(
            &self,
            program: &str,
            arguments: &[String],
            _timeout: Duration,
        ) -> Result<ProcessOutput, PrsError> {
            self.outputs
                .get(&format!("{program} {}", arguments.join(" ")))
                .cloned()
                .ok_or(PrsError::GithubUnavailable)
        }
    }

    fn fixture(now: OffsetDateTime) -> PrInfo {
        PrInfo {
            number: 7,
            title: "Test PR".to_owned(),
            branch: "feature".to_owned(),
            base_branch: "v8".to_owned(),
            author: "alice".to_owned(),
            is_draft: false,
            review_decision: String::new(),
            ci_status: "SUCCESS".to_owned(),
            updated_at: now - time::Duration::days(1),
            expected_base: "v8".to_owned(),
            worktree_path: None,
            communities_touched: Vec::new(),
            nodes_affected: 0,
            files_changed: Vec::new(),
        }
    }

    #[test]
    fn classification_ci_and_text_contracts() {
        let now = OffsetDateTime::UNIX_EPOCH + time::Duration::days(30);
        let mut pr = fixture(now);
        assert_eq!(pr.status(now), "READY");
        pr.is_draft = true;
        pr.updated_at = now - time::Duration::days(20);
        assert_eq!(pr.status(now), "DRAFT");
        pr.is_draft = false;
        assert_eq!(pr.status(now), "STALE");
        pr.updated_at = now - time::Duration::days(1);
        pr.review_decision = "CHANGES_REQUESTED".to_owned();
        assert_eq!(pr.status(now), "CHANGES-REQ");
        pr.review_decision.clear();
        pr.base_branch = "main".to_owned();
        assert_eq!(pr.status(now), "WRONG-BASE");
        pr.base_branch = "v8".to_owned();
        pr.ci_status = "FAILURE".to_owned();
        assert_eq!(pr.status(now), "CI-FAIL");
        assert_eq!(
            parse_ci(&[serde_json::json!({"conclusion":"CANCELLED","status":"COMPLETED"})]),
            "FAILURE"
        );
        assert!(format_prs_text(&[pr], "v8", now).contains("[CI-FAIL]"));
    }

    #[test]
    fn path_matching_is_boundary_safe() {
        assert!(path_match("src/auth/api.py", "api.py"));
        assert!(path_match("api.py", "src/auth/api.py"));
        assert!(!path_match("config.py", "g.py"));
    }

    #[test]
    fn graph_impact_deduplicates_files_and_preserves_path_boundaries() {
        let document: GraphDocument = serde_json::from_value(serde_json::json!({
            "nodes": [
                {"id":"a1","source_file":"src/auth/api.py","community":0},
                {"id":"a2","source_file":"src/auth/api.py","community":0},
                {"id":"b","source_file":"src/admin/api.py","community":1}
            ],
            "links": []
        }))
        .unwrap_or_else(|_| std::process::abort());
        assert_eq!(
            compute_pr_impact(
                &["src/auth/api.py".to_owned(), "api.py".to_owned()],
                &document
            ),
            (vec![0, 1], 3)
        );
        assert_eq!(
            compute_pr_impact(&["src/auth/api.py".to_owned()], &document),
            (vec![0], 2)
        );
        assert_eq!(
            compute_pr_impact(&["g.py".to_owned()], &document),
            (Vec::new(), 0)
        );
    }

    #[test]
    fn labels_fall_back_to_ids_and_honor_the_cap() {
        let document: GraphDocument = serde_json::from_value(serde_json::json!({
            "nodes": [
                {"id":"a","label":"","community":0},
                {"id":"b","label":"Beta","community":0},
                {"id":"c","label":"Gamma","community":0}
            ],
            "links": []
        }))
        .unwrap_or_else(|_| std::process::abort());
        assert_eq!(
            build_community_labels(&document, 2),
            BTreeMap::from([(0, vec!["a".to_owned(), "Beta".to_owned()])])
        );
    }

    #[test]
    fn fetch_and_worktree_parsing_match_github_contract() {
        let list_arguments = [
            "pr",
            "list",
            "--state",
            "open",
            "--limit",
            "50",
            "--json",
            "number,title,headRefName,baseRefName,author,isDraft,reviewDecision,statusCheckRollup,updatedAt",
        ];
        let runner = StubRunner::default()
            .with(
                "gh",
                &list_arguments,
                r#"[{"number":9,"title":"فارسی 🏆","headRefName":"feature","baseRefName":"v8","author":null,"isDraft":false,"reviewDecision":null,"statusCheckRollup":[{"status":"QUEUED"}],"updatedAt":"2026-07-19T08:00:00Z"}]"#,
            )
            .with(
                "git",
                &["worktree", "list", "--porcelain"],
                "worktree /detached\nHEAD a\ndetached\n\nworktree /feature\nHEAD b\nbranch refs/heads/feature\n\n",
            );
        let prs =
            fetch_prs(&runner, None, Some("v8"), None).unwrap_or_else(|_| std::process::abort());
        assert_eq!(prs.len(), 1);
        assert_eq!(prs[0].title, "فارسی 🏆");
        assert_eq!(prs[0].author, "?");
        assert_eq!(prs[0].ci_status, "PENDING");
        assert_eq!(
            fetch_worktrees(&runner),
            BTreeMap::from([("feature".to_owned(), "/feature".to_owned())])
        );
    }

    #[test]
    fn graph_impact_accepts_non_json_filenames() {
        let directory = tempfile::tempdir().unwrap_or_else(|_| std::process::abort());
        let graph = directory.path().join("graph.data");
        std::fs::write(
            &graph,
            r#"{"nodes":[{"id":"a","label":"Alpha","source_file":"src/a.rs","community":4}],"links":[]}"#,
        )
        .unwrap_or_else(|_| std::process::abort());
        let runner =
            StubRunner::default().with("gh", &["pr", "diff", "7", "--name-only"], "src/a.rs\n");
        let now = OffsetDateTime::now_utc();
        let mut prs = vec![fixture(now)];
        let labels = attach_graph_impact(&runner, &mut prs, &graph, None);
        assert_eq!(prs[0].communities_touched, vec![4]);
        assert_eq!(prs[0].nodes_affected, 1);
        assert_eq!(labels.get(&4), Some(&vec!["Alpha".to_owned()]));
    }

    #[test]
    fn renderers_include_python_print_newlines() {
        let now = OffsetDateTime::UNIX_EPOCH + time::Duration::days(30);
        let pr = fixture(now);
        let render = RenderOptions::default();
        assert!(
            render_dashboard(std::slice::from_ref(&pr), "v8", false, now, render).ends_with("\n\n")
        );
        assert!(render_pr_detail(&pr, now, render).ends_with("\n\n"));
        assert!(render_worktrees(&[pr], &BTreeMap::new(), now, render).ends_with("\n\n"));
    }
}
