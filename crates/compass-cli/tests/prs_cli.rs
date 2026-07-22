#![cfg(unix)]

use std::error::Error;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::mpsc;
use std::time::Duration;

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("GRAPHIFY_REPO_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn write_executable(path: &Path, content: &str) -> Result<(), Box<dyn Error>> {
    std::fs::write(path, content)?;
    let mut permissions = path.metadata()?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

fn seed(root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let output = root.join("graphify-out");
    let bin = root.join("bin");
    std::fs::create_dir_all(&output)?;
    std::fs::create_dir_all(&bin)?;
    let graph = br#"{
  "directed": false,
  "multigraph": false,
  "graph": {},
  "nodes": [
    {"id":"auth","label":"AuthService","source_file":"src/auth.py","community":0},
    {"id":"repo","label":"UserRepository","source_file":"src/auth.py","community":0},
    {"id":"docs","label":"Guide","source_file":"docs/guide.md","community":1}
  ],
  "links": []
}"#;
    std::fs::write(output.join("graph.json"), graph)?;
    std::fs::write(output.join("graph.data"), graph)?;
    write_executable(
        &bin.join("gh"),
        r#"#!/bin/sh
if [ "$1" = "pr" ] && [ "$2" = "list" ]; then
  printf '%s\n' '[{"number":101,"title":"Add authentication flow فارسی 🏆","headRefName":"feature-auth","baseRefName":"v8","author":{"login":"alice"},"isDraft":false,"reviewDecision":"","statusCheckRollup":[{"conclusion":"SUCCESS","status":"COMPLETED"}],"updatedAt":"2026-07-19T08:00:00Z"},{"number":102,"title":"Repair auth failure","headRefName":"fix-auth","baseRefName":"v8","author":{"login":"bob"},"isDraft":false,"reviewDecision":"","statusCheckRollup":[{"conclusion":"FAILURE","status":"COMPLETED"}],"updatedAt":"2026-07-18T08:00:00Z"},{"number":103,"title":"Old-base cleanup","headRefName":"cleanup","baseRefName":"main","author":{"login":"carol"},"isDraft":false,"reviewDecision":"","statusCheckRollup":[],"updatedAt":"2026-07-17T08:00:00Z"},{"number":104,"title":"Draft docs","headRefName":"draft-docs","baseRefName":"v8","author":null,"isDraft":true,"reviewDecision":"","statusCheckRollup":[{"conclusion":"SUCCESS","status":"COMPLETED"}],"updatedAt":"2026-07-16T08:00:00Z"},{"number":105,"title":"Changes requested","headRefName":"changes","baseRefName":"v8","author":{"login":"dana"},"isDraft":false,"reviewDecision":"CHANGES_REQUESTED","statusCheckRollup":[{"conclusion":"SUCCESS","status":"COMPLETED"}],"updatedAt":"2026-07-15T08:00:00Z"},{"number":106,"title":"Approved change","headRefName":"approved","baseRefName":"v8","author":{"login":"erin"},"isDraft":false,"reviewDecision":"APPROVED","statusCheckRollup":[{"conclusion":"SUCCESS","status":"COMPLETED"}],"updatedAt":"2026-07-14T08:00:00Z"},{"number":107,"title":"Pending CI","headRefName":"pending","baseRefName":"v8","author":{"login":"frank"},"isDraft":false,"reviewDecision":"","statusCheckRollup":[{"conclusion":null,"status":"QUEUED"}],"updatedAt":"2026-07-13T08:00:00Z"},{"number":108,"title":"Stale change","headRefName":"stale","baseRefName":"v8","author":{"login":"grace"},"isDraft":false,"reviewDecision":"","statusCheckRollup":[{"conclusion":"SUCCESS","status":"COMPLETED"}],"updatedAt":"2020-01-01T08:00:00Z"}]'
  exit 0
fi
if [ "$1" = "pr" ] && [ "$2" = "diff" ]; then
  case "$3" in
    101) printf '%s\n' 'src/auth.py' ;;
    102) printf '%s\n' 'src/auth.py' 'docs/guide.md' ;;
    *) exit 1 ;;
  esac
  exit 0
fi
if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
  printf '%s\n' '{"defaultBranchRef":{"name":"v8"}}'
  exit 0
fi
exit 1
"#,
    )?;
    write_executable(
        &bin.join("git"),
        &format!(
            "#!/bin/sh\nif [ \"$1\" = \"worktree\" ]; then\n  printf '%s\\n' 'worktree {}' 'HEAD abc123' 'branch refs/heads/feature-auth' ''\n  exit 0\nfi\nexit 1\n",
            root.join("feature-auth").display()
        ),
    )?;
    Ok(bin)
}

fn run(
    executable: &Path,
    repo: &Path,
    root: &Path,
    bin: &Path,
    args: &[&str],
) -> Result<Output, Box<dyn Error>> {
    run_with_environment(executable, repo, root, bin, args, &[])
}

fn run_with_environment(
    executable: &Path,
    repo: &Path,
    root: &Path,
    bin: &Path,
    args: &[&str],
    environment: &[(&str, &str)],
) -> Result<Output, Box<dyn Error>> {
    let python = repo.join(".venv/bin/python");
    let mut command = Command::new(executable);
    if executable == python {
        command.args(["-m", "graphify"]);
        command.env("PYTHONPATH", repo);
    }
    let path = std::env::join_paths([
        bin.to_path_buf(),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ])?;
    command
        .current_dir(root)
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .envs(environment.iter().copied())
        .arg("prs")
        .args(args);
    Ok(command.output()?)
}

#[test]
fn prs_dashboard_detail_conflicts_and_worktrees_match_python() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let bin = seed(root)?;
    let python = repo.join(".venv/bin/python");
    for args in [
        vec!["--base", "v8", "--wrong-base"],
        vec!["101", "--base", "v8", "--graph", "graphify-out/graph.json"],
        vec![
            "#101",
            "--base=v8",
            "--repo",
            "owner/project",
            "--graph=graphify-out/graph.data",
        ],
        vec!["999", "--base", "v8"],
        vec!["--triage", "--base", "nonexistent"],
        vec![
            "--conflicts",
            "--base",
            "v8",
            "--graph",
            "graphify-out/graph.json",
        ],
        vec!["--worktrees", "--base", "v8"],
    ] {
        let expected = run(&python, &repo, root, &bin, &args)?;
        let actual = run(
            Path::new(env!("CARGO_BIN_EXE_graphify")),
            &repo,
            root,
            &bin,
            &args,
        )?;
        assert_eq!(actual.status.code(), expected.status.code(), "{args:?}");
        assert_eq!(actual.stdout, expected.stdout, "stdout {args:?}");
        assert_eq!(actual.stderr, expected.stderr, "stderr {args:?}");
    }
    Ok(())
}

#[test]
fn prs_custom_provider_triage_matches_python_without_issuing_a_call() -> Result<(), Box<dyn Error>>
{
    let repo = repository_root();
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let bin = seed(root)?;
    std::fs::create_dir_all(root.join(".graphify"))?;
    std::fs::write(
        root.join(".graphify/providers.json"),
        r#"{"custom":{"base_url":"https://example.com/v1","default_model":"custom-model","env_key":"CUSTOM_API_KEY"}}"#,
    )?;
    let environment = [
        ("GRAPHIFY_ALLOW_LOCAL_PROVIDERS", "1"),
        ("GRAPHIFY_TRIAGE_BACKEND", "custom"),
        ("CUSTOM_API_KEY", "configured"),
    ];
    let args = ["--triage", "--base", "v8"];
    let expected = run_with_environment(
        &repo.join(".venv/bin/python"),
        &repo,
        root,
        &bin,
        &args,
        &environment,
    )?;
    let actual = run_with_environment(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        root,
        &bin,
        &args,
        &environment,
    )?;
    assert_eq!(actual.status.code(), expected.status.code());
    assert_eq!(actual.stdout, expected.stdout);
    assert_eq!(actual.stderr, expected.stderr);
    Ok(())
}

#[test]
fn prs_help_and_compass_namespace_are_compatible() -> Result<(), Box<dyn Error>> {
    let repo = repository_root();
    let directory = tempfile::tempdir()?;
    let bin = seed(directory.path())?;
    for args in [
        vec!["--help"],
        vec!["--base", "v8", "-?"],
        vec!["--base", "--help"],
    ] {
        let expected = run(
            &repo.join(".venv/bin/python"),
            &repo,
            directory.path(),
            &bin,
            &args,
        )?;
        let actual = run(
            Path::new(env!("CARGO_BIN_EXE_graphify")),
            &repo,
            directory.path(),
            &bin,
            &args,
        )?;
        assert_eq!(actual.stdout, expected.stdout, "{args:?}");
    }
    let compass = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["prs", "--help"])
        .output()?;
    assert!(compass.status.success());
    assert!(String::from_utf8_lossy(&compass.stdout).contains("compass prs"));
    let path = std::env::join_paths([bin, PathBuf::from("/usr/bin"), PathBuf::from("/bin")])?;
    let dashboard = Command::new(env!("CARGO_BIN_EXE_compass"))
        .current_dir(directory.path())
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .args(["prs", "--base", "v8"])
        .output()?;
    let stdout = String::from_utf8_lossy(&dashboard.stdout);
    assert!(stdout.contains("compass prs  ·  base: v8"));
    assert!(!stdout.contains("graphify prs  ·"));
    Ok(())
}

#[test]
fn prs_triage_uses_the_native_provider_and_exact_prompt_shape() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let bin = seed(directory.path())?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap_or_else(|_| std::process::abort());
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap_or_else(|_| std::process::abort());
        let mut request = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = stream
                .read(&mut buffer)
                .unwrap_or_else(|_| std::process::abort());
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let header_end = header_end + 4;
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or_default();
            if request.len() >= header_end.saturating_add(content_length) {
                break;
            }
        }
        let request = String::from_utf8_lossy(&request).into_owned();
        let body = r##"{"choices":[{"message":{"content":"#102 — Repair CI first."}}],"usage":{"prompt_tokens":42,"completion_tokens":7}}"##;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .unwrap_or_else(|_| std::process::abort());
        let _ = sender.send(request);
    });
    let path = std::env::join_paths([bin, PathBuf::from("/usr/bin"), PathBuf::from("/bin")])?;
    let output = Command::new(env!("CARGO_BIN_EXE_graphify"))
        .current_dir(directory.path())
        .env("PATH", path)
        .env("NO_COLOR", "1")
        .env("GRAPHIFY_TRIAGE_BACKEND", "openai")
        .env("GRAPHIFY_TRIAGE_MODEL", "triage-test-model")
        .env("OPENAI_API_KEY", "test-key")
        .env("OPENAI_BASE_URL", format!("http://{address}/v1"))
        .env("GRAPHIFY_MAX_RETRIES", "0")
        .args(["prs", "--triage", "--base", "v8", "--graph", "missing.data"])
        .output()?;
    server.join().map_err(|_| "fake provider panicked")?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Triage (openai / triage-test-model)"));
    assert!(stdout.contains("  #102 — Repair CI first."));
    let request = receiver.recv_timeout(Duration::from_secs(1))?;
    assert!(request.starts_with("POST /v1/chat/completions HTTP/1.1"));
    assert!(request.contains("rank them by review priority"));
    assert!(request.contains("PR #102 [CI-FAIL]"));
    Ok(())
}
