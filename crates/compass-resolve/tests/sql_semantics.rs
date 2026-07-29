use std::collections::HashSet;
use std::error::Error;
use std::fs;
use std::path::Path;

use compass_graph::{BuildEvidence, normalize_v1};
use compass_languages::extract_sql_content;
use compass_model::code_graph::{CoverageStatus, EdgeKind, NodeKind};
use compass_model::provenance::EvidenceConfidence;
use compass_model::provenance::EvidenceOrigin;

#[test]
fn sql_text_facts_publish_with_artifact_provenance_and_exact_original_anchors()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V3__trigger.sql");
    let source = br#"
CREATE TABLE "app"."users" (id BIGINT PRIMARY KEY);
CREATE TABLE "audit"."log" (id BIGINT PRIMARY KEY);
CREATE TRIGGER "audit"."capture user"
AFTER UPDATE ON "app"."users"
FOR EACH ROW BEGIN
  INSERT INTO "audit"."log"(id) SELECT id FROM "app"."users";
END;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-semantics")?;
    let graph = normalize_v1(extraction, evidence)?;

    assert!(graph.nodes.iter().all(|node| node.qualified_name != "ON"));
    let trigger = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTrigger)
        .ok_or("missing trigger")?;
    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    assert!(graph.links.iter().any(|edge| {
        edge.source == trigger.id && edge.target == users.id && edge.kind == EdgeKind::Triggers
    }));
    let kinds = graph
        .links
        .iter()
        .map(|edge| edge.kind)
        .collect::<HashSet<_>>();
    assert!(kinds.contains(&EdgeKind::Reads));
    assert!(kinds.contains(&EdgeKind::Writes));

    for node in graph.nodes.iter().filter(|node| {
        !matches!(
            node.kind,
            NodeKind::File | NodeKind::Database | NodeKind::Migration
        )
    }) {
        assert!(
            node.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item
                        .rule
                        .as_deref()
                        .is_some_and(|rule| rule.starts_with("sql-text-"))
                    && item.anchors.len() == 1
            }),
            "node={node:?}"
        );
    }
    for edge in &graph.links {
        if edge.evidence[0].origin == EvidenceOrigin::Convention {
            continue;
        }
        assert!(
            edge.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item
                        .rule
                        .as_deref()
                        .is_some_and(|rule| rule.starts_with("sql-text-"))
                    && item.anchors.len() == 1
            }),
            "edge={edge:?}"
        );
    }
    Ok(())
}

#[test]
fn hardened_sql_boundaries_publish_a_closed_strict_v1_graph() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V4__hardening.sql");
    let source = br#"
CREATE SCHEMA IF NOT EXISTS "App";
CREATE TEMPORARY TABLE IF NOT EXISTS "App"."Users" (id BIGINT);
CREATE TABLE IF NOT EXISTS "App"."users" (id BIGINT);
CREATE UNIQUE INDEX IF NOT EXISTS users_idx ON ONLY "App"."Users"(id);
CREATE PROCEDURE "App"."refresh"() AS $routine$
BEGIN
  INSERT INTO "App"."users"(id) VALUES (1);
  UPDATE "App"."Users" SET id = 2;
END;
$routine$ LANGUAGE plpgsql;
WITH RECURSIVE recent AS NOT MATERIALIZED (
  SELECT id FROM "App"."Users"
), lower_users AS MATERIALIZED (
  SELECT id FROM "App"."users"
)
SELECT recent.id FROM recent JOIN lower_users ON lower_users.id = recent.id;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-hardening")?;
    let graph = normalize_v1(extraction, evidence)?;

    for invalid in [
        "IF",
        "NOT",
        "EXISTS",
        "ONLY",
        "recent",
        "lower_users",
        "RECURSIVE",
        "MATERIALIZED",
    ] {
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.qualified_name != invalid),
            "invalid SQL entity {invalid:?}: {:?}",
            graph.nodes
        );
    }
    let queries = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Query)
        .collect::<Vec<_>>();
    assert_eq!(
        queries.len(),
        1,
        "routine body became a top-level query: {:?}",
        graph.nodes
    );
    let with_start = source
        .windows(b"WITH RECURSIVE".len())
        .position(|window| window == b"WITH RECURSIVE")
        .ok_or("missing WITH")?;
    assert_eq!(
        queries[0].evidence[0].anchors[0].start_byte,
        with_start as u64
    );

    let tables = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::DatabaseTable)
        .collect::<Vec<_>>();
    assert_eq!(tables.len(), 2, "nodes={:?}", graph.nodes);
    for table in tables {
        assert!(
            graph.links.iter().any(|edge| {
                edge.target == table.id && matches!(edge.kind, EdgeKind::Reads | EdgeKind::Writes)
            }),
            "quoted table has no correctly resolved access: {table:?}"
        );
    }
    Ok(())
}

#[test]
fn sql_scanner_remediation_cases_publish_truthful_strict_v1_facts() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V5__scanner_remediation.sql");
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE TABLE app.audit (id BIGINT);
UPDATE LOW_PRIORITY IGNORE app.users SET id = 1;
INSERT HIGH_PRIORITY IGNORE INTO app.users(id) VALUES (1);
DELETE LOW_PRIORITY QUICK IGNORE FROM app.users;
CREATE PROCEDURE app.broken_begin() AS BEGIN
  UPDATE app.users SET id = 2;
SELECT * FROM app.users;
CREATE TABLE app.after_begin (id BIGINT);
CREATE PROCEDURE app.broken_dollar() AS $tag$
  UPDATE app.users SET id = 3;
SELECT * FROM app.users;
CREATE TABLE app.after_dollar (id BIGINT);
CREATE PROCEDURE app.refresh() AS $body$
BEGIN
  UPDATE app.users SET id = 4;
  WITH RECURSIVE recent AS NOT MATERIALIZED (
    SELECT id FROM app.users
  ), enriched AS MATERIALIZED (
    SELECT recent.id FROM recent JOIN app.users ON app.users.id = recent.id
  )
  INSERT INTO app.audit SELECT id FROM enriched;
END;
$body$ LANGUAGE plpgsql;
CREATE TABLE "app"."semi;""colon" (id BIGINT);
SELECT * FROM "app"."semi;""colon";
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-remediation")?;
    let graph = normalize_v1(extraction, evidence)?;

    for invalid in [
        "LOW_PRIORITY",
        "HIGH_PRIORITY",
        "QUICK",
        "IGNORE",
        "recent",
        "enriched",
        "RECURSIVE",
        "MATERIALIZED",
    ] {
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.qualified_name != invalid),
            "invalid exact entity {invalid:?}: {:?}",
            graph.nodes
        );
    }

    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    let writes_to_users = graph
        .links
        .iter()
        .filter(|edge| edge.target == users.id && edge.kind == EdgeKind::Writes)
        .count();
    assert_eq!(writes_to_users, 4, "links={:?}", graph.links);

    for broken in ["app.broken_begin", "app.broken_dollar"] {
        let owner = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == broken)
            .ok_or_else(|| format!("missing {broken}"))?;
        assert!(graph.links.iter().all(|edge| {
            edge.source != owner.id || !matches!(edge.kind, EdgeKind::Reads | EdgeKind::Writes)
        }));
    }
    for recovered in ["app.after_begin", "app.after_dollar"] {
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.qualified_name == recovered),
            "missing recovered declaration {recovered:?}"
        );
    }
    assert_eq!(
        graph
            .graph
            .coverage
            .iter()
            .filter(|record| {
                record.capability == "sql:body_ownership"
                    && record.status == CoverageStatus::Partial
                    && record.anchor.is_some()
            })
            .count(),
        2,
        "coverage={:?}",
        graph.graph.coverage
    );
    assert_eq!(
        graph
            .graph
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "sql_incomplete_body_boundary")
            .count(),
        2,
        "diagnostics={:?}",
        graph.graph.diagnostics
    );

    let procedure = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "app.refresh")
        .ok_or("missing refresh procedure")?;
    assert_eq!(
        graph
            .links
            .iter()
            .filter(|edge| {
                edge.source == procedure.id
                    && edge.target == users.id
                    && edge.kind == EdgeKind::Reads
            })
            .count(),
        2,
        "links={:?}",
        graph.links
    );
    let audit = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.audit")
        .ok_or("missing audit")?;
    assert!(graph.links.iter().any(|edge| {
        edge.source == procedure.id && edge.target == audit.id && edge.kind == EdgeKind::Writes
    }));

    let quoted = graph
        .nodes
        .iter()
        .find(|node| {
            node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.semi;\"colon"
        })
        .ok_or("missing quoted semicolon table")?;
    let quoted_read = graph
        .links
        .iter()
        .find(|edge| edge.target == quoted.id && edge.kind == EdgeKind::Reads)
        .ok_or("missing quoted semicolon read")?;
    assert!(quoted_read.evidence.iter().all(|evidence| {
        evidence.origin == EvidenceOrigin::Artifact
            && evidence.confidence == EvidenceConfidence::Exact
            && evidence.anchors.len() == 1
            && evidence
                .rule
                .as_deref()
                .is_some_and(|rule| rule.starts_with("sql-text-"))
    }));
    Ok(())
}
