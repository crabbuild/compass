use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::{Extraction, RawCall, file_stem, make_id};

const SKIP_CALLS: &[&str] = &[
    "using",
    "return",
    "if",
    "else",
    "elseif",
    "foreach",
    "for",
    "while",
    "do",
    "switch",
    "try",
    "catch",
    "finally",
    "throw",
    "break",
    "continue",
    "exit",
    "param",
    "begin",
    "process",
    "end",
    "import-module",
];

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("psd1"))
    {
        return extract_manifest(path, source, root);
    }
    ScriptState::new(path, source).run(root)
}

struct ScriptState<'source, 'tree> {
    path: &'source Path,
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
    function_bodies: Vec<(String, Node<'tree>)>,
}

impl<'source, 'tree> ScriptState<'source, 'tree> {
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
                                .to_ascii_lowercase(),
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
                    || matches!(
                        edge.attributes.get("relation").and_then(Value::as_str),
                        Some("imports_from" | "imports")
                    ))
        });
        self.extraction
    }

    fn walk(&mut self, node: Node<'tree>, parent_class: Option<&str>) {
        match node.kind() {
            "function_statement" => {
                let Some(name_node) = direct_child(node, "function_name") else {
                    return;
                };
                let name = self.text(name_node).to_owned();
                let at = line(node);
                let id = make_id(&[&self.stem, &name]);
                self.add_node(&id, &format!("{name}()"), at);
                self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
                if let Some(body) = script_block_body(node) {
                    self.function_bodies.push((id, body));
                    self.walk(body, parent_class);
                }
                return;
            }
            "class_statement" => {
                let Some(name_node) = direct_child(node, "simple_name") else {
                    return;
                };
                let name = self.text(name_node).to_owned();
                let at = line(node);
                let id = make_id(&[&self.stem, &name]);
                self.add_node(&id, &name, at);
                self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
                let mut colon = false;
                let mut base_index = 0;
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == ":" {
                        colon = true;
                    } else if colon && child.kind() == "simple_name" {
                        let base_name = self.text(child).to_owned();
                        let target = self.ensure_named(&base_name);
                        if target != id {
                            let relation = if base_index == 0 {
                                "inherits"
                            } else {
                                "implements"
                            };
                            self.add_edge(&id, &target, relation, at, None);
                        }
                        base_index += 1;
                    }
                }
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.walk(child, Some(&id));
                }
                return;
            }
            "class_property_definition" => {
                if let Some(class) = parent_class
                    && let Some(name) = type_name(node, self.source)
                {
                    let target = self.ensure_named(&name);
                    if target != class {
                        self.add_edge(class, &target, "references", line(node), Some("field"));
                    }
                }
                return;
            }
            "class_method_definition" => {
                let Some(name_node) = direct_child(node, "simple_name") else {
                    return;
                };
                let name = self.text(name_node).to_owned();
                let at = line(node);
                let (id, parent, relation, label) = if let Some(class) = parent_class {
                    (
                        make_id(&[class, &name]),
                        class.to_owned(),
                        "method",
                        format!(".{name}()"),
                    )
                } else {
                    (
                        make_id(&[&self.stem, &name]),
                        self.file_id.clone(),
                        "contains",
                        format!("{name}()"),
                    )
                };
                self.add_node(&id, &label, at);
                self.add_edge(&parent, &id, relation, at, None);
                if let Some(name) = direct_children(node, "type_literal")
                    .first()
                    .and_then(|literal| type_name(*literal, self.source))
                {
                    let target = self.ensure_named(&name);
                    if target != id {
                        self.add_edge(&id, &target, "references", at, Some("return_type"));
                    }
                }
                if let Some(parameters) = direct_child(node, "class_method_parameter_list") {
                    for parameter in direct_children(parameters, "class_method_parameter") {
                        if let Some(name) = type_name(parameter, self.source) {
                            let target = self.ensure_named(&name);
                            if target != id {
                                self.add_edge(
                                    &id,
                                    &target,
                                    "references",
                                    line(parameter),
                                    Some("parameter_type"),
                                );
                            }
                        }
                    }
                }
                if let Some(body) = script_block_body(node) {
                    self.function_bodies.push((id, body));
                }
                return;
            }
            "command" => {
                self.add_command(node);
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, parent_class);
        }
    }

    fn add_command(&mut self, node: Node<'tree>) {
        if let Some(operator) = direct_child(node, "command_invokation_operator")
            && self.text(operator).trim() == "."
        {
            if let Some(expression) = direct_child(node, "command_name_expr")
                && let Some(name_node) = direct_child(expression, "command_name")
            {
                let name = module_name(self.text(name_node));
                if !name.is_empty() {
                    self.add_edge(
                        &self.file_id.clone(),
                        &make_id(&[&name]),
                        "imports_from",
                        line(node),
                        None,
                    );
                }
            }
            return;
        }
        let Some(name_node) = direct_child(node, "command_name") else {
            return;
        };
        let command = self.text(name_node).to_ascii_lowercase();
        if command == "using" {
            let mut tokens = Vec::new();
            if let Some(elements) = direct_child(node, "command_elements") {
                for token in direct_children(elements, "generic_token") {
                    let token = self.text(token);
                    if !matches!(
                        token.to_ascii_lowercase().as_str(),
                        "namespace" | "module" | "assembly"
                    ) {
                        tokens.push(token);
                    }
                }
            }
            if let Some(token) = tokens.last() {
                let name = token.rsplit('.').next().unwrap_or_default();
                self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[name]),
                    "imports_from",
                    line(node),
                    None,
                );
            }
        } else if command == "import-module" {
            let mut name = None;
            let mut expect_name = false;
            if let Some(elements) = direct_child(node, "command_elements") {
                let mut cursor = elements.walk();
                for element in elements.children(&mut cursor) {
                    if element.kind() == "command_parameter" {
                        let parameter = self
                            .text(element)
                            .trim_start_matches('-')
                            .to_ascii_lowercase();
                        expect_name = matches!(parameter.as_str(), "name" | "n");
                    } else if element.kind() == "generic_token" && (name.is_none() || expect_name) {
                        name = Some(self.text(element));
                        expect_name = false;
                    }
                }
            }
            if let Some(raw) = name {
                let name = module_name(raw);
                if !name.is_empty() {
                    self.add_edge(
                        &self.file_id.clone(),
                        &make_id(&[&name]),
                        "imports_from",
                        line(node),
                        None,
                    );
                }
            }
        }
    }

    fn walk_calls(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        labels: &HashMap<String, String>,
        seen: &mut HashSet<(String, String)>,
    ) {
        if matches!(node.kind(), "function_statement" | "class_statement") {
            return;
        }
        if node.kind() == "command"
            && let Some(name_node) = direct_child(node, "command_name")
        {
            let command = self.text(name_node).to_owned();
            let lower = command.to_ascii_lowercase();
            if !SKIP_CALLS.contains(&lower.as_str()) {
                if let Some(target) = labels
                    .get(&lower)
                    .filter(|target| target.as_str() != caller)
                {
                    if seen.insert((caller.to_owned(), target.clone())) {
                        self.add_edge(caller, target, "calls", line(node), None);
                    }
                } else if !command.is_empty() {
                    self.extraction.raw_calls_mut().push(RawCall {
                        caller_nid: caller.to_owned(),
                        callee: command,
                        is_member_call: false,
                        source_file: self.source_file.clone(),
                        source_location: format!("L{}", line(node)),
                        receiver: None,
                        receiver_type: None,
                        lang: None,
                    });
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, labels, seen);
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
        node.utf8_text(self.source).unwrap_or_default()
    }
}

fn extract_manifest(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut extraction = Extraction::default();
    let mut attributes = Map::new();
    attributes.insert(
        "label".into(),
        Value::String(
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_owned(),
        ),
    );
    attributes.insert("file_type".into(), Value::String("code".into()));
    attributes.insert("source_file".into(), Value::String(source_file.clone()));
    attributes.insert("source_location".into(), Value::String("L1".into()));
    extraction.nodes.push(NodeRecord {
        id: file_id.clone(),
        attributes,
    });
    walk_manifest(root, source, &source_file, &file_id, &mut extraction.edges);
    extraction
}

fn walk_manifest(
    node: Node<'_>,
    source: &[u8],
    source_file: &str,
    file_id: &str,
    edges: &mut Vec<EdgeRecord>,
) {
    if node.kind() != "hash_entry" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk_manifest(child, source, source_file, file_id, edges);
        }
        return;
    }
    let Some(key_node) = direct_child(node, "key_expression") else {
        return;
    };
    let key = text(key_node, source).trim();
    if !matches!(key, "RootModule" | "NestedModules" | "RequiredModules") {
        return;
    }
    let Some(value) = direct_child(node, "pipeline") else {
        return;
    };
    let at = line(node);
    let values = if key == "RequiredModules" {
        required_modules(value, source)
    } else {
        string_literals(value)
            .into_iter()
            .map(|node| unquote(text(node, source)))
            .collect()
    };
    for raw in values {
        let name = module_name(&raw);
        if name.is_empty() {
            continue;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String("imports_from".into()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert("source_file".into(), Value::String(source_file.to_owned()));
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("weight".into(), Value::from(1.0));
        attributes.insert("context".into(), Value::String("import".into()));
        edges.push(EdgeRecord {
            source: file_id.to_owned(),
            target: make_id(&[&name]),
            attributes,
        });
    }
}

fn required_modules(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let mut handled = HashSet::new();
    let mut named = Vec::new();
    collect_module_entries(node, source, &mut handled, &mut named);
    let mut direct: Vec<_> = string_literals(node)
        .into_iter()
        .filter(|literal| !handled.contains(&literal.start_byte()))
        .map(|literal| unquote(text(literal, source)))
        .collect();
    direct.extend(named);
    direct
}

fn collect_module_entries(
    node: Node<'_>,
    source: &[u8],
    handled: &mut HashSet<usize>,
    named: &mut Vec<String>,
) {
    if node.kind() == "hash_entry" {
        let key = direct_child(node, "key_expression")
            .map(|node| text(node, source).trim())
            .unwrap_or_default();
        for pipeline in direct_children(node, "pipeline") {
            let literals = string_literals(pipeline);
            handled.extend(literals.iter().map(Node::start_byte));
            if key == "ModuleName" {
                named.extend(
                    literals
                        .into_iter()
                        .map(|literal| unquote(text(literal, source))),
                );
            }
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_module_entries(child, source, handled, named);
    }
}

fn string_literals<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    fn collect<'tree>(node: Node<'tree>, values: &mut Vec<Node<'tree>>) {
        if node.kind() == "string_literal" {
            values.push(node);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect(child, values);
        }
    }
    let mut values = Vec::new();
    collect(node, &mut values);
    values
}

fn script_block_body(node: Node<'_>) -> Option<Node<'_>> {
    direct_child(node, "script_block")
        .and_then(|block| direct_child(block, "script_block_body").or(Some(block)))
}

fn type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    first_descendant(node, "type_identifier").map(|node| text(node, source).to_owned())
}

fn first_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = first_descendant(child, kind) {
            return Some(found);
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

fn module_name(raw: &str) -> String {
    let normalized = raw.replace('\\', "/");
    let name = normalized
        .trim_start_matches(['.', '/'])
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .trim();
    name.rsplit_once('.')
        .map_or(name, |(stem, _)| stem)
        .trim()
        .to_owned()
}

fn unquote(raw: &str) -> String {
    raw.trim_matches(['\'', '"']).to_owned()
}

fn text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or_default()
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
