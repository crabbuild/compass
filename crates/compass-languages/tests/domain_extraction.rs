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
    assert!(extraction.nodes.iter().all(|node| {
        node.string("language") == "sql" && node.string("extractor") == "compass.languages.sql"
    }));
    assert!(extraction.edges.iter().all(|edge| {
        edge.string("language") == "sql" && edge.string("extractor") == "compass.languages.sql"
    }));
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

#[test]
fn sql_targets_respect_trigger_index_dml_and_alias_boundaries_with_text_provenance()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE SCHEMA app;
CREATE SCHEMA audit;
CREATE TABLE app.users (id BIGINT PRIMARY KEY);
CREATE TABLE app.accounts (id BIGINT PRIMARY KEY);
CREATE TABLE staging.users (id BIGINT PRIMARY KEY);
CREATE INDEX "users index" ON "app"."users"(id);
CREATE TRIGGER "audit"."users trigger" AFTER UPDATE OF id ON "app"."users"
FOR EACH ROW EXECUTE FUNCTION audit.capture_user();
INSERT INTO app.users(id) SELECT a.id FROM app.accounts AS a;
UPDATE app.users AS u SET id = 2 WHERE u.id = 1;
UPDATE update_alias SET id = 3 FROM app.users AS update_alias WHERE update_alias.id = 2;
DELETE FROM app.accounts AS doomed WHERE doomed.id = 1;
MERGE INTO app.users AS target USING staging.users AS source
ON target.id = source.id WHEN MATCHED THEN UPDATE SET id = source.id;
SELECT u.id FROM app.users u JOIN app.accounts AS a ON a.id = u.id;
WITH recent AS (SELECT id FROM app.users)
SELECT recent.id FROM recent;
"#;
    let extraction = extract_sql_content(Path::new("db/migrations/V2__targets.sql"), source);
    let qualified_names = extraction
        .nodes
        .iter()
        .map(|node| node.string("qualified_name"))
        .collect::<HashSet<_>>();

    for invalid in [
        "ON",
        "on",
        "OF",
        "of",
        "UPDATE",
        "SET",
        "u",
        "a",
        "target",
        "source",
        "doomed",
        "update_alias",
        "recent",
    ] {
        assert!(
            !qualified_names.contains(invalid),
            "invalid SQL entity {invalid:?}: nodes={:?}",
            extraction.nodes
        );
    }
    for expected in [
        "app.users",
        "app.accounts",
        "staging.users",
        "users index",
        "audit.users trigger",
    ] {
        assert!(
            qualified_names.contains(expected),
            "missing {expected:?}: {qualified_names:?}"
        );
    }
    let trigger = extraction
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "database_trigger")
        .ok_or("missing trigger")?;
    let users = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.users")
        .ok_or("missing users")?;
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == trigger.id
            && edge.target == users.id
            && edge.string("relation") == "triggers"
    }));

    for attributes in extraction
        .nodes
        .iter()
        .map(|node| &node.attributes)
        .chain(extraction.edges.iter().map(|edge| &edge.attributes))
        .filter(|attributes| {
            attributes
                .get("_origin")
                .and_then(serde_json::Value::as_str)
                != Some("convention")
        })
    {
        assert_eq!(
            attributes
                .get("_origin")
                .and_then(serde_json::Value::as_str),
            Some("artifact"),
            "text-derived SQL fact mislabeled: {attributes:?}"
        );
        assert!(
            attributes
                .get("rule")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|rule| rule.starts_with("sql-text-")),
            "missing SQL producer rule: {attributes:?}"
        );
        let start = attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| format!("missing start_byte: {attributes:?}"))?;
        let end = attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| format!("missing end_byte: {attributes:?}"))?;
        assert!(
            start < end && end <= source.len(),
            "attributes={attributes:?}"
        );
        assert!(
            source[start..end]
                .iter()
                .any(|byte| !byte.is_ascii_whitespace()),
            "anchor does not cover source text: {attributes:?}"
        );
    }
    Ok(())
}

#[test]
fn quoted_identifier_components_do_not_collapse_into_unquoted_qualification() {
    let source = br#"
CREATE TABLE "tenant.eu"."items" (id BIGINT);
CREATE TABLE tenant.eu.items (id BIGINT);
SELECT quoted.id FROM "tenant.eu"."items" AS quoted;
SELECT plain.id FROM tenant.eu.items AS plain;
"#;
    let extraction = extract_sql_content(Path::new("db/quoted.sql"), source);
    let tables = extraction
        .nodes
        .iter()
        .filter(|node| {
            node.string("symbol_kind") == "database_table"
                && node.string("qualified_name") == "tenant.eu.items"
        })
        .collect::<Vec<_>>();

    assert_eq!(tables.len(), 2, "nodes={:?}", extraction.nodes);
    assert_ne!(tables[0].id, tables[1].id);
    let reads = extraction
        .edges
        .iter()
        .filter(|edge| edge.string("relation") == "reads")
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 2, "edges={:?}", extraction.edges);
    assert_ne!(reads[0].target, reads[1].target);
}
