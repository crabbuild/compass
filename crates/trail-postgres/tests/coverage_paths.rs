use std::error::Error;
use std::path::Path;

use trail_postgres::{
    CatalogSnapshot, ForeignKey, Routine, Table, View, extract_snapshot, introspect_postgres,
    reconstruct_ddl,
};

#[test]
fn snapshot_variants_cover_defaults_filters_escaping_and_fragment_conversion()
-> Result<(), Box<dyn Error>> {
    let snapshot = CatalogSnapshot {
        tables: vec![
            Table {
                schema: "odd\"schema".to_owned(),
                name: "table".to_owned(),
                table_type: "BASE TABLE".to_owned(),
            },
            Table {
                schema: "public".to_owned(),
                name: "ignored".to_owned(),
                table_type: "VIEW".to_owned(),
            },
        ],
        views: vec![
            View {
                schema: "public".to_owned(),
                name: "empty_view".to_owned(),
                definition: None,
            },
            View {
                schema: "public".to_owned(),
                name: "blank_view".to_owned(),
                definition: Some(String::new()),
            },
        ],
        routines: vec![
            Routine {
                schema: "public".to_owned(),
                name: "default_body".to_owned(),
                routine_type: "FUNCTION".to_owned(),
                definition: Some(String::new()),
                language: Some(String::new()),
            },
            Routine {
                schema: "public".to_owned(),
                name: "custom".to_owned(),
                routine_type: "PROCEDURE".to_owned(),
                definition: Some("BEGIN NULL; END;".to_owned()),
                language: Some("PLPGSQL".to_owned()),
            },
            Routine {
                schema: "public".to_owned(),
                name: "ignored".to_owned(),
                routine_type: "AGGREGATE".to_owned(),
                definition: None,
                language: None,
            },
        ],
        foreign_keys: vec![ForeignKey {
            constraint: "fk".to_owned(),
            schema: "odd\"schema".to_owned(),
            table: "table".to_owned(),
            columns: Vec::new(),
            referenced_schema: "public".to_owned(),
            referenced_table: "parent".to_owned(),
            referenced_columns: Vec::new(),
        }],
    };
    let ddl = reconstruct_ddl(&snapshot);
    assert!(ddl.contains(r#""odd""schema"."table""#));
    assert!(ddl.contains("AS SELECT 1"));
    assert!(ddl.contains("FOREIGN KEY ()"));
    assert!(ddl.contains("BEGIN SELECT 1; END;"));
    assert!(ddl.contains("LANGUAGE plpgsql"));
    assert!(!ddl.contains("AGGREGATE"));

    let graph = extract_snapshot(&snapshot, Path::new("postgresql:/fixture/db"));
    assert_eq!(graph.node_count(), graph.extraction.nodes.len());
    assert_eq!(graph.edge_count(), graph.extraction.edges.len());
    assert_eq!(graph.virtual_path, Path::new("postgresql:/fixture/db"));
    let fragment = graph.into_fragment();
    assert_eq!(fragment["input_tokens"], 0);
    assert!(fragment["nodes"].is_array());
    Ok(())
}

#[test]
fn valid_dsn_connection_failure_is_sanitized_and_bounded() {
    let error = introspect_postgres(Some(
        "host=127.0.0.1 port=1 user=fixture password=never-print dbname=fixture connect_timeout=1",
    ))
    .err()
    .map(|error| error.to_string())
    .unwrap_or_default();
    assert!(error.starts_with("could not connect to PostgreSQL:"));
    assert!(!error.contains("never-print"));
}
