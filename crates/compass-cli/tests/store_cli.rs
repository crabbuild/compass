use std::error::Error;
use std::fs;
use std::process::Command;

use compass_files::BuildGuard;
use serde_json::Value;

#[test]
fn store_status_backup_and_restore_are_end_to_end_validated() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(
        root.path().join("main.rs"),
        "fn main() { println!(\"ok\"); }\n",
    )?;
    let init = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--yes", "--store", "sqlite"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(
        init.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );

    let output = root.path().join("compass-out");
    let status = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "store",
            "status",
            output.to_str().ok_or("output path")?,
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        status.status.success(),
        "status: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let status_json: Value = serde_json::from_slice(&status.stdout)?;
    assert_eq!(status_json["schema"], "compass.store.status/1");
    assert_eq!(status_json["store"]["valid"], true);

    let backup = root.path().join("backup");
    let backup_result = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "store",
            "backup",
            output.to_str().ok_or("output path")?,
            "--output",
            backup.to_str().ok_or("backup path")?,
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        backup_result.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&backup_result.stdout),
        String::from_utf8_lossy(&backup_result.stderr)
    );
    assert!(backup.join("manifest.json").is_file());

    let restored = root.path().join("restored");
    let restore_result = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "store",
            "restore",
            "--from",
            backup.to_str().ok_or("backup path")?,
            "--into",
            restored.to_str().ok_or("restore path")?,
            "--format",
            "json",
        ])
        .output()?;
    assert!(
        restore_result.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&restore_result.stdout),
        String::from_utf8_lossy(&restore_result.stderr)
    );
    let restored_validation = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "store",
            "validate",
            restored.to_str().ok_or("restore path")?,
            "--format",
            "json",
        ])
        .output()?;
    assert!(restored_validation.status.success());
    let restored_json: Value = serde_json::from_slice(&restored_validation.stdout)?;
    assert_eq!(restored_json["valid"], true);
    let active_output = BuildGuard::resolve_active_directory(&output)?;
    assert_eq!(
        fs::read(active_output.join("graph.json"))?,
        fs::read(restored.join("graph.json"))?
    );
    Ok(())
}

#[test]
fn store_validate_rejects_a_corrupt_sidecar_without_touching_graph_json()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("main.rs"), "fn main() {}\n")?;
    let init = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["init", ".", "--yes", "--store", "sqlite"])
        .current_dir(root.path())
        .env_remove("COMPASS_OUT")
        .output()?;
    assert!(init.status.success());
    let output = root.path().join("compass-out");
    let active_output = BuildGuard::resolve_active_directory(&output)?;
    let graph = fs::read(active_output.join("graph.json"))?;
    fs::write(active_output.join("compass-store.sqlite3"), b"corrupt")?;
    let result = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["store", "validate", output.to_str().ok_or("output path")?])
        .output()?;
    assert!(!result.status.success());
    assert_eq!(fs::read(active_output.join("graph.json"))?, graph);
    Ok(())
}
