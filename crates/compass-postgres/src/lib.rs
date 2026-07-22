//! Read-only PostgreSQL catalog introspection for Compass.

use std::env;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use compass_languages::{Extraction, extract_sql_content};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_postgres::config::Host;
use tokio_postgres::{Client, Config, Row};

const TABLES_QUERY: &str = r#"
    SELECT table_schema, table_name, table_type
    FROM information_schema.tables
    WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
    ORDER BY table_schema, table_name;
"#;

const VIEWS_QUERY: &str = r#"
    SELECT table_schema, table_name, view_definition
    FROM information_schema.views
    WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
    ORDER BY table_schema, table_name;
"#;

const ROUTINES_QUERY: &str = r#"
    SELECT routine_schema, routine_name, routine_type,
           routine_definition, external_language
    FROM information_schema.routines
    WHERE routine_schema NOT IN ('pg_catalog', 'information_schema')
    ORDER BY routine_schema, routine_name;
"#;

// pg_constraint is intentionally used instead of the privilege-filtered
// information_schema.referential_constraints view. Constraint OIDs also avoid
// cross-matching same-named constraints on different tables.
const FOREIGN_KEYS_QUERY: &str = r#"
    SELECT
        con.conname AS constraint_name,
        ns.nspname AS table_schema,
        rel.relname AS table_name,
        (SELECT ARRAY_AGG(att.attname ORDER BY k.ord)
           FROM UNNEST(con.conkey) WITH ORDINALITY AS k(attnum, ord)
           JOIN pg_catalog.pg_attribute att
             ON att.attrelid = con.conrelid AND att.attnum = k.attnum
        ) AS columns,
        fns.nspname AS foreign_table_schema,
        frel.relname AS foreign_table_name,
        (SELECT ARRAY_AGG(att.attname ORDER BY k.ord)
           FROM UNNEST(con.confkey) WITH ORDINALITY AS k(attnum, ord)
           JOIN pg_catalog.pg_attribute att
             ON att.attrelid = con.confrelid AND att.attnum = k.attnum
        ) AS foreign_columns
    FROM pg_catalog.pg_constraint con
    JOIN pg_catalog.pg_class rel ON rel.oid = con.conrelid
    JOIN pg_catalog.pg_namespace ns ON ns.oid = rel.relnamespace
    JOIN pg_catalog.pg_class frel ON frel.oid = con.confrelid
    JOIN pg_catalog.pg_namespace fns ON fns.oid = frel.relnamespace
    WHERE con.contype = 'f'
      AND ns.nspname NOT IN ('pg_catalog', 'information_schema')
    ORDER BY ns.nspname, rel.relname, con.conname;
"#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub table_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub schema: String,
    pub name: String,
    pub definition: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routine {
    pub schema: String,
    pub name: String,
    pub routine_type: String,
    pub definition: Option<String>,
    pub language: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    pub constraint: String,
    pub schema: String,
    pub table: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub tables: Vec<Table>,
    pub views: Vec<View>,
    pub routines: Vec<Routine>,
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PostgresGraph {
    pub extraction: Extraction,
    pub virtual_path: PathBuf,
}

impl PostgresGraph {
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.extraction.nodes.len()
    }

    #[must_use]
    pub fn edge_count(&self) -> usize {
        self.extraction.edges.len()
    }

    #[must_use]
    pub fn into_fragment(self) -> Value {
        serde_json::json!({
            "nodes": self.extraction.nodes,
            "edges": self.extraction.edges,
            "hyperedges": self.extraction.hyperedges,
            "input_tokens": 0,
            "output_tokens": 0,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PostgresIntrospectionError {
    #[error("invalid PostgreSQL DSN")]
    InvalidDsn,
    #[error("could not initialize PostgreSQL runtime: {0}")]
    Runtime(std::io::Error),
    #[error("could not connect to PostgreSQL: {0}")]
    Connection(String),
    #[error("could not query PostgreSQL {stage}: {source}")]
    Query {
        stage: &'static str,
        source: tokio_postgres::Error,
    },
    #[error("invalid PostgreSQL catalog row in {stage}: {source}")]
    Catalog {
        stage: &'static str,
        source: tokio_postgres::Error,
    },
}

/// Connect to PostgreSQL with the native Rust wire client and extract schema facts.
pub fn introspect_postgres(dsn: Option<&str>) -> Result<PostgresGraph, PostgresIntrospectionError> {
    let config = connection_config(dsn)?;
    let virtual_path = virtual_path(&config);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(PostgresIntrospectionError::Runtime)?;
    let snapshot = runtime.block_on(read_catalog(&config, dsn))?;
    Ok(extract_snapshot(&snapshot, &virtual_path))
}

/// Convert catalog rows into the exact virtual SQL document consumed by the
/// deterministic SQL extractor. This is public so compatibility fixtures do
/// not require a live database.
#[must_use]
pub fn extract_snapshot(snapshot: &CatalogSnapshot, virtual_path: &Path) -> PostgresGraph {
    let ddl = reconstruct_ddl(snapshot);
    PostgresGraph {
        extraction: extract_sql_content(virtual_path, ddl.as_bytes()),
        virtual_path: virtual_path.to_path_buf(),
    }
}

#[must_use]
pub fn reconstruct_ddl(snapshot: &CatalogSnapshot) -> String {
    let mut ddl = Vec::new();
    for table in &snapshot.tables {
        if table.table_type == "BASE TABLE" {
            ddl.push(format!(
                "CREATE TABLE {}.{} (id INT);",
                quote_ident(&table.schema),
                quote_ident(&table.name)
            ));
        }
    }
    for view in &snapshot.views {
        let definition = view
            .definition
            .as_deref()
            .filter(|definition| !definition.is_empty())
            .unwrap_or("SELECT 1");
        ddl.push(format!(
            "CREATE VIEW {}.{} AS {definition};",
            quote_ident(&view.schema),
            quote_ident(&view.name)
        ));
    }
    // Foreign keys precede routines because a dialect-specific routine body
    // can put SQL parsers into error recovery that consumes later statements.
    for foreign_key in &snapshot.foreign_keys {
        let columns = quoted_list(&foreign_key.columns);
        let referenced_columns = quoted_list(&foreign_key.referenced_columns);
        ddl.push(format!(
            "ALTER TABLE {}.{} ADD CONSTRAINT {} FOREIGN KEY ({columns}) REFERENCES {}.{}({referenced_columns});",
            quote_ident(&foreign_key.schema),
            quote_ident(&foreign_key.table),
            quote_ident(&foreign_key.constraint),
            quote_ident(&foreign_key.referenced_schema),
            quote_ident(&foreign_key.referenced_table),
        ));
    }
    for routine in &snapshot.routines {
        if !matches!(routine.routine_type.as_str(), "FUNCTION" | "PROCEDURE") {
            continue;
        }
        let body = routine
            .definition
            .as_deref()
            .filter(|body| !body.is_empty())
            .unwrap_or("BEGIN SELECT 1; END;");
        let language = routine
            .language
            .as_deref()
            .filter(|language| !language.is_empty())
            .unwrap_or("plpgsql")
            .to_ascii_lowercase();
        ddl.push(format!(
            "CREATE FUNCTION {}.{}() RETURNS void AS $gfx$ {body} $gfx$ LANGUAGE {language};",
            quote_ident(&routine.schema),
            quote_ident(&routine.name)
        ));
    }
    ddl.join("\n")
}

async fn read_catalog(
    config: &Config,
    raw_dsn: Option<&str>,
) -> Result<CatalogSnapshot, PostgresIntrospectionError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let tls = tokio_postgres_rustls::MakeRustlsConnect::with_native_certs()
        .map(|(connector, _warnings)| connector)
        .unwrap_or_else(|_errors| tokio_postgres_rustls::MakeRustlsConnect::with_webpki_roots());
    let (client, connection) = config
        .connect(tls)
        .await
        .map_err(|error| connection_error(error, config, raw_dsn))?;
    let connection_task = tokio::spawn(connection);
    let result = query_catalog(&client).await;
    drop(client);
    let _ = connection_task.await;
    result
}

async fn query_catalog(client: &Client) -> Result<CatalogSnapshot, PostgresIntrospectionError> {
    client
        .batch_execute("BEGIN TRANSACTION ISOLATION LEVEL SERIALIZABLE READ ONLY DEFERRABLE")
        .await
        .map_err(|source| PostgresIntrospectionError::Query {
            stage: "transaction",
            source,
        })?;
    let result = query_catalog_rows(client).await;
    let rollback = client.batch_execute("ROLLBACK").await;
    if let Err(source) = rollback
        && result.is_ok()
    {
        return Err(PostgresIntrospectionError::Query {
            stage: "transaction rollback",
            source,
        });
    }
    result
}

async fn query_catalog_rows(
    client: &Client,
) -> Result<CatalogSnapshot, PostgresIntrospectionError> {
    let table_rows = query(client, TABLES_QUERY, "tables").await?;
    let view_rows = query(client, VIEWS_QUERY, "views").await?;
    let routine_rows = query(client, ROUTINES_QUERY, "routines").await?;
    let foreign_key_rows = query(client, FOREIGN_KEYS_QUERY, "foreign keys").await?;
    Ok(CatalogSnapshot {
        tables: table_rows
            .iter()
            .map(table_from_row)
            .collect::<Result<_, _>>()?,
        views: view_rows
            .iter()
            .map(view_from_row)
            .collect::<Result<_, _>>()?,
        routines: routine_rows
            .iter()
            .map(routine_from_row)
            .collect::<Result<_, _>>()?,
        foreign_keys: foreign_key_rows
            .iter()
            .map(foreign_key_from_row)
            .collect::<Result<_, _>>()?,
    })
}

async fn query(
    client: &Client,
    statement: &str,
    stage: &'static str,
) -> Result<Vec<Row>, PostgresIntrospectionError> {
    client
        .query(statement, &[])
        .await
        .map_err(|source| PostgresIntrospectionError::Query { stage, source })
}

fn table_from_row(row: &Row) -> Result<Table, PostgresIntrospectionError> {
    Ok(Table {
        schema: field(row, 0, "tables")?,
        name: field(row, 1, "tables")?,
        table_type: field(row, 2, "tables")?,
    })
}

fn view_from_row(row: &Row) -> Result<View, PostgresIntrospectionError> {
    Ok(View {
        schema: field(row, 0, "views")?,
        name: field(row, 1, "views")?,
        definition: field(row, 2, "views")?,
    })
}

fn routine_from_row(row: &Row) -> Result<Routine, PostgresIntrospectionError> {
    Ok(Routine {
        schema: field(row, 0, "routines")?,
        name: field(row, 1, "routines")?,
        routine_type: field(row, 2, "routines")?,
        definition: field(row, 3, "routines")?,
        language: field(row, 4, "routines")?,
    })
}

fn foreign_key_from_row(row: &Row) -> Result<ForeignKey, PostgresIntrospectionError> {
    Ok(ForeignKey {
        constraint: field(row, 0, "foreign keys")?,
        schema: field(row, 1, "foreign keys")?,
        table: field(row, 2, "foreign keys")?,
        columns: field(row, 3, "foreign keys")?,
        referenced_schema: field(row, 4, "foreign keys")?,
        referenced_table: field(row, 5, "foreign keys")?,
        referenced_columns: field(row, 6, "foreign keys")?,
    })
}

fn field<T>(row: &Row, index: usize, stage: &'static str) -> Result<T, PostgresIntrospectionError>
where
    T: for<'a> tokio_postgres::types::FromSql<'a>,
{
    row.try_get(index)
        .map_err(|source| PostgresIntrospectionError::Catalog { stage, source })
}

fn connection_config(dsn: Option<&str>) -> Result<Config, PostgresIntrospectionError> {
    // psycopg.connect(dsn or "") delegates an empty DSN to libpq's PG*
    // environment. Preserve that public behavior for `--postgres=`.
    if let Some(dsn) = dsn.filter(|dsn| !dsn.is_empty()) {
        return Config::from_str(dsn).map_err(|_error| PostgresIntrospectionError::InvalidDsn);
    }
    let mut config = Config::new();
    if let Ok(value) = env::var("PGHOST") {
        config.host(value);
    }
    if let Ok(value) = env::var("PGPORT")
        && let Ok(port) = value.parse()
    {
        config.port(port);
    }
    if let Ok(value) = env::var("PGUSER") {
        config.user(value);
    }
    if let Ok(value) = env::var("PGPASSWORD") {
        config.password(value);
    }
    if let Ok(value) = env::var("PGDATABASE") {
        config.dbname(value);
    }
    if let Ok(value) = env::var("PGAPPNAME") {
        config.application_name(value);
    }
    Ok(config)
}

fn virtual_path(config: &Config) -> PathBuf {
    let host = match config.get_hosts().first() {
        Some(Host::Tcp(host)) => host.clone(),
        #[cfg(unix)]
        Some(Host::Unix(path)) => path.to_string_lossy().into_owned(),
        None => "localhost".to_owned(),
    };
    let database = config.get_dbname().unwrap_or("db");
    PathBuf::from(format!("postgresql:/{host}/{database}"))
}

fn connection_error(
    error: tokio_postgres::Error,
    config: &Config,
    raw_dsn: Option<&str>,
) -> PostgresIntrospectionError {
    let mut message = error
        .to_string()
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    if let Some(password) = config.get_password()
        && !password.is_empty()
    {
        message = message.replace(String::from_utf8_lossy(password).as_ref(), "[redacted]");
    }
    if let Some(dsn) = raw_dsn
        && !dsn.is_empty()
    {
        message = message.replace(dsn, "[redacted DSN]");
    }
    PostgresIntrospectionError::Connection(message)
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn quoted_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| quote_ident(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> CatalogSnapshot {
        CatalogSnapshot {
            tables: vec![
                Table {
                    schema: "public".to_owned(),
                    name: "order".to_owned(),
                    table_type: "BASE TABLE".to_owned(),
                },
                Table {
                    schema: "public".to_owned(),
                    name: "user-data".to_owned(),
                    table_type: "BASE TABLE".to_owned(),
                },
            ],
            views: vec![View {
                schema: "public".to_owned(),
                name: "active-users".to_owned(),
                definition: Some("SELECT * FROM public.\"user-data\"".to_owned()),
            }],
            routines: vec![Routine {
                schema: "public".to_owned(),
                name: "do\"work".to_owned(),
                routine_type: "PROCEDURE".to_owned(),
                definition: None,
                language: None,
            }],
            foreign_keys: vec![ForeignKey {
                constraint: "fk-owner".to_owned(),
                schema: "public".to_owned(),
                table: "user-data".to_owned(),
                columns: vec!["owner-id".to_owned(), "tenant".to_owned()],
                referenced_schema: "public".to_owned(),
                referenced_table: "order".to_owned(),
                referenced_columns: vec!["id".to_owned(), "tenant".to_owned()],
            }],
        }
    }

    #[test]
    fn ddl_quotes_identifiers_and_places_foreign_keys_before_routines() {
        let ddl = reconstruct_ddl(&snapshot());
        assert!(ddl.contains("\"do\"\"work\""));
        let foreign_key = ddl.find("ALTER TABLE");
        let routine = ddl.find("CREATE FUNCTION");
        assert!(
            foreign_key
                .zip(routine)
                .is_some_and(|(fk, routine)| fk < routine)
        );
        assert!(FOREIGN_KEYS_QUERY.contains("pg_catalog.pg_constraint"));
        assert!(!FOREIGN_KEYS_QUERY.contains("referential_constraints"));
    }

    #[test]
    fn virtual_graph_never_contains_credentials() {
        let graph = extract_snapshot(&snapshot(), Path::new("postgresql:/host/database"));
        assert!(!graph.extraction.nodes.is_empty());
        for node in &graph.extraction.nodes {
            assert_eq!(
                node.attributes.get("source_file").and_then(Value::as_str),
                Some("postgresql:/host/database")
            );
        }
        let labels = graph
            .extraction
            .nodes
            .iter()
            .filter_map(|node| node.attributes.get("label").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert!(labels.contains(&"\"public\".\"order\""));
        assert!(labels.contains(&"\"public\".\"user-data\""));
        assert!(graph.extraction.edges.iter().any(|edge| {
            edge.attributes.get("relation").and_then(Value::as_str) == Some("references")
        }));
    }

    #[test]
    fn invalid_dsn_does_not_echo_credentials() {
        let error = introspect_postgres(Some("not a DSN password=top-secret"))
            .err()
            .map(|error| error.to_string());
        assert_eq!(error.as_deref(), Some("invalid PostgreSQL DSN"));
    }

    #[test]
    fn empty_dsn_uses_environment_configuration() -> Result<(), PostgresIntrospectionError> {
        // Do not mutate the process environment: equality proves that both
        // entry points consume the same ambient PG* values present in CI.
        let implicit = connection_config(None)?;
        let explicit_empty = connection_config(Some(""))?;
        assert_eq!(format!("{implicit:?}"), format!("{explicit_empty:?}"));
        Ok(())
    }
}
