mod support;

use std::fs;

use compass_model::query_contract::{CodeQueryLimits, ExploreRequest, QueryDiagnosticCode};
use compass_query::open;

#[test]
fn explore_connects_symbols_and_groups_digest_verified_source()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let request = ExploreRequest {
        symbols: vec!["Api.caller".to_owned(), "Store.callee".to_owned()],
        root: directory.path().to_string_lossy().into_owned(),
        limits: CodeQueryLimits::default(),
    };
    let response = engine.explore(request.clone())?;
    assert_eq!(response.paths.len(), 1);
    assert_eq!(response.files.len(), 1);
    assert_eq!(response.files[0].source.as_deref(), Some("code"));

    fs::write(directory.path().join("src/lib.rs"), "changed")?;
    let stale = engine.explore(request)?;
    assert!(stale.files[0].source.is_none());
    assert!(
        stale
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code == QueryDiagnosticCode::StaleSourceDigest })
    );
    Ok(())
}

#[test]
fn explore_never_reads_unsafe_relative_paths() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph: serde_json::Value = serde_json::from_slice(&fs::read(&graph_path)?)?;
    graph["graph"]["files"][0]["path"] = serde_json::json!("../secret");
    graph["nodes"][0]["source"]["file"] = serde_json::json!("../secret");
    graph["nodes"][0]["evidence"][0]["anchors"][0]["file"] = serde_json::json!("../secret");
    fs::write(&graph_path, serde_json::to_vec_pretty(&graph)?)?;
    assert!(open(&graph_path, None, &directory.path().join("cache")).is_err());
    Ok(())
}
