use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::{Extraction, make_id};

const META_HEADS: &[&str] = &["count", "each", "self", "path", "terraform"];

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let scope = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("tf")
        .to_owned();
    let mut state = State {
        source,
        source_file,
        file_id: file_id.clone(),
        scope,
        extraction: Extraction {
            raw_calls: None,
            ..Extraction::default()
        },
        seen_nodes: HashSet::from([file_id.clone()]),
        seen_edges: HashSet::new(),
    };
    state.extraction.nodes.push(
        state.node(
            file_id,
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default(),
            None,
        ),
    );

    let body = first_child_of_kind(root, "body").unwrap_or(root);
    let mut cursor = body.walk();
    for block in body.children(&mut cursor) {
        if block.kind() != "block" {
            continue;
        }
        let (block_type, labels) = state.block_parts(block);
        let line = block.start_position().row + 1;
        let block_body = first_child_of_kind(block, "body");
        let owner = match (block_type.as_deref(), labels.as_slice()) {
            (Some("resource"), [resource_type, resource_name, ..]) => Some(state.add_node(
                &format!("{resource_type}.{resource_name}"),
                &format!("{resource_type}.{resource_name}"),
                line,
            )),
            (Some("data"), [data_type, data_name, ..]) => Some(state.add_node(
                &format!("data.{data_type}.{data_name}"),
                &format!("data.{data_type}.{data_name}"),
                line,
            )),
            (Some("module"), [name, ..]) => {
                Some(state.add_node(&format!("module.{name}"), &format!("module.{name}"), line))
            }
            (Some("variable"), [name, ..]) => {
                Some(state.add_node(&format!("var.{name}"), &format!("var.{name}"), line))
            }
            (Some("output"), [name, ..]) => {
                Some(state.add_node(&format!("output.{name}"), &format!("output.{name}"), line))
            }
            (Some("provider"), [name, ..]) => Some(state.add_node(
                &format!("provider.{name}"),
                &format!("provider.{name}"),
                line,
            )),
            (Some("locals"), _) => {
                if let Some(block_body) = block_body {
                    state.add_locals(block_body);
                }
                None
            }
            _ => None,
        };
        if let (Some(owner), Some(block_body)) = (owner, block_body) {
            state.collect_references(block_body, &owner, "references");
        }
    }
    state.extraction
}

struct State<'source> {
    source: &'source [u8],
    source_file: String,
    file_id: String,
    scope: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<(String, String, String)>,
}

impl State<'_> {
    fn block_parts(&self, block: Node<'_>) -> (Option<String>, Vec<String>) {
        let mut block_type = None;
        let mut labels = Vec::new();
        let mut cursor = block.walk();
        for child in block.children(&mut cursor) {
            if matches!(child.kind(), "block_start" | "body" | "block_end") {
                break;
            }
            if child.kind() == "identifier" && block_type.is_none() {
                block_type = Some(self.text(child).to_owned());
            } else if matches!(child.kind(), "string_lit" | "identifier") {
                labels.push(self.text(child).trim().trim_matches('"').to_owned());
            }
        }
        (block_type, labels)
    }

    fn add_locals(&mut self, body: Node<'_>) {
        let mut cursor = body.walk();
        for attribute in body.children(&mut cursor) {
            if attribute.kind() != "attribute" {
                continue;
            }
            let Some(key_node) = attribute.child(0) else {
                continue;
            };
            let key = self.text(key_node).to_owned();
            let line = attribute.start_position().row + 1;
            let owner = self.add_node(&format!("local.{key}"), &format!("local.{key}"), line);
            self.collect_references(attribute, &owner, "references");
        }
    }

    fn collect_references(&mut self, node: Node<'_>, owner: &str, relation: &str) {
        let relation = if node.kind() == "attribute"
            && node
                .child_by_field_name("key")
                .or_else(|| node.child(0))
                .is_some_and(|key| self.text(key) == "depends_on")
        {
            "depends_on"
        } else {
            relation
        };
        if node.kind() == "variable_expr"
            && let Some(address) = self.reference_address(node)
        {
            self.add_edge(owner, &address, relation, node.start_position().row + 1);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_references(child, owner, relation);
        }
    }

    fn reference_address(&self, expression: Node<'_>) -> Option<String> {
        let head = self.text(expression);
        if head.is_empty() || META_HEADS.contains(&head) {
            return None;
        }
        let mut attributes = Vec::new();
        if let Some(parent) = expression.parent() {
            let mut seen_expression = false;
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.id() == expression.id() {
                    seen_expression = true;
                    continue;
                }
                if !seen_expression {
                    continue;
                }
                if child.kind() != "get_attr" {
                    break;
                }
                let mut child_cursor = child.walk();
                let Some(identifier) = child
                    .children(&mut child_cursor)
                    .find(|grandchild| grandchild.kind() == "identifier")
                else {
                    break;
                };
                attributes.push(self.text(identifier));
            }
        }
        match head {
            "var" => attributes.first().map(|name| format!("var.{name}")),
            "local" => attributes.first().map(|name| format!("local.{name}")),
            "module" => attributes.first().map(|name| format!("module.{name}")),
            "data" if attributes.len() >= 2 => {
                Some(format!("data.{}.{}", attributes[0], attributes[1]))
            }
            _ => attributes.first().map(|name| format!("{head}.{name}")),
        }
    }

    fn add_node(&mut self, address: &str, label: &str, line: usize) -> String {
        let id = make_id(&[&self.scope, address]);
        if self.seen_nodes.insert(id.clone()) {
            self.extraction
                .nodes
                .push(self.node(id.clone(), label, Some(line)));
            let mut attributes = edge_attributes(&self.source_file, "contains", line);
            self.extraction.edges.push(EdgeRecord {
                source: self.file_id.clone(),
                target: id.clone(),
                attributes: std::mem::take(&mut attributes),
            });
        }
        id
    }

    fn add_edge(&mut self, source: &str, address: &str, relation: &str, line: usize) {
        let target = make_id(&[&self.scope, address]);
        if source == target {
            return;
        }
        let key = (source.to_owned(), target.clone(), relation.to_owned());
        if !self.seen_edges.insert(key) {
            return;
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target,
            attributes: edge_attributes(&self.source_file, relation, line),
        });
    }

    fn node(&self, id: String, label: &str, line: Option<usize>) -> NodeRecord {
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            line.map_or(Value::Null, |line| Value::String(format!("L{line}"))),
        );
        NodeRecord { id, attributes }
    }

    fn text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.source).unwrap_or_default()
    }
}

fn first_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn edge_attributes(source_file: &str, relation: &str, line: usize) -> Map<String, Value> {
    let mut attributes = Map::new();
    attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
    attributes.insert(
        "confidence".to_owned(),
        Value::String("EXTRACTED".to_owned()),
    );
    attributes.insert(
        "source_file".to_owned(),
        Value::String(source_file.to_owned()),
    );
    attributes.insert(
        "source_location".to_owned(),
        Value::String(format!("L{line}")),
    );
    attributes.insert("weight".to_owned(), json!(1.0));
    attributes
}
