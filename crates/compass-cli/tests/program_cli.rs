use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::path::Path;

use compass_cli::{Frontend, run};

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
    assert!(directory.path().join("compass-out/program.json").is_file());
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
fn graphify_rejects_program_artifacts_and_never_enables_program_output()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    fs::write(directory.path().join("lib.rs"), "pub fn visible() {}\n")?;
    let root = directory.path().to_string_lossy();

    let rejected = run(
        Frontend::Graphify,
        arguments([
            "extract",
            root.as_ref(),
            "--code-only",
            "--program-artifact=index.scip",
        ]),
    );
    assert_ne!(rejected.code, 0);
    assert!(
        rejected
            .stderr
            .contains("--program-artifact is unsupported in Graphify compatibility mode")
    );

    let built = run(
        Frontend::Graphify,
        arguments(["extract", root.as_ref(), "--code-only", "--no-cluster"]),
    );
    assert_eq!(built.code, 0, "{}", built.stderr);
    assert!(!built.stdout.contains("Program analysis:"));
    assert!(!contains_program_json(directory.path()));
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

    let document: serde_json::Value = serde_json::from_slice(&fs::read(&program)?)?;
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
fn program_commands_reject_noncanonical_and_graphify_inputs() -> Result<(), Box<dyn Error>> {
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

    let graphify = run(
        Frontend::Graphify,
        arguments(["program", "summary", "--program", path.as_ref()]),
    );
    assert_ne!(graphify.code, 0);
    Ok(())
}

fn contains_program_json(root: &Path) -> bool {
    ["compass-out", "graphify-out"]
        .into_iter()
        .any(|output| root.join(output).join("program.json").exists())
}
