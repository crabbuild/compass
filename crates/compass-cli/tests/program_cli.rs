use std::error::Error;
use std::ffi::OsString;
use std::fs;

use compass_cli::{Frontend, run};
use compass_files::BuildGuard;
use compass_model::code_graph::{ExtractionStatus, GraphDocument};

fn arguments<const N: usize>(values: [&str; N]) -> Vec<OsString> {
    values.into_iter().map(OsString::from).collect()
}

#[test]
fn native_update_emits_and_reports_program_analysis() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(
        directory.path().join("lib.rs"),
        "pub fn helper() {}\npub fn run() { helper(); }\n",
    )?;
    let root = directory.path().to_string_lossy();
    let args = arguments(["update", root.as_ref(), "--no-cluster", "--no-viz"]);

    let cold = run(Frontend::Compass, args.clone());
    assert_eq!(cold.code, 0, "{}", cold.stderr);
    assert!(cold.stdout.contains(
        "Program analysis: 1 syntax analyzed, 0 syntax reused, 0 artifacts loaded, 0 artifacts reused, 0 artifact documents analyzed, 0 artifact documents reused, 1 modules, 2 summaries, 0 conflicts"
    ));
    assert!(
        BuildGuard::resolve_artifact(&directory.path().join("compass-out"), "program.json")?
            .is_file()
    );
    assert!(
        !directory
            .path()
            .join("compass-out/.compass_program.json")
            .exists()
    );

    let warm = run(Frontend::Compass, args);
    assert_eq!(warm.code, 0, "{}", warm.stderr);
    assert!(warm.stdout.contains(
        "Program analysis: 0 syntax analyzed, 1 syntax reused, 0 artifacts loaded, 0 artifacts reused, 0 artifact documents analyzed, 0 artifact documents reused, 1 modules, 2 summaries, 0 conflicts"
    ));
    Ok(())
}

#[test]
fn native_program_artifact_requires_a_nonempty_path() {
    for arguments in [
        vec!["update", "--program-artifact"],
        vec!["update", "--program-artifact="],
    ] {
        let outcome = run(Frontend::Compass, arguments.into_iter().map(OsString::from));
        assert_ne!(outcome.code, 0);
        assert!(
            outcome
                .stderr
                .contains("--program-artifact requires a path")
        );
    }
}

#[test]
fn native_update_enforces_the_configured_source_size_limit() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("healthy.rs"), "pub fn healthy() {}\n")?;
    fs::write(
        directory.path().join("generated.rs"),
        "pub fn generated() {}\n".repeat(64),
    )?;
    let root = directory.path().to_string_lossy();
    let outcome = run(
        Frontend::Compass,
        arguments([
            "update",
            root.as_ref(),
            "--max-source-bytes=64",
            "--no-cluster",
            "--no-viz",
        ]),
    );
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);

    let output = directory.path().join("compass-out");
    let graph_path = BuildGuard::resolve_artifact(&output, "graph.json")?;
    let graph = GraphDocument::load(&graph_path)?;
    let generated = graph
        .graph
        .files
        .iter()
        .find(|file| file.path == "generated.rs")
        .ok_or("missing oversized source inventory")?;
    assert_eq!(generated.extraction_status, ExtractionStatus::Partial);
    assert!(graph.graph.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "partial_extraction"
            && diagnostic
                .message
                .contains("configured 64 byte extraction limit")
    }));
    let program: serde_json::Value = serde_json::from_slice(&fs::read(
        BuildGuard::resolve_artifact(&output, "program.json")?,
    )?)?;
    assert!(
        program["program"]["modules"]
            .as_array()
            .is_some_and(|modules| {
                modules
                    .iter()
                    .all(|module| module["source_file"] != "generated.rs")
            })
    );
    Ok(())
}

#[test]
fn program_commands_inspect_explain_and_query_canonical_ir() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::create_dir(directory.path().join("src"))?;
    fs::write(
        directory.path().join("src/lib.rs"),
        "pub fn helper() {}\npub fn run() { helper(); }\n",
    )?;
    let root = directory.path().to_string_lossy();
    let built = run(
        Frontend::Compass,
        arguments(["update", root.as_ref(), "--no-cluster", "--no-viz"]),
    );
    assert_eq!(built.code, 0, "{}", built.stderr);
    let program = directory.path().join("compass-out/program.json");
    let program_arg = program.to_string_lossy();

    let summary = run(
        Frontend::Compass,
        arguments([
            "program",
            "summary",
            "--program",
            program_arg.as_ref(),
            "--format",
            "json",
        ]),
    );
    assert_eq!(summary.code, 0, "{}", summary.stderr);
    let summary_json: serde_json::Value = serde_json::from_str(&summary.stdout)?;
    assert_eq!(summary_json["functions"], 2);
    assert_eq!(summary_json["schema"], "http://crab.build/compass/v1");

    let functions = run(
        Frontend::Compass,
        arguments([
            "program",
            "functions",
            "--program",
            program_arg.as_ref(),
            "--name",
            "helper",
            "--format=json",
        ]),
    );
    assert_eq!(functions.code, 0, "{}", functions.stderr);
    let functions_json: serde_json::Value = serde_json::from_str(&functions.stdout)?;
    let symbol = functions_json[0]["symbol_id"]
        .as_str()
        .ok_or("missing helper symbol")?;
    assert!(functions_json[0]["graph_node_id"].is_string());
    assert_eq!(functions_json[0]["call_resolution_state"], "partial");
    assert_eq!(functions_json[0]["impact_eligible"], false);

    let shown = run(
        Frontend::Compass,
        [
            OsString::from("program"),
            OsString::from("show"),
            OsString::from(symbol),
            OsString::from("--program"),
            OsString::from(program_arg.as_ref()),
            OsString::from("--format=json"),
        ],
    );
    assert_eq!(shown.code, 0, "{}", shown.stderr);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&shown.stdout)?["function"]["name"],
        "helper"
    );

    let callers = run(
        Frontend::Compass,
        [
            OsString::from("program"),
            OsString::from("callers"),
            OsString::from(symbol),
            OsString::from("--program"),
            OsString::from(program_arg.as_ref()),
            OsString::from("--format=json"),
        ],
    );
    assert_eq!(callers.code, 0, "{}", callers.stderr);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&callers.stdout)?[0]["name"],
        "run"
    );

    let published_program =
        BuildGuard::resolve_artifact(&directory.path().join("compass-out"), "program.json")?;
    let document: serde_json::Value = serde_json::from_slice(&fs::read(published_program)?)?;
    let call = document["program"]["modules"][0]["functions"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|function| {
            function["blocks"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|block| block["operations"].as_array().into_iter().flatten())
        })
        .find(|operation| operation["kind"]["callee"] == "helper")
        .ok_or("missing helper call")?;
    let byte = call["kind"]["callee_anchor"]["start_byte"]
        .as_u64()
        .ok_or("missing call byte")?;
    let location = format!("src/lib.rs:{byte}");
    let explained = run(
        Frontend::Compass,
        [
            OsString::from("program"),
            OsString::from("explain-call"),
            OsString::from(location),
            OsString::from("--program"),
            OsString::from(program_arg.as_ref()),
            OsString::from("--format=json"),
        ],
    );
    assert_eq!(explained.code, 0, "{}", explained.stderr);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&explained.stdout)?[0]["call"]["callee"],
        "helper"
    );

    let call_graph = run(
        Frontend::Compass,
        [
            OsString::from("program"),
            OsString::from("call-graph"),
            OsString::from("--at"),
            OsString::from(format!("src/lib.rs:{byte}")),
            OsString::from("--direction"),
            OsString::from("both"),
            OsString::from("--depth"),
            OsString::from("2"),
            OsString::from("--program"),
            OsString::from(program_arg.as_ref()),
            OsString::from("--format=json"),
        ],
    );
    assert_eq!(call_graph.code, 0, "{}", call_graph.stderr);
    let call_graph_json: serde_json::Value = serde_json::from_str(&call_graph.stdout)?;
    assert_eq!(call_graph_json["schema"], "compass.program.call_graph/1");
    assert_eq!(call_graph_json["edges"][0]["resolution"], "resolved");
    assert_eq!(call_graph_json["edges"][0]["callee"], "helper");

    let queried = run(
        Frontend::Compass,
        [
            OsString::from("program"),
            OsString::from("query"),
            OsString::from(
                "MATCH (f) WHERE f.kind = 'program_function' RETURN f.symbol_id AS symbol, f.call_resolution_state AS resolution, f.impact_eligible AS impact",
            ),
            OsString::from("--program"),
            OsString::from(program_arg.as_ref()),
            OsString::from("--format=json"),
        ],
    );
    assert_eq!(queried.code, 0, "{}", queried.stderr);
    let queried_json: serde_json::Value = serde_json::from_str(&queried.stdout)?;
    assert_eq!(queried_json["rows"].as_array().map(Vec::len), Some(2));
    assert!(
        queried_json["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .all(|row| {
                row["resolution"]["value"] == "partial" && row["impact"]["value"] == false
            }),
        "{queried_json}"
    );

    let coverage = run(
        Frontend::Compass,
        arguments([
            "program",
            "coverage",
            "--program",
            program_arg.as_ref(),
            "--format=json",
        ]),
    );
    assert_eq!(coverage.code, 0, "{}", coverage.stderr);
    assert!(
        coverage.stdout.contains("\"state\": \"indeterminate\""),
        "{}",
        coverage.stdout
    );
    assert!(
        coverage
            .stdout
            .contains("\"capability\": \"call_resolution\""),
        "{}",
        coverage.stdout
    );
    Ok(())
}

#[test]
fn program_commands_reject_noncanonical_input() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("program.json");
    fs::write(&path, "{}")?;
    let path = path.to_string_lossy();
    let invalid = run(
        Frontend::Compass,
        arguments(["program", "summary", "--program", path.as_ref()]),
    );
    assert_eq!(invalid.code, 3);
    assert!(invalid.stderr.contains("invalid Program IR"));

    Ok(())
}
