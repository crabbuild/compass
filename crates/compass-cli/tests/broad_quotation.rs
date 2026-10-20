mod support;

use std::error::Error;
use std::ffi::OsString;

use compass_cli::{Frontend, run};
use serde_json::Value;

#[test]
fn broad_quotation_keeps_distinctive_topics_below_explicit_actions() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    std::fs::create_dir(&root)?;
    let source = concat!(
        "fn opaque(search_terms: SearchTerms) {\n",
        "    // Uncommon identifiers receive isolated quotas.\n",
        "    // Read search terms with stable ordering.\n",
        "    let result = search_terms;\n",
        "}\n",
        "fn ordinary_read_search_terms(search_terms: SearchTerms) {}\n",
        "fn other_read_search_terms(search_terms: SearchTerms) {}\n",
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
    for (question, expected) in [
        (
            "Where are uncommon search terms?",
            "Uncommon identifiers receive isolated quotas.",
        ),
        (
            "Where are uncommon search terms read?",
            "Read search terms with stable ordering.",
        ),
    ] {
        let response = support::compass_command()
            .args(["query", question, "--graph"])
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
            .and_then(|rows| {
                rows.iter().find(|candidate| {
                    candidate["name"]
                        .as_str()
                        .is_some_and(|name| name.rsplit([':', '.']).next() == Some("opaque"))
                })
            })
            .ok_or("missing quotation owner")?;
        let quotation = candidate["excerpts"]
            .as_array()
            .and_then(|rows| rows.first())
            .ok_or("missing source quotation")?;
        assert_eq!(quotation["kind"], "callable_comment");
        let text = quotation["text"].as_str().ok_or("missing quotation text")?;
        assert!(text.contains(expected), "{question}: {candidate}");
        assert!(source.contains(text));
    }
    Ok(())
}
