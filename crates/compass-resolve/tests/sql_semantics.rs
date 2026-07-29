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

#[test]
fn nested_ddl_remains_owned_by_complete_routines_in_strict_v1() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V6__nested_ddl.sql");
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE PROCEDURE app.refresh_dollar() AS $body$
BEGIN
  CREATE TEMP TABLE app.scratch_dollar (id BIGINT);
  WITH recent AS (
    SELECT id FROM app.users
  )
  INSERT INTO app.scratch_dollar SELECT id FROM recent;
  INSERT INTO app.scratch_dollar SELECT id FROM app.users;
END;
$body$ LANGUAGE plpgsql;
CREATE TRIGGER app.refresh_compound AFTER UPDATE ON app.users
FOR EACH ROW BEGIN
  CREATE TEMP TABLE app.scratch_compound (id BIGINT);
  INSERT INTO app.scratch_compound SELECT id FROM app.users;
END;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-nested-ddl")?;
    let graph = normalize_v1(extraction, evidence)?;

    assert!(
        graph.nodes.iter().all(|node| node.kind != NodeKind::Query),
        "body DML leaked as file-owned queries: {:?}",
        graph.nodes
    );
    assert!(
        graph.graph.coverage.iter().all(|record| {
            record.capability != "sql:body_ownership"
                && record.capability != "sql:statement_boundary"
        }),
        "complete routine received partial coverage: {:?}",
        graph.graph.coverage
    );
    assert!(
        graph
            .graph
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "sql_incomplete_body_boundary"),
        "complete routine received an incomplete-body diagnostic: {:?}",
        graph.graph.diagnostics
    );
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| node.qualified_name != "recent"),
        "routine-local CTE became an entity: {:?}",
        graph.nodes
    );

    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    for (owner_name, target_name, expected_writes) in [
        ("app.refresh_dollar", "app.scratch_dollar", 2),
        ("app.refresh_compound", "app.scratch_compound", 1),
    ] {
        let owner = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == owner_name)
            .ok_or_else(|| format!("missing owner {owner_name}"))?;
        let target = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        let writes = graph
            .links
            .iter()
            .filter(|edge| {
                edge.source == owner.id && edge.target == target.id && edge.kind == EdgeKind::Writes
            })
            .collect::<Vec<_>>();
        assert_eq!(
            writes.len(),
            expected_writes,
            "owner={owner_name}, links={:?}",
            graph.links
        );
        assert_eq!(
            writes
                .iter()
                .filter_map(|edge| edge.relationship_site.as_ref())
                .map(|anchor| anchor.start_byte)
                .collect::<HashSet<_>>()
                .len(),
            expected_writes
        );
        assert!(writes.iter().all(|edge| {
            edge.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item.anchors.len() == 1
                    && item
                        .rule
                        .as_deref()
                        .is_some_and(|rule| rule.starts_with("sql-text-"))
            })
        }));
        assert!(graph.links.iter().any(|edge| {
            edge.source == owner.id && edge.target == users.id && edge.kind == EdgeKind::Reads
        }));
    }

    let repeated = extract_sql_content(relative, source);
    let repeated_evidence =
        BuildEvidence::from_extraction(root, &repeated, "sha256:sql-nested-ddl")?;
    let repeated_graph = normalize_v1(repeated, repeated_evidence)?;
    assert_eq!(
        graph
            .nodes
            .iter()
            .map(|node| (&node.id, node.kind))
            .collect::<Vec<_>>(),
        repeated_graph
            .nodes
            .iter()
            .map(|node| (&node.id, node.kind))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        graph.links.iter().map(|edge| &edge.id).collect::<Vec<_>>(),
        repeated_graph
            .links
            .iter()
            .map(|edge| &edge.id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn unmatched_identifier_quotes_publish_bounded_partial_strict_v1_evidence()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V7__unmatched_quotes.sql");
    let source = br#"
CREATE TABLE app.users (id BIGINT);
SELECT * FROM "broken;
SELECT * FROM app.users;
CREATE TABLE app.after_double (id BIGINT);
SELECT * FROM `broken;
SELECT * FROM app.users;
CREATE TABLE app.after_backtick (id BIGINT);
SELECT * FROM [broken;
SELECT * FROM app.users;
CREATE TABLE app.after_bracket (id BIGINT);
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-quotes")?;
    let graph = normalize_v1(extraction, evidence)?;

    for expected in [
        "app.after_double",
        "app.after_backtick",
        "app.after_bracket",
    ] {
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == NodeKind::DatabaseTable
                    && node.qualified_name == expected),
            "missing recovered declaration {expected}: {:?}",
            graph.nodes
        );
    }
    let queries = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Query)
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 3, "nodes={:?}", graph.nodes);
    assert!(queries.iter().all(|query| {
        query
            .source
            .as_ref()
            .and_then(|anchor| source.get(anchor.start_byte as usize..anchor.end_byte as usize))
            .is_some_and(|statement| statement.starts_with(b"SELECT * FROM app.users"))
    }));

    let partials = graph
        .graph
        .coverage
        .iter()
        .filter(|record| {
            record.capability == "sql:statement_boundary"
                && record.status == CoverageStatus::Partial
        })
        .collect::<Vec<_>>();
    let diagnostics = graph
        .graph
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "sql_incomplete_quoted_identifier")
        .collect::<Vec<_>>();
    assert_eq!(partials.len(), 3, "coverage={:?}", graph.graph.coverage);
    assert_eq!(
        diagnostics.len(),
        3,
        "diagnostics={:?}",
        graph.graph.diagnostics
    );
    for marker in *b"\"`[" {
        let start = source
            .iter()
            .position(|byte| *byte == marker)
            .ok_or("missing quote marker")?;
        let end = source[start..]
            .iter()
            .position(|byte| matches!(*byte, b';' | b'\n'))
            .map(|offset| start + offset + 1)
            .ok_or("missing physical recovery boundary")?;
        assert!(
            partials.iter().any(|record| {
                record.anchor.as_ref().is_some_and(|anchor| {
                    anchor.start_byte == start as u64
                        && anchor.end_byte == end as u64
                        && anchor.file == relative.to_string_lossy()
                })
            }),
            "marker={marker:?}, expected={start}..{end}, coverage={partials:?}"
        );
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.anchor.as_ref().is_some_and(|anchor| {
                anchor.start_byte == start as u64
                    && anchor.end_byte == end as u64
                    && anchor.file == relative.to_string_lossy()
            })
        }));
    }

    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    let reads = graph
        .links
        .iter()
        .filter(|edge| edge.target == users.id && edge.kind == EdgeKind::Reads)
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 3, "links={:?}", graph.links);
    assert!(reads.iter().all(|edge| {
        edge.evidence.iter().all(|item| {
            item.origin == EvidenceOrigin::Artifact
                && item.confidence == EvidenceConfidence::Exact
                && item.anchors.len() == 1
        })
    }));
    Ok(())
}

#[test]
fn dollar_quoted_defaults_do_not_shadow_the_strict_v1_routine_body() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V8__dollar_default.sql");
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE FUNCTION app.refresh(
  arg text DEFAULT $$fallback$$
) RETURNS void AS $body$
BEGIN
  SELECT * FROM app.users;
  SELECT * FROM app.users;
END;
$body$ LANGUAGE plpgsql;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-dollar-default")?;
    let graph = normalize_v1(extraction, evidence)?;

    let procedure = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "app.refresh")
        .ok_or("missing function")?;
    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    let reads = graph
        .links
        .iter()
        .filter(|edge| {
            edge.source == procedure.id && edge.target == users.id && edge.kind == EdgeKind::Reads
        })
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 2, "links={:?}", graph.links);
    assert_eq!(
        reads
            .iter()
            .filter_map(|edge| edge.relationship_site.as_ref())
            .map(|anchor| anchor.start_byte)
            .collect::<HashSet<_>>()
            .len(),
        2
    );
    assert!(reads.iter().all(|edge| {
        edge.evidence.iter().all(|item| {
            item.origin == EvidenceOrigin::Artifact
                && item.confidence == EvidenceConfidence::Exact
                && item.anchors.len() == 1
                && item
                    .rule
                    .as_deref()
                    .is_some_and(|rule| rule.starts_with("sql-text-"))
        })
    }));
    assert!(
        graph.nodes.iter().all(|node| node.kind != NodeKind::Query),
        "body statements leaked into file-owned queries: {:?}",
        graph.nodes
    );
    assert!(
        graph.graph.coverage.iter().all(|record| {
            record.capability != "sql:body_ownership"
                && record.capability != "sql:statement_boundary"
        }),
        "valid body received partial coverage: {:?}",
        graph.graph.coverage
    );
    Ok(())
}

#[test]
fn same_style_identifier_quotes_recover_at_the_original_strict_v1_boundary()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V9__same_style_quotes.sql");
    let source = br#"
CREATE TABLE "app"."double_users" (id BIGINT);
CREATE TABLE `app`.`backtick_users` (id BIGINT);
CREATE TABLE [app].[bracket_users] (id BIGINT);
SELECT * FROM "broken;
SELECT * FROM "app"."double_users";
CREATE TABLE "app"."after_double" (id BIGINT);
CREATE TABLE "broken_double_decl;
CREATE TABLE "app"."after_double_decl" (id BIGINT);
SELECT * FROM `broken;
SELECT * FROM `app`.`backtick_users`;
CREATE TABLE `app`.`after_backtick` (id BIGINT);
CREATE TABLE `broken_backtick_decl;
CREATE TABLE `app`.`after_backtick_decl` (id BIGINT);
SELECT * FROM [broken;
SELECT * FROM [app].[bracket_users];
CREATE TABLE [app].[after_bracket] (id BIGINT);
CREATE TABLE [broken_bracket_decl;
CREATE TABLE [app].[after_bracket_decl] (id BIGINT);
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-same-quotes")?;
    let graph = normalize_v1(extraction, evidence)?;

    for expected in [
        "app.after_double",
        "app.after_double_decl",
        "app.after_backtick",
        "app.after_backtick_decl",
        "app.after_bracket",
        "app.after_bracket_decl",
    ] {
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == NodeKind::DatabaseTable
                    && node.qualified_name == expected),
            "missing recovered declaration {expected}: {:?}",
            graph.nodes
        );
    }
    let queries = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Query)
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 3, "nodes={:?}", graph.nodes);
    for target_name in [
        "app.double_users",
        "app.backtick_users",
        "app.bracket_users",
    ] {
        let target = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        let read = graph
            .links
            .iter()
            .find(|edge| {
                edge.target == target.id
                    && edge.kind == EdgeKind::Reads
                    && queries.iter().any(|query| query.id == edge.source)
            })
            .ok_or_else(|| format!("missing read for {target_name}"))?;
        assert!(read.evidence.iter().all(|item| {
            item.origin == EvidenceOrigin::Artifact
                && item.confidence == EvidenceConfidence::Exact
                && item.anchors.len() == 1
                && item
                    .rule
                    .as_deref()
                    .is_some_and(|rule| rule.starts_with("sql-text-"))
        }));
    }

    let partials = graph
        .graph
        .coverage
        .iter()
        .filter(|record| {
            record.capability == "sql:statement_boundary"
                && record.status == CoverageStatus::Partial
        })
        .collect::<Vec<_>>();
    let diagnostics = graph
        .graph
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "sql_incomplete_quoted_identifier")
        .collect::<Vec<_>>();
    assert_eq!(partials.len(), 6, "coverage={:?}", graph.graph.coverage);
    assert_eq!(
        diagnostics.len(),
        6,
        "diagnostics={:?}",
        graph.graph.diagnostics
    );
    for malformed in [
        b"\"broken;".as_slice(),
        b"\"broken_double_decl",
        b"`broken;",
        b"`broken_backtick_decl",
        b"[broken;",
        b"[broken_bracket_decl",
    ] {
        let start = source
            .windows(malformed.len())
            .position(|window| window == malformed)
            .ok_or("missing malformed quote")?;
        let end = source[start..]
            .iter()
            .position(|byte| matches!(*byte, b';' | b'\n'))
            .map(|offset| start + offset + 1)
            .ok_or("missing recovery boundary")?;
        assert!(partials.iter().any(|record| {
            record.anchor.as_ref().is_some_and(|anchor| {
                anchor.start_byte == start as u64
                    && anchor.end_byte == end as u64
                    && anchor.file == relative.to_string_lossy()
            })
        }));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.anchor.as_ref().is_some_and(|anchor| {
                anchor.start_byte == start as u64
                    && anchor.end_byte == end as u64
                    && anchor.file == relative.to_string_lossy()
            })
        }));
    }
    Ok(())
}

#[test]
fn nested_cte_scopes_publish_only_physical_lineage_in_strict_v1() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V10__nested_ctes.sql");
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE TABLE app.audit (id BIGINT);
WITH outer_cte AS (
  WITH RECURSIVE inner_cte AS MATERIALIZED (
    SELECT id FROM app.users
    UNION ALL SELECT id FROM inner_cte
  ), inner_two AS NOT MATERIALIZED (
    SELECT inner_cte.id FROM inner_cte JOIN app.users ON app.users.id = inner_cte.id
  ), scope_local AS (
    SELECT id FROM inner_two
  )
  SELECT id FROM scope_local
)
INSERT INTO app.audit
SELECT outer_cte.id FROM outer_cte JOIN scope_local ON scope_local.id = outer_cte.id;
CREATE PROCEDURE app.refresh_nested() AS $body$
BEGIN
  WITH shadowed AS (
    WITH shadowed AS NOT MATERIALIZED (
      SELECT id FROM app.users
    )
    SELECT id FROM shadowed
  ), second AS MATERIALIZED (
    SELECT shadowed.id FROM shadowed JOIN app.users ON app.users.id = shadowed.id
  )
  INSERT INTO app.audit SELECT id FROM second;
END;
$body$ LANGUAGE plpgsql;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-nested-ctes")?;
    let graph = normalize_v1(extraction, evidence)?;

    for alias in ["outer_cte", "inner_cte", "inner_two", "shadowed", "second"] {
        assert!(
            graph.nodes.iter().all(|node| node.qualified_name != alias),
            "CTE alias became an exact node {alias}: {:?}",
            graph.nodes
        );
    }
    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    let audit = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.audit")
        .ok_or("missing audit")?;
    let query = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::Query)
        .ok_or("missing query")?;
    let procedure = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "app.refresh_nested")
        .ok_or("missing procedure")?;
    let scope_local = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "scope_local")
        .ok_or("same spelling outside the nested CTE scope must remain a physical table")?;
    assert_eq!(
        graph
            .links
            .iter()
            .filter(|edge| {
                edge.source == query.id
                    && edge.target == scope_local.id
                    && edge.kind == EdgeKind::Reads
            })
            .count(),
        1,
        "nested CTE scope leaked beyond its enclosing subquery: {:?}",
        graph.links
    );
    for owner in [query, procedure] {
        let reads = graph
            .links
            .iter()
            .filter(|edge| {
                edge.source == owner.id && edge.target == users.id && edge.kind == EdgeKind::Reads
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reads.len(),
            2,
            "owner={}, links={:?}",
            owner.id,
            graph.links
        );
        assert_eq!(
            reads
                .iter()
                .filter_map(|edge| edge.relationship_site.as_ref())
                .map(|anchor| anchor.start_byte)
                .collect::<HashSet<_>>()
                .len(),
            2
        );
        assert!(reads.iter().all(|edge| {
            edge.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item.anchors.len() == 1
                    && item
                        .rule
                        .as_deref()
                        .is_some_and(|rule| rule.starts_with("sql-text-"))
            })
        }));
        assert_eq!(
            graph
                .links
                .iter()
                .filter(|edge| {
                    edge.source == owner.id
                        && edge.target == audit.id
                        && edge.kind == EdgeKind::Writes
                })
                .count(),
            1,
            "owner={}, links={:?}",
            owner.id,
            graph.links
        );
    }
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Query)
            .count(),
        1,
        "routine CTE leaked as a file-owned query: {:?}",
        graph.nodes
    );
    assert!(
        graph.graph.coverage.iter().all(|record| {
            record.capability != "sql:body_ownership"
                && record.capability != "sql:statement_boundary"
        }),
        "valid nested CTEs received partial coverage: {:?}",
        graph.graph.coverage
    );

    let repeated = extract_sql_content(relative, source);
    let repeated_evidence =
        BuildEvidence::from_extraction(root, &repeated, "sha256:sql-nested-ctes")?;
    let repeated_graph = normalize_v1(repeated, repeated_evidence)?;
    assert_eq!(
        graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
        repeated_graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        graph.links.iter().map(|edge| &edge.id).collect::<Vec<_>>(),
        repeated_graph
            .links
            .iter()
            .map(|edge| &edge.id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn paired_semicolon_keyword_identifiers_publish_exact_strict_v1_lineage()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V11__quoted_keyword_content.sql");
    let source = br#"
CREATE TABLE "app"."semi;SELECT""tail" (id BIGINT);
SELECT * FROM "app"."semi;SELECT""tail";
CREATE TABLE `app`.`semi;UPDATE``tail` (id BIGINT);
SELECT * FROM `app`.`semi;UPDATE``tail`;
CREATE TABLE [app].[semi;DELETE]]tail] (id BIGINT);
SELECT * FROM [app].[semi;DELETE]]tail];
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-quoted-keywords")?;
    let graph = normalize_v1(extraction, evidence)?;

    let expected = [
        (
            "app.semi;SELECT\"tail",
            br#""app"."semi;SELECT""tail""#.as_slice(),
        ),
        ("app.semi;UPDATE`tail", b"`app`.`semi;UPDATE``tail`"),
        ("app.semi;DELETE]tail", b"[app].[semi;DELETE]]tail]"),
    ];
    for (qualified_name, spelling) in expected {
        let table = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::DatabaseTable && node.qualified_name == qualified_name
            })
            .ok_or_else(|| format!("missing table {qualified_name}"))?;
        let node_anchor = table.source.as_ref().ok_or("missing table anchor")?;
        assert_eq!(
            source.get(node_anchor.start_byte as usize..node_anchor.end_byte as usize),
            Some(spelling)
        );
        let read = graph
            .links
            .iter()
            .find(|edge| edge.target == table.id && edge.kind == EdgeKind::Reads)
            .ok_or_else(|| format!("missing read for {qualified_name}"))?;
        let read_anchor = read
            .relationship_site
            .as_ref()
            .ok_or("missing read anchor")?;
        assert_eq!(
            source.get(read_anchor.start_byte as usize..read_anchor.end_byte as usize),
            Some(spelling)
        );
        assert!(read.evidence.iter().all(|item| {
            item.origin == EvidenceOrigin::Artifact
                && item.confidence == EvidenceConfidence::Exact
                && item.anchors.len() == 1
                && item
                    .rule
                    .as_deref()
                    .is_some_and(|rule| rule.starts_with("sql-text-"))
        }));
    }
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Query)
            .count(),
        3
    );
    assert!(
        graph
            .graph
            .coverage
            .iter()
            .all(|record| record.capability != "sql:statement_boundary"),
        "valid paired identifiers received recovery coverage: {:?}",
        graph.graph.coverage
    );
    assert!(
        graph
            .graph
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "sql_incomplete_quoted_identifier"),
        "valid paired identifiers received diagnostics: {:?}",
        graph.graph.diagnostics
    );
    Ok(())
}

#[test]
fn lexical_alias_scopes_publish_exact_outer_writes_in_strict_v1() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V12__alias_scopes.sql");
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE TABLE app.audit (id BIGINT);
CREATE TABLE app.archive (id BIGINT);
UPDATE u SET id = 1
FROM app.users AS u
WHERE EXISTS (SELECT 1 FROM app.audit AS u WHERE u.id = 1)
   OR EXISTS (SELECT 1 FROM app.archive AS u WHERE u.id = 1);
DELETE FROM u USING app.users AS u
WHERE EXISTS (SELECT 1 FROM app.audit AS u WHERE u.id = 2)
   OR EXISTS (SELECT 1 FROM app.archive AS u WHERE u.id = 2);
MERGE INTO u USING app.users AS u
ON EXISTS (SELECT 1 FROM app.audit AS u WHERE u.id = 3)
OR EXISTS (SELECT 1 FROM app.archive AS u WHERE u.id = 3)
WHEN MATCHED THEN UPDATE SET id = 3;
UPDATE u SET id = 4
WHERE EXISTS (SELECT 1 FROM app.audit AS u WHERE u.id = 4);
CREATE PROCEDURE app.refresh_aliases() AS $body$
BEGIN
  UPDATE u SET id = 5
  FROM app.users AS u
  WHERE EXISTS (SELECT 1 FROM app.audit AS u WHERE u.id = 5)
     OR EXISTS (SELECT 1 FROM app.archive AS u WHERE u.id = 5);
END;
$body$ LANGUAGE plpgsql;
CREATE TRIGGER app.capture_aliases AFTER UPDATE ON app.users
FOR EACH ROW BEGIN
  DELETE FROM u USING app.users AS u
  WHERE EXISTS (SELECT 1 FROM app.audit AS u WHERE u.id = 6);
  MERGE INTO u USING app.users AS u
  ON EXISTS (SELECT 1 FROM app.archive AS u WHERE u.id = 7)
  WHEN MATCHED THEN UPDATE SET id = 7;
END;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-alias-scopes")?;
    let graph = normalize_v1(extraction, evidence)?;

    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    let audit = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.audit")
        .ok_or("missing audit")?;
    let archive = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.archive")
        .ok_or("missing archive")?;
    let physical_u = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "u")
        .ok_or("same spelling outside nested alias scope must remain physical")?;
    let queries = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Query)
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 4, "nodes={:?}", graph.nodes);
    assert_eq!(
        graph
            .links
            .iter()
            .filter(|edge| {
                queries.iter().any(|query| query.id == edge.source)
                    && edge.target == users.id
                    && edge.kind == EdgeKind::Writes
            })
            .count(),
        3,
        "top-level aliases resolved to the wrong target: {:?}",
        graph.links
    );
    assert_eq!(
        graph
            .links
            .iter()
            .filter(|edge| {
                queries.iter().any(|query| query.id == edge.source)
                    && edge.target == physical_u.id
                    && edge.kind == EdgeKind::Writes
            })
            .count(),
        1
    );
    assert!(graph.links.iter().all(|edge| {
        !queries.iter().any(|query| query.id == edge.source)
            || edge.kind != EdgeKind::Writes
            || (edge.target != audit.id && edge.target != archive.id)
    }));

    for (owner_name, expected_writes) in [("app.refresh_aliases", 1), ("app.capture_aliases", 2)] {
        let owner = graph
            .nodes
            .iter()
            .find(|node| node.qualified_name == owner_name)
            .ok_or_else(|| format!("missing owner {owner_name}"))?;
        let writes = graph
            .links
            .iter()
            .filter(|edge| {
                edge.source == owner.id && edge.target == users.id && edge.kind == EdgeKind::Writes
            })
            .collect::<Vec<_>>();
        assert_eq!(
            writes.len(),
            expected_writes,
            "owner={owner_name}, links={:?}",
            graph.links
        );
        assert_eq!(
            writes
                .iter()
                .filter_map(|edge| edge.relationship_site.as_ref())
                .map(|anchor| anchor.start_byte)
                .collect::<HashSet<_>>()
                .len(),
            expected_writes
        );
        for write in writes {
            let anchor = write
                .relationship_site
                .as_ref()
                .ok_or("missing write anchor")?;
            assert_eq!(
                source.get(anchor.start_byte as usize..anchor.end_byte as usize),
                Some(b"u".as_slice())
            );
            assert!(write.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item.anchors.len() == 1
                    && item
                        .rule
                        .as_deref()
                        .is_some_and(|rule| rule.starts_with("sql-text-"))
            }));
        }
    }
    assert!(
        graph.graph.coverage.iter().all(|record| {
            record.capability != "sql:body_ownership"
                && record.capability != "sql:statement_boundary"
        }),
        "valid alias scopes received partial coverage: {:?}",
        graph.graph.coverage
    );

    let repeated = extract_sql_content(relative, source);
    let repeated_evidence =
        BuildEvidence::from_extraction(root, &repeated, "sha256:sql-alias-scopes")?;
    let repeated_graph = normalize_v1(repeated, repeated_evidence)?;
    assert_eq!(
        graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
        repeated_graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        graph.links.iter().map(|edge| &edge.id).collect::<Vec<_>>(),
        repeated_graph
            .links
            .iter()
            .map(|edge| &edge.id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn unified_sql_lexing_publishes_only_proven_identifier_facts_in_strict_v1()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V13__unified_lexing.sql");
    let source = br#"
CREATE TABLE "app"."dash--SELECT" (id BIGINT);
SELECT * FROM "app"."dash--SELECT";
CREATE TABLE `app`.`block/*UPDATE*/tail` (id BIGINT);
SELECT * FROM `app`.`block/*UPDATE*/tail`;
CREATE TABLE [app].[apostrophe'SELECT] (id BIGINT);
SELECT * FROM [app].[apostrophe'SELECT];
CREATE TABLE "app"."multi
double" (id BIGINT);
SELECT * FROM "app"."multi
double";
CREATE TABLE `app`.`multi
backtick` (id BIGINT);
SELECT * FROM `app`.`multi
backtick`;
CREATE TABLE [app].[multi
bracket] (id BIGINT);
SELECT * FROM [app].[multi
bracket];
CREATE TABLE "app"."double_users" (id BIGINT);
CREATE TABLE `app`.`backtick_users` (id BIGINT);
CREATE TABLE [app].[bracket_users] (id BIGINT);
SELECT * FROM "broken; SELECT * FROM "app"."double_users"; CREATE TABLE "app"."after_double" (id BIGINT);
SELECT * FROM `broken; SELECT * FROM `app`.`backtick_users`; CREATE TABLE `app`.`after_backtick` (id BIGINT);
SELECT * FROM [broken; SELECT * FROM [app].[bracket_users]; CREATE TABLE [app].[after_bracket] (id BIGINT);
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-unified-lexing")?;
    let graph = normalize_v1(extraction, evidence)?;

    let expected = [
        ("app.dash--SELECT", br#""app"."dash--SELECT""#.as_slice()),
        ("app.block/*UPDATE*/tail", b"`app`.`block/*UPDATE*/tail`"),
        ("app.apostrophe'SELECT", b"[app].[apostrophe'SELECT]"),
        ("app.multi\ndouble", b"\"app\".\"multi\ndouble\""),
        ("app.multi\nbacktick", b"`app`.`multi\nbacktick`"),
        ("app.multi\nbracket", b"[app].[multi\nbracket]"),
    ];
    for (qualified_name, spelling) in expected {
        let table = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::DatabaseTable && node.qualified_name == qualified_name
            })
            .ok_or_else(|| format!("missing table {qualified_name:?}"))?;
        let read = graph
            .links
            .iter()
            .find(|edge| edge.target == table.id && edge.kind == EdgeKind::Reads)
            .ok_or_else(|| format!("missing read for {qualified_name:?}"))?;
        for anchor in [
            table.source.as_ref().ok_or("missing table anchor")?,
            read.relationship_site
                .as_ref()
                .ok_or("missing read anchor")?,
        ] {
            assert_eq!(
                source.get(anchor.start_byte as usize..anchor.end_byte as usize),
                Some(spelling)
            );
        }
        assert!(read.evidence.iter().all(|item| {
            item.origin == EvidenceOrigin::Artifact
                && item.confidence == EvidenceConfidence::Exact
                && item.anchors.len() == 1
        }));
    }
    for target_name in [
        "app.double_users",
        "app.backtick_users",
        "app.bracket_users",
    ] {
        let target = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        assert_eq!(
            graph
                .links
                .iter()
                .filter(|edge| edge.target == target.id && edge.kind == EdgeKind::Reads)
                .count(),
            1
        );
    }
    for declaration in [
        "app.after_double",
        "app.after_backtick",
        "app.after_bracket",
    ] {
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.kind == NodeKind::DatabaseTable
                    && node.qualified_name == declaration),
            "missing recovered declaration {declaration}"
        );
    }
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| !node.qualified_name.contains("broken")),
        "malformed span created a phantom node: {:?}",
        graph.nodes
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Query)
            .count(),
        9
    );
    let partials = graph
        .graph
        .coverage
        .iter()
        .filter(|record| {
            record.capability == "sql:statement_boundary"
                && record.status == CoverageStatus::Partial
        })
        .collect::<Vec<_>>();
    let diagnostics = graph
        .graph
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "sql_incomplete_quoted_identifier")
        .collect::<Vec<_>>();
    assert_eq!(partials.len(), 3, "coverage={:?}", graph.graph.coverage);
    assert_eq!(
        diagnostics.len(),
        3,
        "diagnostics={:?}",
        graph.graph.diagnostics
    );
    for malformed in [b"\"broken".as_slice(), b"`broken", b"[broken"] {
        let start = source
            .windows(malformed.len())
            .position(|window| window == malformed)
            .ok_or("missing malformed opener")?;
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b';')
            .map(|offset| start + offset + 1)
            .ok_or("missing recovery boundary")?;
        assert!(partials.iter().any(|record| {
            record.anchor.as_ref().is_some_and(|anchor| {
                anchor.start_byte == start as u64
                    && anchor.end_byte == end as u64
                    && anchor.file == relative.to_string_lossy()
            })
        }));
    }
    Ok(())
}

#[test]
fn cte_alias_writes_are_omitted_from_strict_v1_without_phantom_tables() -> Result<(), Box<dyn Error>>
{
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V14__cte_alias_writes.sql");
    let source = br#"
CREATE TABLE app.users (id BIGINT);
WITH src AS (SELECT id FROM app.users)
UPDATE s SET id = 1 FROM src AS s;
WITH src AS (SELECT id FROM app.users)
DELETE FROM s USING src AS s;
WITH src AS (SELECT id FROM app.users)
MERGE INTO s USING src AS s ON 1 = 1 WHEN MATCHED THEN UPDATE SET id = 3;
WITH src AS (
  WITH src AS (SELECT id FROM app.users)
  SELECT id FROM src
)
UPDATE s SET id = 4 FROM src AS s;
CREATE PROCEDURE app.refresh_cte_aliases() AS $body$
BEGIN
  WITH src AS (SELECT id FROM app.users), wrapper AS (SELECT id FROM src)
  UPDATE s SET id = 5 FROM wrapper AS s;
END;
$body$ LANGUAGE plpgsql;
CREATE TRIGGER app.capture_cte_alias AFTER UPDATE ON app.users
FOR EACH ROW BEGIN
  WITH src AS (SELECT id FROM app.users)
  DELETE FROM s USING src AS s;
END;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:sql-cte-alias")?;
    let graph = normalize_v1(extraction, evidence)?;

    for alias in ["src", "wrapper"] {
        assert!(
            graph.nodes.iter().all(|node| node.qualified_name != alias),
            "CTE alias became a physical node {alias}: {:?}",
            graph.nodes
        );
    }
    assert!(
        graph.links.iter().all(|edge| edge.kind != EdgeKind::Writes),
        "unproven CTE updateability became an exact write: {:?}",
        graph.links
    );
    let users = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.users")
        .ok_or("missing users")?;
    let reads = graph
        .links
        .iter()
        .filter(|edge| edge.target == users.id && edge.kind == EdgeKind::Reads)
        .collect::<Vec<_>>();
    assert!(reads.len() >= 6, "links={:?}", graph.links);
    assert!(reads.iter().all(|edge| {
        edge.evidence.iter().all(|item| {
            item.origin == EvidenceOrigin::Artifact
                && item.confidence == EvidenceConfidence::Exact
                && item.anchors.len() == 1
                && item
                    .rule
                    .as_deref()
                    .is_some_and(|rule| rule.starts_with("sql-text-"))
        })
    }));
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Query)
            .count(),
        4
    );
    assert!(
        graph.graph.coverage.iter().all(|record| {
            record.capability != "sql:body_ownership"
                && record.capability != "sql:statement_boundary"
        }),
        "valid CTE aliases received partial coverage: {:?}",
        graph.graph.coverage
    );
    let repeated = extract_sql_content(relative, source);
    let repeated_evidence =
        BuildEvidence::from_extraction(root, &repeated, "sha256:sql-cte-alias")?;
    let repeated_graph = normalize_v1(repeated, repeated_evidence)?;
    assert_eq!(
        graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
        repeated_graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        graph.links.iter().map(|edge| &edge.id).collect::<Vec<_>>(),
        repeated_graph
            .links
            .iter()
            .map(|edge| &edge.id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn quote_family_recovery_cross_product_is_conservative_and_exact_in_strict_v1()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V15__quote_cross_product.sql");
    let source = br#"
CREATE TABLE "app"."double_semi; SELECT" (id BIGINT);
SELECT * FROM "app"."double_semi; SELECT";
CREATE TABLE "app"."double_multi;
SELECT" (id BIGINT);
SELECT * FROM "app"."double_multi;
SELECT";
CREATE TABLE "app"."double_escaped; SELECT ""tail""" (id BIGINT);
SELECT * FROM "app"."double_escaped; SELECT ""tail""";
CREATE TABLE `app`.`backtick_semi; SELECT` (id BIGINT);
SELECT * FROM `app`.`backtick_semi; SELECT`;
CREATE TABLE `app`.`backtick_multi;
SELECT` (id BIGINT);
SELECT * FROM `app`.`backtick_multi;
SELECT`;
CREATE TABLE `app`.`backtick_escaped; SELECT ``tail``` (id BIGINT);
SELECT * FROM `app`.`backtick_escaped; SELECT ``tail```;
CREATE TABLE [app].[bracket_semi; SELECT] (id BIGINT);
SELECT * FROM [app].[bracket_semi; SELECT];
CREATE TABLE [app].[bracket_multi;
SELECT] (id BIGINT);
SELECT * FROM [app].[bracket_multi;
SELECT];
CREATE TABLE [app].[bracket_escaped; SELECT ]]tail] (id BIGINT);
SELECT * FROM [app].[bracket_escaped; SELECT ]]tail];
CREATE TABLE "app"."double_target" (id BIGINT);
CREATE TABLE `app`.`backtick_target` (id BIGINT);
CREATE TABLE [app].[bracket_target] (id BIGINT);
SELECT * FROM "broken_double_ws; SELECT * FROM "app"."double_target";
SELECT * FROM "broken_double_tight;SELECT * FROM "app"."double_target";
SELECT * FROM `broken_backtick_ws; SELECT * FROM `app`.`backtick_target`;
SELECT * FROM `broken_backtick_tight;SELECT * FROM `app`.`backtick_target`;
SELECT * FROM [broken_bracket_ws; SELECT * FROM [app].[bracket_target];
SELECT * FROM [broken_bracket_tight;SELECT * FROM [app].[bracket_target];
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:quote-cross-product")?;
    let graph = normalize_v1(extraction, evidence)?;

    for target_name in [
        "app.double_target",
        "app.backtick_target",
        "app.bracket_target",
    ] {
        let target = graph
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        let reads = graph
            .links
            .iter()
            .filter(|edge| edge.target == target.id && edge.kind == EdgeKind::Reads)
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), 2, "links={:?}", graph.links);
        assert!(reads.iter().all(|edge| {
            edge.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item.anchors.len() == 1
            })
        }));
    }
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| !node.qualified_name.contains("broken_"))
    );
    for (qualified_name, spelling) in [
        (
            "app.double_semi; SELECT",
            br#""app"."double_semi; SELECT""#.as_slice(),
        ),
        (
            "app.double_multi;\nSELECT",
            b"\"app\".\"double_multi;\nSELECT\"".as_slice(),
        ),
        (
            "app.double_escaped; SELECT \"tail\"",
            br#""app"."double_escaped; SELECT ""tail""""#.as_slice(),
        ),
        (
            "app.backtick_semi; SELECT",
            b"`app`.`backtick_semi; SELECT`".as_slice(),
        ),
        (
            "app.backtick_multi;\nSELECT",
            b"`app`.`backtick_multi;\nSELECT`".as_slice(),
        ),
        (
            "app.backtick_escaped; SELECT `tail`",
            b"`app`.`backtick_escaped; SELECT ``tail```".as_slice(),
        ),
        (
            "app.bracket_semi; SELECT",
            b"[app].[bracket_semi; SELECT]".as_slice(),
        ),
        (
            "app.bracket_multi;\nSELECT",
            b"[app].[bracket_multi;\nSELECT]".as_slice(),
        ),
        (
            "app.bracket_escaped; SELECT ]tail",
            b"[app].[bracket_escaped; SELECT ]]tail]".as_slice(),
        ),
    ] {
        let table = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::DatabaseTable && node.qualified_name == qualified_name
            })
            .ok_or_else(|| format!("missing valid paired identifier {qualified_name:?}"))?;
        let read = graph
            .links
            .iter()
            .find(|edge| edge.target == table.id && edge.kind == EdgeKind::Reads)
            .ok_or_else(|| format!("missing read for {qualified_name:?}"))?;
        for anchor in [
            table.source.as_ref().ok_or("missing table source")?,
            read.relationship_site.as_ref().ok_or("missing read site")?,
        ] {
            assert_eq!(
                source.get(anchor.start_byte as usize..anchor.end_byte as usize),
                Some(spelling)
            );
        }
    }
    let partials = graph
        .graph
        .coverage
        .iter()
        .filter(|record| {
            record.capability == "sql:statement_boundary"
                && record.status == CoverageStatus::Partial
        })
        .collect::<Vec<_>>();
    let diagnostics = graph
        .graph
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "sql_incomplete_quoted_identifier")
        .collect::<Vec<_>>();
    assert_eq!(partials.len(), 6, "coverage={:?}", graph.graph.coverage);
    assert_eq!(
        diagnostics.len(),
        6,
        "diagnostics={:?}",
        graph.graph.diagnostics
    );
    for marker in [
        b"\"broken_double_ws".as_slice(),
        b"\"broken_double_tight",
        b"`broken_backtick_ws",
        b"`broken_backtick_tight",
        b"[broken_bracket_ws",
        b"[broken_bracket_tight",
    ] {
        let start = source
            .windows(marker.len())
            .position(|window| window == marker)
            .ok_or("missing malformed marker")?;
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b';')
            .map(|offset| start + offset + 1)
            .ok_or("missing recovery separator")?;
        assert_eq!(
            partials
                .iter()
                .filter(|record| {
                    record.anchor.as_ref().is_some_and(|anchor| {
                        anchor.start_byte == start as u64
                            && anchor.end_byte == end as u64
                            && anchor.file == relative.to_string_lossy()
                    })
                })
                .count(),
            1
        );
    }
    let repeated = extract_sql_content(relative, source);
    let repeated_evidence =
        BuildEvidence::from_extraction(root, &repeated, "sha256:quote-cross-product")?;
    let repeated_graph = normalize_v1(repeated, repeated_evidence)?;
    assert_eq!(
        graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
        repeated_graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        graph.links.iter().map(|edge| &edge.id).collect::<Vec<_>>(),
        repeated_graph
            .links
            .iter()
            .map(|edge| &edge.id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn dollar_literals_are_inert_and_owned_bodies_remain_exact_in_strict_v1()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V16__dollar_lexing.sql");
    let source = br#"
CREATE TABLE app.real (id BIGINT);
CREATE TABLE app.audit (id BIGINT);
SELECT $outer$
  FROM app.literal_phantom;
  UPDATE app.literal_write SET id = 1;
  "broken; SELECT * FROM app.quote_phantom
  -- JOIN app.comment_phantom
  /* DELETE FROM app.block_phantom */
  (($inner$ INSERT INTO app.nested_phantom $inner$))
$outer$ AS payload FROM app.real;
INSERT INTO app.audit
SELECT $value$ MERGE INTO app.merge_phantom USING app.other_phantom
  'apostrophe' [broken] /* FROM app.hidden */
$value$ FROM app.real;
CREATE PROCEDURE app.refresh(
  p_text TEXT DEFAULT $default$ FROM app.default_phantom; "broken $default$
) AS $body$
BEGIN
  SELECT $literal$ FROM app.body_phantom; UPDATE app.body_write
    -- FROM app.body_comment
    /* JOIN app.body_block */
    (($different$ DELETE FROM app.body_nested $different$))
  $literal$ FROM app.real;
  UPDATE app.audit SET id = 2;
END;
$body$ LANGUAGE plpgsql;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:dollar-lexing")?;
    let graph = normalize_v1(extraction, evidence)?;

    for phantom in [
        "app.literal_phantom",
        "app.literal_write",
        "app.quote_phantom",
        "app.comment_phantom",
        "app.block_phantom",
        "app.nested_phantom",
        "app.merge_phantom",
        "app.other_phantom",
        "app.hidden",
        "app.default_phantom",
        "app.body_phantom",
        "app.body_write",
        "app.body_comment",
        "app.body_block",
        "app.body_nested",
    ] {
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.qualified_name != phantom),
            "dollar literal produced phantom {phantom}: {:?}",
            graph.nodes
        );
    }
    let real = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.real")
        .ok_or("missing real table")?;
    let audit = graph
        .nodes
        .iter()
        .find(|node| node.kind == NodeKind::DatabaseTable && node.qualified_name == "app.audit")
        .ok_or("missing audit table")?;
    let routine = graph
        .nodes
        .iter()
        .find(|node| node.qualified_name == "app.refresh")
        .ok_or("missing routine")?;
    let queries = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::Query)
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 2, "nodes={:?}", graph.nodes);
    let exact_edges = graph
        .links
        .iter()
        .filter(|edge| {
            (queries.iter().any(|query| query.id == edge.source)
                && ((edge.target == real.id && edge.kind == EdgeKind::Reads)
                    || (edge.target == audit.id && edge.kind == EdgeKind::Writes)))
                || (edge.source == routine.id
                    && ((edge.target == real.id && edge.kind == EdgeKind::Reads)
                        || (edge.target == audit.id && edge.kind == EdgeKind::Writes)))
        })
        .collect::<Vec<_>>();
    assert_eq!(exact_edges.len(), 5, "links={:?}", graph.links);
    assert!(exact_edges.iter().all(|edge| {
        let expected = if edge.target == real.id {
            b"app.real".as_slice()
        } else {
            b"app.audit".as_slice()
        };
        edge.relationship_site.as_ref().is_some_and(|anchor| {
            source.get(anchor.start_byte as usize..anchor.end_byte as usize) == Some(expected)
        }) && edge.evidence.iter().all(|item| {
            item.origin == EvidenceOrigin::Artifact
                && item.confidence == EvidenceConfidence::Exact
                && item.anchors.len() == 1
                && item.rule.as_deref() == Some("sql-text-data-access")
        })
    }));
    assert!(graph.graph.coverage.iter().all(|record| {
        record.capability != "sql:body_ownership" && record.capability != "sql:statement_boundary"
    }));
    assert!(
        graph
            .graph
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "sql_incomplete_quoted_identifier")
    );
    let repeated = extract_sql_content(relative, source);
    let repeated_evidence =
        BuildEvidence::from_extraction(root, &repeated, "sha256:dollar-lexing")?;
    let repeated_graph = normalize_v1(repeated, repeated_evidence)?;
    assert_eq!(
        graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
        repeated_graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        graph.links.iter().map(|edge| &edge.id).collect::<Vec<_>>(),
        repeated_graph
            .links
            .iter()
            .map(|edge| &edge.id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn quoted_alias_followers_and_multiline_recovery_are_exact_in_strict_v1()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V17__quote_boundaries.sql");
    let source = br#"
CREATE TABLE "app"."double; SELECT 1" (id BIGINT);
CREATE TABLE "app"."after_double" (id BIGINT);
CREATE TABLE `app`.`backtick; SELECT 1` (id BIGINT);
CREATE TABLE `app`.`after_backtick` (id BIGINT);
CREATE TABLE [app].[bracket; SELECT 1] (id BIGINT);
CREATE TABLE [app].[after_bracket] (id BIGINT);
SELECT * FROM "app"."double; SELECT 1" AS "d";
SELECT * FROM "app"."double; SELECT 1" "d2" JOIN "app"."after_double" AS "ad" ON 1 = 1;
SELECT * FROM `app`.`backtick; SELECT 1` AS `b`;
SELECT * FROM `app`.`backtick; SELECT 1` `b2` JOIN `app`.`after_backtick` AS `ab` ON 1 = 1;
SELECT * FROM [app].[bracket; SELECT 1] AS [r];
SELECT * FROM [app].[bracket; SELECT 1] [r2] JOIN [app].[after_bracket] AS [ar] ON 1 = 1;
SELECT "expr; SELECT 1"::text FROM "app"."after_double" WHERE 1 = 1;
SELECT `expr; SELECT 1` + 1 FROM `app`.`after_backtick` WHERE 1 = 1;
SELECT [expr; SELECT 1] = 1 FROM [app].[after_bracket] WHERE 1 = 1;
SELECT * FROM "broken_double_ws
 SELECT * FROM "app"."after_double";
SELECT * FROM "broken_double_tight
SELECT * FROM "app"."after_double";
SELECT * FROM `broken_backtick_ws
 SELECT * FROM `app`.`after_backtick`;
SELECT * FROM `broken_backtick_tight
SELECT * FROM `app`.`after_backtick`;
SELECT * FROM [broken_bracket_ws
 SELECT * FROM [app].[after_bracket];
SELECT * FROM [broken_bracket_tight
SELECT * FROM [app].[after_bracket];
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:quote-boundaries")?;
    let graph = normalize_v1(extraction, evidence)?;

    for (qualified_name, spelling, expected_reads) in [
        (
            "app.double; SELECT 1",
            br#""app"."double; SELECT 1""#.as_slice(),
            2,
        ),
        ("app.after_double", br#""app"."after_double""#.as_slice(), 4),
        (
            "app.backtick; SELECT 1",
            b"`app`.`backtick; SELECT 1`".as_slice(),
            2,
        ),
        (
            "app.after_backtick",
            b"`app`.`after_backtick`".as_slice(),
            4,
        ),
        (
            "app.bracket; SELECT 1",
            b"[app].[bracket; SELECT 1]".as_slice(),
            2,
        ),
        ("app.after_bracket", b"[app].[after_bracket]".as_slice(), 4),
    ] {
        let table = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::DatabaseTable && node.qualified_name == qualified_name
            })
            .ok_or_else(|| format!("missing table {qualified_name}"))?;
        let reads = graph
            .links
            .iter()
            .filter(|edge| edge.target == table.id && edge.kind == EdgeKind::Reads)
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), expected_reads, "links={:?}", graph.links);
        for read in reads {
            let anchor = read.relationship_site.as_ref().ok_or("missing read site")?;
            assert_eq!(
                source.get(anchor.start_byte as usize..anchor.end_byte as usize),
                Some(spelling)
            );
            assert!(read.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item.anchors.len() == 1
                    && item.rule.as_deref() == Some("sql-text-data-access")
            }));
        }
    }
    assert!(
        graph
            .nodes
            .iter()
            .all(|node| !node.qualified_name.contains("broken_"))
    );
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Query)
            .count(),
        15
    );
    let partials = graph
        .graph
        .coverage
        .iter()
        .filter(|record| {
            record.capability == "sql:statement_boundary"
                && record.status == CoverageStatus::Partial
        })
        .collect::<Vec<_>>();
    let diagnostics = graph
        .graph
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "sql_incomplete_quoted_identifier")
        .collect::<Vec<_>>();
    assert_eq!(partials.len(), 6, "coverage={:?}", graph.graph.coverage);
    assert_eq!(
        diagnostics.len(),
        6,
        "diagnostics={:?}",
        graph.graph.diagnostics
    );
    for marker in [
        b"\"broken_double_ws".as_slice(),
        b"\"broken_double_tight",
        b"`broken_backtick_ws",
        b"`broken_backtick_tight",
        b"[broken_bracket_ws",
        b"[broken_bracket_tight",
    ] {
        let start = source
            .windows(marker.len())
            .position(|window| window == marker)
            .ok_or("missing malformed opener")?;
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| start + offset + 1)
            .ok_or("missing recovery newline")?;
        assert_eq!(
            partials
                .iter()
                .filter(|record| {
                    record.anchor.as_ref().is_some_and(|anchor| {
                        anchor.start_byte == start as u64
                            && anchor.end_byte == end as u64
                            && anchor.file == relative.to_string_lossy()
                    })
                })
                .count(),
            1
        );
    }
    let repeated = extract_sql_content(relative, source);
    let repeated_evidence =
        BuildEvidence::from_extraction(root, &repeated, "sha256:quote-boundaries")?;
    let repeated_graph = normalize_v1(repeated, repeated_evidence)?;
    assert_eq!(
        graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
        repeated_graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        graph.links.iter().map(|edge| &edge.id).collect::<Vec<_>>(),
        repeated_graph
            .links
            .iter()
            .map(|edge| &edge.id)
            .collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn dollar_delimiter_text_in_identifiers_stays_exact_in_strict_v1() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let relative = Path::new("db/migrations/V18__dollar_identifiers.sql");
    let source = br#"
CREATE TABLE app.foo$tag$bar (id BIGINT);
CREATE TABLE app.baz$tag$qux (id BIGINT);
CREATE TABLE app.foo$$bar (id BIGINT);
CREATE TABLE app.baz$$qux (id BIGINT);
SELECT * FROM app.foo$tag$bar;
SELECT * FROM app.baz$tag$qux;
SELECT * FROM app.foo$$bar;
SELECT * FROM app.baz$$qux;
SELECT $tag$ FROM app.literal_phantom $tag$::text, $$ UPDATE app.write_phantom $$ || 'x'
FROM app.foo$tag$bar;
"#;
    fs::create_dir_all(root.join("db/migrations"))?;
    fs::write(root.join(relative), source)?;
    let extraction = extract_sql_content(relative, source);
    let evidence = BuildEvidence::from_extraction(root, &extraction, "sha256:dollar-identifiers")?;
    let graph = normalize_v1(extraction, evidence)?;

    for (qualified_name, spelling, expected_reads) in [
        ("app.foo$tag$bar", b"app.foo$tag$bar".as_slice(), 2),
        ("app.baz$tag$qux", b"app.baz$tag$qux".as_slice(), 1),
        ("app.foo$$bar", b"app.foo$$bar".as_slice(), 1),
        ("app.baz$$qux", b"app.baz$$qux".as_slice(), 1),
    ] {
        let table = graph
            .nodes
            .iter()
            .find(|node| {
                node.kind == NodeKind::DatabaseTable && node.qualified_name == qualified_name
            })
            .ok_or_else(|| format!("missing table {qualified_name}"))?;
        let source_anchor = table.source.as_ref().ok_or("missing table source")?;
        assert_eq!(
            source.get(source_anchor.start_byte as usize..source_anchor.end_byte as usize),
            Some(spelling)
        );
        let reads = graph
            .links
            .iter()
            .filter(|edge| edge.target == table.id && edge.kind == EdgeKind::Reads)
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), expected_reads, "links={:?}", graph.links);
        for read in reads {
            let anchor = read.relationship_site.as_ref().ok_or("missing read site")?;
            assert_eq!(
                source.get(anchor.start_byte as usize..anchor.end_byte as usize),
                Some(spelling)
            );
            assert!(read.evidence.iter().all(|item| {
                item.origin == EvidenceOrigin::Artifact
                    && item.confidence == EvidenceConfidence::Exact
                    && item.anchors.len() == 1
            }));
        }
    }
    for phantom in ["app.literal_phantom", "app.write_phantom"] {
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.qualified_name != phantom)
        );
    }
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Query)
            .count(),
        5
    );
    assert!(graph.graph.coverage.iter().all(|record| {
        record.capability != "sql:body_ownership" && record.capability != "sql:statement_boundary"
    }));
    assert!(
        graph
            .graph
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "sql_incomplete_quoted_identifier")
    );
    let repeated = extract_sql_content(relative, source);
    let repeated_evidence =
        BuildEvidence::from_extraction(root, &repeated, "sha256:dollar-identifiers")?;
    let repeated_graph = normalize_v1(repeated, repeated_evidence)?;
    assert_eq!(
        graph.nodes.iter().map(|node| &node.id).collect::<Vec<_>>(),
        repeated_graph
            .nodes
            .iter()
            .map(|node| &node.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        graph.links.iter().map(|edge| &edge.id).collect::<Vec<_>>(),
        repeated_graph
            .links
            .iter()
            .map(|edge| &edge.id)
            .collect::<Vec<_>>()
    );
    Ok(())
}
