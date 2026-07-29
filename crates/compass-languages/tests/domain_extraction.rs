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

#[test]
fn nested_ddl_stays_inside_proven_dollar_and_compound_owners()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/nested_ddl.sql"), source);
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| node.string("symbol_kind") != "query"),
        "body DML leaked as file-owned query: {:?}",
        extraction.nodes
    );
    assert!(
        extraction
            .extensions
            .get("_compass_v1_graph_coverage")
            .is_none(),
        "complete bodies received false partial coverage: {:?}",
        extraction.extensions
    );
    assert!(
        extraction
            .extensions
            .get("_compass_v1_graph_diagnostics")
            .is_none(),
        "complete bodies received false diagnostics: {:?}",
        extraction.extensions
    );
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| node.string("qualified_name") != "recent"),
        "nested CTE became an entity: {:?}",
        extraction.nodes
    );

    for (owner_name, target_name, expected_writes) in [
        ("app.refresh_dollar", "app.scratch_dollar", 2),
        ("app.refresh_compound", "app.scratch_compound", 1),
    ] {
        let owner = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == owner_name)
            .ok_or_else(|| format!("missing owner {owner_name}"))?;
        let target = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        let writes = extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == owner.id
                    && edge.target == target.id
                    && edge.string("relation") == "writes"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            writes.len(),
            expected_writes,
            "owner={owner_name}, edges={:?}",
            extraction.edges
        );
        let sites = writes
            .iter()
            .filter_map(|edge| {
                edge.attributes
                    .get("start_byte")
                    .and_then(serde_json::Value::as_u64)
            })
            .collect::<HashSet<_>>();
        assert_eq!(sites.len(), expected_writes);
        assert!(extraction.edges.iter().any(|edge| {
            edge.source == owner.id
                && edge.string("relation") == "reads"
                && extraction.nodes.iter().any(|node| {
                    node.id == edge.target && node.string("qualified_name") == "app.users"
                })
        }));
    }
    Ok(())
}

#[test]
fn unmatched_identifier_quotes_recover_with_bounded_partial_evidence()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/unmatched_quotes.sql"), source);
    for expected in [
        "app.after_double",
        "app.after_backtick",
        "app.after_bracket",
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
    let queries = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "query")
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 3, "nodes={:?}", extraction.nodes);
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
        .ok_or("missing quote coverage")?;
    let diagnostics = extraction
        .extensions
        .get("_compass_v1_graph_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing quote diagnostics")?;
    assert_eq!(coverage.len(), 3, "coverage={coverage:?}");
    assert_eq!(diagnostics.len(), 3, "diagnostics={diagnostics:?}");
    for record in coverage {
        assert_eq!(
            record.get("capability").and_then(serde_json::Value::as_str),
            Some("sql:statement_boundary")
        );
        assert_eq!(
            record.get("status").and_then(serde_json::Value::as_str),
            Some("partial")
        );
        let anchor = record
            .get("anchor")
            .and_then(serde_json::Value::as_object)
            .ok_or("missing quote anchor")?;
        let start = anchor
            .get("startByte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing quote anchor start")?;
        let end = anchor
            .get("endByte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing quote anchor end")?;
        assert!(start < end && end <= source.len());
        assert!(
            !source[start..end].contains(&b'\n')
                || source[start..end].last().copied() == Some(b'\n'),
            "unbounded quote diagnostic: {:?}",
            &source[start..end]
        );
    }
    Ok(())
}

#[test]
fn dollar_quoted_defaults_do_not_replace_the_as_bound_executable_body()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/dollar_default.sql"), source);
    let procedure = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.refresh")
        .ok_or("missing function")?;
    let users = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.users")
        .ok_or("missing users")?;
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
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| node.string("symbol_kind") != "query")
    );
    assert!(
        extraction.extensions.is_empty(),
        "{:?}",
        extraction.extensions
    );
    Ok(())
}

#[test]
fn unmatched_identifier_quotes_cannot_pair_with_later_same_style_sql()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/same_style_quote_recovery.sql"), source);

    for expected in [
        "app.after_double",
        "app.after_double_decl",
        "app.after_backtick",
        "app.after_backtick_decl",
        "app.after_bracket",
        "app.after_bracket_decl",
    ] {
        assert!(
            extraction
                .nodes
                .iter()
                .any(|node| node.string("qualified_name") == expected),
            "missing recovered declaration {expected}: {:?}",
            extraction.nodes
        );
    }
    let queries = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "query")
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 3, "nodes={:?}", extraction.nodes);
    for target_name in [
        "app.double_users",
        "app.backtick_users",
        "app.bracket_users",
    ] {
        let target = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        assert_eq!(
            extraction
                .edges
                .iter()
                .filter(|edge| {
                    edge.target == target.id
                        && edge.string("relation") == "reads"
                        && queries.iter().any(|query| query.id == edge.source)
                })
                .count(),
            1,
            "target={target_name}, edges={:?}",
            extraction.edges
        );
    }

    let coverage = extraction
        .extensions
        .get("_compass_v1_graph_coverage")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing boundary coverage")?;
    let diagnostics = extraction
        .extensions
        .get("_compass_v1_graph_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing boundary diagnostics")?;
    assert_eq!(coverage.len(), 6, "coverage={coverage:?}");
    assert_eq!(diagnostics.len(), 6, "diagnostics={diagnostics:?}");
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
        assert!(coverage.iter().any(|record| {
            record
                .get("anchor")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|anchor| {
                    anchor.get("startByte").and_then(serde_json::Value::as_u64)
                        == Some(start as u64)
                        && anchor.get("endByte").and_then(serde_json::Value::as_u64)
                            == Some(end as u64)
                })
        }));
        assert!(diagnostics.iter().any(|record| {
            record
                .get("anchor")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|anchor| {
                    anchor.get("startByte").and_then(serde_json::Value::as_u64)
                        == Some(start as u64)
                        && anchor.get("endByte").and_then(serde_json::Value::as_u64)
                            == Some(end as u64)
                })
        }));
    }
    Ok(())
}

#[test]
fn nested_cte_bindings_are_excluded_only_within_their_lexical_scopes()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/nested_ctes.sql"), source);
    for alias in ["outer_cte", "inner_cte", "inner_two", "shadowed", "second"] {
        assert!(
            extraction
                .nodes
                .iter()
                .all(|node| node.string("qualified_name") != alias),
            "CTE alias became a table {alias}: {:?}",
            extraction.nodes
        );
    }

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
    let query = extraction
        .nodes
        .iter()
        .find(|node| node.string("symbol_kind") == "query")
        .ok_or("missing top-level CTE query")?;
    let procedure = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.refresh_nested")
        .ok_or("missing procedure")?;
    let scope_local = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "scope_local")
        .ok_or("same spelling outside the nested CTE scope must remain a physical table")?;
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == query.id
                    && edge.target == scope_local.id
                    && edge.string("relation") == "reads"
            })
            .count(),
        1,
        "nested CTE scope leaked beyond its enclosing subquery: {:?}",
        extraction.edges
    );
    for owner in [query, procedure] {
        let reads = extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == owner.id
                    && edge.target == users.id
                    && edge.string("relation") == "reads"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reads.len(),
            2,
            "owner={}, edges={:?}",
            owner.id,
            extraction.edges
        );
        assert_eq!(
            reads
                .iter()
                .filter_map(|edge| edge.attributes.get("start_byte"))
                .filter_map(serde_json::Value::as_u64)
                .collect::<HashSet<_>>()
                .len(),
            2
        );
        assert_eq!(
            extraction
                .edges
                .iter()
                .filter(|edge| {
                    edge.source == owner.id
                        && edge.target == audit.id
                        && edge.string("relation") == "writes"
                })
                .count(),
            1,
            "owner={}, edges={:?}",
            owner.id,
            extraction.edges
        );
    }
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "query")
            .count(),
        1,
        "routine CTE leaked as a file-owned query: {:?}",
        extraction.nodes
    );
    assert!(
        extraction.extensions.is_empty(),
        "{:?}",
        extraction.extensions
    );
    Ok(())
}

#[test]
fn paired_identifier_quotes_keep_semicolon_keyword_content_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE TABLE "app"."semi;SELECT""tail" (id BIGINT);
SELECT * FROM "app"."semi;SELECT""tail";
CREATE TABLE `app`.`semi;UPDATE``tail` (id BIGINT);
SELECT * FROM `app`.`semi;UPDATE``tail`;
CREATE TABLE [app].[semi;DELETE]]tail] (id BIGINT);
SELECT * FROM [app].[semi;DELETE]]tail];
"#;
    let extraction = extract_sql_content(Path::new("db/quoted_keyword_content.sql"), source);
    let expected = [
        (
            "app.semi;SELECT\"tail",
            br#""app"."semi;SELECT""tail""#.as_slice(),
        ),
        ("app.semi;UPDATE`tail", b"`app`.`semi;UPDATE``tail`"),
        ("app.semi;DELETE]tail", b"[app].[semi;DELETE]]tail]"),
    ];
    for (qualified_name, spelling) in expected {
        let table = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing table {qualified_name}"))?;
        let node_start = table
            .attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing table start")?;
        let node_end = table
            .attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing table end")?;
        assert_eq!(&source[node_start..node_end], spelling);

        let read = extraction
            .edges
            .iter()
            .find(|edge| edge.target == table.id && edge.string("relation") == "reads")
            .ok_or_else(|| format!("missing read for {qualified_name}"))?;
        let read_start = read
            .attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing read start")?;
        let read_end = read
            .attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing read end")?;
        assert_eq!(&source[read_start..read_end], spelling);
        assert_eq!(read.string("_origin"), "artifact");
        assert!(read.string("rule").starts_with("sql-text-"));
    }
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "query")
            .count(),
        3
    );
    assert!(
        extraction.extensions.is_empty(),
        "valid paired identifiers received recovery evidence: {:?}",
        extraction.extensions
    );
    Ok(())
}

#[test]
fn lexical_alias_scopes_keep_outer_writes_out_of_nested_and_sibling_shadows()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/alias_scopes.sql"), source);
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
    let archive = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.archive")
        .ok_or("missing archive")?;
    let physical_u = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "u")
        .ok_or("same spelling outside nested alias scope must remain physical")?;
    let queries = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "query")
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 4, "nodes={:?}", extraction.nodes);
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                queries.iter().any(|query| query.id == edge.source)
                    && edge.target == users.id
                    && edge.string("relation") == "writes"
            })
            .count(),
        3,
        "top-level aliases resolved to the wrong write target: {:?}",
        extraction.edges
    );
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                queries.iter().any(|query| query.id == edge.source)
                    && edge.target == physical_u.id
                    && edge.string("relation") == "writes"
            })
            .count(),
        1,
        "nested alias leaked into the physical outer target: {:?}",
        extraction.edges
    );
    assert!(extraction.edges.iter().all(|edge| {
        !queries.iter().any(|query| query.id == edge.source)
            || edge.string("relation") != "writes"
            || (edge.target != audit.id && edge.target != archive.id)
    }));

    for (owner_name, expected_writes) in [("app.refresh_aliases", 1), ("app.capture_aliases", 2)] {
        let owner = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == owner_name)
            .ok_or_else(|| format!("missing owner {owner_name}"))?;
        let writes = extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == owner.id
                    && edge.target == users.id
                    && edge.string("relation") == "writes"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            writes.len(),
            expected_writes,
            "owner={owner_name}, edges={:?}",
            extraction.edges
        );
        assert_eq!(
            writes
                .iter()
                .filter_map(|edge| edge.attributes.get("start_byte"))
                .filter_map(serde_json::Value::as_u64)
                .collect::<HashSet<_>>()
                .len(),
            expected_writes
        );
        for write in writes {
            let start = write
                .attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing write start")?;
            let end = write
                .attributes
                .get("end_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing write end")?;
            assert_eq!(&source[start..end], b"u");
            assert_eq!(write.string("_origin"), "artifact");
        }
    }
    assert!(
        extraction.extensions.is_empty(),
        "{:?}",
        extraction.extensions
    );
    Ok(())
}

#[test]
fn unified_sql_lexer_preserves_marker_content_and_multiline_identifiers()
-> Result<(), Box<dyn std::error::Error>> {
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
"#;
    let extraction = extract_sql_content(Path::new("db/lexical_identifiers.sql"), source);
    let expected = [
        ("app.dash--SELECT", br#""app"."dash--SELECT""#.as_slice()),
        ("app.block/*UPDATE*/tail", b"`app`.`block/*UPDATE*/tail`"),
        ("app.apostrophe'SELECT", b"[app].[apostrophe'SELECT]"),
        ("app.multi\ndouble", b"\"app\".\"multi\ndouble\""),
        ("app.multi\nbacktick", b"`app`.`multi\nbacktick`"),
        ("app.multi\nbracket", b"[app].[multi\nbracket]"),
    ];
    for (qualified_name, spelling) in expected {
        let table = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing table {qualified_name:?}"))?;
        let read = extraction
            .edges
            .iter()
            .find(|edge| edge.target == table.id && edge.string("relation") == "reads")
            .ok_or_else(|| format!("missing read for {qualified_name:?}"))?;
        for attributes in [&table.attributes, &read.attributes] {
            let start = attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing start")?;
            let end = attributes
                .get("end_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing end")?;
            assert_eq!(&source[start..end], spelling);
        }
    }
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "query")
            .count(),
        6
    );
    assert!(
        extraction.extensions.is_empty(),
        "valid lexical content received recovery evidence: {:?}",
        extraction.extensions
    );
    Ok(())
}

#[test]
fn same_line_malformed_identifiers_recover_before_same_style_valid_sql()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE TABLE "app"."double_users" (id BIGINT);
CREATE TABLE `app`.`backtick_users` (id BIGINT);
CREATE TABLE [app].[bracket_users] (id BIGINT);
SELECT * FROM "broken; SELECT * FROM "app"."double_users"; CREATE TABLE "app"."after_double" (id BIGINT);
SELECT * FROM `broken; SELECT * FROM `app`.`backtick_users`; CREATE TABLE `app`.`after_backtick` (id BIGINT);
SELECT * FROM [broken; SELECT * FROM [app].[bracket_users]; CREATE TABLE [app].[after_bracket] (id BIGINT);
"#;
    let extraction = extract_sql_content(Path::new("db/same_line_recovery.sql"), source);
    for target_name in [
        "app.double_users",
        "app.backtick_users",
        "app.bracket_users",
    ] {
        let target = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        assert_eq!(
            extraction
                .edges
                .iter()
                .filter(|edge| edge.target == target.id && edge.string("relation") == "reads")
                .count(),
            1,
            "target={target_name}, edges={:?}",
            extraction.edges
        );
    }
    for declaration in [
        "app.after_double",
        "app.after_backtick",
        "app.after_bracket",
    ] {
        assert!(
            extraction
                .nodes
                .iter()
                .any(|node| node.string("qualified_name") == declaration),
            "missing recovered declaration {declaration}"
        );
    }
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| !node.string("qualified_name").contains("broken")),
        "malformed span created a phantom node: {:?}",
        extraction.nodes
    );
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "query")
            .count(),
        3
    );
    let coverage = extraction
        .extensions
        .get("_compass_v1_graph_coverage")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing recovery coverage")?;
    let diagnostics = extraction
        .extensions
        .get("_compass_v1_graph_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing recovery diagnostics")?;
    assert_eq!(coverage.len(), 3, "coverage={coverage:?}");
    assert_eq!(diagnostics.len(), 3, "diagnostics={diagnostics:?}");
    for malformed in [b"\"broken".as_slice(), b"`broken", b"[broken"] {
        let start = source
            .windows(malformed.len())
            .position(|window| window == malformed)
            .ok_or("missing malformed opener")?;
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b';')
            .map(|offset| start + offset + 1)
            .ok_or("missing recovery semicolon")?;
        assert!(coverage.iter().any(|record| {
            record
                .get("anchor")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|anchor| {
                    anchor.get("startByte").and_then(serde_json::Value::as_u64)
                        == Some(start as u64)
                        && anchor.get("endByte").and_then(serde_json::Value::as_u64)
                            == Some(end as u64)
                })
        }));
    }
    Ok(())
}

#[test]
fn cte_aliases_never_materialize_as_exact_physical_write_targets()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/cte_alias_writes.sql"), source);
    for alias in ["src", "wrapper"] {
        assert!(
            extraction
                .nodes
                .iter()
                .all(|node| node.string("qualified_name") != alias),
            "CTE alias became a physical node {alias}: {:?}",
            extraction.nodes
        );
    }
    assert!(
        extraction
            .edges
            .iter()
            .all(|edge| edge.string("relation") != "writes"),
        "unproven CTE updateability became an exact write: {:?}",
        extraction.edges
    );
    let users = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.users")
        .ok_or("missing users")?;
    assert!(
        extraction
            .edges
            .iter()
            .filter(|edge| edge.target == users.id && edge.string("relation") == "reads")
            .count()
            >= 6,
        "underlying physical reads were lost: {:?}",
        extraction.edges
    );
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "query")
            .count(),
        4,
        "routine or trigger body leaked as a query: {:?}",
        extraction.nodes
    );
    assert!(
        extraction.extensions.is_empty(),
        "{:?}",
        extraction.extensions
    );
    Ok(())
}

#[test]
fn quote_family_cross_product_separates_valid_pairs_from_proven_recovery()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/quote_cross_product.sql"), source);
    let exact_identifiers = [
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
    ];
    for (qualified_name, spelling) in exact_identifiers {
        let table = extraction
            .nodes
            .iter()
            .find(|node| {
                node.string("symbol_kind") == "database_table"
                    && node.string("qualified_name") == qualified_name
            })
            .ok_or_else(|| format!("missing valid paired identifier {qualified_name:?}"))?;
        let read = extraction
            .edges
            .iter()
            .find(|edge| edge.target == table.id && edge.string("relation") == "reads")
            .ok_or_else(|| format!("missing read for {qualified_name:?}"))?;
        for attributes in [&table.attributes, &read.attributes] {
            let start = attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing exact start")?;
            let end = attributes
                .get("end_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing exact end")?;
            assert_eq!(&source[start..end], spelling);
        }
    }
    for target_name in [
        "app.double_target",
        "app.backtick_target",
        "app.bracket_target",
    ] {
        let target = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        assert_eq!(
            extraction
                .edges
                .iter()
                .filter(|edge| edge.target == target.id && edge.string("relation") == "reads")
                .count(),
            2,
            "later same-style statements were not independently retained: {:?}",
            extraction.edges
        );
    }
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| !node.string("qualified_name").contains("broken_"))
    );
    let coverage = extraction
        .extensions
        .get("_compass_v1_graph_coverage")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing recovery coverage")?;
    let diagnostics = extraction
        .extensions
        .get("_compass_v1_graph_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing recovery diagnostics")?;
    assert_eq!(coverage.len(), 6, "coverage={coverage:?}");
    assert_eq!(diagnostics.len(), 6, "diagnostics={diagnostics:?}");
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
            coverage
                .iter()
                .filter(|record| {
                    record
                        .get("anchor")
                        .and_then(serde_json::Value::as_object)
                        .is_some_and(|anchor| {
                            anchor.get("startByte").and_then(serde_json::Value::as_u64)
                                == Some(start as u64)
                                && anchor.get("endByte").and_then(serde_json::Value::as_u64)
                                    == Some(end as u64)
                        })
                })
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn paired_dollar_literals_are_inert_while_owned_routine_bodies_are_scanned()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/dollar_lexing.sql"), source);
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
            extraction
                .nodes
                .iter()
                .all(|node| node.string("qualified_name") != phantom),
            "dollar literal produced phantom {phantom}: {:?}",
            extraction.nodes
        );
    }
    let real = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.real")
        .ok_or("missing real table")?;
    let audit = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.audit")
        .ok_or("missing audit table")?;
    let routine = extraction
        .nodes
        .iter()
        .find(|node| node.string("qualified_name") == "app.refresh")
        .ok_or("missing routine")?;
    let queries = extraction
        .nodes
        .iter()
        .filter(|node| node.string("symbol_kind") == "query")
        .collect::<Vec<_>>();
    assert_eq!(queries.len(), 2, "nodes={:?}", extraction.nodes);
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                queries.iter().any(|query| query.id == edge.source)
                    && edge.target == real.id
                    && edge.string("relation") == "reads"
            })
            .count(),
        2
    );
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                queries.iter().any(|query| query.id == edge.source)
                    && edge.target == audit.id
                    && edge.string("relation") == "writes"
            })
            .count(),
        1
    );
    for edge in extraction.edges.iter().filter(|edge| {
        (edge.target == real.id && edge.string("relation") == "reads")
            || (edge.target == audit.id && edge.string("relation") == "writes")
    }) {
        let start = edge
            .attributes
            .get("start_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing data-access start")?;
        let end = edge
            .attributes
            .get("end_byte")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or("missing data-access end")?;
        let expected = if edge.target == real.id {
            b"app.real".as_slice()
        } else {
            b"app.audit".as_slice()
        };
        assert_eq!(&source[start..end], expected);
        assert_eq!(edge.string("_origin"), "artifact");
        assert_eq!(edge.string("rule"), "sql-text-data-access");
    }
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == routine.id
                    && edge.target == real.id
                    && edge.string("relation") == "reads"
            })
            .count(),
        1
    );
    assert_eq!(
        extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == routine.id
                    && edge.target == audit.id
                    && edge.string("relation") == "writes"
            })
            .count(),
        1
    );
    assert!(
        extraction.extensions.is_empty(),
        "complete dollar spans produced diagnostics: {:?}",
        extraction.extensions
    );
    Ok(())
}

#[test]
fn sql_looking_paired_identifiers_accept_aliases_and_later_quoted_targets()
-> Result<(), Box<dyn std::error::Error>> {
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
"#;
    let extraction = extract_sql_content(Path::new("db/valid_alias_followers.sql"), source);
    for (qualified_name, spelling, expected_reads) in [
        (
            "app.double; SELECT 1",
            br#""app"."double; SELECT 1""#.as_slice(),
            2,
        ),
        ("app.after_double", br#""app"."after_double""#.as_slice(), 2),
        (
            "app.backtick; SELECT 1",
            b"`app`.`backtick; SELECT 1`".as_slice(),
            2,
        ),
        (
            "app.after_backtick",
            b"`app`.`after_backtick`".as_slice(),
            2,
        ),
        (
            "app.bracket; SELECT 1",
            b"[app].[bracket; SELECT 1]".as_slice(),
            2,
        ),
        ("app.after_bracket", b"[app].[after_bracket]".as_slice(), 2),
    ] {
        let table = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing table {qualified_name}"))?;
        let reads = extraction
            .edges
            .iter()
            .filter(|edge| edge.target == table.id && edge.string("relation") == "reads")
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), expected_reads, "edges={:?}", extraction.edges);
        for read in reads {
            let start = read
                .attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing read start")?;
            let end = read
                .attributes
                .get("end_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing read end")?;
            assert_eq!(&source[start..end], spelling);
        }
    }
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "query")
            .count(),
        9
    );
    assert!(
        extraction.extensions.is_empty(),
        "valid aliases produced recovery evidence: {:?}",
        extraction.extensions
    );
    Ok(())
}

#[test]
fn multiline_malformed_identifiers_recover_at_each_original_opener()
-> Result<(), Box<dyn std::error::Error>> {
    let source = br#"
CREATE TABLE "app"."double_target" (id BIGINT);
CREATE TABLE `app`.`backtick_target` (id BIGINT);
CREATE TABLE [app].[bracket_target] (id BIGINT);
SELECT * FROM "broken_double_ws
 SELECT * FROM "app"."double_target";
SELECT * FROM "broken_double_tight
SELECT * FROM "app"."double_target";
SELECT * FROM `broken_backtick_ws
 SELECT * FROM `app`.`backtick_target`;
SELECT * FROM `broken_backtick_tight
SELECT * FROM `app`.`backtick_target`;
SELECT * FROM [broken_bracket_ws
 SELECT * FROM [app].[bracket_target];
SELECT * FROM [broken_bracket_tight
SELECT * FROM [app].[bracket_target];
"#;
    let extraction = extract_sql_content(Path::new("db/multiline_recovery.sql"), source);
    for target_name in [
        "app.double_target",
        "app.backtick_target",
        "app.bracket_target",
    ] {
        let target = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == target_name)
            .ok_or_else(|| format!("missing target {target_name}"))?;
        assert_eq!(
            extraction
                .edges
                .iter()
                .filter(|edge| edge.target == target.id && edge.string("relation") == "reads")
                .count(),
            2,
            "later valid statements were lost: {:?}",
            extraction.edges
        );
    }
    assert!(
        extraction
            .nodes
            .iter()
            .all(|node| !node.string("qualified_name").contains("broken_"))
    );
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "query")
            .count(),
        6,
        "malformed prefixes became queries: {:?}",
        extraction.nodes
    );
    let coverage = extraction
        .extensions
        .get("_compass_v1_graph_coverage")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing coverage")?;
    let diagnostics = extraction
        .extensions
        .get("_compass_v1_graph_diagnostics")
        .and_then(serde_json::Value::as_array)
        .ok_or("missing diagnostics")?;
    assert_eq!(coverage.len(), 6, "coverage={coverage:?}");
    assert_eq!(diagnostics.len(), 6, "diagnostics={diagnostics:?}");
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
            .ok_or("missing opener")?;
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|offset| start + offset + 1)
            .ok_or("missing newline recovery")?;
        assert_eq!(
            coverage
                .iter()
                .filter(|record| {
                    record
                        .get("anchor")
                        .and_then(serde_json::Value::as_object)
                        .is_some_and(|anchor| {
                            anchor.get("startByte").and_then(serde_json::Value::as_u64)
                                == Some(start as u64)
                                && anchor.get("endByte").and_then(serde_json::Value::as_u64)
                                    == Some(end as u64)
                        })
                })
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn dollar_delimiter_text_inside_unquoted_identifiers_is_never_masked()
-> Result<(), Box<dyn std::error::Error>> {
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
    let extraction = extract_sql_content(Path::new("db/dollar_identifiers.sql"), source);
    for (qualified_name, spelling, expected_reads) in [
        ("app.foo$tag$bar", b"app.foo$tag$bar".as_slice(), 2),
        ("app.baz$tag$qux", b"app.baz$tag$qux".as_slice(), 1),
        ("app.foo$$bar", b"app.foo$$bar".as_slice(), 1),
        ("app.baz$$qux", b"app.baz$$qux".as_slice(), 1),
    ] {
        let table = extraction
            .nodes
            .iter()
            .find(|node| node.string("qualified_name") == qualified_name)
            .ok_or_else(|| format!("missing table {qualified_name}"))?;
        let reads = extraction
            .edges
            .iter()
            .filter(|edge| edge.target == table.id && edge.string("relation") == "reads")
            .collect::<Vec<_>>();
        assert_eq!(reads.len(), expected_reads, "edges={:?}", extraction.edges);
        for attributes in
            std::iter::once(&table.attributes).chain(reads.into_iter().map(|edge| &edge.attributes))
        {
            let start = attributes
                .get("start_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing exact start")?;
            let end = attributes
                .get("end_byte")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or("missing exact end")?;
            assert_eq!(&source[start..end], spelling);
        }
    }
    for phantom in ["app.literal_phantom", "app.write_phantom"] {
        assert!(
            extraction
                .nodes
                .iter()
                .all(|node| node.string("qualified_name") != phantom)
        );
    }
    assert_eq!(
        extraction
            .nodes
            .iter()
            .filter(|node| node.string("symbol_kind") == "query")
            .count(),
        5
    );
    assert!(
        extraction.extensions.is_empty(),
        "valid dollar identifiers produced recovery evidence: {:?}",
        extraction.extensions
    );
    Ok(())
}
