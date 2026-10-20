mod support;

use std::error::Error;
use std::ffi::OsString;

use compass_cli::{Frontend, run};
use serde_json::Value;

#[test]
fn broad_spelling_hint_reaches_the_binary_as_a_low_source_lead() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    std::fs::create_dir(&root)?;
    std::fs::write(
        root.join("sample.py"),
        "def opaque():\n    # Format the fastest payloads.\n    return 0\n",
    )?;
    let output = directory.path().join("artifacts");
    let built = run(
        Frontend::Compass,
        [
            OsString::from("extract"),
            root.into_os_string(),
            OsString::from("--code-only"),
            OsString::from("--no-viz"),
            OsString::from("--max-workers"),
            OsString::from("1"),
            OsString::from("--out"),
            output.as_os_str().to_owned(),
        ],
    );
    assert_eq!(built.code, 0, "{}", built.stderr);
    let response = support::compass_command()
        .args(["query", "Where are fast payloads formatted?", "--graph"])
        .arg(output.join("compass-out/graph.json"))
        .args([
            "--view",
            "ranked",
            "--format",
            "json",
            "--budget",
            "800",
            "--timeout-ms",
            "30000",
        ])
        .output()?;
    assert!(response.status.success(), "{:?}", response);
    assert!(response.stdout.len() <= 3200);
    let answer: Value = serde_json::from_slice(&response.stdout)?;
    assert_eq!(answer["schema"], "compass.query.ranked-view/4");
    let candidate = answer["candidates"]
        .as_array()
        .and_then(|rows| rows.first())
        .ok_or("missing spelling inspection candidate")?;
    assert_eq!(candidate["name"], "sample.opaque");
    assert_eq!(candidate["relevance"], "low");
    assert!(
        candidate["matchedTerms"]
            .as_array()
            .is_some_and(|terms| terms.iter().any(|term| term == "fast"))
    );
    assert!(candidate["reasons"].as_array().is_some_and(|reasons| {
        reasons
            .iter()
            .filter_map(Value::as_str)
            .any(|reason| reason.contains("fast -> fastest"))
    }));
    Ok(())
}
