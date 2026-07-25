use std::error::Error;
use std::fs;

use compass_cli::{Frontend, run};
use serde_json::Value;

#[test]
fn machine_update_emits_exactly_one_terminal_event() -> Result<(), Box<dyn Error>> {
    let project = tempfile::tempdir()?;
    fs::write(
        project.path().join("sample.rs"),
        "pub fn greet(name: &str) -> String { format!(\"hello {name}\") }\n",
    )?;
    let root = project.path().to_string_lossy().into_owned();
    let output = run(
        Frontend::Compass,
        ["update", &root, "--no-viz", "--events", "jsonl"].map(Into::into),
    );
    assert_eq!(output.code, 0, "{}", output.stderr);
    let events = output
        .stdout
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        events
            .iter()
            .filter(|event| event["terminal"] == true)
            .count(),
        1
    );
    assert_eq!(
        events.last().and_then(|event| event["state"].as_str()),
        Some("succeeded")
    );
    assert!(!output.stderr.is_empty());
    Ok(())
}
