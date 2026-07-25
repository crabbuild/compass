use std::error::Error;

use compass_cli::{Frontend, run};
use serde_json::Value;

#[test]
fn capabilities_reports_versioned_ide_contracts() -> Result<(), Box<dyn Error>> {
    let output = run(
        Frontend::Compass,
        ["capabilities", "--format", "json"].map(Into::into),
    );
    assert_eq!(output.code, 0, "{}", output.stderr);
    let value: Value = serde_json::from_str(&output.stdout)?;
    assert_eq!(value["schema"], "compass.ide.capabilities/1");
    assert_eq!(value["contracts"]["graph_viewer"], "compass.viewer.graph/1");
    assert!(value["compass_version"].is_string());
    Ok(())
}
