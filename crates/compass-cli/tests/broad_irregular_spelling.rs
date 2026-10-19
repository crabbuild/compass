mod support;

use std::error::Error;
use std::ffi::OsString;
use std::path::Path;

use compass_cli::{Frontend, run};
use serde_json::Value;

fn ranked(graph: &Path, question: &str) -> Result<Value, Box<dyn Error>> {
    let response = support::compass_command()
        .args(["query", question, "--graph"])
        .arg(graph)
        .args([
            "--view",
            "ranked",
            "--format",
            "json",
            "--budget",
            "4000",
            "--timeout-ms",
            "30000",
        ])
        .output()?;
    assert!(response.status.success(), "{:?}", response);
    assert!(response.stdout.len() <= 16000);
    let answer: Value = serde_json::from_slice(&response.stdout)?;
    assert_eq!(answer["schema"], "compass.query.ranked-view/4");
    Ok(answer)
}

#[test]
fn ordinary_irregular_operations_reach_native_cli_without_rewriting_exact_names()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    std::fs::create_dir(&root)?;
    std::fs::write(
        root.join("sample.py"),
        "def rebuild_widgets(): pass\ndef build_widgets(): pass\ndef rebuild_gadgets(): pass\ndef rebuilt(): pass\ndef write_reports(): pass\ndef rewrite_reports(): pass\ndef written(): pass\ndef send_parcels(): pass\ndef resend_parcels(): pass\ndef sent(): pass\n",
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
    let graph = output.join("compass-out/graph.json");
    for (question, name, term) in [
        (
            "Where are widgets rebuilt?",
            "sample.rebuild_widgets",
            "rebuilt",
        ),
        (
            "Where are reports written?",
            "sample.write_reports",
            "written",
        ),
        ("Where are parcels sent?", "sample.send_parcels", "send"),
    ] {
        let answer = ranked(&graph, question)?;
        let first = answer["candidates"]
            .as_array()
            .and_then(|rows| rows.first())
            .ok_or("missing operation candidate")?;
        assert_eq!(first["name"], name, "{question}");
        assert!(
            first["matchedTerms"]
                .as_array()
                .is_some_and(|terms| terms.iter().any(|matched| matched == term)),
            "{question}"
        );
    }
    for name in ["rebuilt", "written", "sent"] {
        let answer = ranked(&graph, name)?;
        assert_eq!(answer["candidates"][0]["name"], format!("sample.{name}"));
        assert_eq!(answer["candidates"][0]["nameMatch"], "exact_name");
    }
    Ok(())
}
