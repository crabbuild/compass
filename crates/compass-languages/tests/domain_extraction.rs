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

#[test]
fn sql_declaration_modifiers_bind_only_real_qualified_objects()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE SCHEMA IF NOT EXISTS app;
CREATE TEMPORARY TABLE IF NOT EXISTS app.users (id BIGINT);
CREATE MATERIALIZED VIEW IF NOT EXISTS app.active_users AS
  SELECT id FROM app.users;
CREATE UNIQUE INDEX IF NOT EXISTS users_idx ON ONLY app.users(id);
CREATE TRIGGER IF NOT EXISTS audit_users AFTER UPDATE ON app.users
FOR EACH ROW BEGIN
  INSERT INTO app.audit(id) VALUES (1);
END;
ALTER TABLE IF EXISTS app.users ADD CONSTRAINT users_id_unique UNIQUE (id);
"#;
    let extraction = extract_sql_content(Path::new("db/modifiers.sql"), source);
    let qualified_names = extraction
        .nodes
        .iter()
        .map(|node| node.string("qualified_name"))
        .collect::<HashSet<_>>();

    for expected in [
        "app",
        "app.users",
        "app.active_users",
        "users_idx",
        "audit_users",
        "app.audit",
    ] {
        assert!(
            qualified_names.contains(expected),
            "missing {expected:?}: {qualified_names:?}"
        );
    }
    for invalid in [
        "IF",
        "if",
        "NOT",
        "not",
        "EXISTS",
        "exists",
        "ONLY",
        "only",
        "TEMPORARY",
        "temporary",
        "MATERIALIZED",
        "materialized",
    ] {
        assert!(
            !qualified_names.contains(invalid),
            "modifier became database entity {invalid:?}: {:?}",
            extraction.nodes
        );
    }

    let table = extraction
        .nodes
        .iter()
        .find(|node| {
            node.string("symbol_kind") == "database_table"
                && node.string("qualified_name") == "app.users"
        })
        .ok_or("missing app.users")?;
    let index = extraction
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "database_index")
        .ok_or("missing index")?;
    let trigger = extraction
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "database_trigger")
        .ok_or("missing trigger")?;
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == table.id && edge.target == index.id && edge.string("relation") == "contains"
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == trigger.id
            && edge.target == table.id
            && edge.string("relation") == "triggers"
    }));
    Ok(())
}

#[test]
fn quoted_case_distinct_schema_tables_keep_distinct_read_targets() {
    let source = br#"
CREATE SCHEMA "App";
CREATE SCHEMA "app";
CREATE TABLE "App"."Users" (id BIGINT);
CREATE TABLE "app"."users" (id BIGINT);
SELECT upper_name.id FROM "App"."Users" AS upper_name;
SELECT lower_name.id FROM "app"."users" AS lower_name;
"#;
    let extraction = extract_sql_content(Path::new("db/quoted_case.sql"), source);
    let tables = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "database_table")
        .collect::<Vec<_>>();
    assert_eq!(tables.len(), 2, "nodes={:?}", extraction.nodes);
    assert_ne!(tables[0].id, tables[1].id);

    let schemas = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "database_schema")
        .collect::<Vec<_>>();
    assert_eq!(schemas.len(), 2, "nodes={:?}", extraction.nodes);
    assert_ne!(schemas[0].id, schemas[1].id);

    for table in tables {
        let reads = extraction
            .edges
            .iter()
            .filter(|edge| edge.string("relation") == "reads" && edge.target == table.id)
            .count();
        assert_eq!(
            reads, 1,
            "each quoted table must receive its own read: table={table:?}, edges={:?}",
            extraction.edges
        );
    }
}

#[test]
fn compound_procedure_and_trigger_bodies_never_become_file_owned_queries()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE TABLE app.audit (id BIGINT);
CREATE PROCEDURE app.refresh() AS $body$
BEGIN
  INSERT INTO app.audit(id) VALUES (1);
  UPDATE app.users SET id = 2;
  INSERT INTO app.audit(id) VALUES (3);
END;
$body$ LANGUAGE plpgsql;
CREATE TRIGGER app.capture AFTER UPDATE ON app.users
FOR EACH ROW BEGIN
  INSERT INTO app.audit(id) VALUES (2);
  SELECT id FROM app.users;
END;
"#;
    let extraction = extract_sql_content(Path::new("db/compound.sql"), source);
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| node.string("symbol_kind") != "query"),
        "routine or trigger body became a top-level query: {:?}",
        extraction.nodes
    );

    for owner_name in ["app.refresh", "app.capture"] {
        let owner = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == owner_name)
            .ok_or_else(|| format!("missing owner {owner_name}"))?;
        assert!(
            extraction.edges.iter().any(|edge| {
                edge.source == owner.id
                    && edge.string("relation") == "writes"
                    && extraction.nodes.iter().any(|node| {
                        node.id == edge.target && node.string("qualified_name") == "app.audit"
                    })
            }),
            "missing owner-attributed write for {owner_name}: {:?}",
            extraction.edges
        );
    }
    let procedure = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.refresh")
        .ok_or("missing procedure")?;
    let audit = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.audit")
        .ok_or("missing audit table")?;
    let repeated_writes = extraction
        .edges
        .iter()
        .filter(|edge| {
            edge.source == procedure.id
                && edge.target == audit.id
                && edge.string("relation") == "writes"
        })
        .collect::<Vec<_>>();
    assert_eq!(
        repeated_writes.len(),
        2,
        "repeated body accesses lost occurrence identity: {:?}",
        extraction.edges
    );
    assert_ne!(
        repeated_writes[0].attributes.get("start_byte"),
        repeated_writes[1].attributes.get("start_byte")
    );
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == procedure.id
            && edge.string("relation") == "writes"
            && extraction
                .nodes
                .iter()
                .any(|node| node.id == edge.target && node.string("qualified_name") == "app.users")
    }));
    Ok(())
}

#[test]
fn recursive_multi_ctes_publish_underlying_reads_without_cte_entities()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE TABLE app.accounts (id BIGINT);
WITH RECURSIVE recent AS NOT MATERIALIZED (
  SELECT id FROM app.users
), enriched AS MATERIALIZED (
  SELECT recent.id FROM recent JOIN app.accounts ON app.accounts.id = recent.id
)
SELECT enriched.id FROM enriched;
"#;
    let extraction = extract_sql_content(Path::new("db/recursive_cte.sql"), source);
    let qualified_names = extraction
        .nodes
        .iter()
        .map(|node| node.string("qualified_name"))
        .collect::<HashSet<_>>();
    for cte in ["recent", "enriched", "RECURSIVE", "MATERIALIZED"] {
        assert!(
            !qualified_names.contains(cte),
            "CTE syntax became a database entity {cte:?}: {:?}",
            extraction.nodes
        );
    }
    let query = extraction
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "query")
        .ok_or("missing WITH query")?;
    let read_names = extraction
        .edges
        .iter()
        .filter(|edge| edge.source == query.id && edge.string("relation") == "reads")
        .filter_map(|edge| {
            extraction
                .nodes
                .iter()
                .find(|node| node.id == edge.target)
                .map(|node| node.string("qualified_name"))
        })
        .collect::<HashSet<_>>();
    assert_eq!(
        read_names,
        HashSet::from(["app.users".to_owned(), "app.accounts".to_owned()])
    );
    Ok(())
}

#[test]
fn mysql_dml_modifiers_preserve_real_write_targets_and_omit_modifiers() {
    let source = br#"
CREATE TABLE app.users (id BIGINT);
UPDATE LOW_PRIORITY IGNORE app.users SET id = 1;
INSERT LOW_PRIORITY IGNORE INTO app.users(id) VALUES (1);
DELETE LOW_PRIORITY QUICK IGNORE FROM app.users;
UPDATE IGNORE LOW_PRIORITY app.users SET id = 2;
"#;
    let extraction = extract_sql_content(Path::new("db/mysql_modifiers.sql"), source);
    let users = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.users")
        .map(|node| node.id.as_str());
    assert!(users.is_some(), "nodes={:?}", extraction.nodes);
    let writes = extraction
        .edges
        .iter()
        .filter(|edge| users == Some(edge.target.as_str()) && edge.string("relation") == "writes")
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 3, "edges={:?}", extraction.edges);
    let write_sites = writes
        .iter()
        .filter_map(|edge| {
            edge.attributes
                .get("start_byte")
                .and_then(|value| value.as_u64())
        })
        .collect::<HashSet<_>>();
    assert_eq!(write_sites.len(), 3, "writes={writes:?}");

    let qualified_names = extraction
        .nodes
        .iter()
        .map(|node| node.string("qualified_name").to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for modifier in [
        "low_priority",
        "high_priority",
        "delayed",
        "quick",
        "ignore",
    ] {
        assert!(
            !qualified_names.contains(modifier),
            "modifier became an exact entity {modifier:?}: {:?}",
            extraction.nodes
        );
    }
}

#[test]
fn incomplete_routine_bodies_recover_without_absorbing_following_statements()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE PROCEDURE app.broken_begin() AS BEGIN
  UPDATE app.users SET id = 1;
SELECT * FROM app.users;
CREATE TABLE app.after_begin (id BIGINT);
CREATE PROCEDURE app.broken_dollar() AS $tag$
  UPDATE app.users SET id = 2;
SELECT * FROM app.users;
CREATE TABLE app.after_dollar (id BIGINT);
"#;
    let extraction = extract_sql_content(Path::new("db/incomplete_bodies.sql"), source);
    for expected in [
        "app.broken_begin",
        "app.after_begin",
        "app.broken_dollar",
        "app.after_dollar",
    ] {
        assert!(
            extraction
                .nodes
                .iter()
                .any(|node| node.string("qualified_name") == expected),
            "missing recovered declaration {expected:?}: {:?}",
            extraction.nodes
        );
    }
    for owner_name in ["app.broken_begin", "app.broken_dollar"] {
        let owner = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == owner_name)
            .ok_or_else(|| format!("missing {owner_name}"))?;
        assert!(
            extraction.edges.iter().all(|edge| {
                edge.source != owner.id
                    || !matches!(edge.string("relation").as_str(), "reads" | "writes")
            }),
            "incomplete routine received exact body ownership: owner={owner:?}, edges={:?}",
            extraction.edges
        );
    }
    let queries = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "query")
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 2, "nodes={:?}", extraction.nodes);
    assert!(
        queries
            .iter()
            .all(|query| extraction.edges.iter().any(|edge| {
                edge.source == query.id
                    && edge.string("relation") == "reads"
                    && extraction.nodes.iter().any(|node| {
                        node.id == edge.target && node.string("qualified_name") == "app.users"
                    })
            }))
    );

    let coverage = extraction
        .extensions
        .get("_compass_v1_graph_coverage")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing partial coverage")?;
    assert_eq!(coverage.len(), 2, "coverage={coverage:?}");
    assert!(coverage.iter().all(|record| {
        record.get("status").and_then(serde_json::Value::as_str) == Some("partial")
            && record.get("capability").and_then(serde_json::Value::as_str)
                == Some("sql:body_ownership")
            && record.get("anchor").is_some()
    }));
    let diagnostics = extraction
        .extensions
        .get("_compass_v1_graph_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing incomplete-body diagnostics")?;
    assert_eq!(diagnostics.len(), 2, "diagnostics={diagnostics:?}");
    Ok(())
}

#[test]
fn routine_statement_scopes_exclude_recursive_materialized_ctes()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE TABLE app.users (id BIGINT);
CREATE TABLE app.audit (id BIGINT);
CREATE PROCEDURE app.refresh() AS $body$
BEGIN
  UPDATE app.users SET id = 1;
  WITH RECURSIVE recent AS NOT MATERIALIZED (
    SELECT id FROM app.users
  ), enriched AS MATERIALIZED (
    SELECT recent.id FROM recent JOIN app.users ON app.users.id = recent.id
  )
  INSERT INTO app.audit SELECT id FROM enriched;
END;
$body$ LANGUAGE plpgsql;
"#;
    let extraction = extract_sql_content(Path::new("db/routine_cte.sql"), source);
    for invalid in ["recent", "enriched", "RECURSIVE", "MATERIALIZED"] {
        assert!(
            extraction
                .nodes
                .iter()
                .all(|node| node.string("qualified_name") != invalid),
            "routine CTE became entity {invalid:?}: {:?}",
            extraction.nodes
        );
    }
    let procedure = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.refresh")
        .ok_or("missing procedure")?;
    let users = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.users")
        .ok_or("missing users")?;
    let audit = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.audit")
        .ok_or("missing audit")?;
    let reads = extraction
        .edges
        .iter()
        .filter(|edge| {
            edge.source == procedure.id
                && edge.target == users.id
                && edge.string("relation") == "reads"
        })
        .collect::<Vec<_>>();
    assert_eq!(reads.len(), 2, "edges={:?}", extraction.edges);
    assert_ne!(
        reads[0].attributes.get("start_byte"),
        reads[1].attributes.get("start_byte")
    );
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == procedure.id
            && edge.target == users.id
            && edge.string("relation") == "writes"
    }));
    assert!(extraction.edges.iter().any(|edge| {
        edge.source == procedure.id
            && edge.target == audit.id
            && edge.string("relation") == "writes"
    }));
    Ok(())
}

#[test]
fn statement_scanner_ignores_semicolons_in_escaped_quoted_identifiers()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE TABLE "app"."semi;""colon" (id BIGINT);
CREATE TABLE `app`.`semi;``colon` (id BIGINT);
CREATE TABLE [app].[semi;]]colon] (id BIGINT);
SELECT * FROM "app"."semi;""colon";
SELECT * FROM `app`.`semi;``colon`;
SELECT * FROM [app].[semi;]]colon];
"#;
    let extraction = extract_sql_content(Path::new("db/quoted_semicolons.sql"), source);
    let tables = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "database_table")
        .collect::<Vec<_>>();
    assert_eq!(tables.len(), 3, "nodes={:?}", extraction.nodes);
    assert!(tables.iter().all(|table| {
        table.string("qualified_name") != "app"
            && extraction
                .edges
                .iter()
                .any(|edge| edge.target == table.id && edge.string("relation") == "reads")
    }));
    let queries = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "query")
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 3, "nodes={:?}", extraction.nodes);
    for query in queries {
        let end = query
            .attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing query end")?;
        assert_eq!(source.get(end.saturating_sub(1)), Some(&b';'));
        assert_eq!(query.string("_origin"), "artifact");
        assert!(query.string("rule").starts_with("sql-text-"));
    }
    Ok(())
}
