use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use trail_files::write_text_atomic;
use trail_model::{GraphDocument, NodeRecord};

use crate::{Aggregate, ProvenanceEvent, ReflectError};

pub const LEARNING_SIDECAR_NAME: &str = ".graphify_learning.json";
const LEARNING_SCHEMA_VERSION: u8 = 1;
const PROVENANCE_CAP: usize = 5;

#[derive(Default)]
struct GraphMaps {
    ids: HashMap<String, String>,
    label_to_ids: HashMap<String, Vec<String>>,
    nodes: HashMap<String, NodeRecord>,
}

#[must_use]
pub fn build_learning_overlay(
    aggregate: &Aggregate,
    graph_path: &Path,
    now: OffsetDateTime,
) -> Value {
    let maps = build_maps(graph_path);
    let mut nodes = Map::new();
    for (status, sources) in [
        ("preferred", &aggregate.preferred),
        ("tentative", &aggregate.tentative),
    ] {
        for source in sources {
            let Some(id) = canonical_id(&source.node, &maps) else {
                continue;
            };
            if nodes.contains_key(&id) {
                continue;
            }
            let node = maps.nodes.get(&id);
            let provenance = provenance_for(&source.node, &aggregate.provenance);
            let last = provenance
                .first()
                .map(|event| event["date"].as_str().unwrap_or_default())
                .unwrap_or_default();
            nodes.insert(
                id,
                json!({
                    "status":status,
                    "score":source.score,
                    "uses":source.n,
                    "last":last,
                    "label":node.map_or(source.node.as_str(), NodeRecord::label),
                    "source_file":node.map_or_else(String::new, |node| node.string("source_file")),
                    "code_fingerprint":code_fingerprint(node, graph_path),
                    "provenance":provenance,
                }),
            );
        }
    }
    for source in &aggregate.contested {
        let Some(id) = canonical_id(&source.node, &maps) else {
            continue;
        };
        if nodes.contains_key(&id) {
            continue;
        }
        let node = maps.nodes.get(&id);
        nodes.insert(
            id,
            json!({
                "status":"contested",
                "score":source.score,
                "uses":source.pos,
                "last":source.last,
                "label":node.map_or(source.node.as_str(), NodeRecord::label),
                "source_file":node.map_or_else(String::new, |node| node.string("source_file")),
                "code_fingerprint":code_fingerprint(node, graph_path),
                "provenance":provenance_for(&source.node, &aggregate.provenance),
                "verdict":source.verdict,
                "neg":source.neg,
            }),
        );
    }
    recursively_sorted(json!({
        "version":LEARNING_SCHEMA_VERSION,
        "generated_at":iso_timestamp(now),
        "nodes":nodes,
    }))
}

pub fn write_learning_sidecar(
    aggregate: &Aggregate,
    graph_path: &Path,
    now: OffsetDateTime,
) -> Result<PathBuf, ReflectError> {
    let sidecar = graph_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(LEARNING_SIDECAR_NAME);
    let overlay = build_learning_overlay(aggregate, graph_path, now);
    let encoded = serde_json::to_string_pretty(&overlay).unwrap_or_else(|_| "{}".to_owned());
    write_text_atomic(&sidecar, &format!("{encoded}\n")).map_err(|source| ReflectError::Write {
        path: sidecar.clone(),
        source,
    })?;
    Ok(sidecar)
}

fn build_maps(graph_path: &Path) -> GraphMaps {
    let Ok(graph) = GraphDocument::load(graph_path) else {
        return GraphMaps::default();
    };
    let mut maps = GraphMaps::default();
    for node in graph.nodes {
        maps.ids.insert(node.id.clone(), node.id.clone());
        maps.label_to_ids
            .entry(node.label().to_owned())
            .or_default()
            .push(node.id.clone());
        maps.nodes.insert(node.id.clone(), node);
    }
    maps
}

fn canonical_id(cited: &str, maps: &GraphMaps) -> Option<String> {
    if let Some(id) = maps.ids.get(cited) {
        return Some(id.clone());
    }
    let ids = maps.label_to_ids.get(cited)?;
    (ids.len() == 1).then(|| ids[0].clone())
}

fn provenance_for(node: &str, provenance: &HashMap<String, Vec<ProvenanceEvent>>) -> Vec<Value> {
    let mut events = provenance.get(node).cloned().unwrap_or_default();
    events.sort_by(|left, right| (&right.date, &right.question).cmp(&(&left.date, &left.question)));
    events
        .into_iter()
        .take(PROVENANCE_CAP)
        .map(|event| {
            json!({
                "q":event.question,
                "date":event.date,
                "outcome":event.outcome,
            })
        })
        .collect()
}

fn code_fingerprint(node: Option<&NodeRecord>, graph_path: &Path) -> String {
    let Some(node) = node else {
        return String::new();
    };
    let source = node.string("source_file");
    let Some(path) = resolve_source_path(&source, graph_path) else {
        return String::new();
    };
    fs::read(path)
        .ok()
        .map(|content| format!("{:x}", Sha256::digest(content)))
        .unwrap_or_default()
}

fn resolve_source_path(source: &str, graph_path: &Path) -> Option<PathBuf> {
    if source.is_empty() {
        return None;
    }
    let source = Path::new(source);
    if source.is_absolute() {
        return source.is_file().then(|| source.to_path_buf());
    }
    let output = graph_path.parent().unwrap_or_else(|| Path::new("."));
    let mut candidates = Vec::new();
    if let Ok(recorded) = fs::read_to_string(output.join(".graphify_root")) {
        let recorded = recorded.trim();
        if !recorded.is_empty() {
            candidates.push(PathBuf::from(recorded));
        }
    }
    if output.file_name().and_then(|name| name.to_str()) == Some("graphify-out") {
        candidates.push(
            output
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
        );
        candidates.push(output.to_path_buf());
    } else {
        candidates.push(output.to_path_buf());
        candidates.push(
            output
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .to_path_buf(),
        );
    }
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current);
    }
    let mut seen = std::collections::HashSet::new();
    candidates.into_iter().find_map(|base| {
        if !seen.insert(base.clone()) {
            return None;
        }
        let candidate = base.join(source);
        candidate.is_file().then_some(candidate)
    })
}

fn recursively_sorted(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, recursively_sorted(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(recursively_sorted).collect()),
        value => value,
    }
}

fn iso_timestamp(now: OffsetDateTime) -> String {
    let base = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let fraction = if now.microsecond() == 0 {
        String::new()
    } else {
        format!(".{:06}", now.microsecond())
    };
    let offset = now.offset();
    let total = offset.whole_seconds();
    let sign = if total < 0 { '-' } else { '+' };
    let absolute = total.unsigned_abs();
    format!(
        "{base}{fraction}{sign}{:02}:{:02}",
        absolute / 3_600,
        absolute % 3_600 / 60
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use crate::{ContestedSource, Counts, SourceScore};

    #[test]
    fn overlay_resolves_unique_labels_and_hashes_source() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let output = directory.path().join("graphify-out");
        fs::create_dir_all(&output)?;
        fs::write(directory.path().join("source.rs"), "fn main() {}")?;
        let graph = output.join("graph.json");
        fs::write(
            &graph,
            r#"{"directed":true,"multigraph":false,"graph":{},"nodes":[{"id":"source_main","label":"main","source_file":"source.rs"}],"links":[]}"#,
        )?;
        let aggregate = Aggregate {
            total: 1,
            counts: Counts {
                useful: 1,
                ..Counts::default()
            },
            min_corroboration: 1,
            preferred: vec![SourceScore {
                node: "main".to_owned(),
                n: 1,
                score: 1.0,
            }],
            ..Aggregate::default()
        };
        let overlay = build_learning_overlay(&aggregate, &graph, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(overlay["nodes"]["source_main"]["status"], "preferred");
        assert_ne!(overlay["nodes"]["source_main"]["code_fingerprint"], "");
        Ok(())
    }

    #[test]
    fn contested_overlay_skips_unknown_and_ambiguous_labels() -> Result<(), Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let graph = directory.path().join("graph.json");
        fs::write(
            &graph,
            r#"{"directed":true,"multigraph":false,"graph":{},"nodes":[{"id":"a","label":"duplicate"},{"id":"b","label":"duplicate"},{"id":"c","label":"contested"}],"links":[]}"#,
        )?;
        let aggregate = Aggregate {
            contested: vec![
                ContestedSource {
                    node: "contested".to_owned(),
                    pos: 2,
                    neg: 1,
                    score: 0.5,
                    verdict: "mixed".to_owned(),
                    last: "2026-01-01".to_owned(),
                },
                ContestedSource {
                    node: "duplicate".to_owned(),
                    pos: 1,
                    neg: 1,
                    score: 0.0,
                    verdict: "mixed".to_owned(),
                    last: String::new(),
                },
                ContestedSource {
                    node: "missing".to_owned(),
                    pos: 1,
                    neg: 1,
                    score: 0.0,
                    verdict: "mixed".to_owned(),
                    last: String::new(),
                },
            ],
            ..Aggregate::default()
        };
        let overlay = build_learning_overlay(&aggregate, &graph, OffsetDateTime::UNIX_EPOCH);
        assert_eq!(overlay["nodes"]["c"]["status"], "contested");
        assert_eq!(overlay["nodes"]["c"]["uses"], 2);
        assert_eq!(overlay["nodes"]["c"]["neg"], 1);
        assert_eq!(overlay["nodes"].as_object().map(Map::len), Some(1));
        Ok(())
    }

    #[test]
    fn source_resolution_covers_absolute_output_and_parent_candidates() -> Result<(), Box<dyn Error>>
    {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.rs");
        fs::write(&source, "fn source() {}")?;
        let output = directory.path().join("custom-output");
        fs::create_dir_all(&output)?;
        let graph = output.join("graph.json");

        assert_eq!(resolve_source_path("", &graph), None);
        assert_eq!(
            resolve_source_path(&source.to_string_lossy(), &graph),
            Some(source.clone())
        );
        assert_eq!(resolve_source_path("missing.rs", &graph), None);
        assert_eq!(resolve_source_path("source.rs", &graph), Some(source));
        assert_eq!(code_fingerprint(None, &graph), "");
        let sorted = recursively_sorted(json!({"z":{"b":1,"a":2},"a":[{"d":1,"c":2}]}));
        assert_eq!(
            serde_json::to_string(&sorted)?,
            r#"{"a":[{"c":2,"d":1}],"z":{"a":2,"b":1}}"#
        );
        Ok(())
    }
}
