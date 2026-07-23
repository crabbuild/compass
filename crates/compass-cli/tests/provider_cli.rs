mod support;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn run_python(repo: &Path, home: &Path, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(Command::new(python_executable(repo))
        .args(["-m", "graphify"])
        .args(arguments)
        .current_dir(repo)
        .env("PYTHONPATH", repo)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .output()?)
}

fn run_native(repo: &Path, home: &Path, arguments: &[&str]) -> Result<Output, Box<dyn Error>> {
    Ok(support::compat_command()
        .args(arguments)
        .current_dir(repo)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .output()?)
}

fn assert_same(python: &Output, native: &Output) {
    assert_eq!(
        native.status.code(),
        python.status.code(),
        "native stderr: {}\npython stderr: {}",
        String::from_utf8_lossy(&native.stderr),
        String::from_utf8_lossy(&python.stderr)
    );
    assert_eq!(native.stdout, python.stdout);
    assert_eq!(native.stderr, python.stderr);
}

#[test]
fn provider_cli_round_trip_matches_python() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let python_home = directory.path().join("python-home");
    let native_home = directory.path().join("native-home");
    std::fs::create_dir_all(&python_home)?;
    std::fs::create_dir_all(&native_home)?;
    let repo = repository_root();
    let add = [
        "provider",
        "add",
        "nvidia",
        "--base-url",
        "https://integrate.api.nvidia.com/v1",
        "--default-model",
        "minimaxai/minimax-m2.7",
        "--env-key",
        "NVIDIA_API_KEY",
        "--pricing-input",
        "0.25",
        "--pricing-output",
        "1.5",
    ];
    let python = run_python(&repo, &python_home, &add)?;
    let native = run_native(&repo, &native_home, &add)?;
    assert_same(&python, &native);
    assert_eq!(
        std::fs::read(python_home.join(".graphify/providers.json"))?,
        std::fs::read(native_home.join(".graphify/providers.json"))?
    );

    for arguments in [
        vec!["provider", "list"],
        vec!["provider", "show", "nvidia"],
        vec!["provider"],
        vec!["provider", "remove", "nvidia"],
        vec!["provider", "list"],
    ] {
        let python = run_python(&repo, &python_home, &arguments)?;
        let native = run_native(&repo, &native_home, &arguments)?;
        assert_same(&python, &native);
    }
    assert_eq!(
        std::fs::read(python_home.join(".graphify/providers.json"))?,
        std::fs::read(native_home.join(".graphify/providers.json"))?
    );

    let unsafe_add = [
        "provider",
        "add",
        "unsafe",
        "--base-url=file:///etc/passwd",
        "--default-model=m",
        "--env-key=K",
    ];
    let python = run_python(&repo, &python_home, &unsafe_add)?;
    let native = run_native(&repo, &native_home, &unsafe_add)?;
    assert_same(&python, &native);

    let plaintext_add = [
        "provider",
        "add",
        "plain",
        "--base-url=http://example.com/v1",
        "--default-model=m",
        "--env-key=K",
    ];
    let python = run_python(&repo, &python_home, &plaintext_add)?;
    let native = run_native(&repo, &native_home, &plaintext_add)?;
    assert_same(&python, &native);
    assert_eq!(
        std::fs::read(python_home.join(".graphify/providers.json"))?,
        std::fs::read(native_home.join(".graphify/providers.json"))?
    );

    let unicode_add = [
        "provider",
        "add",
        "本地",
        "--base-url=https://example.test/v1",
        "--default-model=模型",
        "--env-key=MODEL_KEY",
    ];
    let python = run_python(&repo, &python_home, &unicode_add)?;
    let native = run_native(&repo, &native_home, &unicode_add)?;
    assert_same(&python, &native);
    let python = run_python(&repo, &python_home, &["provider", "show", "本地"])?;
    let native = run_native(&repo, &native_home, &["provider", "show", "本地"])?;
    assert_same(&python, &native);
    assert_eq!(
        std::fs::read(python_home.join(".graphify/providers.json"))?,
        std::fs::read(native_home.join(".graphify/providers.json"))?
    );
    Ok(())
}
