use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use trail_cargo::introspect_cargo;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

#[test]
fn cargo_workspace_graph_matches_python_oracle() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    std::fs::create_dir_all(root.join("crates/app"))?;
    std::fs::create_dir_all(root.join("crates/core"))?;
    std::fs::create_dir_all(root.join("crates/unused"))?;
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"root-package\"\nversion = \"0.1.0\"\n[workspace]\nmembers = [\"crates/*\"]\n",
    )?;
    std::fs::write(
        root.join("crates/app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n[dependencies]\ncore_alias = { workspace = true, package = \"core\" }\nserde = \"1\"\n",
    )?;
    std::fs::write(
        root.join("crates/core/Cargo.toml"),
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\n[dependencies]\nroot-package = { path = \"../..\" }\n",
    )?;
    std::fs::write(
        root.join("crates/unused/Cargo.toml"),
        "[package]\nversion = \"0.1.0\"\n",
    )?;

    let repository = repository_root();
    let output = Command::new(repository.join(".venv/bin/python"))
        .env("PYTHONPATH", &repository)
        .args([
            "-c",
            "import json,sys; from graphify.cargo_introspect import introspect_cargo; print(json.dumps(introspect_cargo(sys.argv[1]), sort_keys=True))",
        ])
        .arg(root)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let expected: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let actual = introspect_cargo(root)?.into_fragment();
    assert_eq!(actual.get("nodes"), expected.get("nodes"));
    assert_eq!(actual.get("edges"), expected.get("edges"));
    Ok(())
}
