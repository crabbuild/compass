mod support;

use std::error::Error;
use std::ffi::OsString;

use compass_cli::{Frontend, run};
use serde_json::Value;

#[test]
fn unrelated_comments_keep_the_complete_source_lead_first() -> Result<(), Box<dyn Error>> {
    for padding in [0, 250] {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("source");
        std::fs::create_dir(&root)?;
        let noise = (0..padding)
            .map(|n| format!("padding{n} "))
            .collect::<String>();
        let other = (0..35).map(|n| format!("context{n} ")).collect::<String>();
        let source = format!(
            "fn opaque() {{\n    // Format record errors.\n    // {noise}\n}}\nfn companion() {{\n    // Format record errors. {other}\n}}\n"
        );
        std::fs::write(root.join("sample.rs"), source)?;
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
            .args(["query", "Where are record errors formatted?", "--graph"])
            .arg(output.join("compass-out/graph.json"))
            .args([
                "--view",
                "ranked",
                "--format",
                "json",
                "--budget",
                "4096",
                "--timeout-ms",
                "30000",
            ])
            .output()?;
        assert!(response.status.success(), "{:?}", response);
        assert!(response.stdout.len() <= 16384);
        let answer: Value = serde_json::from_slice(&response.stdout)?;
        assert_eq!(answer["schema"], "compass.query.ranked-view/4");
        let candidate = answer["candidates"]
            .as_array()
            .and_then(|rows| rows.first())
            .ok_or("missing source lead")?;
        assert_eq!(
            candidate["name"]
                .as_str()
                .and_then(|name| name.rsplit([':', '.']).next()),
            Some("opaque"),
            "unrelated padding changes source-lead order: {answer}"
        );
        assert_eq!(candidate["relevance"], "low");
        assert!(candidate["excerpts"].as_array().is_some_and(|rows| {
            rows.iter().any(|quote| {
                quote["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Format record errors."))
            })
        }));
    }
    Ok(())
}
