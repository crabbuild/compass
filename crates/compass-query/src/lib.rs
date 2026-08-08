//! Native graph search, traversal, explanation, and impact analysis.

mod affected;
mod benchmark;
mod code_query;
mod cql;
mod graph_engine;
mod index;
mod intent;
mod program_join;
mod ranking;
mod recall;
mod relevance;
mod score;
mod source;
mod telemetry;
mod text;
mod traversal;

pub use affected::{DEFAULT_AFFECTED_RELATIONS, affected_nodes, format_affected, resolve_seed};
pub use benchmark::{BenchmarkQuestion, BenchmarkResult, format_benchmark, run_benchmark};
pub use code_query::CodeQueryEngine;
pub use cql::{
    CacheStats, ExplainPlan, OperatorProfile, PlanCache, PlanCacheConfig, QueryError,
    QueryErrorKind, QueryLimits, QueryProfile, QueryRequest, QueryResult, execute,
};
pub use graph_engine::{GraphEngine, JsonGraphEngine, StoreGraphEngine, open_graph_engine};
pub use index::{EngineSelection, QueryEngineKind, open, open_with_engine, open_with_store};
pub use intent::{
    NaturalQueryIntent, NaturalQueryPlan, NaturalQueryRequest, QUERY_PLANNER_PROFILE_V1,
    plan_natural_query,
};
pub use program_join::join_program_evidence;
pub use ranking::QUERY_RANKER_PROFILE_V2;
pub use relevance::{
    EdgeIdentity, EdgeJudgment, IdJudgment, IntentMetrics, JudgedQuery, JudgmentCorpus,
    MAX_JUDGMENTS_PER_QUERY, MAX_QUESTIONS, MAX_TEXT_BYTES, MetricValue, ObservedEdge,
    ObservedPath, PathJudgment, PathPattern, QUERY_JUDGMENTS_SCHEMA_V1,
    QUERY_QUALIFICATION_SCHEMA_V1, QualificationReport, QueryClass, QueryObservation,
    RelevanceError, RelevanceMetrics, qualification_report, score,
};
pub use score::{QueryScores, ScoredNode, find_node, pick_scored_endpoint, score_nodes};
pub use telemetry::{
    ProfiledCodeQueryResponse, QUERY_EXECUTION_PROFILE_V1, QueryExecutionProfile,
    QueryStageTimings, WorkCounts,
};
pub use text::{normalize_context_filters, query_terms, sanitize_label, search_tokens};
pub use traversal::{
    DEFAULT_TEXT_TOKEN_BUDGET, TextPageOptions, TextPaginationError, TraversalMode,
    query_graph_text, query_graph_text_page, render_explanation, render_explanation_page,
    render_shortest_path,
};

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::error::Error;

    use compass_model::{Graph, GraphDocument};

    use super::*;

    fn load(raw: &str) -> Result<Graph, Box<dyn Error>> {
        let document = serde_json::from_str::<GraphDocument>(raw)?;
        Ok(Graph::from_document(document)?)
    }

    #[test]
    fn query_terms_remove_question_noise() {
        assert_eq!(
            query_terms("how does the frontier cache work"),
            vec!["frontier", "cache"]
        );
        assert_eq!(
            query_terms("Wie funktioniert die Authentifizierung?"),
            vec!["authentifizierung"]
        );
    }

    #[test]
    fn explicit_context_limits_query_traversal() -> Result<(), Box<dyn Error>> {
        let graph = load(
            r#"{
                "directed": false, "multigraph": false, "graph": {},
                "nodes": [
                    {"id":"n1","label":"extract","source_file":"extract.py","source_location":"L10","community":0},
                    {"id":"n2","label":"cluster","source_file":"cluster.py","source_location":"L5","community":0},
                    {"id":"n3","label":"build","source_file":"build.py","source_location":"L1","community":1}
                ],
                "links": [
                    {"source":"n1","target":"n2","relation":"calls","confidence":"EXTRACTED","context":"call"},
                    {"source":"n2","target":"n3","relation":"imports","confidence":"EXTRACTED","context":"import"}
                ]
            }"#,
        )?;
        let output = query_graph_text(
            &graph,
            "extract",
            TraversalMode::Bfs,
            2,
            2000,
            &["call".to_owned()],
            &HashMap::new(),
        );
        assert!(output.contains("Context: call (explicit)"));
        assert!(output.contains("cluster"));
        assert!(!output.contains("NODE build"));
        Ok(())
    }

    #[test]
    fn natural_query_projects_typed_source_location_and_community() -> Result<(), Box<dyn Error>> {
        let graph = load(
            r#"{
                "directed": true, "multigraph": true, "graph": {},
                "nodes": [{
                    "id": "method",
                    "kind": "method",
                    "name": ".process_delayed_slices()",
                    "qualifiedName": "crate::RedisMetaStore::process_delayed_slices",
                    "source": {
                        "file": "src/meta/stores/redis/mod.rs",
                        "startByte": 100,
                        "endByte": 122,
                        "startLine": 3086,
                        "startColumn": 13,
                        "endLine": 3086,
                        "endColumn": 35
                    },
                    "community": {"id": 4, "label": "RedisMetaStore"}
                }],
                "links": []
            }"#,
        )?;

        let output = query_graph_text(
            &graph,
            "process_delayed_slices",
            TraversalMode::Bfs,
            2,
            2000,
            &[],
            &HashMap::new(),
        );

        assert!(output.contains(
            "NODE .process_delayed_slices() [src=src/meta/stores/redis/mod.rs \
             loc=L3086:13-L3086:35 community=RedisMetaStore]"
        ));
        Ok(())
    }

    #[test]
    fn natural_query_prefers_declarations_and_renders_relationship_sites()
    -> Result<(), Box<dyn Error>> {
        let graph = load(
            r#"{
                "directed": true, "multigraph": true, "graph": {},
                "nodes": [{
                    "id": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "kind": "method",
                    "name": ".process_delayed_slices()",
                    "source": {
                        "file": "src/store.rs", "startLine": 20, "startColumn": 4,
                        "endLine": 20, "endColumn": 28
                    }
                }, {
                    "id": "placeholder",
                    "kind": "function",
                    "name": "process_delayed_slices",
                    "evidence": [{
                        "origin": "heuristic",
                        "extractor": "compass.graph.external-placeholder",
                        "confidence": "inferred",
                        "wiringSite": {
                            "file": "tests/workflow.rs", "startLine": 96, "startColumn": 8,
                            "endLine": 96, "endColumn": 30
                        }
                    }]
                }],
                "links": [{
                    "id": "edge", "source": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "target": "placeholder", "kind": "calls",
                    "relationshipSite": {
                        "file": "src/store.rs", "startLine": 25, "startColumn": 8,
                        "endLine": 25, "endColumn": 32
                    },
                    "evidence": [{"origin": "ast", "extractor": "test", "confidence": "exact"}]
                }]
            }"#,
        )?;

        let output = query_graph_text(
            &graph,
            "process_delayed_slices",
            TraversalMode::Bfs,
            2,
            2000,
            &[],
            &HashMap::new(),
        );

        assert!(output.contains("Start: ['.process_delayed_slices()']"));
        assert!(!output.contains("Start: ['.process_delayed_slices()', 'process_delayed_slices']"));
        assert!(output.contains("wiring=src/store.rs:L25:8-L25:32"));
        assert!(output.contains("at=src/store.rs:L25:8-L25:32"));
        Ok(())
    }

    #[test]
    fn explanation_reports_ambiguity_and_exact_ids_preserve_locations() -> Result<(), Box<dyn Error>>
    {
        let first = format!("sha256:{}", "a".repeat(64));
        let second = format!("sha256:{}", "b".repeat(64));
        let graph = load(&format!(
            r#"{{
                "directed": true, "multigraph": true, "graph": {{}},
                "nodes": [{{
                    "id": "{first}", "kind": "method", "name": ".process_delayed_slices()",
                    "source": {{"file": "src/database.rs", "startLine": 10, "startColumn": 4,
                        "endLine": 10, "endColumn": 28}}
                }}, {{
                    "id": "{second}", "kind": "method", "name": ".process_delayed_slices()",
                    "source": {{"file": "src/tikv.rs", "startLine": 20, "startColumn": 4,
                        "endLine": 20, "endColumn": 28}}
                }}, {{
                    "id": "placeholder", "kind": "function", "name": "process_delayed_slices",
                    "evidence": [{{"origin": "heuristic", "extractor": "compass.graph.external-placeholder",
                        "confidence": "inferred", "wiringSite": {{"file": "tests/workflow.rs",
                        "startLine": 96, "startColumn": 8, "endLine": 96, "endColumn": 30}}}}]
                }}],
                "links": [{{
                    "id": "edge", "source": "{second}", "target": "placeholder", "kind": "calls",
                    "relationshipSite": {{"file": "src/tikv.rs", "startLine": 24, "startColumn": 8,
                        "endLine": 24, "endColumn": 30}},
                    "evidence": [{{"origin": "ast", "extractor": "test", "confidence": "exact"}}]
                }}]
            }}"#
        ))?;

        let ambiguous = render_explanation(&graph, "process_delayed_slices", &HashMap::new());
        assert!(
            ambiguous
                .contains("Ambiguous: 'process_delayed_slices' matches 2 source-backed nodes.")
        );
        assert!(ambiguous.contains("src/database.rs L10:4-L10:28"));
        assert!(ambiguous.contains("src/tikv.rs L20:4-L20:28"));
        assert!(!ambiguous.contains("tests/workflow.rs"));

        let exact = render_explanation(&graph, &second, &HashMap::new());
        assert!(exact.contains(&format!("ID:        {second}")));
        assert!(exact.contains("Source:    src/tikv.rs L20:4-L20:28"));
        assert!(exact.contains("Type:      code"));
        assert!(
            exact.contains(
                "--> process_delayed_slices [calls] [EXTRACTED] src/tikv.rs:L24:8-L24:30"
            )
        );
        Ok(())
    }

    #[test]
    fn omitted_multigraph_preserves_parallel_edge_hub_semantics() -> Result<(), Box<dyn Error>> {
        let links = std::iter::once(serde_json::json!({
            "source": "seed",
            "target": "hub",
            "relation": "calls"
        }))
        .chain((0..50).map(|index| {
            serde_json::json!({
                "source": "hub",
                "target": "leaf",
                "relation": format!("parallel-{index}")
            })
        }))
        .collect::<Vec<_>>();
        let raw = serde_json::json!({
            "nodes": [
                {"id": "seed", "label": "extract"},
                {"id": "hub", "label": "hub"},
                {"id": "leaf", "label": "leaf"}
            ],
            "links": links
        });
        let graph = Graph::from_document(serde_json::from_value(raw)?)?;
        let output = query_graph_text(
            &graph,
            "extract",
            TraversalMode::Bfs,
            2,
            2000,
            &[],
            &HashMap::new(),
        );
        assert!(output.contains("2 nodes found"));
        assert!(!output.contains("NODE leaf"));
        Ok(())
    }

    #[test]
    fn shortest_path_preserves_stored_arrow_direction() -> Result<(), Box<dyn Error>> {
        let mut document = serde_json::from_str::<GraphDocument>(
            r#"{
                "directed": true, "multigraph": false, "graph": {},
                "nodes": [
                    {"id":"create","label":"createPatchHandler()"},
                    {"id":"validate","label":"validateSanitySession()"}
                ],
                "links": [
                    {"source":"create","target":"validate","relation":"calls","confidence":"EXTRACTED"}
                ]
            }"#,
        )?;
        document.directed = true;
        let graph = Graph::from_document(document)?;
        let forward = render_shortest_path(&graph, "createPatchHandler", "validateSanitySession")?;
        let reverse = render_shortest_path(&graph, "validateSanitySession", "createPatchHandler")?;
        assert!(
            forward.contains("createPatchHandler() --calls [EXTRACTED]--> validateSanitySession()")
        );
        assert!(
            reverse.contains("validateSanitySession() <--calls [EXTRACTED]-- createPatchHandler()")
        );
        Ok(())
    }

    #[test]
    fn explanation_separates_inbound_and_outbound_edges() -> Result<(), Box<dyn Error>> {
        let graph = load(
            r#"{
                "directed": true, "multigraph": false, "graph": {},
                "nodes": [
                    {"id":"caller","label":"caller()"},
                    {"id":"target","label":"target()"},
                    {"id":"callee","label":"callee()"}
                ],
                "links": [
                    {"source":"caller","target":"target","relation":"calls","confidence":"EXTRACTED"},
                    {"source":"target","target":"callee","relation":"calls","confidence":"EXTRACTED"}
                ]
            }"#,
        )?;
        let output = render_explanation(&graph, "target", &HashMap::new());
        assert!(output.contains("<-- caller() [calls] [EXTRACTED]"));
        assert!(output.contains("--> callee() [calls] [EXTRACTED]"));
        Ok(())
    }

    #[test]
    fn explanation_lists_each_multigraph_neighbor_once_but_keeps_parallel_degree()
    -> Result<(), Box<dyn Error>> {
        let graph = load(
            r#"{
                "directed": true, "multigraph": true, "graph": {},
                "nodes": [
                    {"id":"barrel","label":"barrel.ts"},
                    {"id":"module","label":"module.ts"}
                ],
                "links": [
                    {"source":"barrel","target":"module","relation":"imports_from","confidence":"EXTRACTED"},
                    {"source":"barrel","target":"module","relation":"re_exports","confidence":"EXTRACTED"}
                ]
            }"#,
        )?;

        let output = render_explanation(&graph, "barrel", &HashMap::new());

        assert!(output.contains("Degree:    2"));
        assert!(output.contains("Connections (1):"));
        assert_eq!(output.matches("--> module.ts").count(), 1);
        assert!(output.contains("[imports_from] [EXTRACTED]"));
        Ok(())
    }

    #[test]
    fn natural_query_and_explanation_pages_are_complete_and_deterministic()
    -> Result<(), Box<dyn Error>> {
        let mut nodes = vec![serde_json::json!({
            "id": "seed",
            "label": "Seed",
            "source_file": "src/seed.rs",
            "source_location": "L1"
        })];
        let mut links = Vec::new();
        for index in 0..8 {
            nodes.push(serde_json::json!({
                "id": format!("neighbor-{index}"),
                "label": format!("Neighbor{index}"),
                "source_file": format!("src/neighbor_{index}.rs"),
                "source_location": "L1"
            }));
            links.push(serde_json::json!({
                "source": "seed",
                "target": format!("neighbor-{index}"),
                "relation": "calls",
                "confidence": "EXTRACTED"
            }));
        }
        let document = serde_json::from_value::<GraphDocument>(serde_json::json!({
            "directed": true,
            "multigraph": false,
            "graph": {},
            "nodes": nodes,
            "links": links
        }))?;
        let graph = Graph::from_document(document)?;

        let query_first = query_graph_text_page(
            &graph,
            "Seed",
            TraversalMode::Bfs,
            2,
            TextPageOptions {
                token_budget: 60,
                page: 1,
            },
            &[],
            &HashMap::new(),
        )?;
        let query_second = query_graph_text_page(
            &graph,
            "Seed",
            TraversalMode::Bfs,
            2,
            TextPageOptions {
                token_budget: 60,
                page: 2,
            },
            &[],
            &HashMap::new(),
        )?;
        assert!(query_first.contains("Pagination: page=1/"));
        assert!(query_first.contains("next=2"));
        assert!(query_second.contains("Pagination: page=2/"));
        assert_ne!(query_first, query_second);
        let query_complete = query_graph_text_page(
            &graph,
            "Seed",
            TraversalMode::Bfs,
            2,
            TextPageOptions {
                token_budget: 100_000,
                page: 1,
            },
            &[],
            &HashMap::new(),
        )?;
        assert!(query_complete.contains("page=1/1"));
        assert!(query_complete.contains("next=none"));

        let explain_first = render_explanation_page(&graph, "Seed", 60, 1, &HashMap::new())?;
        let explain_second = render_explanation_page(&graph, "Seed", 60, 2, &HashMap::new())?;
        assert!(explain_first.contains("Connections (8):"));
        assert!(explain_first.contains("connections=1-1/8"));
        assert!(explain_second.contains("connections=2-2/8"));
        assert_ne!(explain_first, explain_second);
        assert!(matches!(
            render_explanation_page(&graph, "Seed", 60, 99, &HashMap::new()),
            Err(TextPaginationError::PageOutOfRange { .. })
        ));
        Ok(())
    }

    #[test]
    fn affected_walks_incoming_impact_edges() -> Result<(), Box<dyn Error>> {
        let graph = load(
            r#"{
                "directed": true, "multigraph": false, "graph": {},
                "nodes": [
                    {"id":"target","label":"Foo","source_file":"foo.py","source_location":"L1"},
                    {"id":"caller","label":"X()","source_file":"app.py","source_location":"L4"}
                ],
                "links": [
                    {"source":"caller","target":"target","relation":"calls","context":"call","confidence":"EXTRACTED"}
                ]
            }"#,
        )?;
        let output = format_affected(&graph, "Foo", &["calls".to_owned()], 2);
        assert!(output.contains("Affected nodes for Foo"));
        assert!(output.contains("- X() [calls] app.py:L4"));
        Ok(())
    }
}
