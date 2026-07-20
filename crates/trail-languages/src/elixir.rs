use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::builtins::LANGUAGE_BUILTIN_GLOBALS;
use crate::{Extraction, RawCall, file_stem, make_id};

const IMPORT_KEYWORDS: &[&str] = &["alias", "import", "require", "use"];
const SKIP_KEYWORDS: &[&str] = &[
    "def",
    "defp",
    "defmodule",
    "defmacro",
    "defmacrop",
    "defstruct",
    "defprotocol",
    "defimpl",
    "defguard",
    "alias",
    "import",
    "require",
    "use",
    "if",
    "unless",
    "case",
    "cond",
    "with",
    "for",
];
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
            extraction: Extraction::default(),
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
        self.walk(root, None);

        let labels: HashMap<String, String> = self
            .extraction
            .nodes
            .iter()
            .filter_map(|node| {
                node.attributes
                    .get("label")
                    .and_then(Value::as_str)
                    .map(|label| {
                        (
                            label
                                .trim_matches(['(', ')'])
                                .trim_start_matches('.')
                                .to_owned(),
                            node.id.clone(),
                        )
                    })
            })
            .collect();
        let mut seen_calls = HashSet::new();
        for (caller, body) in self.function_bodies.clone() {
            self.walk_calls(body, &caller, &labels, &mut seen_calls);
        }
        self.extraction.edges.retain(|edge| {
            self.seen.contains(&edge.source)
                && (self.seen.contains(&edge.target)
                    || edge.attributes.get("relation").and_then(Value::as_str) == Some("imports"))
        });
        self.extraction
            .extensions
            .insert("input_tokens".to_owned(), json!(0));
        self.extraction
            .extensions
            .insert("output_tokens".to_owned(), json!(0));
        self.extraction
    }

    fn walk(&mut self, node: Node<'tree>, parent_module: Option<&str>) {
        if node.kind() != "call" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk(child, parent_module);
            }
            return;
        }
        let identifier = direct_child(node, "identifier");
        let arguments = direct_child(node, "arguments");
        let do_block = direct_child(node, "do_block");
        let Some(identifier) = identifier else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk(child, parent_module);
            }
            return;
        };
        let keyword = self.text(identifier).to_owned();
        let at = line(node);
        if keyword == "defmodule" {
            let Some(name) = arguments.and_then(|arguments| alias_text(arguments, self.source))
            else {
                return;
            };
            let id = make_id(&[&self.stem, &name]);
            self.add_node(&id, &name, at);
            self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
            if let Some(block) = do_block {
                let mut cursor = block.walk();
                for child in block.children(&mut cursor) {
                    self.walk(child, Some(&id));
                }
            }
            return;
        }
        if matches!(keyword.as_str(), "def" | "defp") {
            let Some(name) = arguments.and_then(|arguments| function_name(arguments, self.source))
            else {
                return;
            };
            let container = parent_module.unwrap_or(&self.file_id).to_owned();
            let id = make_id(&[&container, &name]);
            self.add_node(&id, &format!("{name}()"), at);
            self.add_edge(
                &container,
                &id,
                if parent_module.is_some() {
                    "method"
                } else {
                    "contains"
                },
                at,
                None,
            );
            if let Some(block) = do_block {
                self.function_bodies.push((id, block));
            }
            return;
        }
        if IMPORT_KEYWORDS.contains(&keyword.as_str())
            && let Some(arguments) = arguments
        {
            for module in alias_modules(arguments, self.source) {
                self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[&module]),
                    "imports",
                    at,
                    Some("import"),
                );
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, parent_module);
        }
    }

    fn walk_calls(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        labels: &HashMap<String, String>,
        seen: &mut HashSet<(String, String)>,
    ) {
        if node.kind() != "call" {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk_calls(child, caller, labels, seen);
            }
            return;
        }
        if let Some(keyword) = direct_child(node, "identifier")
            .map(|identifier| self.text(identifier))
            .filter(|keyword| SKIP_KEYWORDS.contains(keyword))
        {
            let _ = keyword;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk_calls(child, caller, labels, seen);
            }
            return;
        }
        let mut callee = None;
        let mut member = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "dot" {
                member = true;
                callee = self
                    .text(child)
                    .trim_end_matches('.')
                    .rsplit('.')
                    .next()
                    .map(str::to_owned);
                break;
            }
            if child.kind() == "identifier" {
                callee = Some(self.text(child).to_owned());
                break;
            }
        }
        if let Some(callee) =
            callee.filter(|callee| !LANGUAGE_BUILTIN_GLOBALS.contains(&callee.as_str()))
        {
            if let Some(target) = labels
                .get(&callee)
                .filter(|target| target.as_str() != caller)
            {
                if seen.insert((caller.to_owned(), target.clone())) {
                    self.add_edge(caller, target, "calls", line(node), Some("call"));
                }
            } else {
                self.extraction.raw_calls_mut().push(RawCall {
                    caller_nid: caller.to_owned(),
                    callee,
                    is_member_call: member,
                    source_file: self.source_file.clone(),
                    source_location: format!("L{}", line(node)),
                    receiver: None,
                    receiver_type: None,
                    lang: None,
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, labels, seen);
        }
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

fn alias_text(node: Node<'_>, source: &[u8]) -> Option<String> {
    direct_child(node, "alias").map(|alias| text(alias, source).to_owned())
}

fn alias_modules(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "alias" {
            return vec![text(child, source).to_owned()];
        }
        if child.kind() == "dot" {
            let mut base = None;
            let mut tuple = None;
            let mut dot_cursor = child.walk();
            for part in child.children(&mut dot_cursor) {
                if part.kind() == "alias" && base.is_none() {
                    base = Some(text(part, source));
                } else if part.kind() == "tuple" {
                    tuple = Some(part);
                }
            }
            if let (Some(base), Some(tuple)) = (base, tuple) {
                let members = direct_children(tuple, "alias");
                if !members.is_empty() {
                    return members
                        .into_iter()
                        .map(|member| format!("{base}.{}", text(member, source)))
                        .collect();
                }
            }
            return vec![text(child, source).to_owned()];
        }
    }
    Vec::new()
}

fn function_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "call"
            && let Some(identifier) = direct_child(child, "identifier")
        {
            return Some(text(identifier, source).to_owned());
        }
        if child.kind() == "identifier" {
            return Some(text(child, source).to_owned());
        }
    }
    None
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
