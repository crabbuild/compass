use std::collections::HashSet;
use std::path::Path;

use serde_json::{Map, Value};
use trail_model::{EdgeRecord, NodeRecord};
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
    scope_bodies: Vec<(String, Node<'tree>)>,
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
            scope_bodies: Vec::new(),
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
        for (scope, body) in self.scope_bodies.clone() {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if !matches!(
                    child.kind(),
                    "subroutine_statement"
                        | "function_statement"
                        | "program_statement"
                        | "module_statement"
                ) {
                    self.walk_calls(child, &scope);
                }
            }
        }
        self.extraction
    }

    fn walk(&mut self, node: Node<'tree>, scope: &str) {
        match node.kind() {
            "program" => {
                if let Some(statement) = direct_child(node, "program_statement")
                    && let Some(name) = fortran_name(statement, self.source)
                {
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &name, line(node));
                    self.add_edge(&self.file_id.clone(), &id, "defines", line(node), None);
                    self.scope_bodies.push((id.clone(), node));
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        self.walk(child, &id);
                    }
                }
                return;
            }
            "module" => {
                if let Some(statement) = direct_child(node, "module_statement")
                    && let Some(name) = fortran_name(statement, self.source)
                {
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &name, line(node));
                    self.add_edge(&self.file_id.clone(), &id, "defines", line(node), None);
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        self.walk(child, &id);
                    }
                }
                return;
            }
            "internal_procedures" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.walk(child, scope);
                }
                return;
            }
            "derived_type_definition" => {
                if let Some(statement) = direct_child(node, "derived_type_statement")
                    && let Some(name_node) = direct_child(statement, "type_name")
                {
                    let name = self.text(name_node).to_ascii_lowercase();
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &name, line(node));
                    self.add_edge(scope, &id, "defines", line(node), None);
                }
                return;
            }
            "subroutine" => {
                self.add_procedure(node, scope, false);
                return;
            }
            "function" => {
                self.add_procedure(node, scope, true);
                return;
            }
            "use_statement" => {
                let mut cursor = node.walk();
                if let Some(name_node) = node
                    .children(&mut cursor)
                    .find(|child| matches!(child.kind(), "module_name" | "name" | "identifier"))
                {
                    let name = self.text(name_node).to_ascii_lowercase();
                    let id = make_id(&[&name]);
                    self.add_node(&id, &name, line(node));
                    self.add_edge(scope, &id, "imports", line(node), Some("use"));
                }
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, scope);
        }
    }

    fn add_procedure(&mut self, node: Node<'tree>, scope: &str, is_function: bool) {
        let statement_kind = if is_function {
            "function_statement"
        } else {
            "subroutine_statement"
        };
        let Some(statement) = direct_child(node, statement_kind) else {
            return;
        };
        let Some(name) = fortran_name(statement, self.source) else {
            return;
        };
        let id = make_id(&[&self.stem, &name]);
        self.add_node(&id, &format!("{name}()"), line(node));
        self.add_edge(scope, &id, "defines", line(node), None);
        self.scope_bodies.push((id.clone(), node));
        self.add_signature_references(node, &id, is_function);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, &id);
        }
    }

    fn add_signature_references(&mut self, node: Node<'tree>, function: &str, is_function: bool) {
        let statement_kind = if is_function {
            "function_statement"
        } else {
            "subroutine_statement"
        };
        let Some(statement) = direct_child(node, statement_kind) else {
            return;
        };
        let parameters: HashSet<String> = direct_child(statement, "parameters")
            .map(|parameters| {
                direct_children(parameters, "identifier")
                    .into_iter()
                    .map(|identifier| self.text(identifier).to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        let result = if is_function {
            direct_child(statement, "function_result")
                .and_then(|result| direct_child(result, "identifier"))
                .map(|identifier| self.text(identifier).to_ascii_lowercase())
                .or_else(|| fortran_name(statement, self.source))
        } else {
            None
        };
        for declaration in direct_children(node, "variable_declaration") {
            let Some(derived) = direct_child(declaration, "derived_type") else {
                continue;
            };
            let Some(type_node) = direct_child(derived, "type_name") else {
                continue;
            };
            let type_name = self.text(type_node).to_ascii_lowercase();
            for variable in direct_children(declaration, "identifier") {
                let name = self.text(variable).to_ascii_lowercase();
                let context = if parameters.contains(&name) {
                    Some("parameter_type")
                } else if is_function && result.as_deref() == Some(name.as_str()) {
                    Some("return_type")
                } else {
                    None
                };
                if let Some(context) = context {
                    let target = self.ensure_named(&type_name);
                    if target != function {
                        self.add_edge(
                            function,
                            &target,
                            "references",
                            line(variable),
                            Some(context),
                        );
                    }
                }
            }
        }
    }

    fn walk_calls(&mut self, node: Node<'tree>, scope: &str) {
        if matches!(
            node.kind(),
            "subroutine" | "function" | "module" | "program" | "internal_procedures"
        ) {
            return;
        }
        if node.kind() == "subroutine_call" {
            if let Some(name_node) = direct_child(node, "identifier") {
                let name = self.text(name_node).to_ascii_lowercase();
                self.add_edge(
                    scope,
                    &make_id(&[&self.stem, &name]),
                    "calls",
                    line(node),
                    Some("call"),
                );
            }
        } else if node.kind() == "call_expression"
            && let Some(name_node) = direct_child(node, "identifier")
        {
            let name = self.text(name_node).to_ascii_lowercase();
            let target = make_id(&[&self.stem, &name]);
            if self.seen.contains(&target) && target != scope {
                self.add_edge(scope, &target, "calls", line(node), Some("call"));
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, scope);
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

fn fortran_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| matches!(child.kind(), "name" | "identifier"))
        .map(|name| text(name, source).to_ascii_lowercase())
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
