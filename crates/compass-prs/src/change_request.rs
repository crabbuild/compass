use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use compass_pr_intelligence::{
    ChangeHunk, ChangeRequest, MergeOutcome, RepositoryIdentity, RevisionSet, SourceRange,
};
use regex::Regex;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{ProcessOutput, ProcessRunner, PrsError};

const GIT_CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
const GH_CAPTURE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CAPTURED_FILES: usize = 10_000;
const MAX_CAPTURED_HUNKS: usize = 100_000;
const MAX_GITHUB_PAGES: usize = 100;

/// Forge-neutral immutable change-request capture boundary.
pub trait ChangeRequestSource {
    fn capture(&self) -> Result<ChangeRequest, PrsError>;
}

pub struct LocalGitChangeRequestSource<'runner, Runner> {
    runner: &'runner Runner,
    repository_root: PathBuf,
    repository: RepositoryIdentity,
    target_revision: String,
    head_revision: String,
    pull_request_number: Option<u64>,
}

impl<'runner, Runner: ProcessRunner> LocalGitChangeRequestSource<'runner, Runner> {
    #[must_use]
    pub fn new(
        runner: &'runner Runner,
        repository_root: impl Into<PathBuf>,
        repository: RepositoryIdentity,
        target_revision: impl Into<String>,
        head_revision: impl Into<String>,
    ) -> Self {
        Self {
            runner,
            repository_root: repository_root.into(),
            repository,
            target_revision: target_revision.into(),
            head_revision: head_revision.into(),
            pull_request_number: None,
        }
    }

    #[must_use]
    pub fn with_pull_request_number(mut self, number: u64) -> Self {
        self.pull_request_number = Some(number);
        self
    }

    fn git(&self, arguments: &[&str]) -> Result<ProcessOutput, PrsError> {
        let mut command = vec!["-C".to_owned(), path_text(&self.repository_root)?];
        command.extend(arguments.iter().map(|argument| (*argument).to_owned()));
        self.runner.run("git", &command, GIT_CAPTURE_TIMEOUT)
    }

    fn git_with_input(&self, arguments: &[&str], input: &[u8]) -> Result<ProcessOutput, PrsError> {
        let mut command = vec!["-C".to_owned(), path_text(&self.repository_root)?];
        command.extend(arguments.iter().map(|argument| (*argument).to_owned()));
        self.runner
            .run_with_input("git", &command, input, GIT_CAPTURE_TIMEOUT)
    }

    fn resolve_commit(&self, revision: &str) -> Result<String, PrsError> {
        let expression = format!("{revision}^{{commit}}");
        let output = self.git(&["rev-parse", "--verify", "--end-of-options", &expression])?;
        output_line("git rev-parse", output).and_then(validate_object_id)
    }

    fn merge_outcome(&self, target: &str, head: &str) -> Result<MergeOutcome, PrsError> {
        let output = self.git(&["merge-tree", "--write-tree", target, head])?;
        if output.code == 1 {
            let mut digest = Sha256::new();
            digest.update(output.stdout.as_bytes());
            digest.update([0]);
            digest.update(output.stderr.as_bytes());
            return Ok(MergeOutcome::Conflicted {
                evidence_digest: format!("sha256:{:x}", digest.finalize()),
            });
        }
        if output.code != 0 {
            return Err(command_failure("git merge-tree", &output));
        }
        let tree =
            output.stdout.lines().next().ok_or_else(|| {
                PrsError::InvalidRecord("git merge-tree returned no tree".to_owned())
            })?;
        let tree = validate_object_id(tree.trim().to_owned())?;
        let commit = format!(
            "tree {tree}\nparent {target}\nparent {head}\nauthor Compass <compass@localhost> 0 +0000\ncommitter Compass <compass@localhost> 0 +0000\n\nCompass synthetic merge\n"
        );
        let output = self.git_with_input(
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            commit.as_bytes(),
        )?;
        let object_id = output_line("git hash-object", output).and_then(validate_object_id)?;
        Ok(MergeOutcome::Clean { object_id })
    }

    fn capture_hunks(&self, merge_base: &str, head: &str) -> Result<Vec<ChangeHunk>, PrsError> {
        let output = self.git(&[
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--no-prefix",
            "--unified=0",
            "--find-renames",
            merge_base,
            head,
            "--",
        ])?;
        if output.code != 0 {
            return Err(command_failure("git diff", &output));
        }
        let mut hunks = parse_unified_hunks(&output.stdout)?;
        let files = self.git(&[
            "diff",
            "--name-status",
            "--find-renames",
            merge_base,
            head,
            "--",
        ])?;
        if files.code != 0 {
            return Err(command_failure("git diff --name-status", &files));
        }
        let existing = hunks
            .iter()
            .map(|hunk| (hunk.old_path.clone(), hunk.new_path.clone()))
            .collect::<BTreeSet<_>>();
        for (status, old_path, new_path) in parse_name_status(&files.stdout)? {
            if existing.contains(&(old_path.clone(), new_path.clone())) {
                continue;
            }
            let identity = format!("{status}\0{old_path}\0{new_path}");
            hunks.push(ChangeHunk {
                old_path,
                new_path,
                status,
                old: SourceRange {
                    start_line: 0,
                    line_count: 0,
                },
                new: SourceRange {
                    start_line: 0,
                    line_count: 0,
                },
                patch_digest: format!("sha256:{:x}", Sha256::digest(identity.as_bytes())),
            });
            if hunks.len() > MAX_CAPTURED_HUNKS {
                return Err(PrsError::EvidenceLimit(format!(
                    "diff contains more than {MAX_CAPTURED_HUNKS} change records"
                )));
            }
        }
        hunks.sort();
        Ok(hunks)
    }
}

impl<Runner: ProcessRunner> ChangeRequestSource for LocalGitChangeRequestSource<'_, Runner> {
    fn capture(&self) -> Result<ChangeRequest, PrsError> {
        self.repository
            .validate()
            .map_err(|error| PrsError::InvalidRecord(error.to_string()))?;
        let target_head = self.resolve_commit(&self.target_revision)?;
        let pull_request_head = self.resolve_commit(&self.head_revision)?;
        let merge_base = output_line(
            "git merge-base",
            self.git(&["merge-base", &target_head, &pull_request_head])?,
        )
        .and_then(validate_object_id)?;
        let merge_result = self.merge_outcome(&target_head, &pull_request_head)?;
        let hunks = self.capture_hunks(&merge_base, &pull_request_head)?;
        Ok(ChangeRequest {
            repository: self.repository.clone(),
            pull_request_number: self.pull_request_number,
            revisions: RevisionSet {
                merge_base,
                pull_request_head,
                target_head,
                merge_result,
            },
            hunks,
        })
    }
}

pub struct GithubChangeRequestSource<'runner, Runner> {
    runner: &'runner Runner,
    repository_root: PathBuf,
    host: String,
    owner: String,
    repository: String,
    number: u64,
}

impl<'runner, Runner: ProcessRunner> GithubChangeRequestSource<'runner, Runner> {
    #[must_use]
    pub fn new(
        runner: &'runner Runner,
        repository_root: impl Into<PathBuf>,
        host: impl Into<String>,
        owner: impl Into<String>,
        repository: impl Into<String>,
        number: u64,
    ) -> Self {
        Self {
            runner,
            repository_root: repository_root.into(),
            host: host.into(),
            owner: owner.into(),
            repository: repository.into(),
            number,
        }
    }

    fn endpoint(&self, suffix: &str) -> String {
        format!(
            "repos/{}/{}/pulls/{}{}",
            self.owner, self.repository, self.number, suffix
        )
    }

    fn metadata(&self) -> Result<GithubRevisionMetadata, PrsError> {
        let arguments = vec![
            "api".to_owned(),
            "--method".to_owned(),
            "GET".to_owned(),
            "--hostname".to_owned(),
            self.host.clone(),
            self.endpoint(""),
        ];
        let output = self.runner.run("gh", &arguments, GH_CAPTURE_TIMEOUT)?;
        if output.code != 0 {
            return Err(command_failure("gh api", &output));
        }
        parse_github_metadata(&output.stdout)
    }

    fn files(&self) -> Result<BTreeSet<String>, PrsError> {
        let arguments = vec![
            "api".to_owned(),
            "--method".to_owned(),
            "GET".to_owned(),
            "--hostname".to_owned(),
            self.host.clone(),
            "--paginate".to_owned(),
            "--slurp".to_owned(),
            self.endpoint("/files?per_page=100"),
        ];
        let output = self.runner.run("gh", &arguments, GH_CAPTURE_TIMEOUT)?;
        if output.code != 0 {
            return Err(command_failure("gh api --paginate", &output));
        }
        parse_github_files(&output.stdout)
    }
}

impl<Runner: ProcessRunner> ChangeRequestSource for GithubChangeRequestSource<'_, Runner> {
    fn capture(&self) -> Result<ChangeRequest, PrsError> {
        let before = self.metadata()?;
        let files = self.files()?;
        let after = self.metadata()?;
        if before != after {
            return Err(PrsError::RevisionDrift(format!(
                "GitHub PR #{} changed from base/head {}/{} to {}/{}",
                self.number, before.base, before.head, after.base, after.head
            )));
        }
        let repository = RepositoryIdentity {
            forge: "github".to_owned(),
            host: self.host.clone(),
            owner: self.owner.clone(),
            name: self.repository.clone(),
        };
        let request = LocalGitChangeRequestSource::new(
            self.runner,
            &self.repository_root,
            repository,
            &after.base,
            &after.head,
        )
        .with_pull_request_number(self.number)
        .capture()?;
        let captured_files = request
            .hunks
            .iter()
            .flat_map(|hunk| [&hunk.old_path, &hunk.new_path])
            .filter(|path| path.as_str() != "/dev/null")
            .cloned()
            .collect::<BTreeSet<_>>();
        if captured_files != files {
            return Err(PrsError::RevisionDrift(
                "GitHub changed-file pages do not match the frozen local diff".to_owned(),
            ));
        }
        Ok(request)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GithubRevisionMetadata {
    base: String,
    head: String,
}

fn parse_github_metadata(text: &str) -> Result<GithubRevisionMetadata, PrsError> {
    let value: Value = serde_json::from_str(text).map_err(PrsError::InvalidJson)?;
    let base = value
        .pointer("/base/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| PrsError::InvalidRecord("GitHub PR has no base SHA".to_owned()))?;
    let head = value
        .pointer("/head/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| PrsError::InvalidRecord("GitHub PR has no head SHA".to_owned()))?;
    Ok(GithubRevisionMetadata {
        base: validate_object_id(base.to_owned())?,
        head: validate_object_id(head.to_owned())?,
    })
}

fn parse_github_files(text: &str) -> Result<BTreeSet<String>, PrsError> {
    let pages: Value = serde_json::from_str(text).map_err(PrsError::InvalidJson)?;
    let pages = pages
        .as_array()
        .ok_or_else(|| PrsError::InvalidRecord("GitHub file pages must be an array".to_owned()))?;
    if pages.len() > MAX_GITHUB_PAGES {
        return Err(PrsError::EvidenceLimit(format!(
            "GitHub returned {} file pages; limit is {MAX_GITHUB_PAGES}",
            pages.len()
        )));
    }
    let mut files = BTreeSet::new();
    for page in pages {
        let records = page.as_array().ok_or_else(|| {
            PrsError::InvalidRecord("each GitHub file page must be an array".to_owned())
        })?;
        for record in records {
            let path = record
                .get("filename")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    PrsError::InvalidRecord("GitHub file record has no filename".to_owned())
                })?;
            if path.is_empty() || path.contains('\0') {
                return Err(PrsError::InvalidRecord(
                    "GitHub returned an invalid changed path".to_owned(),
                ));
            }
            files.insert(path.to_owned());
            if let Some(previous) = record.get("previous_filename").and_then(Value::as_str) {
                if previous.is_empty() || previous.contains('\0') {
                    return Err(PrsError::InvalidRecord(
                        "GitHub returned an invalid previous changed path".to_owned(),
                    ));
                }
                files.insert(previous.to_owned());
            }
            if files.len() > MAX_CAPTURED_FILES {
                return Err(PrsError::EvidenceLimit(format!(
                    "GitHub returned more than {MAX_CAPTURED_FILES} changed files"
                )));
            }
        }
    }
    Ok(files)
}

pub fn detect_repository_identity(
    runner: &impl ProcessRunner,
    repository_root: &Path,
) -> Result<RepositoryIdentity, PrsError> {
    let arguments = vec![
        "-C".to_owned(),
        path_text(repository_root)?,
        "remote".to_owned(),
        "get-url".to_owned(),
        "origin".to_owned(),
    ];
    let output = runner.run("git", &arguments, GIT_CAPTURE_TIMEOUT)?;
    if output.code == 0
        && let Some(identity) = parse_remote_identity(output.stdout.trim())
    {
        return Ok(identity);
    }
    let name = repository_root
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("repository")
        .to_owned();
    Ok(RepositoryIdentity {
        forge: "git".to_owned(),
        host: "local".to_owned(),
        owner: "local".to_owned(),
        name,
    })
}

fn parse_remote_identity(remote: &str) -> Option<RepositoryIdentity> {
    let (host, path) = if let Some(rest) = remote.strip_prefix("git@") {
        rest.split_once(':')?
    } else {
        let rest = remote
            .strip_prefix("https://")
            .or_else(|| remote.strip_prefix("http://"))
            .or_else(|| remote.strip_prefix("ssh://git@"))?;
        rest.split_once('/')?
    };
    let path = path.strip_suffix(".git").unwrap_or(path).trim_matches('/');
    let (owner, name) = path.rsplit_once('/')?;
    if host.is_empty() || owner.is_empty() || name.is_empty() {
        return None;
    }
    Some(RepositoryIdentity {
        forge: if host.eq_ignore_ascii_case("github.com") {
            "github"
        } else {
            "git"
        }
        .to_owned(),
        host: host.to_ascii_lowercase(),
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

fn parse_unified_hunks(patch: &str) -> Result<Vec<ChangeHunk>, PrsError> {
    let range = Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")
        .map_err(|error| PrsError::InvalidRecord(error.to_string()))?;
    let mut old_path = String::new();
    let mut new_path = String::new();
    let mut pending: Option<(SourceRange, SourceRange, String)> = None;
    let mut hunks = Vec::new();
    for line in patch.lines() {
        if line.starts_with("diff --git ") {
            finish_hunk(&mut hunks, &old_path, &new_path, pending.take())?;
            old_path.clear();
            new_path.clear();
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            finish_hunk(&mut hunks, &old_path, &new_path, pending.take())?;
            old_path = unquote_path(path)?;
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            new_path = unquote_path(path)?;
            continue;
        }
        if let Some(captures) = range.captures(line) {
            finish_hunk(&mut hunks, &old_path, &new_path, pending.take())?;
            let old = SourceRange {
                start_line: parse_range_value(&captures, 1)?,
                line_count: parse_optional_count(&captures, 2)?,
            };
            let new = SourceRange {
                start_line: parse_range_value(&captures, 3)?,
                line_count: parse_optional_count(&captures, 4)?,
            };
            pending = Some((old, new, format!("{line}\n")));
        } else if let Some((_, _, body)) = pending.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    finish_hunk(&mut hunks, &old_path, &new_path, pending)?;
    Ok(hunks)
}

fn parse_name_status(output: &str) -> Result<Vec<(String, String, String)>, PrsError> {
    let mut changes = Vec::new();
    for line in output.lines() {
        let fields = line.split('\t').collect::<Vec<_>>();
        let Some(code) = fields.first().copied() else {
            continue;
        };
        let kind = code.as_bytes().first().copied().ok_or_else(|| {
            PrsError::InvalidRecord("git returned an empty file status".to_owned())
        })?;
        let (status, old_path, new_path) = match (kind, fields.as_slice()) {
            (b'A', [_, path]) => ("added", "/dev/null".to_owned(), unquote_path(path)?),
            (b'D', [_, path]) => ("deleted", unquote_path(path)?, "/dev/null".to_owned()),
            (b'M' | b'T', [_, path]) => {
                let path = unquote_path(path)?;
                ("modified", path.clone(), path)
            }
            (b'R', [_, old, new]) => ("renamed", unquote_path(old)?, unquote_path(new)?),
            _ => {
                return Err(PrsError::InvalidRecord(format!(
                    "unsupported git file status record {line:?}"
                )));
            }
        };
        changes.push((status.to_owned(), old_path, new_path));
        if changes.len() > MAX_CAPTURED_FILES {
            return Err(PrsError::EvidenceLimit(format!(
                "diff contains more than {MAX_CAPTURED_FILES} changed files"
            )));
        }
    }
    Ok(changes)
}

fn finish_hunk(
    hunks: &mut Vec<ChangeHunk>,
    old_path: &str,
    new_path: &str,
    pending: Option<(SourceRange, SourceRange, String)>,
) -> Result<(), PrsError> {
    let Some((old, new, body)) = pending else {
        return Ok(());
    };
    if old_path.is_empty() || new_path.is_empty() {
        return Err(PrsError::InvalidRecord(
            "unified diff hunk has no file header".to_owned(),
        ));
    }
    if hunks.len() >= MAX_CAPTURED_HUNKS {
        return Err(PrsError::EvidenceLimit(format!(
            "diff contains more than {MAX_CAPTURED_HUNKS} hunks"
        )));
    }
    let status = match (old_path, new_path) {
        ("/dev/null", _) => "added",
        (_, "/dev/null") => "deleted",
        _ if old_path != new_path => "renamed",
        _ => "modified",
    };
    hunks.push(ChangeHunk {
        old_path: old_path.to_owned(),
        new_path: new_path.to_owned(),
        status: status.to_owned(),
        old,
        new,
        patch_digest: format!("sha256:{:x}", Sha256::digest(body.as_bytes())),
    });
    Ok(())
}

fn parse_range_value(captures: &regex::Captures<'_>, index: usize) -> Result<u64, PrsError> {
    captures
        .get(index)
        .and_then(|value| value.as_str().parse().ok())
        .ok_or_else(|| PrsError::InvalidRecord("invalid unified diff range".to_owned()))
}

fn parse_optional_count(captures: &regex::Captures<'_>, index: usize) -> Result<u64, PrsError> {
    captures.get(index).map_or(Ok(1), |value| {
        value
            .as_str()
            .parse()
            .map_err(|_| PrsError::InvalidRecord("invalid unified diff count".to_owned()))
    })
}

fn unquote_path(value: &str) -> Result<String, PrsError> {
    if !value.starts_with('"') {
        if value.contains('\u{fffd}') || value.chars().any(char::is_control) {
            return Err(PrsError::InvalidRecord(
                "Git returned a path that cannot be represented safely as UTF-8".to_owned(),
            ));
        }
        return Ok(value.to_owned());
    }
    if !value.ends_with('"') || value.len() < 2 {
        return Err(PrsError::InvalidRecord(
            "invalid quoted Git path".to_owned(),
        ));
    }
    let mut bytes = Vec::new();
    let encoded = value.as_bytes();
    let mut index = 1;
    while index + 1 < encoded.len() {
        if encoded[index] != b'\\' {
            bytes.push(encoded[index]);
            index += 1;
            continue;
        }
        index += 1;
        let escaped = *encoded
            .get(index)
            .ok_or_else(|| PrsError::InvalidRecord("invalid quoted Git path escape".to_owned()))?;
        match escaped {
            b'\\' | b'"' => {
                bytes.push(escaped);
                index += 1;
            }
            b'a' => {
                bytes.push(0x07);
                index += 1;
            }
            b'b' => {
                bytes.push(0x08);
                index += 1;
            }
            b't' => {
                bytes.push(b'\t');
                index += 1;
            }
            b'n' => {
                bytes.push(b'\n');
                index += 1;
            }
            b'v' => {
                bytes.push(0x0b);
                index += 1;
            }
            b'f' => {
                bytes.push(0x0c);
                index += 1;
            }
            b'r' => {
                bytes.push(b'\r');
                index += 1;
            }
            b'0'..=b'7' => {
                let mut value = 0_u16;
                let mut digits = 0;
                while digits < 3
                    && index < encoded.len() - 1
                    && matches!(encoded[index], b'0'..=b'7')
                {
                    value = value * 8 + u16::from(encoded[index] - b'0');
                    index += 1;
                    digits += 1;
                }
                let byte = u8::try_from(value).map_err(|_| {
                    PrsError::InvalidRecord("Git path octal escape exceeds one byte".to_owned())
                })?;
                bytes.push(byte);
            }
            _ => {
                return Err(PrsError::InvalidRecord(
                    "unsupported quoted Git path escape".to_owned(),
                ));
            }
        }
    }
    let decoded = String::from_utf8(bytes)
        .map_err(|_| PrsError::InvalidRecord("Git path is not valid UTF-8".to_owned()))?;
    if decoded.chars().any(char::is_control) {
        return Err(PrsError::InvalidRecord(
            "Git path contains control characters".to_owned(),
        ));
    }
    Ok(decoded)
}

fn output_line(operation: &str, output: ProcessOutput) -> Result<String, PrsError> {
    if output.code != 0 {
        return Err(command_failure(operation, &output));
    }
    let line = output.stdout.trim();
    if line.is_empty() || line.contains('\n') {
        return Err(PrsError::InvalidRecord(format!(
            "{operation} returned an invalid value"
        )));
    }
    Ok(line.to_owned())
}

fn command_failure(operation: &str, output: &ProcessOutput) -> PrsError {
    let stderr = output.stderr.trim();
    PrsError::InvalidRecord(format!(
        "{operation} exited with {}{}",
        output.code,
        if stderr.is_empty() {
            String::new()
        } else {
            format!(": {stderr}")
        }
    ))
}

fn validate_object_id(value: String) -> Result<String, PrsError> {
    if matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(value)
    } else {
        Err(PrsError::InvalidRecord(
            "expected a full lowercase Git object ID".to_owned(),
        ))
    }
}

fn path_text(path: &Path) -> Result<String, PrsError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| PrsError::InvalidRecord("repository path is not valid UTF-8".to_owned()))
}
