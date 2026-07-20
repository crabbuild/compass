use std::error::Error;
use std::process::Command;

#[test]
fn postgres_extract_rejects_invalid_dsn_without_echoing_it() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_trail"))
        .current_dir(directory.path())
        .args([
            "graph",
            "extract",
            "--postgres",
            "not a DSN password=top-secret",
            "--code-only",
        ])
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert_eq!(stderr, "error: invalid PostgreSQL DSN\n");
    assert!(!stderr.contains("top-secret"));
    Ok(())
}

#[test]
fn postgres_connection_failure_redacts_credentials() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let secret = "postgresql://trail-user:top-secret@127.0.0.1:1/trail?connect_timeout=1";
    let output = Command::new(env!("CARGO_BIN_EXE_trail"))
        .current_dir(directory.path())
        .args(["graph", "extract", "--postgres", secret, "--code-only"])
        .output()?;
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.starts_with("error: could not connect to PostgreSQL:"));
    assert!(!stderr.contains("top-secret"));
    assert!(!stderr.contains(secret));
    Ok(())
}
