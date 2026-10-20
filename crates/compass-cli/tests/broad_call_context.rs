use std::collections::BTreeSet;
use std::error::Error;
use std::ffi::OsString;

use compass_cli::{Frontend, run};
use compass_model::query_contract::DiscoveryQueryResponse;
use serde_json::Value;

#[test]
fn raw_broad_query_retains_downstream_calls_without_changing_ranked_candidates()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path().join("source");
    std::fs::create_dir(&root)?;
    for (name, text) in [
        ("pyproject.toml", "[project]\nname = 'example'\n"),
        (
            "factory.py",
            "from validation import intact\n\ndef open_snapshot(path):\n    if not intact(path):\n        return create_snapshot(path)\n    return path.read_text()\n\ndef create_snapshot(path):\n    path.write_text('{}')\n    return '{}'\n",
        ),
        (
            "validation.py",
            "import json\n\ndef intact(path):\n    try:\n        json.loads(path.read_text())\n    except (OSError, ValueError):\n        return False\n    return True\n",
        ),
        ("disconnected.py", "def intact(path): return False\n"),
        (
            "test_recovery.py",
            "from factory import open_snapshot\n\ndef test_damaged_local_indexes_are_checked_and_restored(path):\n    assert open_snapshot(path) == '{}'\n",
        ),
    ] {
        std::fs::write(root.join(name), text)?;
    }
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
    let query = |extra: &[&str]| -> Result<String, Box<dyn Error>> {
        let mut arguments = vec![
            OsString::from("query"),
            OsString::from("Where are damaged local indexes checked and restored?"),
            OsString::from("--graph"),
            output.join("compass-out/graph.json").into_os_string(),
            OsString::from("--format"),
            OsString::from("json"),
        ];
        arguments.extend(extra.iter().map(OsString::from));
        let answer = run(Frontend::Compass, arguments);
        assert_eq!(answer.code, 0, "{}", answer.stderr);
        Ok(answer.stdout)
    };
    for depth in ["1", "2"] {
        let answer: DiscoveryQueryResponse =
            serde_json::from_str(&query(&["--max-depth", depth])?)?;
        let names: BTreeSet<_> = answer
            .nodes
            .iter()
            .map(|n| n.qualified_name.as_str())
            .collect();
        assert!(names.contains("factory.open_snapshot"));
        assert_eq!(names.contains("validation.intact"), depth == "2");
        assert_eq!(names.contains("factory.create_snapshot"), depth == "2");
        assert!(!names.contains("disconnected.intact"));
        assert!(!answer.truncated);
        assert!(answer.stats.expanded_relationships <= answer.limits.max_expanded_relationships);
    }
    let ranked: Value = serde_json::from_str(&query(&["--view", "ranked"])?)?;
    let context: Value = serde_json::from_str(&query(&["--view", "ranked", "--include-context"])?)?;
    assert_eq!(ranked["schema"], "compass.query.ranked-view/4");
    assert_eq!(context["candidates"], ranked["candidates"]);
    let rows = context["context"]["nodes"]
        .as_array()
        .ok_or("missing context nodes")?;
    assert!(
        rows.iter()
            .any(|n| n["qualifiedName"] == "factory.open_snapshot")
    );
    assert!(
        rows.iter()
            .all(|n| n["qualifiedName"] != "disconnected.intact")
    );
    // The ranked presentation retains its published one-hop contract even
    // when native raw discovery contains a deeper witnessed path.
    assert!(
        rows.iter()
            .all(|n| n["qualifiedName"] != "validation.intact")
    );
    Ok(())
}
