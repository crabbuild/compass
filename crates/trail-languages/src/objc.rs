use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::{Extraction, RawCall, file_stem, make_id};

pub(crate) fn mask_annotation_macros(source: &mut [u8]) {
    for marker in [
        b"NS_ASSUME_NONNULL_BEGIN".as_slice(),
        b"NS_ASSUME_NONNULL_END",
    ] {
        let mut start = 0;
        while let Some(offset) = source[start..]
            .windows(marker.len())
            .position(|window| window == marker)
        {
            let absolute = start + offset;
            source[absolute..absolute + marker.len()].fill(b' ');
            start = absolute + marker.len();
        }
    }
}

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = State {
        source,
        source_file: source_file.clone(),
        stem,
        file_id: file_id.clone(),
        extraction: Extraction::default(),
        seen_nodes: HashSet::new(),
        method_bodies: Vec::new(),
        type_table: HashMap::new(),
    };
    state
        .extraction
        .extensions
        .insert("input_tokens".to_owned(), json!(0));
    state
        .extraction
        .extensions
        .insert("output_tokens".to_owned(), json!(0));
    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
    );
    state.walk(root, None);
    state.add_calls();
    if !state.type_table.is_empty() {
        state.extraction.extensions.insert(
            "objc_type_table".to_owned(),
            json!({"path": source_file, "table": state.type_table}),
        );
    }
    state.extraction
}

struct MethodBody<'tree> {
    id: String,
    node: Node<'tree>,
    container: String,
}

struct CallIndex<'a> {
    all_ids: &'a HashSet<String>,
    sibling_ids: &'a HashSet<String>,
    methods: &'a [(String, String)],
}

struct State<'source, 'tree> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    method_bodies: Vec<MethodBody<'tree>>,
    type_table: HashMap<String, String>,
}

impl<'tree> State<'_, 'tree> {
    fn walk(&mut self, node: Node<'tree>, parent: Option<&str>) {
        match node.kind() {
            "preproc_include" => {
                self.add_include(node);
                return;
            }
            "module_import" => {
                self.add_module_import(node);
                return;
            }
            "class_interface" => {
                self.add_interface(node);
                return;
            }
            "class_implementation" => {
                self.add_implementation(node);
                return;
            }
            "protocol_declaration" => {
                self.add_protocol(node);
                return;
            }
            "method_declaration" | "method_definition" => {
                self.add_method(node, parent);
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, parent);
        }
    }

    fn add_include(&mut self, node: Node<'tree>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let target = if child.kind() == "system_lib_string" {
                let raw = self.text(child).trim_matches(['<', '>']);
                let module = raw
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .trim_end_matches(".h");
                (!module.is_empty()).then(|| make_id(&[module]))
            } else if child.kind() == "string_literal" {
                let content = first_descendant(child, "string_content")
                    .map(|content| self.text(content))
                    .unwrap_or_default();
                if content.is_empty() {
                    None
                } else {
                    let resolved = Path::new(&self.source_file)
                        .parent()
                        .and_then(|parent| fs::canonicalize(parent.join(content)).ok())
                        .filter(|candidate| candidate.is_file());
                    Some(resolved.map_or_else(
                        || {
                            let module = content
                                .rsplit('/')
                                .next()
                                .unwrap_or_default()
                                .trim_end_matches(".h");
                            make_id(&[module])
                        },
                        |path| make_id(&[&path.to_string_lossy()]),
                    ))
                }
            } else {
                None
            };
            if let Some(target) = target {
                self.add_edge(
                    &self.file_id.clone(),
                    &target,
                    "imports",
                    line(node),
                    Some("import"),
                );
            }
        }
    }

    fn add_module_import(&mut self, node: Node<'tree>) {
        let Some(path) = node.child_by_field_name("path") else {
            return;
        };
        let module = self.text(path).split('.').next().unwrap_or_default().trim();
        if !module.is_empty() {
            self.add_edge(
                &self.file_id.clone(),
                &make_id(&[module]),
                "imports",
                line(node),
                Some("import"),
            );
        }
    }

    fn add_interface(&mut self, node: Node<'tree>) {
        let identifiers = direct_children(node, "identifier");
        let Some(name_node) = identifiers.first() else {
            return;
        };
        let name = self.text(*name_node).to_owned();
        let id = make_id(&[&self.stem, &name]);
        self.add_node(id.clone(), &name, line(node));
        self.add_edge(&self.file_id.clone(), &id, "contains", line(node), None);
        let mut colon = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == ":" {
                colon = true;
            } else if colon && child.kind() == "identifier" {
                let name = self.text(child).to_owned();
                let target = self.ensure_named(&name);
                self.add_edge(&id, &target, "inherits", line(node), None);
                colon = false;
            } else if child.kind() == "parameterized_arguments" {
                for type_name in direct_children(child, "type_name") {
                    if let Some(type_id) = first_descendant(type_name, "type_identifier") {
                        let name = self.text(type_id).to_owned();
                        let target = self.ensure_named(&name);
                        self.add_edge(&id, &target, "implements", line(node), None);
                    }
                }
            } else if child.kind() == "property_declaration" {
                self.add_property(child, &id);
            } else if child.kind() == "method_declaration" {
                self.add_method(child, Some(&id));
            }
        }
    }

    fn add_property(&mut self, node: Node<'tree>, class_id: &str) {
        for structure in direct_children(node, "struct_declaration") {
            let mut seen = HashSet::new();
            let mut cursor = structure.walk();
            for child in structure.children(&mut cursor) {
                if matches!(child.kind(), "struct_declarator" | ";") {
                    continue;
                }
                let mut identifiers = Vec::new();
                collect_nodes(child, "type_identifier", &mut identifiers);
                for identifier in identifiers {
                    let name = self.text(identifier).to_owned();
                    if seen.insert(name.clone()) {
                        let target = self.ensure_named(&name);
                        self.add_edge(class_id, &target, "references", line(node), Some("field"));
                    }
                }
            }
        }
    }

    fn add_implementation(&mut self, node: Node<'tree>) {
        let Some(name_node) = direct_children(node, "identifier").first().copied() else {
            return;
        };
        let name = self.text(name_node).to_owned();
        let id = make_id(&[&self.stem, &name]);
        if !self.seen_nodes.contains(&id) {
            self.add_node(id.clone(), &name, line(node));
            self.add_edge(&self.file_id.clone(), &id, "contains", line(node), None);
        }
        for definition in direct_children(node, "implementation_definition") {
            let mut cursor = definition.walk();
            for child in definition.children(&mut cursor) {
                self.walk(child, Some(&id));
            }
        }
    }

    fn add_protocol(&mut self, node: Node<'tree>) {
        let Some(name_node) = direct_children(node, "identifier").first().copied() else {
            return;
        };
        let name = self.text(name_node).to_owned();
        let id = make_id(&[&self.stem, &name]);
        self.add_node(id.clone(), &format!("<{name}>"), line(node));
        self.add_edge(&self.file_id.clone(), &id, "contains", line(node), None);
        for references in direct_children(node, "protocol_reference_list") {
            for base in direct_children(references, "identifier") {
                let name = self.text(base).to_owned();
                let target = self.ensure_named(&name);
                if target != id {
                    self.add_edge(&id, &target, "implements", line(node), None);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, Some(&id));
        }
    }

    fn add_method(&mut self, node: Node<'tree>, parent: Option<&str>) {
        let container = parent.unwrap_or(&self.file_id).to_owned();
        let prefix = {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|child| matches!(child.kind(), "+" | "-"))
                .map_or("-", |child| child.kind())
        };
        let parts: Vec<_> = direct_children(node, "identifier")
            .into_iter()
            .map(|part| self.text(part))
            .collect();
        if parts.is_empty() {
            return;
        }
        let name = parts.join("");
        let id = make_id(&[&container, &name]);
        self.add_node(id.clone(), &format!("{prefix}{name}"), line(node));
        self.add_edge(&container, &id, "method", line(node), None);
        if node.kind() == "method_definition" {
            self.method_bodies.push(MethodBody {
                id,
                node,
                container,
            });
        }
    }

    fn add_calls(&mut self) {
        let all_ids: HashSet<String> = self
            .extraction
            .nodes
            .iter()
            .filter(|node| node.id != self.file_id)
            .map(|node| node.id.clone())
            .collect();
        let mut siblings: HashMap<String, HashSet<String>> = HashMap::new();
        for method in &self.method_bodies {
            siblings
                .entry(method.container.clone())
                .or_default()
                .insert(method.id.clone());
            collect_local_types(method.node, self.source, &mut self.type_table);
        }
        let method_ids: Vec<(String, String)> = self
            .method_bodies
            .iter()
            .map(|method| (method.id.clone(), method.container.clone()))
            .collect();
        let methods = std::mem::take(&mut self.method_bodies);
        let mut seen = HashSet::new();
        for method in methods {
            let empty_siblings = HashSet::new();
            let index = CallIndex {
                all_ids: &all_ids,
                sibling_ids: siblings.get(&method.container).unwrap_or(&empty_siblings),
                methods: &method_ids,
            };
            self.walk_calls(
                method.node,
                &method.id,
                &method.container,
                &index,
                &mut seen,
            );
        }
    }

    fn walk_calls(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        container: &str,
        index: &CallIndex<'_>,
        seen: &mut HashSet<(String, String)>,
    ) {
        if node.kind() == "message_expression" {
            let method_nodes = children_with_field(node, "method");
            let method_name = method_nodes
                .iter()
                .filter(|method| method.kind() == "identifier")
                .map(|method| self.text(*method))
                .collect::<String>();
            let receiver = node.child_by_field_name("receiver");
            if method_name == "alloc"
                && let Some(receiver) = receiver.filter(|receiver| receiver.kind() == "identifier")
            {
                let name = self.text(receiver).to_owned();
                let target = self.ensure_named(&name);
                if target != caller {
                    self.add_edge(caller, &target, "references", line(node), Some("type"));
                }
            }
            if !method_name.is_empty() {
                let needle = make_id(&[&method_name]);
                for candidate in index.all_ids.iter().filter(|candidate| {
                    candidate.ends_with(&needle) && candidate.as_str() != caller
                }) {
                    if seen.insert((caller.to_owned(), candidate.clone())) {
                        self.add_edge(caller, candidate, "calls", line(node), Some("call"));
                    }
                }
                if let Some(receiver) = receiver.filter(|receiver| receiver.kind() == "identifier")
                {
                    let receiver = self.text(receiver).to_owned();
                    self.extraction.raw_calls_mut().push(RawCall {
                        caller_nid: caller.to_owned(),
                        callee: method_name,
                        is_member_call: true,
                        source_file: self.source_file.clone(),
                        source_location: format!("L{}", line(node)),
                        receiver: Some(Some(receiver)),
                        receiver_type: None,
                        lang: Some("objc".to_owned()),
                    });
                }
            }
        } else if node.kind() == "field_expression" {
            for field in direct_children(node, "field_identifier") {
                let target = make_id(&[container, self.text(field)]);
                if index.sibling_ids.contains(&target)
                    && target != caller
                    && seen.insert((caller.to_owned(), target.clone()))
                {
                    self.add_edge(caller, &target, "accesses", line(node), None);
                }
            }
        } else if node.kind() == "selector_expression" {
            let name = direct_children(node, "identifier")
                .into_iter()
                .map(|identifier| self.text(identifier))
                .collect::<String>();
            let matches: HashSet<_> = index
                .methods
                .iter()
                .filter(|(id, owner)| id == &make_id(&[owner, &name]) && id != caller)
                .map(|(id, _)| id.clone())
                .collect();
            if matches.len() == 1 {
                let target = matches.into_iter().next().unwrap_or_default();
                if seen.insert((caller.to_owned(), target.clone())) {
                    self.add_edge(caller, &target, "calls", line(node), Some("call"));
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, container, index, seen);
        }
    }

    fn ensure_named(&mut self, name: &str) -> String {
        let local = make_id(&[&self.stem, name]);
        if self.seen_nodes.contains(&local) {
            return local;
        }
        let id = make_id(&[name]);
        if self.seen_nodes.insert(id.clone()) {
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(name.to_owned()));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert("source_file".to_owned(), Value::String(String::new()));
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            attributes.insert(
                "origin_file".to_owned(),
                Value::String(self.source_file.clone()),
            );
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
        }
        id
    }

    fn text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.source).unwrap_or_default()
    }

    fn add_node(&mut self, id: String, label: &str, at_line: usize) {
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{at_line}")),
        );
        self.extraction.nodes.push(NodeRecord { id, attributes });
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        at_line: usize,
        context: Option<&str>,
    ) {
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{at_line}")),
        );
        attributes.insert("weight".to_owned(), json!(1.0));
        if let Some(context) = context {
            attributes.insert("context".to_owned(), Value::String(context.to_owned()));
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }
}

fn collect_local_types(node: Node<'_>, source: &[u8], table: &mut HashMap<String, String>) {
    if node.kind() == "declaration" {
        let type_node = node
            .child_by_field_name("type")
            .or_else(|| first_child(node, "type_identifier"));
        if let Some(type_node) = type_node.filter(|node| node.kind() == "type_identifier") {
            let type_name = text(type_node, source).trim();
            let declarators: Vec<_> = {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .filter(|child| {
                        matches!(
                            child.kind(),
                            "identifier" | "pointer_declarator" | "init_declarator"
                        )
                    })
                    .collect()
            };
            if type_name.starts_with(char::is_uppercase)
                && declarators.len() == 1
                && let Some(name) = declarator_name(declarators[0], source)
            {
                table.entry(name).or_insert_with(|| type_name.to_owned());
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_local_types(child, source, table);
    }
}

fn declarator_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(text(node, source).to_owned());
    }
    if matches!(node.kind(), "pointer_declarator" | "init_declarator") {
        return node
            .child_by_field_name("declarator")
            .or_else(|| {
                let mut cursor = node.walk();
                node.children(&mut cursor)
                    .find(|child| matches!(child.kind(), "identifier" | "pointer_declarator"))
            })
            .and_then(|inner| declarator_name(inner, source));
    }
    None
}

fn children_with_field<'tree>(node: Node<'tree>, field: &str) -> Vec<Node<'tree>> {
    let Some(field_id) = node.language().field_id_for_name(field) else {
        return Vec::new();
    };
    let mut cursor = node.walk();
    let mut result = Vec::new();
    if cursor.goto_first_child() {
        loop {
            if cursor.field_id() == Some(field_id) {
                result.push(cursor.node());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    result
}

fn first_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
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

fn collect_nodes<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    if node.kind() == kind {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_nodes(child, kind, output);
    }
}

fn first_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
        if let Some(found) = first_descendant(child, kind) {
            return Some(found);
        }
    }
    None
}

fn text<'source>(node: Node<'_>, source: &'source [u8]) -> &'source str {
    node.utf8_text(source).unwrap_or_default()
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
