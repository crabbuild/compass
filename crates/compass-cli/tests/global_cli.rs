mod support;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::{Value, json};

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("GRAPHIFY_REPO_ROOT") {
        return PathBuf::from(root);
    }
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
    home: &Path,
    arguments: &[&str],
) -> Result<Output, Box<dyn Error>> {
    let mut command = support::command(executable);
    if executable == python_executable(repo) {
        command.args(["-m", "graphify"]);
        command.env("PYTHONPATH", repo);
    }
    Ok(command
        .args(arguments)
        .current_dir(home)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env_remove("GRAPHIFY_OUT")
        .output()?)
}

fn seed_graph(path: &Path, module: &str, local_id: &str) -> Result<(), Box<dyn Error>> {
    std::fs::write(
        path,
        serde_json::to_vec(&json!({
            "directed":false,
            "multigraph":false,
            "graph":{},
            "nodes":[
                {"label":module,"source_file":format!("src/{local_id}.py"),"id":local_id},
                {"label":"requests","id":"requests"}
            ],
            "links":[{"relation":"imports","source":local_id,"target":"requests"}]
        }))?,
    )?;
    Ok(())
}

fn normalize_stdout(bytes: &[u8], home: &Path) -> String {
    String::from_utf8_lossy(bytes).replace(&home.to_string_lossy().to_string(), "<HOME>")
}

fn normalize_manifest(path: &Path) -> Result<Value, Box<dyn Error>> {
    let mut value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    if let Some(repos) = value.get_mut("repos").and_then(Value::as_object_mut) {
        for info in repos.values_mut().filter_map(Value::as_object_mut) {
            info.insert(
                "added_at".to_owned(),
                Value::String("<timestamp>".to_owned()),
            );
        }
    }
    Ok(value)
}

#[test]
fn global_cli_lifecycle_matches_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let python_home = directory.path().join("python-home");
    let native_home = directory.path().join("native-home");
    std::fs::create_dir_all(&python_home)?;
    std::fs::create_dir_all(&native_home)?;
    let source_a = directory.path().join("a.json");
    let source_b = directory.path().join("b.json");
    seed_graph(&source_a, "ModA", "a")?;
    seed_graph(&source_b, "ModB", "b")?;
    let repo = repository_root();
    let python_exe = python_executable(&repo);
    let native_exe = support::compat_executable();

    for arguments in [
        vec![
            "global",
            "add",
            source_a.to_str().unwrap_or_default(),
            "--as",
            "repoA",
        ],
        vec![
            "global",
            "add",
            source_a.to_str().unwrap_or_default(),
            "--as",
            "repoA",
        ],
        vec![
            "global",
            "add",
            source_b.to_str().unwrap_or_default(),
            "--as",
            "repoB",
        ],
        vec!["global", "list"],
        vec!["global", "remove", "repoB"],
        vec!["global", "path"],
    ] {
        let python = run(&python_exe, &repo, &python_home, &arguments)?;
        let native = run(native_exe, &repo, &native_home, &arguments)?;
        assert_eq!(native.status.code(), python.status.code(), "{arguments:?}");
        assert_eq!(
            normalize_stdout(&native.stdout, &native_home),
            normalize_stdout(&python.stdout, &python_home),
            "{arguments:?}"
        );
        assert_eq!(native.stderr, python.stderr, "{arguments:?}");
    }

    let python_graph = python_home.join(".graphify/global-graph.json");
    let native_graph = native_home.join(".graphify/global-graph.json");
    assert_eq!(std::fs::read(&native_graph)?, std::fs::read(&python_graph)?);
    assert_eq!(
        normalize_manifest(&native_home.join(".graphify/global-manifest.json"))?,
        normalize_manifest(&python_home.join(".graphify/global-manifest.json"))?
    );
    Ok(())
}
