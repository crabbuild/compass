//! Compare Compass graph output against Graphify's required shared facts.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use serde_json::Value;

const OBSERVABLE_NODE_FIELDS: &[&str] = &["label", "file_type", "source_file", "source_location"];
const MAX_EXAMPLES: usize = 50;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct EdgeKey {
    source: String,
    target: String,
    relation: String,
}

#[derive(Debug, Default)]
struct Report {
    graphify_nodes: usize,
    compass_nodes: usize,
    graphify_edges: usize,
    compass_edges: usize,
    missing_nodes: Vec<String>,
    mismatched_nodes: Vec<String>,
    missing_edges: Vec<EdgeKey>,
    compass_only_nodes: Vec<String>,
    compass_only_edges: Vec<EdgeKey>,
}

impl Report {
    fn compatible(&self) -> bool {
        self.missing_nodes.is_empty()
            && self.mismatched_nodes.is_empty()
            && self.missing_edges.is_empty()
    }
}

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let program = arguments
        .next()
        .and_then(|value| Path::new(&value).file_name().map(|name| name.to_owned()))
        .and_then(|value| value.into_string().ok())
        .unwrap_or_else(|| "compare-graphs".to_owned());
    let Some(compass_path) = arguments.next() else {
        eprintln!("usage: {program} <compass-graph.json> <graphify-graph.json>");
        return ExitCode::from(2);
    };
    let Some(graphify_path) = arguments.next() else {
        eprintln!("usage: {program} <compass-graph.json> <graphify-graph.json>");
        return ExitCode::from(2);
    };
    if arguments.next().is_some() {
        eprintln!("usage: {program} <compass-graph.json> <graphify-graph.json>");
        return ExitCode::from(2);
    }

    let result = read_graph(Path::new(&compass_path))
        .and_then(|compass| {
            read_graph(Path::new(&graphify_path)).map(|graphify| (compass, graphify))
        })
        .and_then(|(compass, graphify)| compare(&compass, &graphify));
    match result {
        Ok(report) => {
            print_report(&report);
            if report.compatible() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn read_graph(path: &Path) -> Result<Value, String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn compare(compass: &Value, graphify: &Value) -> Result<Report, String> {
    let compass_nodes = node_map(compass, "Compass")?;
    let graphify_nodes = node_map(graphify, "Graphify")?;
    let compass_edges = edge_set(compass, "Compass")?;
    let graphify_edges = edge_set(graphify, "Graphify")?;

    let mut report = Report {
        graphify_nodes: graphify_nodes.len(),
        compass_nodes: compass_nodes.len(),
        graphify_edges: graphify_edges.len(),
        compass_edges: compass_edges.len(),
        ..Report::default()
    };

    for (id, expected) in &graphify_nodes {
        let Some(actual) = compass_nodes.get(id) else {
            report.missing_nodes.push(id.clone());
            continue;
        };
        for field in OBSERVABLE_NODE_FIELDS {
            let Some(expected_value) = comparable_field(expected, field) else {
                continue;
            };
            let actual_value = comparable_field(actual, field).unwrap_or("<missing>");
            if actual_value != expected_value {
                report.mismatched_nodes.push(format!(
                    "{id}: {field} expected {expected_value:?}, got {actual_value:?}"
                ));
            }
        }
    }

    report.compass_only_nodes = compass_nodes
        .keys()
        .filter(|id| !graphify_nodes.contains_key(*id))
        .cloned()
        .collect();
    report.missing_edges = graphify_edges.difference(&compass_edges).cloned().collect();
    report.compass_only_edges = compass_edges.difference(&graphify_edges).cloned().collect();
    Ok(report)
}

fn node_map<'a>(graph: &'a Value, tool: &str) -> Result<BTreeMap<String, &'a Value>, String> {
    let values = graph
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{tool} graph has no nodes array"))?;
    let mut nodes = BTreeMap::new();
    for (index, node) in values.iter().enumerate() {
        let id = node
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("{tool} node {index} has no canonical id"))?;
        nodes.entry(id.to_owned()).or_insert(node);
    }
    Ok(nodes)
}

fn edge_set(graph: &Value, tool: &str) -> Result<BTreeSet<EdgeKey>, String> {
    let values = graph
        .get("links")
        .or_else(|| graph.get("edges"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{tool} graph has no links or edges array"))?;
    values
        .iter()
        .enumerate()
        .map(|(index, edge)| {
            let source = edge
                .get("source")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{tool} edge {index} has no source"))?;
            let target = edge
                .get("target")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| format!("{tool} edge {index} has no target"))?;
            let relation = edge
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Ok(EdgeKey {
                source: source.to_owned(),
                target: target.to_owned(),
                relation: relation.to_owned(),
            })
        })
        .collect()
}

fn comparable_field<'a>(node: &'a Value, field: &str) -> Option<&'a str> {
    node.get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn print_report(report: &Report) {
    println!(
        "nodes: Graphify={} Compass={}",
        report.graphify_nodes, report.compass_nodes
    );
    println!(
        "edges: Graphify={} Compass={}",
        report.graphify_edges, report.compass_edges
    );
    println!("missing Graphify nodes: {}", report.missing_nodes.len());
    println!("mismatched shared nodes: {}", report.mismatched_nodes.len());
    println!("missing Graphify edges: {}", report.missing_edges.len());
    println!("Compass-only nodes: {}", report.compass_only_nodes.len());
    println!("Compass-only edges: {}", report.compass_only_edges.len());

    print_examples("missing nodes", &report.missing_nodes);
    print_examples("field mismatches", &report.mismatched_nodes);
    let missing_edges = report
        .missing_edges
        .iter()
        .map(|edge| format!("{} --{}--> {}", edge.source, edge.relation, edge.target))
        .collect::<Vec<_>>();
    print_examples("missing edges", &missing_edges);
}

fn print_examples(title: &str, examples: &[String]) {
    if examples.is_empty() {
        return;
    }
    println!("{title}:");
    for example in examples.iter().take(MAX_EXAMPLES) {
        println!("  {example}");
    }
    if examples.len() > MAX_EXAMPLES {
        println!("  ... {} more", examples.len() - MAX_EXAMPLES);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn graphify_subset_with_compass_extras_passes() {
        let graphify = json!({
            "nodes": [
                {
                    "id": "a",
                    "label": "a()",
                    "file_type": "code",
                    "source_file": "a.c",
                    "source_location": "L1"
                }
            ],
            "links": []
        });
        let compass = json!({
            "nodes": [
                {
                    "id": "a",
                    "label": "a()",
                    "file_type": "code",
                    "source_file": "a.c",
                    "source_location": "L1"
                },
                {
                    "id": "perl_extra",
                    "label": "run()",
                    "file_type": "code",
                    "source_file": "tool"
                }
            ],
            "links": []
        });

        let report = compare(&compass, &graphify).expect("valid graphs");

        assert!(report.compatible());
        assert_eq!(report.compass_only_nodes, ["perl_extra"]);
    }

    #[test]
    fn missing_node_field_and_edge_fail() {
        let graphify = json!({
            "nodes": [
                {
                    "id": "a",
                    "label": "a()",
                    "file_type": "code",
                    "source_file": "a.c",
                    "source_location": "L1"
                },
                {
                    "id": "b",
                    "label": "b()",
                    "file_type": "code",
                    "source_file": "a.c",
                    "source_location": "L2"
                }
            ],
            "links": [{"source": "a", "target": "b", "relation": "calls"}]
        });
        let compass = json!({
            "nodes": [
                {
                    "id": "a",
                    "label": "wrong()",
                    "file_type": "code",
                    "source_file": "a.c",
                    "source_location": "L1"
                }
            ],
            "links": []
        });

        let report = compare(&compass, &graphify).expect("valid graphs");

        assert!(!report.compatible());
        assert_eq!(report.missing_nodes, ["b"]);
        assert_eq!(report.mismatched_nodes.len(), 1);
        assert_eq!(report.missing_edges.len(), 1);
    }
}
