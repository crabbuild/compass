use std::error::Error;
use std::fs;

use rmcp::ServerHandler;
use serde_json::{Map, Value, json};
use trail_mcp::GraphifyMcp;

fn write_fixture(root: &std::path::Path) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let graph = root.join("graph.json");
    fs::write(
        &graph,
        r#"{
          "directed":true,"multigraph":false,"graph":{},
          "nodes":[
            {"id":"a","label":"Alpha()","source_file":"src/a.py","source_location":"L1","file_type":"code","community":0,"community_name":"Core"},
            {"id":"b","label":"Beta()","source_file":"src/b.py","source_location":"L2","file_type":"code","community":"0","community_name":"Core"},
            {"id":"c","label":"Gamma","source_file":"src/c.py","source_location":"L3","file_type":"code","community":1},
            {"id":"d","label":"Delta","source_file":"src/d.py","source_location":"L4","file_type":"code","community":2},
            {"id":"e","label":"Epsilon","source_file":"src/e.py","source_location":"L5","file_type":"code","community":3}
          ],
          "links":[
            {"source":"a","target":"b","relation":"calls","context":"call","confidence":"EXTRACTED"},
            {"source":"c","target":"b","relation":"imports","context":"import","confidence":"INFERRED"},
            {"source":"c","target":"d","relation":"references","context":"field","confidence":"AMBIGUOUS"}
          ]
        }"#,
    )?;
    fs::write(root.join("GRAPH_REPORT.md"), "# Fixture report\n")?;
    fs::write(
        root.join(".graphify_labels.json"),
        r#"{"0":"Core","1":"Feature","2":"Boundary"}"#,
    )?;
    Ok(graph)
}

fn args(entries: &[(&str, Value)]) -> Map<String, Value> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn tool_contract_and_all_local_tools_cover_success_and_validation_paths()
-> Result<(), Box<dyn Error>> {
    let temp = tempfile::tempdir()?;
    let graph = write_fixture(temp.path())?;
    let server = GraphifyMcp::new(&graph);

    let info = server.get_info();
    assert_eq!(info.server_info.name, "graphify");
    assert_eq!(GraphifyMcp::tools().len(), 10);
    assert!(GraphifyMcp::tools().iter().all(|tool| {
        tool.input_schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| properties.contains_key("project_path"))
    }));
    assert_eq!(GraphifyMcp::resources().len(), 6);

    assert!(
        server
            .invoke("query_graph", Map::new())
            .contains("'question'")
    );
    let query = server.invoke(
        "query_graph",
        args(&[
            ("question", json!("which calls Alpha")),
            ("mode", json!("dfs")),
            ("depth", json!("99")),
            ("token_budget", json!(-1)),
            ("context_filter", json!(["call", 4, "field"])),
        ]),
    );
    assert!(query.contains("Traversal: DFS"));

    assert!(server.invoke("get_node", Map::new()).contains("'label'"));
    assert!(
        server
            .invoke("get_node", args(&[("label", json!("alpha"))]))
            .contains("Node: Alpha()")
    );
    assert!(
        server
            .invoke("get_node", args(&[("label", json!("missing"))]))
            .contains("No node matching")
    );

    let neighbors = server.invoke(
        "get_neighbors",
        args(&[("label", json!("Beta")), ("relation_filter", json!("call"))]),
    );
    assert!(neighbors.contains("<-- Alpha() [calls]"));
    assert!(!neighbors.contains("Gamma"));
    assert!(
        server
            .invoke("get_neighbors", args(&[("label", json!("none"))]))
            .contains("No node matching")
    );

    assert!(
        server
            .invoke("get_community", args(&[("community_id", json!(0))]))
            .contains("Community 0 — Core (2 nodes)")
    );
    assert!(
        server
            .invoke("get_community", args(&[("community_id", json!(-1))]))
            .contains("Community -1 not found")
    );
    assert!(
        server
            .invoke("get_community", args(&[("community_id", json!(99))]))
            .contains("Community 99 not found")
    );

    assert!(
        server
            .invoke("god_nodes", args(&[("top_n", json!("2"))]))
            .contains("God nodes")
    );
    let stats = server.invoke("graph_stats", Map::new());
    assert!(stats.contains("Nodes: 5"));
    assert!(stats.contains("EXTRACTED: 33%"));
    assert!(stats.contains("INFERRED: 33%"));
    assert!(stats.contains("AMBIGUOUS: 33%"));

    assert!(
        server
            .invoke(
                "shortest_path",
                args(&[("source", json!("none")), ("target", json!("Beta"))])
            )
            .contains("No node matching source")
    );
    assert!(
        server
            .invoke(
                "shortest_path",
                args(&[("source", json!("Alpha")), ("target", json!("none"))])
            )
            .contains("No node matching target")
    );
    assert!(
        server
            .invoke(
                "shortest_path",
                args(&[("source", json!("Alpha")), ("target", json!("Alpha"))])
            )
            .contains("same node")
    );
    assert!(
        server
            .invoke(
                "shortest_path",
                args(&[("source", json!("Alpha")), ("target", json!("Epsilon"))])
            )
            .contains("No path found")
    );
    assert!(
        server
            .invoke(
                "shortest_path",
                args(&[
                    ("source", json!("Alpha")),
                    ("target", json!("Gamma")),
                    ("max_hops", json!(0))
                ])
            )
            .contains("Path exceeds max_hops=0")
    );
    assert!(
        server
            .invoke(
                "shortest_path",
                args(&[
                    ("source", json!("Alpha")),
                    ("target", json!("Gamma")),
                    ("max_hops", json!(8))
                ])
            )
            .contains("Shortest path (2 hops)")
    );

    assert!(
        server
            .invoke("get_pr_impact", Map::new())
            .contains("'pr_number'")
    );
    Ok(())
}

#[test]
fn resources_and_hot_reload_cover_reports_analysis_and_cache_refresh() -> Result<(), Box<dyn Error>>
{
    let temp = tempfile::tempdir()?;
    let graph = write_fixture(temp.path())?;
    let server = GraphifyMcp::new(&graph);

    assert_eq!(server.read("graphify://report")?, "# Fixture report\n");
    assert!(server.read("graphify://stats")?.contains("Nodes: 5"));
    assert!(server.read("graphify://god-nodes")?.contains("God nodes"));
    assert!(server.read("graphify://audit")?.contains("Total edges: 3"));
    assert!(!server.read("graphify://surprises")?.is_empty());
    assert!(!server.read("graphify://questions")?.is_empty());
    assert!(server.read("graphify://unknown").is_err());

    fs::remove_file(temp.path().join("GRAPH_REPORT.md"))?;
    assert!(
        server
            .read("graphify://report")?
            .contains("GRAPH_REPORT.md not found")
    );

    fs::write(
        &graph,
        r#"{"directed":true,"multigraph":false,"graph":{},"nodes":[{"id":"only","label":"Only"}],"links":[]}"#,
    )?;
    assert!(
        server
            .invoke("graph_stats", Map::new())
            .contains("Nodes: 1")
    );
    Ok(())
}

#[test]
fn missing_graph_and_unknown_tool_errors_are_stable() {
    let server = GraphifyMcp::new("definitely-missing-graph.json");
    assert_eq!(
        server.invoke("not_a_tool", Map::new()),
        "Unknown tool: not_a_tool"
    );
    assert!(
        server
            .invoke("graph_stats", Map::new())
            .contains("graph.json not found")
    );
    assert!(server.read("graphify://stats").is_err());
}
