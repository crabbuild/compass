use std::error::Error;
use std::ffi::OsString;
use std::fs;

use compass_cli::{Frontend, run};
use serde_json::{Value, json};

#[test]
fn top_level_call_graph_uses_structural_go_evidence_without_program_ir()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    fs::write(
        &graph_path,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": true,
            "graph": {},
            "nodes": [
                {
                    "id": "resolve",
                    "label": "ResolveControlPlaneTarget()",
                    "source_file": "cmd/entire/cli/auth/control_plane.go",
                    "source_location": "L42",
                    "symbol_kind": "function",
                    "language": "go",
                    "line_start": 42,
                    "line_end": 55
                },
                {
                    "id": "active",
                    "label": "activeContext()",
                    "source_file": "cmd/entire/cli/auth/control_plane.go",
                    "source_location": "L104",
                    "symbol_kind": "function",
                    "language": "go",
                    "line_start": 104,
                    "line_end": 114
                },
                {
                    "id": "target",
                    "label": "targetForContext()",
                    "source_file": "cmd/entire/cli/auth/control_plane.go",
                    "source_location": "L92",
                    "symbol_kind": "function",
                    "language": "go",
                    "line_start": 92,
                    "line_end": 98
                }
            ],
            "links": [
                {
                    "source": "resolve",
                    "target": "active",
                    "relation": "calls",
                    "source_file": "cmd/entire/cli/auth/control_plane.go",
                    "source_location": "L43",
                    "confidence": "EXTRACTED"
                },
                {
                    "source": "resolve",
                    "target": "target",
                    "relation": "calls",
                    "source_file": "cmd/entire/cli/auth/control_plane.go",
                    "source_location": "L54",
                    "confidence": "EXTRACTED"
                }
            ]
        }))?,
    )?;
    let graph = graph_path.to_string_lossy();
    let outcome = run(
        Frontend::Compass,
        arguments([
            "call-graph",
            "--file",
            "cmd/entire/cli/auth/control_plane.go",
            "--byte",
            "1683",
            "--line",
            "42",
            "--direction",
            "callees",
            "--depth",
            "1",
            "--graph",
            graph.as_ref(),
            "--format",
            "json",
        ]),
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let value: Value = serde_json::from_str(&outcome.stdout)?;
    assert_eq!(value["schema"], "compass.call_graph/1");
    assert_eq!(value["rootSymbol"], "resolve");
    assert_eq!(value["coverage"]["evidenceLayer"], "structural_graph");
    assert_eq!(value["edges"].as_array().map(Vec::len), Some(2));
    Ok(())
}

#[test]
fn top_level_call_graph_requires_complete_source_position() {
    let outcome = run(
        Frontend::Compass,
        arguments([
            "call-graph",
            "--file",
            "src/lib.rs",
            "--line",
            "4",
            "--format",
            "json",
        ]),
    );

    assert_eq!(outcome.code, 2);
    assert!(
        outcome
            .stderr
            .contains("--file, --byte, and --line must be provided together"),
        "{}",
        outcome.stderr
    );
}

fn arguments<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(OsString::from).collect()
}
