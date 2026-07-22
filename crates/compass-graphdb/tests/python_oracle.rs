use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::Command;

use compass_graphdb::graph_operations;
use compass_model::GraphDocument;
use serde_json::{Value, json};

const PYTHON_ORACLE: &str = r#"
import json
import sys
import types

import networkx as nx

neo4j_calls = []
falkordb_calls = []

class Session:
    def __enter__(self):
        return self
    def __exit__(self, *_args):
        return None
    def run(self, statement, **params):
        neo4j_calls.append({"statement": statement, "params": params})

class Driver:
    def session(self):
        return Session()
    def close(self):
        return None

class GraphDatabase:
    @staticmethod
    def driver(_uri, auth):
        assert auth == ("neo4j", "secret")
        return Driver()

neo4j = types.ModuleType("neo4j")
neo4j.GraphDatabase = GraphDatabase
sys.modules["neo4j"] = neo4j

class FalkorGraph:
    def query(self, statement, params):
        falkordb_calls.append({"statement": statement, "params": params})

class FalkorDB:
    def __init__(self, **kwargs):
        assert kwargs == {
            "host": "localhost", "port": 6379,
            "username": None, "password": None,
        }
    def select_graph(self, name):
        assert name == "graphify"
        return FalkorGraph()

falkordb = types.ModuleType("falkordb")
falkordb.FalkorDB = FalkorDB
sys.modules["falkordb"] = falkordb

from graphify.exporters.graphdb import push_to_falkordb, push_to_neo4j

graph = nx.DiGraph()
graph.add_node(
    "n'1", label="First", file_type="co-de", count=2, enabled=True,
    ratio=1.25, _private="drop", nested={"drop": True},
)
graph.add_node("n2", label="Second", file_type="", optional=None)
graph.add_edge(
    "n'1", "n2", relation="calls-to!", confidence="EXTRACTED",
    weight=3, _private="drop", nested=["drop"],
)
communities = {7: ["n'1"]}
push_to_neo4j(graph, "bolt://localhost", "neo4j", "secret", communities)
push_to_falkordb(graph, "localhost:6379", communities=communities)
print(json.dumps({"neo4j": neo4j_calls, "falkordb": falkordb_calls}, sort_keys=True))
"#;

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("GRAPHIFY_REPO_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .map_or_else(|| PathBuf::from("../.."), Path::to_path_buf)
}

fn python_executable(repository: &Path) -> PathBuf {
    if cfg!(windows) {
        repository.join(".venv/Scripts/python.exe")
    } else {
        repository.join(".venv/bin/python")
    }
}

#[test]
fn operation_contract_matches_python_for_both_databases() -> Result<(), Box<dyn Error>> {
    let repository = repository_root();
    let output = Command::new(python_executable(&repository))
        .args(["-c", PYTHON_ORACLE])
        .current_dir(&repository)
        .env("PYTHONPATH", &repository)
        .output()?;
    assert!(
        output.status.success(),
        "Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected: Value = serde_json::from_slice(&output.stdout)?;

    let document: GraphDocument = serde_json::from_value(json!({
        "directed": true,
        "multigraph": false,
        "graph": {},
        "nodes": [
            {
                "id": "n'1", "label": "First", "file_type": "co-de",
                "count": 2, "enabled": true, "ratio": 1.25,
                "_private": "drop", "nested": {"drop": true}
            },
            {"id": "n2", "label": "Second", "file_type": "", "optional": null}
        ],
        "links": [{
            "source": "n'1", "target": "n2", "relation": "calls-to!",
            "confidence": "EXTRACTED", "weight": 3,
            "_private": "drop", "nested": ["drop"]
        }]
    }))?;
    let communities = BTreeMap::from([(7, vec!["n'1".to_owned()])]);
    let operations = graph_operations(&document, Some(&communities));
    let actual = operations
        .nodes
        .iter()
        .chain(&operations.edges)
        .map(|operation| {
            json!({
                "statement": operation.statement,
                "params": operation.params,
            })
        })
        .collect::<Vec<_>>();

    assert_eq!(expected["neo4j"], json!(actual));
    assert_eq!(expected["falkordb"], json!(actual));
    Ok(())
}
