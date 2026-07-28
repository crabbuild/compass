use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use compass_languages::extract_sql_content;
use serde::Deserialize;

#[derive(Deserialize)]
struct ExpectedDomain {
    node_kinds: Vec<String>,
    edge_kinds: Vec<String>,
}

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/code-graph/domain")
        .join(path)
}

#[test]
fn sql_fixture_emits_every_declared_database_kind_and_relation_without_dangling_endpoints()
-> Result<(), Box<dyn std::error::Error>> {
    let source = fs::read(fixture("database.sql"))?;
    let expected: ExpectedDomain =
        serde_json::from_slice(&fs::read(fixture("database.expected.json"))?)?;
    let extraction = extract_sql_content(
        Path::new("db/migrations/V20260728__accounting.sql"),
        &source,
    );
    let node_kinds = extraction
        .nodes
        .iter()
        .map(|node| node.string("symbol_kind"))
        .collect::<HashSet<_>>();
    let edge_kinds = extraction
        .edges
        .iter()
        .map(|edge| edge.string("relation"))
        .collect::<HashSet<_>>();
    for kind in expected.node_kinds {
        assert!(node_kinds.contains(&kind), "missing node kind {kind}");
    }
    for kind in expected.edge_kinds {
        assert!(edge_kinds.contains(&kind), "missing edge kind {kind}");
    }
    let ids = extraction
        .nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    assert!(
        extraction
            .edges
            .iter()
            .all(|edge| ids.contains(edge.source.as_str()) && ids.contains(edge.target.as_str())),
        "domain extraction emitted a dangling endpoint"
    );
    Ok(())
}

#[test]
fn dynamic_and_malformed_sql_never_become_exact_database_facts() {
    let source = br#"
CREATE PROCEDURE run_dynamic() AS $$
BEGIN
  EXECUTE 'SELECT * FROM private.secrets';
END;
$$ LANGUAGE plpgsql;
-- SELECT * FROM commented.table_name;
CREATE TABLE
"#;
    let extraction = extract_sql_content(Path::new("db/runtime.sql"), source);
    assert!(
        extraction.nodes.iter().all(|node| {
            !matches!(
                node.string("qualified_name").as_str(),
                "private.secrets" | "commented.table_name"
            )
        }),
        "dynamic or commented SQL was promoted to an exact node"
    );
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| node.string("symbol_kind") != "query"),
        "dynamic SQL was promoted to an exact query"
    );
}
