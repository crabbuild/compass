mod support;

use compass_model::query_contract::{CodeQueryLimits, SearchRequest};
use compass_query::open;

#[test]
fn fts_search_ranks_exact_prefix_alias_unicode_and_ties_stably()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let search = |query: &str| {
        engine.search(SearchRequest {
            query: query.to_owned(),
            limits: CodeQueryLimits::default(),
        })
    };

    let exact = search("UserService.list")?;
    assert_eq!(exact.results[0].node_id, "n:list");
    let prefix = search("list")?;
    let exact_name_ids = prefix
        .results
        .iter()
        .take(2)
        .map(|hit| hit.node_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        exact_name_ids,
        std::collections::HashSet::from(["n:list", "n:other"])
    );
    assert_eq!(
        serde_json::to_value(&prefix)?,
        serde_json::to_value(search("list")?)?
    );
    assert!(
        search("fetchUsers")?
            .results
            .iter()
            .any(|hit| hit.node_id == "n:list")
    );
    assert!(
        search("cafe")?
            .results
            .iter()
            .any(|hit| hit.node_id == "n:unicode")
    );
    assert!(
        search(r#""list" OR * -"#)?
            .results
            .iter()
            .any(|hit| hit.node_id == "n:list")
    );
    Ok(())
}

#[test]
fn search_limits_are_enforced_before_sqlite_work() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.search(SearchRequest {
        query: "list".to_owned(),
        limits: CodeQueryLimits {
            max_nodes: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    assert_eq!(response.results.len(), 1);
    assert!(response.truncated);
    assert!(
        engine
            .search(SearchRequest {
                query: "x ".repeat(33),
                limits: CodeQueryLimits::default(),
            })
            .is_err()
    );
    Ok(())
}
