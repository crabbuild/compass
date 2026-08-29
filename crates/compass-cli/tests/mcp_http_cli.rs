use std::error::Error;
use std::process::Command;

const WARNING: &str = "warning: --session-timeout is deprecated and ignored because MCP HTTP is stateless; it will be removed in Compass 0.5.0";

#[test]
fn deprecated_session_timeout_warns_and_preserves_cli_exit_taxonomy() -> Result<(), Box<dyn Error>>
{
    let runtime_failure = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args([
            "serve",
            "--transport=http",
            "--path=invalid",
            "--session-timeout=12.5",
        ])
        .output()?;
    assert_eq!(runtime_failure.status.code(), Some(1));
    let stderr = String::from_utf8(runtime_failure.stderr)?;
    assert!(stderr.contains(WARNING));
    assert!(stderr.contains("HTTP mount path must start with '/'"));

    let usage_failure = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["serve", "--session-timeout=not-a-number"])
        .output()?;
    assert_eq!(usage_failure.status.code(), Some(2));
    let stderr = String::from_utf8(usage_failure.stderr)?;
    assert!(stderr.contains("invalid float value"));
    assert!(!stderr.contains(WARNING));
    Ok(())
}
