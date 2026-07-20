use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::{Extraction, RawCall, file_stem, make_id};

const TYPE_DECLARATIONS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "struct_declaration",
    "record_declaration",
];

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let interface_names = collect_interface_names(root, source);
    let mut state = State {
        source,
        source_file: source_file.clone(),
        stem,
        file_id: file_id.clone(),
        extraction: Extraction::default(),
        seen_nodes: HashSet::new(),
        namespace_stack: Vec::new(),
        scope_stack: Vec::new(),
        interface_names,
        function_bodies: Vec::new(),
    };
    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
        None,
        None,
    );
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        state.walk(child, None);
    }
    state.add_calls();
    let type_table = member_type_table(root, source);
    if !type_table.is_empty() {
        state.extraction.extensions.insert(
            "csharp_type_table".to_owned(),
            json!({"path": source_file, "table": type_table}),
        );
    }
    state.extraction
}

struct FunctionBody<'tree> {
    id: String,
    node: Node<'tree>,
}

struct State<'source, 'tree> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    namespace_stack: Vec<String>,
    scope_stack: Vec<String>,
    interface_names: HashSet<String>,
    function_bodies: Vec<FunctionBody<'tree>>,
}

impl<'tree> State<'_, 'tree> {
    fn walk(&mut self, node: Node<'tree>, parent_type: Option<&str>) {
        let kind = node.kind();
        if kind == "using_directive" {
            self.add_using(node);
            return;
        }
        if matches!(
            kind,
            "namespace_declaration" | "file_scoped_namespace_declaration"
        ) {
            self.add_namespace(node, parent_type);
            return;
        }
        if TYPE_DECLARATIONS.contains(&kind) {
            self.add_type(node, parent_type);
            return;
        }
        if kind == "field_declaration" && parent_type.is_some() {
            self.add_field_reference(node, parent_type.unwrap_or_default());
            return;
        }
        if kind == "property_declaration" && parent_type.is_some() {
            self.add_property_references(node, parent_type.unwrap_or_default());
            return;
        }
        if kind == "method_declaration" {
            self.add_method(node, parent_type);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, parent_type);
        }
    }

    fn add_namespace(&mut self, node: Node<'tree>, parent_type: Option<&str>) {
        let Some(name_node) = node
            .child_by_field_name("name")
            .or_else(|| first_child_matching(node, &["identifier", "qualified_name"]))
        else {
            return;
        };
        let name = self.text(name_node).trim().to_owned();
        if name.is_empty() {
            return;
        }
        self.namespace_stack.push(name);
        self.scope_stack.push(format!("s{}", node.start_byte()));
        let label = self.namespace_stack.join(".");
        let id = namespace_id(&label);
        let mut metadata = Map::new();
        metadata.insert(
            "kind".to_owned(),
            Value::String("csharp_namespace".to_owned()),
        );
        self.add_node(
            id.clone(),
            &label,
            node.start_position().row + 1,
            Some("namespace"),
            Some(metadata),
        );
        self.add_edge(
            &self.file_id.clone(),
            &id,
            "contains",
            node.start_position().row + 1,
            None,
            None,
        );
        if node.kind() == "file_scoped_namespace_declaration" {
            return;
        }
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                self.walk(child, parent_type);
            }
        }
        self.namespace_stack.pop();
        self.scope_stack.pop();
    }

    fn add_using(&mut self, node: Node<'tree>) {
        let mut text = self.text(node).trim().trim_end_matches(';').trim();
        if let Some(rest) = text.strip_prefix("global ") {
            text = rest.trim();
        }
        let Some(mut body) = text.strip_prefix("using") else {
            return;
        };
        body = body.trim();
        let (using_kind, alias, target) = if let Some(target) = body.strip_prefix("static ") {
            ("static", None, target.trim())
        } else if let Some((alias, target)) = body.split_once('=') {
            ("alias", Some(alias.trim()), target.trim())
        } else {
            ("namespace", None, body)
        };
        if target.is_empty() {
            return;
        }
        let mut metadata = Map::new();
        metadata.insert(
            "using_kind".to_owned(),
            Value::String(using_kind.to_owned()),
        );
        if let Some(alias) = alias {
            metadata.insert("alias".to_owned(), Value::String(alias.to_owned()));
        }
        metadata.insert("target_fqn".to_owned(), Value::String(target.to_owned()));
        metadata.insert(
            "scope_kind".to_owned(),
            Value::String(
                if self.scope_stack.is_empty() {
                    "file"
                } else {
                    "namespace"
                }
                .to_owned(),
            ),
        );
        if let Some(scope) = self.scope_stack.last() {
            metadata.insert("scope_id".to_owned(), Value::String(scope.clone()));
        }
        self.add_edge(
            &self.file_id.clone(),
            &make_id(&[target]),
            "imports",
            node.start_position().row + 1,
            Some("import"),
            Some(metadata),
        );
    }

    fn add_type(&mut self, node: Node<'tree>, parent_type: Option<&str>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_owned();
        let namespace = self.namespace_stack.join(".");
        let id = make_id(&[&self.stem, &namespace, &name]);
        let mut metadata = Map::new();
        if parent_type.is_some() {
            metadata.insert("is_nested_type".to_owned(), Value::Bool(true));
        }
        self.add_node(
            id.clone(),
            &name,
            node.start_position().row + 1,
            None,
            Some(metadata),
        );
        self.mark_callable(&id);
        self.add_edge(
            &self.file_id.clone(),
            &id,
            "contains",
            node.start_position().row + 1,
            None,
            None,
        );
        let type_parameters = type_parameters_in_scope(node, self.source);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "base_list" {
                continue;
            }
            let mut base_cursor = child.walk();
            for base in child.children(&mut base_cursor).filter(|base| {
                matches!(
                    base.kind(),
                    "identifier" | "generic_name" | "qualified_name"
                )
            }) {
                let Some(reference) = read_type_name(base, self.source) else {
                    continue;
                };
                if type_parameters.contains(&reference.name) {
                    continue;
                }
                let target = self.ensure_base(&reference.name);
                let relation = if self.interface_names.contains(&reference.name)
                    || is_interface_convention(&reference.name)
                {
                    "implements"
                } else {
                    "inherits"
                };
                self.add_edge(
                    &id,
                    &target,
                    relation,
                    node.start_position().row + 1,
                    None,
                    Some(reference.metadata()),
                );
                if base.kind() == "generic_name" {
                    let mut references = Vec::new();
                    collect_type_references(
                        base,
                        self.source,
                        true,
                        &type_parameters,
                        &mut references,
                    );
                    for reference in references.into_iter().skip(1) {
                        let target =
                            self.ensure_named(&reference.name, node.start_position().row + 1);
                        self.add_edge(
                            &id,
                            &target,
                            "references",
                            node.start_position().row + 1,
                            Some("generic_arg"),
                            Some(reference.metadata()),
                        );
                    }
                }
            }
        }
        if let Some(body) = node
            .child_by_field_name("body")
            .or_else(|| first_child_matching(node, &["declaration_list"]))
        {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                self.walk(child, Some(&id));
            }
        }
    }

    fn add_field_reference(&mut self, node: Node<'tree>, owner: &str) {
        let type_node = node.child_by_field_name("type").or_else(|| {
            first_child_matching(node, &["variable_declaration"])
                .and_then(|declaration| declaration.child_by_field_name("type"))
        });
        let Some(reference) = type_node.and_then(|node| read_type_name(node, self.source)) else {
            return;
        };
        if type_parameters_in_scope(node, self.source).contains(&reference.name) {
            return;
        }
        let target = self.ensure_named(&reference.name, node.start_position().row + 1);
        self.add_edge(
            owner,
            &target,
            "references",
            node.start_position().row + 1,
            Some("field"),
            Some(reference.metadata()),
        );
    }

    fn add_property_references(&mut self, node: Node<'tree>, owner: &str) {
        let Some(type_node) = node.child_by_field_name("type") else {
            return;
        };
        let mut references = Vec::new();
        collect_type_references(
            type_node,
            self.source,
            false,
            &type_parameters_in_scope(node, self.source),
            &mut references,
        );
        for reference in references {
            let target = self.ensure_named(&reference.name, node.start_position().row + 1);
            if target != owner {
                self.add_edge(
                    owner,
                    &target,
                    "references",
                    node.start_position().row + 1,
                    Some(if reference.generic {
                        "generic_arg"
                    } else {
                        "field"
                    }),
                    Some(reference.metadata()),
                );
            }
        }
    }

    fn add_method(&mut self, node: Node<'tree>, parent_type: Option<&str>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_owned();
        let id = parent_type.map_or_else(
            || make_id(&[&self.stem, &name]),
            |owner| make_id(&[owner, &name]),
        );
        self.add_node(
            id.clone(),
            &if parent_type.is_some() {
                format!(".{name}()")
            } else {
                format!("{name}()")
            },
            node.start_position().row + 1,
            None,
            None,
        );
        self.mark_callable(&id);
        self.add_edge(
            parent_type.unwrap_or(&self.file_id.clone()),
            &id,
            if parent_type.is_some() {
                "method"
            } else {
                "contains"
            },
            node.start_position().row + 1,
            None,
            None,
        );
        let skip = type_parameters_in_scope(node, self.source);
        if let Some(parameters) = node.child_by_field_name("parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters
                .children(&mut cursor)
                .filter(|parameter| parameter.kind() == "parameter")
            {
                let mut references = Vec::new();
                collect_type_references(
                    parameter.child_by_field_name("type").unwrap_or(parameter),
                    self.source,
                    false,
                    &skip,
                    &mut references,
                );
                for reference in references {
                    let target = self.ensure_named(&reference.name, node.start_position().row + 1);
                    self.add_edge(
                        &id,
                        &target,
                        "references",
                        node.start_position().row + 1,
                        Some(if reference.generic {
                            "generic_arg"
                        } else {
                            "parameter_type"
                        }),
                        Some(reference.metadata()),
                    );
                }
            }
        }
        if let Some(returns) = node.child_by_field_name("returns") {
            let mut references = Vec::new();
            collect_type_references(returns, self.source, false, &skip, &mut references);
            for reference in references {
                let target = self.ensure_named(&reference.name, node.start_position().row + 1);
                self.add_edge(
                    &id,
                    &target,
                    "references",
                    node.start_position().row + 1,
                    Some(if reference.generic {
                        "generic_arg"
                    } else {
                        "return_type"
                    }),
                    Some(reference.metadata()),
                );
            }
        }
        for reference in attribute_references(node, self.source, &skip) {
            let target = self.ensure_named(&reference.name, node.start_position().row + 1);
            self.add_edge(
                &id,
                &target,
                "references",
                node.start_position().row + 1,
                Some("attribute"),
                Some(reference.metadata()),
            );
        }
        self.function_bodies.push(FunctionBody { id, node });
    }

    fn add_calls(&mut self) {
        let mut callables: HashMap<String, Vec<String>> = HashMap::new();
        for node in &self.extraction.nodes {
            let label = node.label();
            if label.ends_with("()") {
                callables
                    .entry(label.trim_matches(['.', '(', ')']).to_owned())
                    .or_default()
                    .push(node.id.clone());
            }
        }
        let mut seen = HashSet::new();
        let functions = std::mem::take(&mut self.function_bodies);
        for function in functions {
            self.walk_calls(function.node, &function.id, &callables, &mut seen, true);
        }
    }

    fn walk_calls(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        callables: &HashMap<String, Vec<String>>,
        seen: &mut HashSet<(String, String)>,
        root: bool,
    ) {
        if !root && node.kind() == "method_declaration" {
            return;
        }
        if node.kind() == "invocation_expression" {
            let function = node.child_by_field_name("function");
            let mut callee = None;
            let mut member = false;
            let mut receiver = None;
            if let Some(function) = function {
                if function.kind() == "member_access_expression" {
                    member = true;
                    callee = function
                        .child_by_field_name("name")
                        .map(|name| self.text(name).to_owned());
                    receiver = function
                        .child_by_field_name("expression")
                        .filter(|receiver| {
                            matches!(receiver.kind(), "identifier" | "this_expression")
                        })
                        .map(|receiver| {
                            if receiver.kind() == "this_expression" {
                                "this".to_owned()
                            } else {
                                self.text(receiver).to_owned()
                            }
                        });
                } else if function.kind() == "identifier" {
                    callee = Some(self.text(function).to_owned());
                }
            }
            if let Some(callee) = callee.filter(|callee| !callee.is_empty()) {
                let target = (!member || receiver.is_none())
                    .then(|| callables.get(&callee))
                    .flatten()
                    .filter(|targets| targets.len() == 1)
                    .and_then(|targets| targets.first())
                    .filter(|target| target.as_str() != caller)
                    .cloned();
                if let Some(target) = target {
                    let pair = (caller.to_owned(), target.clone());
                    if seen.insert(pair) {
                        self.add_edge(
                            caller,
                            &target,
                            "calls",
                            node.start_position().row + 1,
                            Some("call"),
                            None,
                        );
                    }
                } else {
                    self.extraction.raw_calls_mut().push(RawCall {
                        caller_nid: caller.to_owned(),
                        callee,
                        is_member_call: Some(member),
                        source_file: self.source_file.clone(),
                        source_location: format!("L{}", node.start_position().row + 1),
                        receiver: Some(receiver),
                        receiver_type: None,
                        lang: Some("csharp".to_owned()),
                        extensions: Map::new(),
                    });
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, callables, seen, false);
        }
    }

    fn ensure_named(&mut self, name: &str, line: usize) -> String {
        let local = make_id(&[&self.stem, &self.namespace_stack.join("."), name]);
        if self.seen_nodes.contains(&local) {
            return local;
        }
        let id = make_id(&[name]);
        if !self.seen_nodes.contains(&id) {
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(name.to_owned()));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert("source_file".to_owned(), Value::String(String::new()));
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            attributes.insert(
                "origin_file".to_owned(),
                Value::String(self.source_file.clone()),
            );
            self.seen_nodes.insert(id.clone());
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
        }
        let _ = line;
        id
    }

    fn ensure_base(&mut self, name: &str) -> String {
        let local = make_id(&[&self.stem, &self.namespace_stack.join("."), name]);
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
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
        }
        id
    }

    fn add_node(
        &mut self,
        id: String,
        label: &str,
        line: usize,
        node_type: Option<&str>,
        metadata: Option<Map<String, Value>>,
    ) {
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut merged = metadata.unwrap_or_default();
        if !self.namespace_stack.is_empty() {
            merged
                .entry("namespace".to_owned())
                .or_insert_with(|| Value::String(self.namespace_stack.join(".")));
        }
        if !self.scope_stack.is_empty() && node_type != Some("namespace") {
            merged
                .entry("scope_chain".to_owned())
                .or_insert_with(|| json!(self.scope_stack));
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
            Value::String(format!("L{line}")),
        );
        if let Some(node_type) = node_type {
            attributes.insert("type".to_owned(), Value::String(node_type.to_owned()));
        }
        if !merged.is_empty() {
            attributes.insert("metadata".to_owned(), Value::Object(merged));
        }
        self.extraction.nodes.push(NodeRecord { id, attributes });
    }

    fn mark_callable(&mut self, id: &str) {
        if let Some(node) = self.extraction.nodes.iter_mut().find(|node| node.id == id) {
            node.attributes
                .insert("_callable".to_owned(), Value::Bool(true));
        }
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        line: usize,
        context: Option<&str>,
        metadata: Option<Map<String, Value>>,
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
            Value::String(format!("L{line}")),
        );
        attributes.insert("weight".to_owned(), json!(1.0));
        if let Some(context) = context {
            attributes.insert("context".to_owned(), Value::String(context.to_owned()));
        }
        if let Some(metadata) = metadata.filter(|metadata| !metadata.is_empty()) {
            attributes.insert("metadata".to_owned(), Value::Object(metadata));
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

#[derive(Clone)]
struct TypeReference {
    name: String,
    generic: bool,
    qualified: bool,
    qualifier: String,
}

impl TypeReference {
    fn metadata(&self) -> Map<String, Value> {
        let mut metadata = Map::new();
        metadata.insert("ref_token".to_owned(), Value::String(self.name.clone()));
        if self.qualified {
            metadata.insert("qualified".to_owned(), Value::Bool(true));
        }
        if !self.qualifier.is_empty() {
            metadata.insert(
                "ref_qualifier".to_owned(),
                Value::String(self.qualifier.clone()),
            );
        }
        metadata
    }
}

fn namespace_id(name: &str) -> String {
    let digest = Sha1::digest(name.as_bytes());
    format!("csharp_namespace:{:x}", digest)[..33].to_owned()
}

fn collect_interface_names(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut output = HashSet::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.kind() == "interface_declaration"
            && let Some(name) = node.child_by_field_name("name")
        {
            output.insert(text(name, source).to_owned());
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    output
}

fn is_interface_convention(name: &str) -> bool {
    let mut characters = name.chars();
    characters.next() == Some('I') && characters.next().is_some_and(char::is_uppercase)
}

fn read_type_name(node: Node<'_>, source: &[u8]) -> Option<TypeReference> {
    match node.kind() {
        "identifier" | "predefined_type" => Some(TypeReference {
            name: text(node, source).to_owned(),
            generic: false,
            qualified: false,
            qualifier: String::new(),
        }),
        "qualified_name" => {
            let raw = text(node, source);
            let (qualifier, name) = raw.rsplit_once('.').unwrap_or(("", raw));
            Some(TypeReference {
                name: name.split('<').next().unwrap_or_default().to_owned(),
                generic: false,
                qualified: true,
                qualifier: qualifier.to_owned(),
            })
        }
        "generic_name" => {
            let name_node = node
                .child_by_field_name("name")
                .or_else(|| first_child_matching(node, &["identifier", "qualified_name"]))?;
            let raw = text(name_node, source);
            let (qualifier, name) = raw.rsplit_once('.').unwrap_or(("", raw));
            Some(TypeReference {
                name: name.to_owned(),
                generic: false,
                qualified: name_node.kind() == "qualified_name",
                qualifier: qualifier.to_owned(),
            })
        }
        _ => {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find_map(|child| read_type_name(child, source))
        }
    }
}

fn collect_type_references(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    skip: &HashSet<String>,
    output: &mut Vec<TypeReference>,
) {
    match node.kind() {
        "predefined_type" => {}
        "identifier" | "qualified_name" => {
            if let Some(mut reference) = read_type_name(node, source)
                && !skip.contains(&reference.name)
            {
                reference.generic = generic;
                output.push(reference);
            }
        }
        "generic_name" => {
            if let Some(mut reference) = read_type_name(node, source)
                && !skip.contains(&reference.name)
            {
                reference.generic = generic;
                output.push(reference);
            }
            let mut cursor = node.walk();
            for arguments in node
                .children(&mut cursor)
                .filter(|child| child.kind() == "type_argument_list")
            {
                let mut argument_cursor = arguments.walk();
                for argument in arguments
                    .children(&mut argument_cursor)
                    .filter(|child| child.is_named())
                {
                    collect_type_references(argument, source, true, skip, output);
                }
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                collect_type_references(child, source, generic, skip, output);
            }
        }
    }
}

fn type_parameters_in_scope(node: Node<'_>, source: &[u8]) -> HashSet<String> {
    let mut output = HashSet::new();
    let mut scope = Some(node);
    while let Some(node) = scope {
        if TYPE_DECLARATIONS.contains(&node.kind()) || node.kind() == "method_declaration" {
            let mut cursor = node.walk();
            for list in node
                .children(&mut cursor)
                .filter(|child| child.kind() == "type_parameter_list")
            {
                let mut list_cursor = list.walk();
                for parameter in list
                    .children(&mut list_cursor)
                    .filter(|child| child.is_named())
                {
                    if let Some(identifier) = if parameter.kind() == "identifier" {
                        Some(parameter)
                    } else {
                        first_child_matching(parameter, &["identifier"])
                    } {
                        output.insert(text(identifier, source).to_owned());
                    }
                }
            }
        }
        scope = node.parent();
    }
    output
}

fn attribute_references(
    node: Node<'_>,
    source: &[u8],
    skip: &HashSet<String>,
) -> Vec<TypeReference> {
    let mut output = Vec::new();
    let mut cursor = node.walk();
    for list in node
        .children(&mut cursor)
        .filter(|child| child.kind() == "attribute_list")
    {
        let mut list_cursor = list.walk();
        for attribute in list
            .children(&mut list_cursor)
            .filter(|child| child.kind() == "attribute")
        {
            if let Some(name) = attribute
                .child_by_field_name("name")
                .or_else(|| first_child_matching(attribute, &["identifier", "qualified_name"]))
                && let Some(reference) = read_type_name(name, source)
                && !skip.contains(&reference.name)
            {
                output.push(reference);
            }
        }
    }
    output
}

fn member_type_table(root: Node<'_>, source: &[u8]) -> Map<String, Value> {
    let mut table = Map::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "field_declaration" | "local_declaration_statement" => {
                if let Some(declaration) = first_descendant_of_kind(node, "variable_declaration") {
                    let declared = declaration
                        .child_by_field_name("type")
                        .and_then(|node| typed_name(node, source));
                    let mut cursor = declaration.walk();
                    for declarator in declaration
                        .children(&mut cursor)
                        .filter(|child| child.kind() == "variable_declarator")
                    {
                        if let Some(name) = declarator
                            .child_by_field_name("name")
                            .or_else(|| first_child_matching(declarator, &["identifier"]))
                        {
                            let resolved = declared.clone().or_else(|| {
                                first_descendant_of_kind(declarator, "object_creation_expression")
                                    .and_then(|creation| creation.child_by_field_name("type"))
                                    .and_then(|node| typed_name(node, source))
                            });
                            if let Some(resolved) = resolved {
                                table
                                    .entry(text(name, source).to_owned())
                                    .or_insert(Value::String(resolved));
                            }
                        }
                    }
                }
            }
            "property_declaration" | "parameter" => {
                if let (Some(name), Some(resolved)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("type")
                        .and_then(|node| typed_name(node, source)),
                ) {
                    table
                        .entry(text(name, source).to_owned())
                        .or_insert(Value::String(resolved));
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    table
}

fn typed_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let name = read_type_name(node, source)?.name;
    name.chars()
        .next()
        .is_some_and(char::is_uppercase)
        .then_some(name)
}

fn first_child_matching<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| kinds.contains(&child.kind()))
}

fn first_descendant_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| first_descendant_of_kind(child, kind))
}

fn text<'source>(node: Node<'_>, source: &'source [u8]) -> &'source str {
    node.utf8_text(source).unwrap_or_default()
}
