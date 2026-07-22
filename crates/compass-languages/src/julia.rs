use std::collections::HashSet;
use std::path::Path;

use compass_model::{EdgeRecord, NodeRecord};
use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::{Extraction, file_stem, make_id};

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    State::new(path, source).run(root)
}

struct State<'source, 'tree> {
    path: &'source Path,
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
    function_bodies: Vec<(String, Node<'tree>)>,
}

impl<'source, 'tree> State<'source, 'tree> {
    fn new(path: &'source Path, source: &'source [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        Self {
            path,
            source,
            stem: file_stem(path),
            file_id: make_id(&[&source_file]),
            source_file,
            extraction: Extraction {
                raw_calls: None,
                ..Extraction::default()
            },
            seen: HashSet::new(),
            function_bodies: Vec::new(),
        }
    }

    fn run(mut self, root: Node<'tree>) -> Extraction {
        let label = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        self.add_node(&self.file_id.clone(), label, 1);
        self.walk(root, &self.file_id.clone());
        for (function, body) in self.function_bodies.clone() {
            if body.kind() == "function_definition" {
                let mut cursor = body.walk();
                for child in body.children(&mut cursor) {
                    if child.kind() != "signature" {
                        self.walk_calls(child, &function);
                    }
                }
            } else {
                self.walk_calls(body, &function);
            }
        }
        self.extraction
    }

    fn walk(&mut self, node: Node<'tree>, scope: &str) {
        match node.kind() {
            "module_definition" => {
                let Some(name_node) = direct_child(node, "identifier") else {
                    return;
                };
                let name = self.text(name_node).to_owned();
                let id = make_id(&[&self.stem, &name]);
                self.add_node(&id, &name, line(node));
                self.add_edge(&self.file_id.clone(), &id, "defines", line(node), None);
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.walk(child, &id);
                }
                return;
            }
            "struct_definition" => {
                self.add_struct(node, scope);
                return;
            }
            "abstract_definition" => {
                if let Some(head) = direct_child(node, "type_head")
                    && let Some(name_node) = direct_child(head, "identifier")
                {
                    let name = self.text(name_node).to_owned();
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &name, line(node));
                    self.add_edge(scope, &id, "defines", line(node), None);
                }
                return;
            }
            "function_definition" => {
                if let Some(signature) = direct_child(node, "signature")
                    && let Some(name) = function_name(signature, self.source)
                {
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &format!("{name}()"), line(node));
                    self.add_edge(scope, &id, "defines", line(node), None);
                    self.function_bodies.push((id, node));
                }
                return;
            }
            "assignment" => {
                if let Some(lhs) = node.child(0)
                    && lhs.kind() == "call_expression"
                    && let Some(callee) =
                        lhs.child(0).filter(|callee| callee.kind() == "identifier")
                {
                    let name = self.text(callee).to_owned();
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &format!("{name}()"), line(node));
                    self.add_edge(scope, &id, "defines", line(node), None);
                    if node.child_count() >= 3
                        && let Some(rhs) = node.child((node.child_count() - 1) as u32)
                    {
                        self.function_bodies.push((id, rhs));
                    }
                }
                return;
            }
            "using_statement" | "import_statement" => {
                self.add_imports(node, scope);
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    fn add_struct(&mut self, node: Node<'tree>, scope: &str) {
        let Some(head) = direct_child(node, "type_head") else {
            return;
        };
        let mut name = None;
        let mut superclass = None;
        if let Some(binary) = direct_child(head, "binary_expression") {
            let identifiers = direct_children(binary, "identifier");
            if let Some(first) = identifiers.first() {
                name = Some(self.text(*first).to_owned());
                if identifiers.len() >= 2 {
                    superclass = identifiers.last().map(|node| self.text(*node).to_owned());
                }
            }
        } else if let Some(identifier) = direct_child(head, "identifier") {
            name = Some(self.text(identifier).to_owned());
        }
        let Some(name) = name else {
            return;
        };
        let id = make_id(&[&self.stem, &name]);
        self.add_node(&id, &name, line(node));
        self.add_edge(scope, &id, "defines", line(node), None);
        if let Some(superclass) = superclass {
            let target = self.ensure_named(&superclass);
            self.add_edge(&id, &target, "inherits", line(node), None);
        }
        let fields = if let Some(block) = direct_child(node, "block") {
            direct_children(block, "typed_expression")
        } else {
            direct_children(node, "typed_expression")
        };
        for field in fields {
            let identifiers = direct_children(field, "identifier");
            if identifiers.len() >= 2 {
                let type_name = self
                    .text(*identifiers.last().unwrap_or(&identifiers[0]))
                    .to_owned();
                let target = self.ensure_named(&type_name);
                self.add_edge(&id, &target, "references", line(field), Some("field"));
            }
        }
    }

    fn add_imports(&mut self, node: Node<'tree>, scope: &str) {
        let at = line(node);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(
                child.kind(),
                "identifier" | "scoped_identifier" | "import_path"
            ) {
                if let Some(name) = import_name(child, self.source) {
                    self.add_import(scope, &name, at);
                }
            } else if child.kind() == "selected_import" {
                let mut selected_cursor = child.walk();
                if let Some(package) = child.children(&mut selected_cursor).find(|part| {
                    matches!(
                        part.kind(),
                        "identifier" | "scoped_identifier" | "import_path"
                    )
                }) && let Some(name) = import_name(package, self.source)
                {
                    self.add_import(scope, &name, at);
                }
            }
        }
    }

    fn add_import(&mut self, scope: &str, name: &str, at: usize) {
        let id = make_id(&[name]);
        self.add_node(&id, name, at);
        self.add_edge(scope, &id, "imports", at, Some("import"));
    }

    fn walk_calls(&mut self, node: Node<'tree>, caller: &str) {
        if matches!(
            node.kind(),
            "function_definition" | "short_function_definition"
        ) {
            return;
        }
        if node.kind() == "call_expression"
            && let Some(callee) = node.child(0)
        {
            let name = if callee.kind() == "identifier" {
                Some(self.text(callee))
            } else if callee.kind() == "field_expression" && callee.child_count() >= 3 {
                callee
                    .child((callee.child_count() - 1) as u32)
                    .map(|method| self.text(method))
            } else {
                None
            };
            if let Some(name) = name {
                self.add_edge(
                    caller,
                    &make_id(&[&self.stem, name]),
                    "calls",
                    line(node),
                    Some("call"),
                );
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller);
        }
    }

    fn ensure_named(&mut self, name: &str) -> String {
        let local = make_id(&[&self.stem, name]);
        if self.seen.contains(&local) {
            return local;
        }
        let id = make_id(&[name]);
        if self.seen.insert(id.clone()) {
            let mut attributes = Map::new();
            attributes.insert("label".into(), Value::String(name.to_owned()));
            attributes.insert("file_type".into(), Value::String("code".into()));
            attributes.insert("source_file".into(), Value::String(String::new()));
            attributes.insert("source_location".into(), Value::String(String::new()));
            attributes.insert(
                "origin_file".into(),
                Value::String(self.source_file.clone()),
            );
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
        }
        id
    }

    fn add_node(&mut self, id: &str, label: &str, at: usize) {
        if !self.seen.insert(id.to_owned()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(label.to_owned()));
        attributes.insert("file_type".into(), Value::String("code".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        self.extraction.nodes.push(NodeRecord {
            id: id.to_owned(),
            attributes,
        });
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        at: usize,
        context: Option<&str>,
    ) {
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("weight".into(), Value::from(1.0));
        if let Some(context) = context {
            attributes.insert("context".into(), Value::String(context.to_owned()));
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn text(&self, node: Node<'_>) -> &str {
        text(node, self.source)
    }
}

fn function_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let call = direct_child(node, "call_expression")?;
    call.child(0)
        .filter(|callee| callee.kind() == "identifier")
        .map(|callee| text(callee, source).to_owned())
}

fn import_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() == "import_path" {
        let identifiers = direct_children(node, "identifier");
        if text(node, source).trim_start().starts_with('.') {
            return identifiers
                .last()
                .map(|identifier| text(*identifier, source).to_owned());
        }
        return (!identifiers.is_empty()).then(|| {
            identifiers
                .into_iter()
                .map(|identifier| text(identifier, source))
                .collect::<Vec<_>>()
                .join(".")
        });
    }
    matches!(node.kind(), "identifier" | "scoped_identifier").then(|| text(node, source).to_owned())
}

fn direct_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn direct_children<'tree>(node: Node<'tree>, kind: &str) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.kind() == kind)
        .collect()
}

fn text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or_default()
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
