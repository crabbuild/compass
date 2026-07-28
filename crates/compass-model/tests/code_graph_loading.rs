use std::fs;

use compass_model::GraphError;
use compass_model::code_graph::{BuildMetadata, GraphDocument};

fn document() -> GraphDocument {
    GraphDocument::empty_v1(BuildMetadata {
        builder_version: "test".to_owned(),
        schema_fingerprint: "schema".to_owned(),
        source_tree_digest: "tree".to_owned(),
        configuration_digest: "config".to_owned(),
        generation_id: "generation".to_owned(),
        source_commit: None,
    })
}

#[test]
fn strict_loading_rejects_pre_contract_and_unknown_graphs() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    fs::write(&graph_path, r#"{"nodes":[],"links":[]}"#)?;
    assert!(matches!(
        GraphDocument::load(&graph_path),
        Err(GraphError::UnsupportedGraphSchema { found: None })
    ));

    fs::write(
        &graph_path,
        r#"{"directed":true,"multigraph":true,"graph":{"schema":"compass.graph/2"},"nodes":[],"links":[]}"#,
    )?;
    assert!(matches!(
        GraphDocument::load(&graph_path),
        Err(GraphError::UnsupportedGraphSchema { found: Some(schema) }) if schema == "compass.graph/2"
    ));
    Ok(())
}

#[test]
fn strict_loading_uses_a_content_addressed_validated_cache()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    fs::write(&graph_path, serde_json::to_vec(&document())?)?;

    assert_eq!(GraphDocument::load(&graph_path)?, document());
    let cache_entries =
        fs::read_dir(directory.path().join("cache"))?.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(cache_entries.len(), 1);
    assert!(
        cache_entries[0]
            .file_name()
            .to_string_lossy()
            .contains(".graph.json.")
    );
    assert_eq!(GraphDocument::load(&graph_path)?, document());
    Ok(())
}
