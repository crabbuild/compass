mod support;

use compass_model::query_contract::{CodeQueryLimits, SearchRequest};
use compass_model::{
    code_graph::{DiagnosticSeverity, GraphDiagnostic, GraphDocument},
    query_contract::QueryDiagnosticCode,
};
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
    let candidate_bounded = engine.search(SearchRequest {
        query: "list".to_owned(),
        limits: CodeQueryLimits {
            max_candidates: 1,
            ..CodeQueryLimits::default()
        },
    })?;
    assert_eq!(candidate_bounded.results.len(), 1);
    assert!(candidate_bounded.truncated);
    assert!(candidate_bounded.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == QueryDiagnosticCode::BoundedTruncation
            && diagnostic.message.contains("limited to 1 candidate")
    }));
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

#[test]
fn search_discloses_partial_publication_coverage() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let graph_path = directory.path().join("graph.json");
    support::write_graph(&graph_path)?;
    let mut graph = GraphDocument::load(&graph_path)?;
    graph.graph.diagnostics.push(GraphDiagnostic {
        severity: DiagnosticSeverity::Warning,
        code: "publication_omission_summary".to_owned(),
        message:
            "partial graph published after quarantining 2 nodes and 3 edges with 1 identity collisions; 0 examples omitted by the diagnostic cap"
                .to_owned(),
        anchor: None,
        related_ids: Vec::new(),
    });
    std::fs::write(&graph_path, serde_json::to_vec(&graph)?)?;

    let engine = open(&graph_path, None, &directory.path().join("cache"))?;
    let response = engine.search(SearchRequest {
        query: "list".to_owned(),
        limits: CodeQueryLimits::default(),
    })?;
    assert!(response.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == QueryDiagnosticCode::IncompleteCoverage
            && diagnostic.message.contains("2 nodes and 3 edges")
    }));
    Ok(())
}
