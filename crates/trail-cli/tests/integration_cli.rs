use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn python_executable(repo: &Path) -> PathBuf {
    if cfg!(windows) {
        repo.join(".venv/Scripts/python.exe")
    } else {
        repo.join(".venv/bin/python")
    }
}

fn run(
    executable: &Path,
    repo: &Path,
    root: &Path,
    arguments: &[&str],
    stdin: &str,
    environment: &[(&str, &str)],
) -> Result<Output, Box<dyn Error>> {
    let mut command = Command::new(executable);
    if executable == python_executable(repo) {
        command.args(["-m", "graphify"]);
        command.env("PYTHONPATH", repo);
    }
    command
        .args(arguments)
        .current_dir(root)
        .env_remove("GRAPHIFY_OUT")
        .env_remove("GRAPHIFY_HOOK_STRICT")
        .env_remove("GRAPHIFY_HOOK_STRICT_TTL")
        .envs(environment.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    if let Some(mut pipe) = child.stdin.take() {
        pipe.write_all(stdin.as_bytes())?;
    }
    Ok(child.wait_with_output()?)
}

fn compare(
    root: &Path,
    arguments: &[&str],
    stdin: &str,
    environment: &[(&str, &str)],
) -> Result<(Output, Output), Box<dyn Error>> {
    let repo = repository_root();
    let python = run(
        &python_executable(&repo),
        &repo,
        root,
        arguments,
        stdin,
        environment,
    )?;
    let native = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        root,
        arguments,
        stdin,
        environment,
    )?;
    Ok((python, native))
}

#[test]
fn hook_runtime_commands_match_python_oracle() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = directory.path().join("graphify-out");
    std::fs::create_dir_all(&output)?;
    std::fs::write(output.join("graph.json"), "{}")?;
    let cases = [
        (
            vec!["hook-guard", "search"],
            r#"{"tool_input":{"command":"rg symbol src"}}"#,
        ),
        (
            vec!["hook-guard", "read"],
            r#"{"tool_input":{"file_path":"src/app.py"}}"#,
        ),
        (vec!["hook-guard", "gemini"], ""),
        (vec!["hook-guard", "bogus"], r#"{"tool_input":{}}"#),
        (vec!["hook-check"], ""),
    ];
    for (arguments, stdin) in cases {
        let (python, native) = compare(directory.path(), &arguments, stdin, &[])?;
        assert_eq!(native.status.code(), python.status.code(), "{arguments:?}");
        assert_eq!(native.stdout, python.stdout, "{arguments:?}");
        assert_eq!(native.stderr, python.stderr, "{arguments:?}");
    }
    Ok(())
}

#[test]
fn strict_read_guard_matches_python_and_denies_once() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let source = root.join("src/app.py");
    std::fs::create_dir_all(source.parent().unwrap_or(root))?;
    std::fs::write(&source, "def app(): pass\n")?;
    let output = root.join("graphify-out");
    std::fs::create_dir_all(&output)?;
    std::fs::write(
        output.join("manifest.json"),
        r#"{"src/app.py":{"mtime":1}}"#,
    )?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(output.join("graph.json"), r#"{"nodes":[],"links":[]}"#)?;
    let stdin = json!({
        "session_id":"oracle",
        "tool_name":"Read",
        "tool_input":{"file_path":source},
    })
    .to_string();

    let python_root = root.join("python");
    let native_root = root.join("native");
    copy_fixture(root, &python_root)?;
    copy_fixture(root, &native_root)?;
    let python_source = python_root
        .join("src/app.py")
        .to_string_lossy()
        .into_owned();
    let native_source = native_root
        .join("src/app.py")
        .to_string_lossy()
        .into_owned();
    let python_input = stdin.replace(&source.to_string_lossy().to_string(), &python_source);
    let native_input = stdin.replace(&source.to_string_lossy().to_string(), &native_source);
    let repo = repository_root();
    let args = ["hook-guard", "read", "--strict"];
    let python = run(
        &python_executable(&repo),
        &repo,
        &python_root,
        &args,
        &python_input,
        &[],
    )?;
    let native = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &args,
        &native_input,
        &[],
    )?;
    assert_eq!(native.stdout, python.stdout);
    let native_second = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &args,
        &native_input,
        &[],
    )?;
    assert!(String::from_utf8(native_second.stdout)?.contains("additionalContext"));
    Ok(())
}

fn copy_fixture(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    std::fs::create_dir_all(destination.join("src"))?;
    std::fs::create_dir_all(destination.join("graphify-out"))?;
    std::fs::copy(source.join("src/app.py"), destination.join("src/app.py"))?;
    std::fs::copy(
        source.join("graphify-out/manifest.json"),
        destination.join("graphify-out/manifest.json"),
    )?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::copy(
        source.join("graphify-out/graph.json"),
        destination.join("graphify-out/graph.json"),
    )?;
    Ok(())
}

#[test]
fn check_update_and_merge_driver_match_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let check_root = directory.path().join("check");
    std::fs::create_dir_all(check_root.join("graphify-out"))?;
    std::fs::write(check_root.join("graphify-out/needs_update"), "1")?;
    let check_path = check_root.to_string_lossy().into_owned();
    let (python, native) = compare(directory.path(), &["check-update", &check_path], "", &[])?;
    assert_eq!(native.stdout, python.stdout);
    assert_eq!(native.stderr, python.stderr);

    let python_root = directory.path().join("merge-python");
    let native_root = directory.path().join("merge-native");
    std::fs::create_dir_all(&python_root)?;
    std::fs::create_dir_all(&native_root)?;
    let current = json!({
        "directed":true,"multigraph":false,"graph":{"left":1},
        "nodes":[{"id":"a","label":"café","old":true},{"id":"b"}],
        "links":[{"source":"a","target":"b","relation":"old","weight":1}]
    });
    let other = json!({
        "directed":true,"multigraph":false,"graph":{"right":2},
        "nodes":[{"id":"a","label":"café","new":true},{"id":"c"}],
        "links":[{"source":"a","target":"b","relation":"new"},{"source":"b","target":"c"}]
    });
    for root in [&python_root, &native_root] {
        std::fs::write(root.join("base.json"), "{}")?;
        std::fs::write(root.join("current.json"), serde_json::to_vec(&current)?)?;
        std::fs::write(root.join("other.json"), serde_json::to_vec(&other)?)?;
    }
    let repo = repository_root();
    let arguments = ["merge-driver", "base.json", "current.json", "other.json"];
    let python = run(
        &python_executable(&repo),
        &repo,
        &python_root,
        &arguments,
        "",
        &[],
    )?;
    let native = run(
        Path::new(env!("CARGO_BIN_EXE_graphify")),
        &repo,
        &native_root,
        &arguments,
        "",
        &[],
    )?;
    assert_eq!(native.status.code(), python.status.code());
    assert_eq!(native.stdout, python.stdout);
    assert_eq!(native.stderr, python.stderr);
    assert_eq!(
        std::fs::read(native_root.join("current.json"))?,
        std::fs::read(python_root.join("current.json"))?
    );
    let merged: Value = serde_json::from_slice(&std::fs::read(native_root.join("current.json"))?)?;
    assert_eq!(merged["nodes"].as_array().map(Vec::len), Some(3));
    Ok(())
}

#[cfg(unix)]
#[test]
fn clone_command_matches_python_without_network() -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir()?;
    let bin = directory.path().join("bin");
    std::fs::create_dir_all(&bin)?;
    let git = bin.join("git");
    std::fs::write(&git, "#!/bin/sh\nexit 0\n")?;
    let mut permissions = std::fs::metadata(&git)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&git, permissions)?;
    let destination = directory.path().join("checkout");
    let destination_text = destination.to_string_lossy().into_owned();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let arguments = [
        "clone",
        "git@github.com:Graphify-Labs/graphify.git",
        "--branch",
        "main",
        "--out",
        &destination_text,
    ];
    let (python, native) = compare(directory.path(), &arguments, "", &[("PATH", &path)])?;
    assert_eq!(native.status.code(), python.status.code());
    assert_eq!(native.stdout, python.stdout);
    assert_eq!(native.stderr, python.stderr);

    let invalid = [
        "clone",
        "https://github.com/a/b",
        "--branch",
        "--upload-pack=x",
    ];
    let (python, native) = compare(directory.path(), &invalid, "", &[])?;
    assert_eq!(native.status.code(), python.status.code());
    assert_eq!(native.stdout, python.stdout);
    assert_eq!(native.stderr, python.stderr);
    Ok(())
}
