use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use trail_postgres::{CatalogSnapshot, ForeignKey, Routine, Table, View, extract_snapshot};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

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
            name: "refresh-users".to_owned(),
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
fn catalog_snapshot_graph_matches_python_oracle() -> Result<(), Box<dyn Error>> {
    let repository = repository_root();
    let directory = tempfile::tempdir()?;
    let fixture = directory.path().join("snapshot.json");
    std::fs::write(&fixture, serde_json::to_vec(&snapshot())?)?;
    let script = r#"
import json, sys, types
data = json.load(open(sys.argv[1], encoding='utf-8'))
class OperationalError(Exception): pass
class Cursor:
    def __enter__(self): return self
    def __exit__(self, *args): return False
    def execute(self, query): self.query = query
    def fetchall(self):
        if 'information_schema.tables' in self.query:
            return [(x['schema'], x['name'], x['table_type']) for x in data['tables']]
        if 'information_schema.views' in self.query:
            return [(x['schema'], x['name'], x['definition']) for x in data['views']]
        if 'information_schema.routines' in self.query:
            return [(x['schema'], x['name'], x['routine_type'], x['definition'], x['language']) for x in data['routines']]
        if 'pg_catalog.pg_constraint' in self.query:
            return [(x['constraint'], x['schema'], x['table'], x['columns'], x['referenced_schema'], x['referenced_table'], x['referenced_columns']) for x in data['foreign_keys']]
        raise AssertionError(self.query)
class Connection:
    def execute(self, query): return None
    def cursor(self): return Cursor()
    def close(self): pass
module = types.ModuleType('psycopg')
module.OperationalError = OperationalError
module.connect = lambda dsn: Connection()
module.conninfo = types.SimpleNamespace(conninfo_to_dict=lambda dsn: {'host':'db.internal','dbname':'app'})
sys.modules['psycopg'] = module
from graphify.pg_introspect import introspect_postgres
print(json.dumps(introspect_postgres('postgresql://user:secret@db.internal/app'), sort_keys=True))
"#;
    let output = Command::new(repository.join(".venv/bin/python"))
        .env("PYTHONPATH", &repository)
        .args(["-c", script])
        .arg(&fixture)
        .output()?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned().into());
    }
    let expected: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let actual =
        extract_snapshot(&snapshot(), Path::new("postgresql:/db.internal/app")).into_fragment();
    assert_eq!(actual.get("nodes"), expected.get("nodes"));
    assert_eq!(actual.get("edges"), expected.get("edges"));
    Ok(())
}
