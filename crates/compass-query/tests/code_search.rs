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

fn exact_search_graph() -> Result<GraphDocument, Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("graph.json");
    support::write_graph(&path)?;
    let mut graph = GraphDocument::load(&path)?;
    let template = graph.nodes.first().ok_or("fixture node missing")?.clone();
    let mut other_file = graph
        .graph
        .files
        .first()
        .ok_or("fixture file missing")?
        .clone();
    other_file.path = "other.rs".to_owned();
    other_file.id = compass_model::identity::file_id("other.rs");
    graph.graph.files.push(other_file);
    graph.nodes.clear();
    graph.links.clear();
    for (id, name, kind, file, line) in [
        (
            "a:declaration",
            "Target()",
            compass_model::code_graph::NodeKind::Function,
            "src/lib.rs",
            10,
        ),
        (
            "b:export",
            "Target",
            compass_model::code_graph::NodeKind::Export,
            "src/lib.rs",
            10,
        ),
        (
            "c:overload",
            "Target()",
            compass_model::code_graph::NodeKind::Function,
            "src/lib.rs",
            20,
        ),
        (
            "d:other",
            "Target()",
            compass_model::code_graph::NodeKind::Function,
            "other.rs",
            10,
        ),
    ] {
        let mut node = template.clone();
        node.id = id.to_owned();
        node.name = name.to_owned();
        node.qualified_name = format!("Scope.{name}");
        node.kind = kind;
        let source = node.source.as_mut().ok_or("fixture source missing")?;
        source.file = file.to_owned();
        source.start_line = line;
        source.end_line = line;
        graph.nodes.push(node);
    }
    for i in 0..300 {
        let mut node = template.clone();
        node.id = format!("noise:{i:03}");
        node.name = format!("TargetAdapter{i}");
        node.qualified_name = format!("Other.{}", node.name);
        graph.nodes.push(node);
    }
    Ok(graph)
}

fn exact_search_engines(
    directory: &std::path::Path,
    graph: GraphDocument,
) -> Result<Vec<compass_query::CodeQueryEngine>, Box<dyn std::error::Error>> {
    let path = directory.join("graph.json");
    std::fs::write(&path, serde_json::to_vec(&graph)?)?;
    let json = compass_query::open_with_engine(
        &path,
        None,
        &directory.join("json-cache"),
        compass_query::EngineSelection::Json,
    )?;
    let direct = compass_query::open_with_document(
        graph.clone(),
        &path,
        None,
        &directory.join("direct-cache"),
    )?;
    let store = compass_store::SqliteStore::open(directory.join("store.sqlite"))?;
    let prepared = compass_graph::GraphSnapshotBuilder::new().prepare(&store, &graph)?;
    compass_graph::GraphSnapshotBuilder::new().activate(&store, &prepared)?;
    let stored =
        compass_query::open_with_store(&store, &path, None, &directory.join("store-cache"))?;
    Ok(vec![json, direct, stored])
}

#[test]
fn exact_search_avoids_lexical_overflow_and_keeps_explicit_identity_constraints()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_model::code_graph::NodeKind;
    use compass_query::ExactSearchFilter;
    let directory = tempfile::tempdir()?;
    let engines = exact_search_engines(directory.path(), exact_search_graph()?)?;
    let request = || SearchRequest {
        query: "Target".to_owned(),
        limits: CodeQueryLimits {
            max_candidates: 256,
            max_nodes: 500,
            ..CodeQueryLimits::default()
        },
    };
    let mut responses = Vec::new();
    for engine in engines {
        assert!(engine.search(request())?.truncated);
        let exact = engine.search_exact(request(), ExactSearchFilter::default())?;
        assert!(!exact.truncated);
        assert_eq!(exact.results.len(), 4);
        let filter = ExactSearchFilter {
            source_file: Some("src/lib.rs".to_owned()),
            start_line: Some(10),
            kind: None,
        };
        let ambiguity = engine.search_exact(request(), filter.clone())?;
        assert!(!ambiguity.truncated);
        assert_eq!(
            ambiguity
                .nodes
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            ["a:declaration", "b:export"]
        );
        let selected = engine.search_exact(
            request(),
            ExactSearchFilter {
                kind: Some(NodeKind::Function),
                ..filter.clone()
            },
        )?;
        assert!(!selected.truncated);
        assert_eq!(selected.results.len(), 1);
        assert_eq!(selected.results[0].node_id, "a:declaration");
        let miss = engine.search_exact(
            request(),
            ExactSearchFilter {
                start_line: Some(11),
                ..filter.clone()
            },
        )?;
        assert!(!miss.truncated && miss.results.is_empty());
        assert!(
            miss.diagnostics
                .iter()
                .any(|d| d.code == QueryDiagnosticCode::NoMatch)
        );
        for filter in [
            ExactSearchFilter {
                source_file: Some("missing.rs".to_owned()),
                ..filter.clone()
            },
            ExactSearchFilter {
                kind: Some(NodeKind::Class),
                ..filter
            },
        ] {
            let miss = engine.search_exact(request(), filter)?;
            assert!(!miss.truncated && miss.results.is_empty());
        }
        let lexical_only = engine.search_exact(
            SearchRequest {
                query: "Adapter".to_owned(),
                ..request()
            },
            ExactSearchFilter::default(),
        )?;
        assert!(lexical_only.results.is_empty() && !lexical_only.truncated);
        responses.push(serde_json::to_value(selected)?);
    }
    assert!(responses.windows(2).all(|pair| pair[0] == pair[1]));
    Ok(())
}

#[test]
fn exact_search_limits_never_turn_a_filtered_prefix_into_proof()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::ExactSearchFilter;
    let directory = tempfile::tempdir()?;
    for engine in exact_search_engines(directory.path(), exact_search_graph()?)? {
        let request = || SearchRequest {
            query: "Target".to_owned(),
            limits: CodeQueryLimits {
                max_candidates: 1,
                ..CodeQueryLimits::default()
            },
        };
        for file in ["src/lib.rs", "other.rs", "missing.rs"] {
            let r = engine.search_exact(
                request(),
                ExactSearchFilter {
                    source_file: Some(file.to_owned()),
                    ..ExactSearchFilter::default()
                },
            )?;
            assert!(r.truncated);
            assert!(
                !r.diagnostics
                    .iter()
                    .any(|d| d.code == QueryDiagnosticCode::NoMatch)
            );
            assert!(
                r.diagnostics
                    .iter()
                    .any(|d| d.message.contains("before filtering"))
            );
        }
        let bounded = engine.search_exact(
            SearchRequest {
                limits: CodeQueryLimits {
                    max_nodes: 1,
                    ..CodeQueryLimits::default()
                },
                ..request()
            },
            ExactSearchFilter::default(),
        )?;
        assert!(bounded.truncated && bounded.nodes.len() == 1 && bounded.results.len() == 1);
        let id = engine.search_exact(
            SearchRequest {
                query: "a:declaration".to_owned(),
                ..request()
            },
            ExactSearchFilter::default(),
        )?;
        assert!(!id.truncated && id.results.len() == 1);
        let constrained_id = engine.search_exact(
            SearchRequest {
                query: "a:declaration".to_owned(),
                ..request()
            },
            ExactSearchFilter {
                source_file: Some("other.rs".to_owned()),
                ..ExactSearchFilter::default()
            },
        )?;
        assert!(!constrained_id.truncated && constrained_id.results.is_empty());
        assert!(
            engine
                .search_exact(
                    SearchRequest {
                        limits: CodeQueryLimits {
                            max_response_bytes: 1,
                            ..CodeQueryLimits::default()
                        },
                        ..request()
                    },
                    ExactSearchFilter::default()
                )
                .is_err()
        );
    }
    Ok(())
}

#[test]
fn exact_search_retains_same_coordinate_overloads_and_rejects_invalid_filters()
-> Result<(), Box<dyn std::error::Error>> {
    use compass_query::ExactSearchFilter;
    let directory = tempfile::tempdir()?;
    let mut graph = exact_search_graph()?;
    let mut duplicate = graph.nodes[0].clone();
    duplicate.id = "a:second-declaration".to_owned();
    graph.nodes.push(duplicate);
    graph.nodes.reverse();
    for engine in exact_search_engines(directory.path(), graph)? {
        let request = || SearchRequest {
            query: " .TARGET() ".to_owned(),
            limits: CodeQueryLimits::default(),
        };
        let filter = ExactSearchFilter {
            source_file: Some("src/lib.rs".to_owned()),
            start_line: Some(10),
            kind: Some(compass_model::code_graph::NodeKind::Function),
        };
        let result = engine.search_exact(request(), filter.clone())?;
        assert!(!result.truncated && result.results.len() == 2);
        for bad in [
            ExactSearchFilter {
                source_file: None,
                ..filter.clone()
            },
            ExactSearchFilter {
                start_line: Some(0),
                ..filter.clone()
            },
            ExactSearchFilter {
                source_file: Some("a\nb".to_owned()),
                ..filter.clone()
            },
            ExactSearchFilter {
                source_file: Some("x".repeat(4097)),
                ..filter.clone()
            },
        ] {
            assert!(engine.search_exact(request(), bad).is_err());
        }
        for bad in ["".to_owned(), "a\nb".to_owned(), "x".repeat(4097)] {
            assert!(
                engine
                    .search_exact(
                        SearchRequest {
                            query: bad,
                            ..request()
                        },
                        filter.clone()
                    )
                    .is_err()
            );
        }
    }
    Ok(())
}
