//! Native graph search, traversal, explanation, and impact analysis.

mod affected;
mod benchmark;
mod cql;
mod score;
mod text;
mod traversal;

pub use affected::{DEFAULT_AFFECTED_RELATIONS, affected_nodes, format_affected, resolve_seed};
pub use benchmark::{BenchmarkQuestion, BenchmarkResult, format_benchmark, run_benchmark};
pub use cql::{
    CacheStats, ExplainPlan, OperatorProfile, PlanCache, PlanCacheConfig, QueryError,
    QueryErrorKind, QueryLimits, QueryProfile, QueryRequest, QueryResult, execute,
};
pub use score::{QueryScores, ScoredNode, find_node, pick_scored_endpoint, score_nodes};
pub use text::{normalize_context_filters, query_terms, sanitize_label, search_tokens};
pub use traversal::{TraversalMode, query_graph_text, render_explanation, render_shortest_path};

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
