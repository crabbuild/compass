use std::collections::BTreeMap;
use std::error::Error;
use std::time::Duration;

use compass_prs::{
    PrInfo, ProcessOutput, ProcessRunner, RenderOptions, SystemRunner, detect_default_branch,
    fetch_pr_files, fetch_prs, fetch_worktrees, format_prs_text, parse_ci, render_conflicts,
    render_dashboard, render_pr_detail, render_worktrees, triage_prompt,
};
use serde_json::json;
use time::OffsetDateTime;

#[derive(Default)]
struct Runner(BTreeMap<String, Result<ProcessOutput, String>>);

impl ProcessRunner for Runner {
    fn run(
        &self,
        program: &str,
        arguments: &[String],
        _timeout: Duration,
    ) -> Result<ProcessOutput, compass_prs::PrsError> {
        let key = format!("{program} {}", arguments.join(" "));
        match self.0.get(&key) {
            Some(Ok(output)) => Ok(output.clone()),
            _ => Err(compass_prs::PrsError::GithubUnavailable),
        }
    }
}

fn pr(number: u64, base: &str, now: OffsetDateTime) -> PrInfo {
    PrInfo {
        number,
        title: format!("A very detailed pull request title {number}"),
        branch: format!("feature-{number}"),
        base_branch: base.to_owned(),
        author: "dev".to_owned(),
        is_draft: false,
        review_decision: String::new(),
        ci_status: "NONE".to_owned(),
        updated_at: now,
        expected_base: "main".to_owned(),
        worktree_path: None,
        communities_touched: Vec::new(),
        nodes_affected: 0,
        files_changed: Vec::new(),
    }
}

#[test]
fn process_parsers_cover_repo_fallback_invalid_records_files_and_worktrees()
-> Result<(), Box<dyn Error>> {
    let repo_command = "gh repo view --json defaultBranchRef --repo org/repo";
    let list_command = "gh pr list --state open --limit 3 --json number,title,headRefName,baseRefName,author,isDraft,reviewDecision,statusCheckRollup,updatedAt --repo org/repo";
    let files_command = "gh pr diff 7 --name-only --repo org/repo";
    let worktree_command = "git worktree list --porcelain";
    let runner = Runner(BTreeMap::from([
        (
            repo_command.to_owned(),
            Ok(ProcessOutput {
                code: 0,
                stdout: "{\"defaultBranchRef\":{\"name\":\"trunk\"}}".to_owned(),
                stderr: String::new(),
            }),
        ),
        (
            list_command.to_owned(),
            Ok(ProcessOutput {
                code: 0,
                stdout: serde_json::to_string(&json!([{
                    "number":7,"title":"Ship","headRefName":"feature","baseRefName":"trunk",
                    "author":{"login":"ada"},"isDraft":false,"reviewDecision":"APPROVED",
                    "statusCheckRollup":[],"updatedAt":"2026-01-02T03:04:05Z"
                }]))?,
                stderr: String::new(),
            }),
        ),
        (
            files_command.to_owned(),
            Ok(ProcessOutput {
                code: 0,
                stdout: " src/a.rs \n\nREADME.md\n".to_owned(),
                stderr: String::new(),
            }),
        ),
        (
            worktree_command.to_owned(),
            Ok(ProcessOutput {
                code: 0,
                stdout: "worktree /tmp/main\nHEAD abc\nbranch refs/heads/main\n\nworktree /tmp/feature\nbranch refs/heads/feature\n".to_owned(),
                stderr: String::new(),
            }),
        ),
    ]));
    assert_eq!(detect_default_branch(&runner, Some("org/repo")), "trunk");
    let prs = fetch_prs(&runner, Some("org/repo"), Some("trunk"), Some(3))?;
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].ci_status, "NONE");
    assert_eq!(
        fetch_pr_files(&runner, 7, Some("org/repo")),
        ["src/a.rs", "README.md"]
    );
    assert_eq!(fetch_worktrees(&runner)["feature"], "/tmp/feature");
    assert!(fetch_prs(&Runner::default(), None, None, None).is_err());
    assert!(fetch_pr_files(&Runner::default(), 1, None).is_empty());
    assert!(fetch_worktrees(&Runner::default()).is_empty());
    assert_eq!(parse_ci(&[json!({"status":"UNKNOWN"})]), "NONE");
    Ok(())
}

#[test]
fn renderers_cover_color_unknown_status_unowned_worktrees_and_detail_overflow()
-> Result<(), Box<dyn Error>> {
    let now = OffsetDateTime::from_unix_timestamp(1_800_000_000)?;
    let mut ready = pr(1, "main", now);
    ready.nodes_affected = 1;
    ready.communities_touched = vec![2];
    ready.review_decision = "UNRECOGNIZED".to_owned();
    ready.ci_status = "MYSTERY".to_owned();
    ready.worktree_path = Some("/tmp/feature-1".to_owned());
    ready.files_changed = (0..12)
        .map(|index| format!("src/file-{index}.rs"))
        .collect();
    let wrong = pr(2, "release", now);
    let render = RenderOptions {
        color: true,
        command_name: "compass prs",
    };
    let dashboard = render_dashboard(&[ready.clone(), wrong.clone()], "main", true, now, render);
    assert!(dashboard.contains("\u{1b}["));
    assert!(dashboard.contains("wrong base"));
    assert!(format_prs_text(&[ready.clone()], "main", now).contains("blast_radius=1 node"));
    let worktrees = BTreeMap::from([
        ("feature-1".to_owned(), "/tmp/owned".to_owned()),
        ("orphan".to_owned(), "/tmp/orphan".to_owned()),
    ]);
    assert!(render_worktrees(&[ready.clone()], &worktrees, now, render).contains("no open PR"));
    assert!(render_worktrees(&[], &BTreeMap::new(), now, render).contains("No active"));
    assert!(
        render_conflicts(&[], "main", &BTreeMap::new(), now, render).contains("No graph impact")
    );
    assert!(
        render_conflicts(&[ready.clone()], "main", &BTreeMap::new(), now, render)
            .contains("safe to merge")
    );
    let mut overlap = pr(3, "main", now);
    overlap.communities_touched = vec![2];
    assert!(
        render_conflicts(
            &[ready.clone(), overlap],
            "main",
            &BTreeMap::from([(2, vec!["Parser".to_owned()])]),
            now,
            render,
        )
        .contains("Community 2")
    );
    let detail = render_pr_detail(&ready, now, render);
    assert!(detail.contains("review:"));
    assert!(detail.contains("… and 2 more"));
    assert!(triage_prompt(&[wrong], "main", now).is_none());
    assert!(triage_prompt(&[ready], "main", now).is_some());
    Ok(())
}

#[cfg(unix)]
#[test]
fn system_runner_reports_missing_program_and_timeout() {
    assert!(
        SystemRunner
            .run(
                "definitely-not-a-real-compass-program",
                &[],
                Duration::from_secs(1)
            )
            .is_err()
    );
    assert!(
        SystemRunner
            .run(
                "/bin/sh",
                &["-c".to_owned(), "sleep 1".to_owned()],
                Duration::from_millis(10),
            )
            .is_err()
    );
}
