mod support;

use std::error::Error;
use std::process::Command;

#[test]
fn compass_add_help_is_namespaced() -> Result<(), Box<dyn Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_compass"))
        .args(["add", "--help"])
        .output()?;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("compass add"));
    Ok(())
}
