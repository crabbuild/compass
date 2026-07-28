use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{Extraction, RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value, json};
use tree_sitter::Node;

use crate::make_id;

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
        seen_nodes: HashSet::new(),
        seen_edges: HashSet::new(),
        addresses: HashMap::new(),
    };
    state.add_file(
        &file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
    );

    let body = first_child_of_kind(root, "body").unwrap_or(root);
    let blocks = named_blocks(body);
    // Declare all owners before resolving expressions so forward references
    // are exact and never leave dangling endpoints.
    for block in &blocks {
        state.declare_block(*block);
    }
    for block in blocks {
        state.link_block(block);
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
    seen_edges: HashSet<(String, String, String, usize)>,
    addresses: HashMap<String, String>,
}

impl State<'_> {
    fn declare_block(&mut self, block: Node<'_>) {
        let (block_type, labels) = self.block_parts(block);
        let line = block.start_position().row + 1;
        match (block_type.as_deref(), labels.as_slice()) {
            (Some("resource"), [resource_type, resource_name, ..]) => {
                let address = format!("{resource_type}.{resource_name}");
                self.add_address_node(
                    &address,
                    &address,
                    "resource",
                    line,
                    Some(format!("terraform:{address}")),
                );
            }
            (Some("data"), [data_type, data_name, ..]) => {
                let address = format!("data.{data_type}.{data_name}");
                self.add_address_node(
                    &address,
                    &address,
                    "resource",
                    line,
                    Some(format!("terraform:{address}")),
                );
            }
            (Some("module"), [name, ..]) => {
                let address = format!("module.{name}");
                self.add_address_node(&address, &address, "package", line, None);
            }
            (Some("variable"), [name, ..]) => {
                let address = format!("var.{name}");
                self.add_address_node(&address, &address, "config_key", line, None);
            }
            (Some("output"), [name, ..]) => {
                let address = format!("output.{name}");
                self.add_address_node(&address, &address, "config_key", line, None);
            }
            (Some("provider"), [name, ..]) => {
                let address = format!("provider.{name}");
                self.add_address_node(&address, &address, "config_key", line, None);
            }
            (Some("terraform"), _) => {
                self.add_address_node("terraform", "terraform", "config_key", line, None);
            }
            (Some("locals"), _) => {
                if let Some(block_body) = first_child_of_kind(block, "body") {
                    self.declare_locals(block_body);
                }
            }
            _ => {}
        }
    }

    fn link_block(&mut self, block: Node<'_>) {
        let (block_type, labels) = self.block_parts(block);
        let owner = match (block_type.as_deref(), labels.as_slice()) {
            (Some("resource"), [resource_type, resource_name, ..]) => {
                self.address_id(&format!("{resource_type}.{resource_name}"))
            }
            (Some("data"), [data_type, data_name, ..]) => {
                self.address_id(&format!("data.{data_type}.{data_name}"))
            }
            (Some("module"), [name, ..]) => self.address_id(&format!("module.{name}")),
            (Some("variable"), [name, ..]) => self.address_id(&format!("var.{name}")),
            (Some("output"), [name, ..]) => self.address_id(&format!("output.{name}")),
            (Some("provider"), [name, ..]) => self.address_id(&format!("provider.{name}")),
            (Some("terraform"), _) => self.address_id("terraform"),
            (Some("locals"), _) => {
                if let Some(block_body) = first_child_of_kind(block, "body") {
                    self.link_locals(block_body);
                }
                None
            }
            _ => None,
        };
        if let (Some(owner), Some(block_body)) = (owner, first_child_of_kind(block, "body")) {
            self.collect_references(block_body, &owner, "references");
        }
    }

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

    fn declare_locals(&mut self, body: Node<'_>) {
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
            let address = format!("local.{key}");
            self.add_address_node(&address, &address, "config_key", line, None);
        }
    }

    fn link_locals(&mut self, body: Node<'_>) {
        let mut cursor = body.walk();
        for attribute in body.children(&mut cursor) {
            if attribute.kind() != "attribute" {
                continue;
            }
            let Some(key_node) = attribute.child(0) else {
                continue;
            };
            let address = format!("local.{}", self.text(key_node));
            if let Some(owner) = self.address_id(&address) {
                self.collect_references(attribute, &owner, "references");
            }
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
            let line = node.start_position().row + 1;
            let target = self.ensure_reference_target(&address, line);
            self.add_edge(owner, &target, relation, line);
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
            "var" | "local" | "module" if !attributes.is_empty() => {
                Some(format!("{head}.{}", attributes[0]))
            }
            "data" if attributes.len() >= 2 => {
                Some(format!("data.{}.{}", attributes[0], attributes[1]))
            }
            _ if !attributes.is_empty() => Some(format!("{head}.{}", attributes[0])),
            _ => None,
        }
    }

    fn ensure_reference_target(&mut self, address: &str, line: usize) -> String {
        if let Some(id) = self.address_id(address) {
            return id;
        }
        let kind = if address.starts_with("module.") {
            "package"
        } else if address.starts_with("var.")
            || address.starts_with("local.")
            || address.starts_with("output.")
            || address.starts_with("provider.")
        {
            "config_key"
        } else {
            "resource"
        };
        let uri = (kind == "resource").then(|| format!("terraform:{address}"));
        self.add_address_node(address, address, kind, line, uri)
    }

    fn add_file(&mut self, id: &str, name: &str) {
        let attributes = self.base_attributes("file", name, name, 1);
        self.push_node(id.to_owned(), attributes);
    }

    fn add_address_node(
        &mut self,
        address: &str,
        label: &str,
        kind: &str,
        line: usize,
        uri: Option<String>,
    ) -> String {
        if let Some(id) = self.address_id(address) {
            return id;
        }
        let id = make_id(&["terraform", &self.scope, address]);
        let mut attributes = self.base_attributes(kind, label, address, line);
        match kind {
            "resource" => {
                attributes.insert("file_type".into(), Value::String("concept".into()));
                if let Some(uri) = uri {
                    attributes.insert("uri".into(), Value::String(uri));
                }
            }
            "config_key" => {
                attributes.insert("format".into(), Value::String("terraform".into()));
                attributes.insert("key_path".into(), Value::String(address.to_owned()));
            }
            _ => {}
        }
        self.push_node(id.clone(), attributes);
        self.addresses.insert(address.to_owned(), id.clone());
        self.add_edge(&self.file_id.clone(), &id, "contains", line);
        id
    }

    fn address_id(&self, address: &str) -> Option<String> {
        self.addresses.get(address).cloned()
    }

    fn base_attributes(
        &self,
        kind: &str,
        name: &str,
        qualified_name: &str,
        line: usize,
    ) -> Map<String, Value> {
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(name.to_owned()));
        attributes.insert("name".into(), Value::String(name.to_owned()));
        attributes.insert(
            "qualified_name".into(),
            Value::String(qualified_name.to_owned()),
        );
        attributes.insert("symbol_kind".into(), Value::String(kind.to_owned()));
        attributes.insert("file_type".into(), Value::String("code".into()));
        attributes.insert("language".into(), Value::String("terraform".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{line}")));
        attributes.insert("line_start".into(), Value::from(line));
        attributes.insert("line_end".into(), Value::from(line));
        attributes.insert("_origin".into(), Value::String("config".into()));
        attributes.insert(
            "extractor".into(),
            Value::String("compass.languages.terraform".into()),
        );
        attributes
    }

    fn push_node(&mut self, id: String, attributes: Map<String, Value>) {
        if self.seen_nodes.insert(id.clone()) {
            self.extraction.nodes.push(NodeRecord { id, attributes });
        }
    }

    fn add_edge(&mut self, source: &str, target: &str, relation: &str, line: usize) {
        if source == target
            || !self.seen_edges.insert((
                source.to_owned(),
                target.to_owned(),
                relation.to_owned(),
                line,
            ))
        {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{line}")));
        attributes.insert("weight".into(), json!(1.0));
        attributes.insert("_origin".into(), Value::String("config".into()));
        attributes.insert(
            "extractor".into(),
            Value::String("compass.languages.terraform".into()),
        );
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.source).unwrap_or_default()
    }
}

fn named_blocks(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.kind() == "block")
        .collect()
}

fn first_child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}
