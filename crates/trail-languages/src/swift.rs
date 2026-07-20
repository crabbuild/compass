use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::{Extraction, RawCall, file_stem, make_id};

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let (protocols, classes) = pre_scan(root, source);
    let mut state = State {
        source,
        source_file: source_file.clone(),
        stem,
        file_id: file_id.clone(),
        extraction: Extraction::default(),
        seen_nodes: HashSet::new(),
        functions: Vec::new(),
        protocols,
        classes,
        extensions: Vec::new(),
        type_table: HashMap::new(),
    };
    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
        false,
        None,
    );
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        state.walk(child, None);
    }
    state.add_calls();
    if !state.extensions.is_empty() {
        state.extraction.extensions.insert(
            "swift_extensions".to_owned(),
            Value::Array(state.extensions),
        );
    }
    if !state.type_table.is_empty() {
        state.extraction.extensions.insert(
            "swift_type_table".to_owned(),
            json!({"path": source_file, "table": state.type_table}),
        );
    }
    state.extraction
}

struct FunctionBody<'tree> {
    id: String,
    body: Node<'tree>,
}

struct State<'source, 'tree> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    functions: Vec<FunctionBody<'tree>>,
    protocols: HashSet<String>,
    classes: HashSet<String>,
    extensions: Vec<Value>,
    type_table: HashMap<String, String>,
}

impl<'tree> State<'_, 'tree> {
    fn walk(&mut self, node: Node<'tree>, parent_class: Option<&str>) {
        match node.kind() {
            "import_declaration" => {
                self.add_import(node);
                return;
            }
            "class_declaration" | "protocol_declaration" => {
                self.add_type(node);
                return;
            }
            "property_declaration" if parent_class.is_some() => {
                self.add_property(node, parent_class.unwrap_or_default());
                return;
            }
            "enum_entry" if parent_class.is_some() => {
                self.add_enum_case(node, parent_class.unwrap_or_default());
                return;
            }
            "function_declaration"
            | "init_declaration"
            | "deinit_declaration"
            | "subscript_declaration" => {
                self.add_function(node, parent_class);
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, None);
        }
    }

    fn add_import(&mut self, node: Node<'tree>) {
        let Some(identifier) = first_descendant(node, "simple_identifier") else {
            return;
        };
        let name = self.text(identifier).to_owned();
        if name.is_empty() {
            return;
        }
        let id = make_id(&[&name]);
        self.add_edge(
            &self.file_id.clone(),
            &id,
            "imports",
            line(node),
            Some("import"),
        );
        self.add_node(id, &name, line(node), false, Some("module"));
    }

    fn add_type(&mut self, node: Node<'tree>) {
        let Some(name_node) = node
            .child_by_field_name("name")
            .or_else(|| first_descendant(node, "type_identifier"))
        else {
            return;
        };
        let name = type_head(name_node, self.source);
        if name.is_empty() {
            return;
        }
        let id = make_id(&[&self.stem, &name]);
        self.add_node(id.clone(), &name, line(node), true, None);
        self.add_edge(&self.file_id.clone(), &id, "contains", line(node), None);
        let kind = if node.kind() == "protocol_declaration" {
            Some("protocol")
        } else {
            declaration_keyword(node)
        };
        if kind == Some("extension") {
            self.extensions.push(json!({"nid": id, "label": name}));
        }
        let mut first = true;
        let mut cursor = node.walk();
        for inheritance in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "inheritance_specifier")
        {
            let Some(user_type) = inheritance
                .child_by_field_name("inherits_from")
                .or_else(|| first_descendant(inheritance, "user_type"))
            else {
                continue;
            };
            let Some(type_node) = first_descendant(user_type, "type_identifier") else {
                continue;
            };
            let base = self.text(type_node).to_owned();
            if base.is_empty() {
                continue;
            }
            let relation = self.classify_base(&base, kind, first);
            first = false;
            let target = self.ensure_named(&base);
            self.add_edge(&id, &target, relation, line(node), None);
            if let Some(arguments) = first_descendant(user_type, "type_arguments") {
                let mut refs = Vec::new();
                collect_type_refs(arguments, self.source, true, &mut refs);
                for (reference, _) in refs {
                    let target = self.ensure_named(&reference);
                    if target != id {
                        self.add_edge(&id, &target, "references", line(node), Some("generic_arg"));
                    }
                }
            }
        }
        let body = node.child_by_field_name("body").or_else(|| {
            ["class_body", "enum_class_body", "protocol_body"]
                .iter()
                .find_map(|kind| first_child(node, kind))
        });
        if let Some(body) = body {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                self.walk(child, Some(&id));
            }
        }
    }

    fn classify_base(&self, name: &str, kind: Option<&str>, first: bool) -> &'static str {
        if self.protocols.contains(name) {
            return "implements";
        }
        if self.classes.contains(name) {
            return "inherits";
        }
        if matches!(kind, Some("struct" | "enum" | "extension" | "actor")) {
            return "implements";
        }
        if first { "inherits" } else { "implements" }
    }

    fn add_property(&mut self, node: Node<'tree>, class_id: &str) {
        let mut property_type = None;
        if let Some(annotation) = first_child(node, "type_annotation") {
            let mut references = Vec::new();
            collect_type_refs(annotation, self.source, false, &mut references);
            for (reference, generic) in &references {
                let target = self.ensure_named(reference);
                if target != class_id {
                    self.add_edge(
                        class_id,
                        &target,
                        "references",
                        line(node),
                        Some(if *generic { "generic_arg" } else { "field" }),
                    );
                }
                if property_type.is_none() && !generic {
                    property_type = Some(reference.clone());
                }
            }
        }
        if let Some(name) = property_name(node, self.source)
            && let Some(property_type) = property_type
        {
            self.type_table.insert(name, property_type);
        }
    }

    fn add_enum_case(&mut self, node: Node<'tree>, enum_id: &str) {
        let mut cursor = node.walk();
        for child in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "simple_identifier")
        {
            let name = self.text(child).to_owned();
            let id = make_id(&[enum_id, &name]);
            self.add_node(id.clone(), &name, line(node), false, None);
            self.add_edge(enum_id, &id, "case_of", line(node), None);
        }
        if let Some(parameters) = first_child(node, "enum_type_parameters") {
            let mut references = Vec::new();
            collect_type_refs(parameters, self.source, false, &mut references);
            for (reference, generic) in references {
                let target = self.ensure_named(&reference);
                if target != enum_id {
                    self.add_edge(
                        enum_id,
                        &target,
                        "references",
                        line(node),
                        Some(if generic { "generic_arg" } else { "type" }),
                    );
                }
            }
        }
    }

    fn add_function(&mut self, node: Node<'tree>, parent_class: Option<&str>) {
        let name = match node.kind() {
            "deinit_declaration" => "deinit".to_owned(),
            "subscript_declaration" => "subscript".to_owned(),
            _ => node
                .child_by_field_name("name")
                .or_else(|| first_child(node, "simple_identifier"))
                .map(|name| self.text(name).to_owned())
                .unwrap_or_default(),
        };
        if name.is_empty() {
            return;
        }
        let id = parent_class.map_or_else(
            || make_id(&[&self.stem, &name]),
            |class| make_id(&[class, &name]),
        );
        let label = if parent_class.is_some() {
            format!(".{name}()")
        } else {
            format!("{name}()")
        };
        self.add_node(id.clone(), &label, line(node), true, None);
        let owner = parent_class.unwrap_or(&self.file_id).to_owned();
        self.add_edge(
            &owner,
            &id,
            if parent_class.is_some() {
                "method"
            } else {
                "contains"
            },
            line(node),
            None,
        );
        let mut cursor = node.walk();
        for parameter in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "parameter")
        {
            let type_node = parameter.child_by_field_name("type").or_else(|| {
                let mut cursor = parameter.walk();
                let children: Vec<_> = parameter.children(&mut cursor).collect();
                children.into_iter().rev().find(|child| {
                    matches!(
                        child.kind(),
                        "user_type"
                            | "array_type"
                            | "dictionary_type"
                            | "optional_type"
                            | "tuple_type"
                            | "type_identifier"
                    )
                })
            });
            let mut references = Vec::new();
            if let Some(type_node) = type_node {
                collect_type_refs(type_node, self.source, false, &mut references);
            }
            let mut parameter_type = None;
            for (reference, generic) in references {
                let target = self.ensure_named(&reference);
                if target != id {
                    self.add_edge(
                        &id,
                        &target,
                        "references",
                        line(node),
                        Some(if generic {
                            "generic_arg"
                        } else {
                            "parameter_type"
                        }),
                    );
                }
                if parameter_type.is_none() && !generic {
                    parameter_type = Some(reference);
                }
            }
            if let Some(parameter_type) = parameter_type
                && let Some(name) = parameter_name(parameter, self.source)
            {
                self.type_table.insert(name, parameter_type);
            }
        }
        if let Some(return_type) = node
            .child_by_field_name("return_type")
            .or_else(|| return_type_after_parameters(node))
        {
            let mut references = Vec::new();
            collect_type_refs(return_type, self.source, false, &mut references);
            for (reference, generic) in references {
                let target = self.ensure_named(&reference);
                if target != id {
                    self.add_edge(
                        &id,
                        &target,
                        "references",
                        line(node),
                        Some(if generic {
                            "generic_arg"
                        } else {
                            "return_type"
                        }),
                    );
                }
            }
        }
        if let Some(body) = node
            .child_by_field_name("body")
            .or_else(|| first_child(node, "function_body"))
        {
            collect_local_types(body, self.source, &mut self.type_table);
            self.functions.push(FunctionBody { id, body });
        }
    }

    fn add_calls(&mut self) {
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
        let functions = std::mem::take(&mut self.functions);
        let mut seen = HashSet::new();
        for function in functions {
            self.walk_calls(function.body, &function.id, &labels, &mut seen);
        }
    }

    fn walk_calls(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        labels: &HashMap<String, String>,
        seen: &mut HashSet<(String, String)>,
    ) {
        if matches!(
            node.kind(),
            "function_declaration"
                | "init_declaration"
                | "deinit_declaration"
                | "subscript_declaration"
        ) {
            return;
        }
        if node.kind() == "call_expression"
            && let Some(call) = swift_call(node, self.source)
            && !matches!(call.name.as_str(), "filter" | "print")
        {
            if let Some(target) = labels
                .get(&call.name)
                .filter(|target| target.as_str() != caller)
            {
                if seen.insert((caller.to_owned(), (*target).clone())) {
                    self.add_edge(caller, target, "calls", line(node), Some("call"));
                }
            } else {
                self.extraction.raw_calls_mut().push(RawCall {
                    caller_nid: caller.to_owned(),
                    callee: call.name,
                    is_member_call: call.member,
                    source_file: self.source_file.clone(),
                    source_location: format!("L{}", line(node)),
                    receiver: Some(call.receiver),
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

    fn add_node(
        &mut self,
        id: String,
        label: &str,
        at_line: usize,
        callable: bool,
        node_type: Option<&str>,
    ) {
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        if let Some(node_type) = node_type {
            attributes.insert("type".to_owned(), Value::String(node_type.to_owned()));
        }
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{at_line}")),
        );
        if callable {
            attributes.insert("_callable".to_owned(), Value::Bool(true));
        }
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

struct Call {
    name: String,
    member: bool,
    receiver: Option<String>,
}

fn swift_call(node: Node<'_>, source: &[u8]) -> Option<Call> {
    let first = {
        let mut cursor = node.walk();
        node.children(&mut cursor).next()
    }?;
    if first.kind() == "simple_identifier" {
        return Some(Call {
            name: text(first, source).to_owned(),
            member: false,
            receiver: None,
        });
    }
    if first.kind() != "navigation_expression" {
        return None;
    }
    let receiver = first
        .child_by_field_name("target")
        .and_then(|target| match target.kind() {
            "simple_identifier" => Some(text(target, source).to_owned()),
            _ => None,
        });
    let suffixes = direct_children(first, "navigation_suffix");
    let name = suffixes
        .last()
        .and_then(|suffix| first_descendant(*suffix, "simple_identifier"))
        .map(|name| text(name, source).to_owned())?;
    Some(Call {
        name,
        member: true,
        receiver,
    })
}

fn pre_scan(root: Node<'_>, source: &[u8]) -> (HashSet<String>, HashSet<String>) {
    let mut protocols = HashSet::new();
    let mut classes = HashSet::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "protocol_declaration" {
            if let Some(name) = node.child_by_field_name("name") {
                protocols.insert(text(name, source).to_owned());
            }
        } else if node.kind() == "class_declaration"
            && matches!(
                declaration_keyword(node),
                Some("class" | "struct" | "enum" | "actor")
            )
            && let Some(name) = node.child_by_field_name("name")
        {
            classes.insert(type_head(name, source));
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    (protocols, classes)
}

fn declaration_keyword(node: Node<'_>) -> Option<&'static str> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| {
            !child.is_named()
                && matches!(
                    child.kind(),
                    "class" | "struct" | "enum" | "extension" | "actor"
                )
        })
        .map(|child| match child.kind() {
            "class" => "class",
            "struct" => "struct",
            "enum" => "enum",
            "extension" => "extension",
            "actor" => "actor",
            _ => unreachable!(),
        })
}

fn collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    match node.kind() {
        "type_annotation" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                collect_type_refs(child, source, generic, output);
            }
        }
        "user_type" => {
            let mut cursor = node.walk();
            if let Some(name) = node
                .children(&mut cursor)
                .find(|child| child.kind() == "type_identifier")
            {
                output.push((text(name, source).to_owned(), generic));
            }
            if let Some(arguments) = first_child(node, "type_arguments") {
                let mut cursor = arguments.walk();
                for argument in arguments
                    .children(&mut cursor)
                    .filter(|child| child.is_named())
                {
                    collect_type_refs(argument, source, true, output);
                }
            }
        }
        "type_identifier" => output.push((text(node, source).to_owned(), generic)),
        "optional_type"
        | "implicitly_unwrapped_optional_type"
        | "array_type"
        | "dictionary_type"
        | "tuple_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                collect_type_refs(child, source, generic, output);
            }
        }
        _ if node.is_named() => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                collect_type_refs(child, source, generic, output);
            }
        }
        _ => {}
    }
}

fn property_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|pattern| first_descendant(pattern, "simple_identifier"))
        .or_else(|| first_descendant(node, "simple_identifier"))
        .map(|name| text(name, source).to_owned())
}

fn parameter_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let names = direct_children(node, "simple_identifier");
    names.last().map(|name| text(*name, source).to_owned())
}

fn return_type_after_parameters(node: Node<'_>) -> Option<Node<'_>> {
    let mut saw_parameter = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "parameter" {
            saw_parameter = true;
            continue;
        }
        if saw_parameter
            && matches!(
                child.kind(),
                "user_type"
                    | "array_type"
                    | "dictionary_type"
                    | "optional_type"
                    | "tuple_type"
                    | "type_identifier"
            )
        {
            return Some(child);
        }
    }
    None
}

fn collect_local_types(node: Node<'_>, source: &[u8], table: &mut HashMap<String, String>) {
    if matches!(node.kind(), "function_declaration" | "lambda_literal") {
        return;
    }
    if node.kind() == "property_declaration"
        && let Some(name) = property_name(node, source)
    {
        let mut inferred = None;
        if let Some(call) = first_child(node, "call_expression") {
            let first = {
                let mut cursor = call.walk();
                call.children(&mut cursor).next()
            };
            if let Some(first) = first
                && first.kind() == "simple_identifier"
                && text(first, source).starts_with(char::is_uppercase)
            {
                inferred = Some(text(first, source).to_owned());
            }
        }
        if let Some(value) = inferred {
            table.entry(name).or_insert(value);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_local_types(child, source, table);
    }
}

fn type_head(node: Node<'_>, source: &[u8]) -> String {
    if node.kind() == "type_identifier" {
        return text(node, source).to_owned();
    }
    first_descendant(node, "type_identifier")
        .map(|name| text(name, source).to_owned())
        .unwrap_or_default()
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
