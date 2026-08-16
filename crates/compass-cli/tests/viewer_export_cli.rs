mod support;

use std::error::Error;
use std::process::Command;

use serde_json::{Value, json};

#[test]
fn cluster_only_preserves_the_typed_graph_used_by_orientation_export() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    std::fs::write(
        directory.path().join("src/lib.rs"),
        "pub fn caller() {\n    target();\n    target();\n}\nfn target() {}\n",
    )?;
    let build = support::compass_command()
        .args(["update", ".", "--no-viz"])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        build.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let clustered = support::compass_command()
        .args([
            "cluster-only",
            ".",
            "--no-viz",
            "--no-label",
            "--min-community-size=1",
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        clustered.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&clustered.stderr)
    );
    let active = compass_files::BuildGuard::resolve_current_snapshot_directory(
        &directory.path().join("compass-out"),
    )?;
    let typed = compass_model::code_graph::GraphDocument::load(&active.join("graph.json"))?;
    assert_eq!(typed.graph.schema, "compass.graph/1");
    assert_eq!(
        typed
            .links
            .iter()
            .filter(|edge| edge.kind == compass_model::code_graph::EdgeKind::Calls)
            .count(),
        2
    );

    let exported = support::compass_command()
        .args(["export", "orientation-json"])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        exported.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let orientation: Value = serde_json::from_slice(&exported.stdout)?;
    assert_eq!(orientation["schema"], "compass.orientation/2");
    assert_eq!(orientation["graphSummary"]["edges"], typed.links.len());
    Ok(())
}

#[test]
fn orientation_json_export_is_bound_to_the_selected_graph_generation() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    std::fs::write(
        directory.path().join("src/lib.rs"),
        "pub fn caller() { target(); }\nfn target() {}\n",
    )?;
    let build = support::compass_command()
        .args(["update", ".", "--no-viz"])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        build.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let exported = support::compass_command()
        .args(["export", "orientation-json"])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        exported.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let orientation: Value = serde_json::from_slice(&exported.stdout)?;
    assert_eq!(orientation["schema"], "compass.orientation/2");
    assert!(orientation["evidenceStatus"]["generationId"].is_string());

    let active = compass_files::BuildGuard::resolve_current_snapshot_directory(
        &directory.path().join("compass-out"),
    )?;
    let orientation_path = active.join("orientation.json");
    let detached = directory.path().join("detached");
    std::fs::create_dir(&detached)?;
    std::fs::copy(active.join("graph.json"), detached.join("graph.json"))?;
    std::fs::copy(&orientation_path, detached.join("orientation.json"))?;
    let detached_export = support::compass_command()
        .args([
            "export",
            "orientation-json",
            "--graph",
            detached.join("graph.json").to_string_lossy().as_ref(),
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        detached_export.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&detached_export.stderr)
    );
    let detached_graph_path = detached.join("graph.json");
    let mut changed_topology: Value =
        serde_json::from_slice(&std::fs::read(&detached_graph_path)?)?;
    let nodes = changed_topology["nodes"]
        .as_array_mut()
        .ok_or("fixture graph did not contain nodes")?;
    let mut changed_communities = 0_usize;
    for node in nodes {
        if let Some(community) = node
            .as_object_mut()
            .and_then(|node| node.get_mut("community"))
            .and_then(Value::as_object_mut)
        {
            community.insert("label".to_owned(), json!("Changed without changing counts"));
            changed_communities += 1;
        }
    }
    assert!(changed_communities > 0);
    std::fs::write(
        &detached_graph_path,
        serde_json::to_vec_pretty(&changed_topology)?,
    )?;
    let topology_rejected = support::compass_command()
        .args([
            "export",
            "orientation-json",
            "--graph",
            detached_graph_path.to_string_lossy().as_ref(),
        ])
        .current_dir(directory.path())
        .output()?;
    assert_ne!(topology_rejected.status.code(), Some(0));
    let topology_error = String::from_utf8_lossy(&topology_rejected.stderr);
    assert!(
        topology_error.contains("artifact-set identity does not match"),
        "{topology_error}"
    );

    let mut mismatched: Value = serde_json::from_slice(&std::fs::read(&orientation_path)?)?;
    mismatched["evidenceStatus"]["generationId"] = json!("sha256:not-this-graph");
    std::fs::write(&orientation_path, serde_json::to_vec_pretty(&mismatched)?)?;
    let rejected = support::compass_command()
        .args(["export", "orientation-json"])
        .current_dir(directory.path())
        .output()?;
    assert_ne!(rejected.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&rejected.stderr)
            .contains("does not match the selected graph generation")
    );
    Ok(())
}

#[test]
fn viewer_json_exposes_the_same_versioned_graph_model() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": [
                {"id":"run","label":"run","source_file":"src/lib.rs","line_start":3},
                {"id":"helper","label":"helper"}
            ],
            "links": [
                {"source":"run","target":"helper","relation":"calls","confidence":"EXTRACTED"}
            ]
        }))?,
    )?;
    let output = support::compass_command()
        .args([
            "export",
            "viewer-json",
            "--graph",
            graph.to_string_lossy().as_ref(),
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["schema"], "compass.viewer.graph/1");
    assert_eq!(value["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["edges"][0]["relation"], "calls");
    assert_eq!(value["edges"][0]["source"], "run");
    assert_eq!(value["nodes"][0]["source"]["file"], "src/lib.rs");
    Ok(())
}

#[test]
fn canonical_json_exports_one_complete_community() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {"hyperedges":[
                {"id":"inside","nodes":["run","helper"]},
                {"id":"cross","nodes":["run","other"]}
            ]},
            "nodes": [
                {"id":"run","label":"run","community":7,"source_file":"src/lib.rs","line_start":3},
                {"id":"helper","label":"helper","community":7},
                {"id":"other","label":"other","community":8}
            ],
            "links": [
                {"source":"run","target":"helper","relation":"calls","confidence":"EXTRACTED"},
                {"source":"run","target":"other","relation":"calls","confidence":"INFERRED"}
            ]
        }))?,
    )?;
    let output = support::compass_command()
        .args([
            "export",
            "json",
            "--graph",
            graph.to_string_lossy().as_ref(),
            "--community",
            "7",
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["schema"], "compass.viewer.graph/1");
    assert_eq!(value["stats"]["aggregated"], false);
    assert_eq!(value["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["edges"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["hyperedges"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["hyperedges"][0]["id"], "inside");

    let unsupported = support::compass_command()
        .args([
            "export",
            "html",
            "--graph",
            graph.to_string_lossy().as_ref(),
            "--community",
            "7",
        ])
        .current_dir(directory.path())
        .output()?;
    assert_ne!(unsupported.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("only valid with export json"));
    Ok(())
}

#[test]
fn workbench_json_preserves_requested_view_order_and_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {"schema":"compass.graph/1"},
            "nodes": [
                {"id":"caller","label":"caller","name":"caller","qualified_name":"caller","kind":"function","community":0,"source_file":"src/lib.rs","line_start":1},
                {"id":"target","label":"target","name":"target","qualified_name":"target","kind":"function","community":0,"source_file":"src/lib.rs","line_start":2},
                {"id":"test","label":"test","name":"test","qualified_name":"test","kind":"function","community":1,"source_file":"tests/test.rs","line_start":1}
            ],
            "links": [
                {"source":"caller","target":"target","relation":"calls","kind":"calls","confidence":"extracted"},
                {"source":"test","target":"target","relation":"tests","kind":"tests","confidence":"extracted"}
            ]
        }))?,
    )?;
    let output = support::compass_command()
        .args([
            "export",
            "workbench-json",
            "--graph",
            graph.to_string_lossy().as_ref(),
            "--code-graph",
            "--call-graph",
            "target",
            "--affected-graph",
            "target",
            "--relation",
            "calls",
            "--artifact-lens",
            "tests",
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["schema"], "compass.viewer.workbench/1");
    let kinds = value["views"]
        .as_array()
        .ok_or("missing workbench views")?
        .iter()
        .map(|view| view["kind"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(kinds, ["code", "call", "affected", "artifact"]);
    assert_eq!(value["defaultView"], "code");
    assert!(value["views"][0]["communityDetails"].is_object());
    assert!(value["views"][0].get("community_details").is_none());
    assert_eq!(value["views"][2]["model"]["nodes"][0]["depth"], 1);

    Ok(())
}

#[test]
fn impact_graph_export_uses_the_typed_query_contract() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    std::fs::create_dir_all(directory.path().join("src"))?;
    std::fs::write(
        directory.path().join("src/lib.rs"),
        "pub fn caller() { target(); }\npub fn target() {}\n",
    )?;
    let build = support::compass_command()
        .args(["update", ".", "--code-only", "--no-viz"])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        build.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let impact = support::compass_command()
        .args([
            "export",
            "workbench-json",
            "--impact-graph",
            "target",
            "--include-heuristic",
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        impact.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&impact.stderr)
    );
    let impact: Value = serde_json::from_slice(&impact.stdout)?;
    assert_eq!(impact["views"][0]["kind"], "impact");
    assert_eq!(impact["views"][0]["result"]["operation"], "impact");
    Ok(())
}

#[test]
fn html_export_embeds_one_workbench_for_multiple_views() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let initialized = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(directory.path())
        .status()?;
    assert!(initialized.success());
    let remote = Command::new("git")
        .args(["remote", "add", "origin", "git@gitlab.com:acme/compass.git"])
        .current_dir(directory.path())
        .status()?;
    assert!(remote.success());
    std::fs::create_dir_all(directory.path().join("src"))?;
    std::fs::write(directory.path().join("src/lib.rs"), "fn caller() {}\n")?;
    let added = Command::new("git")
        .args(["add", "src/lib.rs"])
        .current_dir(directory.path())
        .status()?;
    assert!(added.success());
    let committed = Command::new("git")
        .args([
            "-c",
            "user.name=Compass Test",
            "-c",
            "user.email=compass@example.com",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ])
        .current_dir(directory.path())
        .status()?;
    assert!(committed.success());
    let source_commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(directory.path())
        .output()?;
    assert!(source_commit.status.success());
    let source_commit = String::from_utf8(source_commit.stdout)?.trim().to_owned();
    let graph = directory.path().join("graph.json");
    let html = directory.path().join("review.html");
    std::fs::write(
        &graph,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {
                "schema":"compass.graph/1",
                "build":{"sourceCommit":source_commit}
            },
            "nodes": [
                {"id":"caller","label":"caller","kind":"function","community":0,"source_file":"src/lib.rs","line_start":1},
                {"id":"target","label":"target","kind":"function","community":0,"source_file":"src/lib.rs","line_start":2}
            ],
            "links": [
                {"source":"caller","target":"target","relation":"calls","kind":"calls","confidence":"extracted"}
            ]
        }))?,
    )?;
    let output = support::compass_command()
        .args([
            "export",
            "html",
            "--graph",
            graph.to_string_lossy().as_ref(),
            "--output",
            html.to_string_lossy().as_ref(),
            "--code-graph",
            "--call-graph",
            "target",
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let document = std::fs::read_to_string(html)?;
    assert_eq!(document.matches("id=\"compass-viewer-model\"").count(), 1);
    assert!(document.contains("compass.viewer.workbench/1"));
    assert!(document.contains("\"kind\":\"call\""));
    assert!(document.contains("id=\"compass-source-navigation\""));
    assert!(document.contains("\"provider\":\"gitlab\""));
    assert!(document.contains("\"repositoryUrl\":\"https://gitlab.com/acme/compass\""));
    assert!(document.contains(&format!("\"revision\":\"{source_commit}\"")));
    Ok(())
}

#[test]
fn export_rejects_unknown_and_view_incompatible_options_before_io() -> Result<(), Box<dyn Error>> {
    let unknown = support::compass_command()
        .args(["export", "html", "--typo"])
        .output()?;
    assert!(!unknown.status.success());
    assert!(
        String::from_utf8_lossy(&unknown.stderr).contains("unexpected export html argument --typo")
    );

    let incompatible = support::compass_command()
        .args(["export", "html", "--direction", "callers"])
        .output()?;
    assert!(!incompatible.status.success());
    assert!(String::from_utf8_lossy(&incompatible.stderr).contains("requires a call graph view"));
    Ok(())
}

#[test]
fn callflow_json_exposes_the_shared_architecture_model() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let graph = directory.path().join("graph.json");
    std::fs::write(
        &graph,
        serde_json::to_vec(&json!({
            "directed": true,
            "multigraph": false,
            "graph": {"project_name":"Fixture"},
            "nodes": [
                {"id":"run","label":"run","community":0,"source_file":"src/lib.rs"},
                {"id":"store","label":"store","community":1,"source_file":"src/store.rs"}
            ],
            "links": [
                {"source":"run","target":"store","relation":"calls","confidence":"INFERRED"}
            ]
        }))?,
    )?;
    let output = support::compass_command()
        .args([
            "export",
            "callflow-json",
            "--graph",
            graph.to_string_lossy().as_ref(),
        ])
        .current_dir(directory.path())
        .output()?;
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(value["schema"], "compass.viewer.callflow/1");
    assert_eq!(value["statistics"]["inferred"], 1);
    assert_eq!(value["coverage"]["crossSection"], 1);
    assert_eq!(value["crossSectionCalls"][0]["source"], "run");
    assert_eq!(value["crossSectionCalls"][0]["target"], "store");
    assert_eq!(value["crossSectionCalls"][0]["confidence"], "inferred");
    assert!(
        value["sections"]
            .as_array()
            .is_some_and(|sections| sections.len() >= 2)
    );
    Ok(())
}
