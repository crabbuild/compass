use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;

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

fn only_memory_file(root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let mut files = std::fs::read_dir(root.join("graphify-out/memory"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    files.sort();
    if files.len() != 1 {
        return Err(format!("expected one memory file, got {}", files.len()).into());
    }
    Ok(files.remove(0))
}

fn normalize_timestamp(value: &str) -> Result<String, Box<dyn Error>> {
    let filename = Regex::new(r"query_\d{8}_\d{6}_")?;
    let date = Regex::new(r#"date: "\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{6})?\+00:00""#)?;
    Ok(date
        .replace_all(
            &filename.replace_all(value, "query_<timestamp>_"),
            "date: \"<timestamp>\"",
        )
        .into_owned())
}

#[test]
fn save_result_cli_matches_python_artifact() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let python_root = directory.path().join("python");
    let native_root = directory.path().join("native");
    std::fs::create_dir_all(&python_root)?;
    std::fs::create_dir_all(&native_root)?;
    let python_answer = python_root.join("answer.txt");
    let native_answer = native_root.join("answer.txt");
    let answer = "  line one\nline two with a \"quote\"\n  ";
    std::fs::write(&python_answer, answer)?;
    std::fs::write(&native_answer, answer)?;
    let common = [
        "--question",
        "path is C:\\Users and a \"quote\"?",
        "--type",
        "explain",
        "--nodes",
        "Node\"With\\Quote",
        "AuthMiddleware",
        "--outcome",
        "corrected",
        "--correction",
        "line1\nline2",
    ];
    let repo = repository_root();
    let python = Command::new(python_executable(&repo))
        .args(["-m", "graphify", "save-result"])
        .args(common)
        .args(["--answer-file"])
        .arg(&python_answer)
        .current_dir(&python_root)
        .env("PYTHONPATH", &repo)
        .output()?;
    assert!(
        python.status.success(),
        "{}",
        String::from_utf8_lossy(&python.stderr)
    );
    let native = Command::new(env!("CARGO_BIN_EXE_graphify"))
        .arg("save-result")
        .args(common)
        .args(["--answer-file"])
        .arg(&native_answer)
        .current_dir(&native_root)
        .output()?;
    assert!(
        native.status.success(),
        "{}",
        String::from_utf8_lossy(&native.stderr)
    );
    assert_eq!(
        normalize_timestamp(&String::from_utf8(native.stdout)?)?,
        normalize_timestamp(&String::from_utf8(python.stdout)?)?
    );
    let python_memory = only_memory_file(&python_root)?;
    let native_memory = only_memory_file(&native_root)?;
    assert_eq!(
        normalize_timestamp(
            &python_memory
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )?,
        normalize_timestamp(
            &native_memory
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        )?
    );
    assert_eq!(
        normalize_timestamp(&std::fs::read_to_string(python_memory)?)?,
        normalize_timestamp(&std::fs::read_to_string(native_memory)?)?
    );
    Ok(())
}
