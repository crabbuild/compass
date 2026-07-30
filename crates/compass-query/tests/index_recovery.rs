mod support;

use std::fs;

use compass_model::query_contract::{CodeQueryLimits, SearchRequest};
use compass_query::open;

#[test]
fn deleted_and_corrupt_indexes_rebuild_to_identical_results()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    let cache = directory.path().join("cache");
    support::write_graph(&graph_path)?;
    let request = SearchRequest {
        query: "list".to_owned(),
        limits: CodeQueryLimits::default(),
    };
    let engine = open(&graph_path, None, &cache)?;
    let expected = serde_json::to_value(engine.search(request.clone())?)?;
    let index = engine.index_path().to_path_buf();
    drop(engine);

    fs::remove_file(&index)?;
    let rebuilt = open(&graph_path, None, &cache)?;
    assert_eq!(
        serde_json::to_value(rebuilt.search(request.clone())?)?,
        expected
    );
    drop(rebuilt);

    fs::write(&index, b"not sqlite")?;
    let recovered = open(&graph_path, None, &cache)?;
    assert_eq!(serde_json::to_value(recovered.search(request)?)?, expected);
    Ok(())
}

#[test]
fn repeated_open_reuses_the_content_addressed_complete_index()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    let cache = directory.path().join("cache");
    support::write_graph(&graph_path)?;
    let first = open(&graph_path, None, &cache)?;
    let index = first.index_path().to_path_buf();
    let bytes = fs::read(&index)?;
    drop(first);
    let second = open(&graph_path, None, &cache)?;
    assert_eq!(second.index_path(), index);
    assert_eq!(fs::read(&index)?, bytes);
    Ok(())
}
