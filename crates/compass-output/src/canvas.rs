use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use compass_files::write_text_atomic;
use compass_graph::Communities;
use compass_model::GraphDocument;
use serde_json::{Map, Value};

use crate::OutputError;
use crate::json::escape_non_ascii;
use crate::obsidian::{node_filenames, safe_note_name};

const COLORS: [&str; 6] = ["1", "2", "3", "4", "5", "6"];

#[derive(Clone, Debug, Default)]
pub struct CanvasOptions<'a> {
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub node_filenames: Option<&'a BTreeMap<String, String>>,
}

#[must_use]
pub fn canvas_document(
    document: &GraphDocument,
    source_communities: &Communities,
    options: &CanvasOptions<'_>,
) -> String {
    let generated;
    let filenames = if let Some(filenames) = options.node_filenames {
        filenames
    } else {
        generated = node_filenames(document);
        &generated
    };
    let synthetic;
    let communities = if source_communities.is_empty() && !document.nodes.is_empty() {
        synthetic = BTreeMap::from([(
            0,
            document.nodes.iter().map(|node| node.id.clone()).collect(),
        )]);
        &synthetic
    } else {
        source_communities
    };

    let node_lookup = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<HashMap<_, _>>();
    let count = communities.len();
    let cols = if count == 0 { 1 } else { ceil_sqrt(count) };
    let rows = if count == 0 { 1 } else { count.div_ceil(cols) };
    let community_ids = communities.keys().copied().collect::<Vec<_>>();
    let mut sizes = BTreeMap::new();
    let mut inner_columns = BTreeMap::new();
    for (community, members) in communities {
        let n = members
            .iter()
            .filter(|member| {
                node_lookup.contains_key(member.as_str()) && filenames.contains_key(*member)
            })
            .count();
        let inner = ceil_sqrt(n).max(1);
        sizes.insert(
            *community,
            (
                (220 * inner).max(600),
                (100 * n.div_ceil(inner) + 120).max(400),
            ),
        );
        inner_columns.insert(*community, inner);
    }
    let col_widths = (0..cols)
        .map(|column| {
            (0..rows)
                .filter_map(|row| community_ids.get(row * cols + column))
                .filter_map(|community| sizes.get(community).map(|size| size.0))
                .max()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let row_heights = (0..rows)
        .map(|row| {
            (0..cols)
                .filter_map(|column| community_ids.get(row * cols + column))
                .filter_map(|community| sizes.get(community).map(|size| size.1))
                .max()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();

    let mut nodes = Vec::new();
    for (index, community) in community_ids.iter().enumerate() {
        let column = index % cols;
        let row = index / cols;
        let x = col_widths[..column].iter().sum::<usize>() + column * 80;
        let y = row_heights[..row].iter().sum::<usize>() + row * 80;
        let (width, height) = sizes[community];

        let mut group = Map::new();
        group.insert("id".into(), Value::String(format!("g{community}")));
        group.insert("type".into(), Value::String("group".into()));
        group.insert(
            "label".into(),
            Value::String(community_name(*community, options.community_labels)),
        );
        group.insert("x".into(), Value::from(x));
        group.insert("y".into(), Value::from(y));
        group.insert("width".into(), Value::from(width));
        group.insert("height".into(), Value::from(height));
        group.insert(
            "color".into(),
            Value::String(COLORS[index % COLORS.len()].into()),
        );
        nodes.push(Value::Object(group));

        let mut members = communities[community]
            .iter()
            .filter_map(|id| node_lookup.get(id.as_str()).map(|node| (id, *node)))
            .filter(|(id, _)| filenames.contains_key(*id))
            .collect::<Vec<_>>();
        members.sort_by(|left, right| left.1.label().cmp(right.1.label()));
        let inner = inner_columns[community];
        for (member_index, (id, node)) in members.into_iter().enumerate() {
            let card_x = x + 20 + (member_index % inner) * 200;
            let card_y = y + 80 + (member_index / inner) * 80;
            let filename = filenames
                .get(id)
                .cloned()
                .unwrap_or_else(|| safe_note_name(node.label()));
            let mut card = Map::new();
            card.insert("id".into(), Value::String(format!("n_{id}")));
            card.insert("type".into(), Value::String("file".into()));
            card.insert("file".into(), Value::String(format!("{filename}.md")));
            card.insert("x".into(), Value::from(card_x));
            card.insert("y".into(), Value::from(card_y));
            card.insert("width".into(), Value::from(180));
            card.insert("height".into(), Value::from(60));
            nodes.push(Value::Object(card));
        }
    }

    let canvas_members = communities
        .values()
        .flatten()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut weighted = document
        .links
        .iter()
        .filter(|edge| {
            canvas_members.contains(edge.source.as_str())
                && canvas_members.contains(edge.target.as_str())
        })
        .map(|edge| {
            let weight = edge
                .attributes
                .get("weight")
                .and_then(Value::as_f64)
                .unwrap_or(1.0);
            let relation = edge
                .attributes
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let confidence = edge
                .attributes
                .get("confidence")
                .and_then(Value::as_str)
                .unwrap_or("EXTRACTED");
            let label = if relation.is_empty() {
                format!("[{confidence}]")
            } else {
                format!("{relation} [{confidence}]")
            };
            (weight, &edge.source, &edge.target, label)
        })
        .collect::<Vec<_>>();
    weighted.sort_by(|left, right| right.0.partial_cmp(&left.0).unwrap_or(Ordering::Equal));
    let edges = weighted
        .into_iter()
        .take(200)
        .map(|(_, source, target, label)| {
            let mut edge = Map::new();
            edge.insert("id".into(), Value::String(format!("e_{source}_{target}")));
            edge.insert("fromNode".into(), Value::String(format!("n_{source}")));
            edge.insert("toNode".into(), Value::String(format!("n_{target}")));
            edge.insert("label".into(), Value::String(label));
            Value::Object(edge)
        })
        .collect();
    let mut root = Map::new();
    root.insert("nodes".into(), Value::Array(nodes));
    root.insert("edges".into(), Value::Array(edges));
    escape_non_ascii(&serde_json::to_string_pretty(&Value::Object(root)).unwrap_or_default())
}

pub fn write_canvas(
    document: &GraphDocument,
    communities: &Communities,
    output_path: impl AsRef<Path>,
    options: &CanvasOptions<'_>,
) -> Result<(), OutputError> {
    write_text_atomic(
        output_path,
        &canvas_document(document, communities, options),
    )?;
    Ok(())
}

fn ceil_sqrt(value: usize) -> usize {
    let mut root = (value as f64).sqrt() as usize;
    if root * root < value {
        root += 1;
    }
    root
}

fn community_name(community: usize, labels: Option<&BTreeMap<usize, String>>) -> String {
    labels
        .and_then(|labels| labels.get(&community).cloned())
        .unwrap_or_else(|| format!("Community {community}"))
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::*;

    #[test]
    fn populated_graph_without_communities_gets_synthetic_group() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[{"id":"a","label":"A"},{"id":"b","label":"B"}],
            "links":[{"source":"a","target":"b"}]
        }))?;
        let value: Value = serde_json::from_str(&canvas_document(
            &graph,
            &Communities::new(),
            &CanvasOptions::default(),
        ))?;
        assert_eq!(value["nodes"].as_array().map(Vec::len), Some(3));
        assert_eq!(value["edges"].as_array().map(Vec::len), Some(1));
        Ok(())
    }

    #[test]
    fn cards_use_the_same_square_grid_as_the_group() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes": (0..10).map(|index| json!({"id":format!("n{index}"),"label":format!("N{index:02}")})).collect::<Vec<_>>(),
            "links": []
        }))?;
        let communities = BTreeMap::from([(0, (0..10).map(|index| format!("n{index}")).collect())]);
        let value: Value = serde_json::from_str(&canvas_document(
            &graph,
            &communities,
            &CanvasOptions::default(),
        ))?;
        let cards = value["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|node| node["type"] == "file")
            .collect::<Vec<_>>();
        assert_eq!(
            cards
                .iter()
                .filter_map(|card| card["x"].as_u64())
                .collect::<HashSet<_>>()
                .len(),
            4
        );
        assert_eq!(
            cards
                .iter()
                .filter_map(|card| card["y"].as_u64())
                .collect::<HashSet<_>>()
                .len(),
            3
        );
        Ok(())
    }
}
