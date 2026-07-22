use std::collections::HashSet;
use std::path::{Path, PathBuf};

use compass_model::{EdgeRecord, NodeRecord};
use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::{Extraction, make_id};

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    BashState::new(path, source).run(root)
}

struct BashState<'source, 'tree> {
    path: &'source Path,
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    entry_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
    defined_functions: HashSet<String>,
    function_bodies: Vec<(String, Node<'tree>)>,
}

impl<'source, 'tree> BashState<'source, 'tree> {
    fn new(path: &'source Path, source: &'source [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let stem = crate::file_stem(path);
        let file_id = make_id(&[&source_file]);
        let entry_id = format!("{file_id}__entry");
        Self {
            path,
            source,
            source_file,
            stem,
            file_id,
            entry_id,
            extraction: Extraction {
                raw_calls: None,
                ..Extraction::default()
            },
            seen: HashSet::new(),
            defined_functions: HashSet::new(),
            function_bodies: Vec::new(),
        }
    }

    fn run(mut self, root: Node<'tree>) -> Extraction {
        let label = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        self.add_node(&self.file_id.clone(), label, 1, "file");
        self.add_node(
            &self.entry_id.clone(),
            &format!("{label} script"),
            1,
            "bash_entrypoint",
        );
        self.add_edge(
            &self.file_id.clone(),
            &self.entry_id.clone(),
            "contains",
            1,
            None,
        );
        self.prescan(root);
        self.walk(root, &self.file_id.clone());
        let mut top_seen = HashSet::new();
        self.walk_calls(root, &self.entry_id.clone(), &mut top_seen);
        for (function, body) in self.function_bodies.clone() {
            self.walk_calls(body, &function, &mut HashSet::new());
        }
        self.extraction
    }

    fn prescan(&mut self, node: Node<'tree>) {
        if node.kind() == "function_definition"
            && let Some(name) = self.function_name(node)
        {
            self.defined_functions.insert(name);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.prescan(child);
        }
    }

    fn walk(&mut self, node: Node<'tree>, parent: &str) {
        match node.kind() {
            "function_definition" => {
                if let Some(name) = self.function_name(node) {
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &format!("{name}()"), line(node), "bash_function");
                    self.add_edge(parent, &id, "defines", line(node), None);
                    self.defined_functions.insert(name);
                    let mut cursor = node.walk();
                    if let Some(body) = node
                        .children(&mut cursor)
                        .find(|child| child.kind() == "compound_statement")
                    {
                        self.function_bodies.push((id.clone(), body));
                        self.walk(body, &id);
                    }
                }
                return;
            }
            "command" => {
                self.add_command(node, parent);
                return;
            }
            "declaration_command" => {
                self.add_declaration(node);
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, parent);
        }
    }

    fn add_command(&mut self, node: Node<'tree>, parent: &str) {
        if inside_expansion(node) {
            return;
        }
        let Some(name_node) = self.command_name_node(node) else {
            return;
        };
        let command = self.literal(name_node);
        let mut cursor = node.walk();
        let arguments: Vec<_> = node
            .children(&mut cursor)
            .filter(|child| {
                *child != name_node && matches!(child.kind(), "word" | "string" | "concatenation")
            })
            .collect();
        if command
            .as_deref()
            .is_some_and(|name| matches!(name, "source" | "."))
            && command
                .as_deref()
                .is_none_or(|name| !self.defined_functions.contains(name))
        {
            let Some(argument) = arguments.first() else {
                return;
            };
            let raw = self
                .text(*argument)
                .trim()
                .trim_matches(['\'', '"'])
                .to_owned();
            if raw.starts_with(['.', '/']) {
                if let Some(resolved) = canonical_candidate(self.path, &raw)
                    && resolved.exists()
                {
                    self.add_edge(
                        &self.file_id.clone(),
                        &make_id(&[&resolved.to_string_lossy()]),
                        "imports_from",
                        line(node),
                        Some("import"),
                    );
                }
            } else if !raw.is_empty() {
                self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[&raw]),
                    "imports",
                    line(node),
                    Some("import"),
                );
            }
            return;
        }
        let Some(command) = command.filter(|name| !self.defined_functions.contains(name)) else {
            return;
        };
        let raw = if command.ends_with(".sh") {
            Some(command)
        } else if matches!(command.as_str(), "bash" | "sh" | "zsh" | "ksh" | "dash") {
            arguments
                .first()
                .and_then(|argument| self.literal(*argument))
        } else {
            None
        };
        let Some(raw) = raw.filter(|raw| raw.ends_with(".sh")) else {
            return;
        };
        let Some(resolved) = canonical_candidate(self.path, &raw).filter(|path| path.is_file())
        else {
            return;
        };
        let caller = if parent == self.file_id {
            self.entry_id.clone()
        } else {
            parent.to_owned()
        };
        self.add_edge(
            &caller,
            &format!("{}__entry", make_id(&[&resolved.to_string_lossy()])),
            "calls",
            line(node),
            Some("script_invocation"),
        );
    }

    fn add_declaration(&mut self, node: Node<'tree>) {
        if node
            .parent()
            .is_none_or(|parent| parent.kind() != "program")
        {
            return;
        }
        let mut cursor = node.walk();
        for assignment in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "variable_assignment")
        {
            let Some(name) = assignment.child_by_field_name("name") else {
                continue;
            };
            let name = self.text(name).trim().to_owned();
            if name.is_empty() {
                continue;
            }
            let id = make_id(&[&self.stem, &name]);
            self.add_node(&id, &name, line(assignment), "code");
            self.add_edge(
                &self.file_id.clone(),
                &id,
                "defines",
                line(assignment),
                None,
            );
        }
    }

    fn walk_calls(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        seen: &mut HashSet<(String, String)>,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "function_definition" {
                continue;
            }
            if child.kind() == "command"
                && !inside_expansion(child)
                && let Some(name_node) = self.command_name_node(child)
                && let Some(name) = self.literal(name_node)
                && self.defined_functions.contains(&name)
            {
                let target = make_id(&[&self.stem, &name]);
                if seen.insert((caller.to_owned(), target.clone())) {
                    self.add_edge(caller, &target, "calls", line(child), Some("call"));
                }
            }
            self.walk_calls(child, caller, seen);
        }
    }

    fn function_name(&self, node: Node<'tree>) -> Option<String> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(|child| child.kind() == "word")
            .and_then(|child| self.literal(child))
    }

    fn command_name_node(&self, node: Node<'tree>) -> Option<Node<'tree>> {
        node.child_by_field_name("name").or_else(|| node.child(0))
    }

    fn literal(&self, node: Node<'_>) -> Option<String> {
        let mut raw = self.text(node).trim().to_owned();
        if raw.len() >= 2
            && matches!(raw.as_bytes().first(), Some(b'\'' | b'"'))
            && raw.as_bytes().first() == raw.as_bytes().last()
        {
            raw = raw[1..raw.len() - 1].to_owned();
        }
        (!raw.is_empty()
            && !["$", "`", "$(", "<(", ">", "|", ";", "&"]
                .iter()
                .any(|token| raw.contains(token)))
        .then_some(raw)
    }

    fn text(&self, node: Node<'_>) -> String {
        node.utf8_text(self.source).unwrap_or_default().to_owned()
    }

    fn add_node(&mut self, id: &str, label: &str, at: usize, kind: &str) {
        if id.is_empty() || !self.seen.insert(id.to_owned()) {
            return;
        }
        let mut metadata = Map::new();
        metadata.insert("language".into(), Value::String("bash".into()));
        metadata.insert("kind".into(), Value::String(kind.to_owned()));
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(label.to_owned()));
        attributes.insert("file_type".into(), Value::String("code".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("metadata".into(), Value::Object(metadata));
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
        if source.is_empty() || target.is_empty() || source == target {
            return;
        }
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
}

fn inside_expansion(node: Node<'_>) -> bool {
    let mut parent = node.parent();
    while let Some(node) = parent {
        if matches!(node.kind(), "command_substitution" | "process_substitution") {
            return true;
        }
        parent = node.parent();
    }
    false
}

fn canonical_candidate(path: &Path, raw: &str) -> Option<PathBuf> {
    let candidate = path.parent()?.join(raw);
    candidate.canonicalize().ok()
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
