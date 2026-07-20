use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::{Extraction, RawCall, file_stem, make_id};

const TYPE_NODES: &[&str] = &[
    "named_type",
    "primitive_type",
    "nullable_type",
    "union_type",
    "intersection_type",
    "optional_type",
];
const CALL_NODES: &[&str] = &[
    "function_call_expression",
    "member_call_expression",
    "scoped_call_expression",
    "class_constant_access_expression",
];
const CONTAINER_METHODS: &[&str] = &["bind", "singleton", "scoped", "instance"];

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = State {
        source,
        source_file,
        stem,
        file_id: file_id.clone(),
        extraction: Extraction::default(),
        seen_nodes: HashSet::new(),
        functions: Vec::new(),
        pending_listeners: Vec::new(),
    };
    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
        true,
        false,
    );
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        state.walk(child, None);
    }
    state.add_calls();
    state.add_listener_edges();
    state.extraction
}

struct FunctionBody<'tree> {
    id: String,
    body: Node<'tree>,
}

struct Listener {
    event: String,
    listener: String,
    line: usize,
}

struct State<'source, 'tree> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    functions: Vec<FunctionBody<'tree>>,
    pending_listeners: Vec<Listener>,
}

impl<'tree> State<'_, 'tree> {
    fn walk(&mut self, node: Node<'tree>, parent_class: Option<&str>) {
        match node.kind() {
            "namespace_use_clause" => {
                self.add_import(node);
                return;
            }
            "class_declaration" => {
                self.add_class(node);
                return;
            }
            "property_declaration" if parent_class.is_some() => {
                self.add_property(node, parent_class.unwrap_or_default());
                return;
            }
            "function_definition" | "method_declaration" => {
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
        let mut cursor = node.walk();
        let Some(name) = node
            .children(&mut cursor)
            .find(|child| matches!(child.kind(), "qualified_name" | "name" | "identifier"))
        else {
            return;
        };
        let target = php_name(self.text(name));
        if !target.is_empty() {
            self.add_edge(
                &self.file_id.clone(),
                &make_id(&[target]),
                "imports",
                line(node),
                Some("import"),
                false,
            );
        }
    }

    fn add_class(&mut self, node: Node<'tree>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_owned();
        let id = make_id(&[&self.stem, &name]);
        self.add_node(id.clone(), &name, line(node), true, true);
        self.add_edge(
            &self.file_id.clone(),
            &id,
            "contains",
            line(node),
            None,
            false,
        );

        let mut cursor = node.walk();
        for clause in node.children(&mut cursor) {
            let relation = match clause.kind() {
                "base_clause" => "inherits",
                "class_interface_clause" => "implements",
                _ => continue,
            };
            let mut clause_cursor = clause.walk();
            for base in clause
                .children(&mut clause_cursor)
                .filter(|child| matches!(child.kind(), "name" | "qualified_name"))
            {
                let base_name = php_name(self.text(base)).to_owned();
                if !base_name.is_empty() {
                    let target = self.ensure_base(&base_name);
                    self.add_edge(&id, &target, relation, line(clause), None, false);
                }
            }
        }

        let body = node.child_by_field_name("body").or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .find(|child| child.kind() == "declaration_list")
        });
        if let Some(body) = body {
            let mut cursor = body.walk();
            let children: Vec<_> = body.children(&mut cursor).collect();
            for member in &children {
                if member.kind() != "use_declaration" {
                    continue;
                }
                let mut member_cursor = member.walk();
                for used in member
                    .children(&mut member_cursor)
                    .filter(|child| matches!(child.kind(), "name" | "qualified_name"))
                {
                    let used_name = php_name(self.text(used)).to_owned();
                    if !used_name.is_empty() {
                        let target = self.ensure_base(&used_name);
                        self.add_edge(&id, &target, "mixes_in", line(*member), None, false);
                    }
                }
            }
            for child in children {
                self.walk(child, Some(&id));
            }
        }
    }

    fn add_property(&mut self, node: Node<'tree>, class_id: &str) {
        if self.collect_listeners(node) {
            return;
        }
        let mut cursor = node.walk();
        if let Some(type_node) = node
            .children(&mut cursor)
            .find(|child| TYPE_NODES.contains(&child.kind()))
        {
            let mut references = Vec::new();
            collect_type_refs(type_node, self.source, false, &mut references);
            for (reference, generic) in references {
                let target = self.ensure_reference(&reference);
                if target != class_id {
                    self.add_edge(
                        class_id,
                        &target,
                        "references",
                        line(node),
                        Some(if generic { "generic_arg" } else { "field" }),
                        false,
                    );
                }
            }
        }
    }

    fn collect_listeners(&mut self, node: Node<'tree>) -> bool {
        let mut handled = false;
        let mut cursor = node.walk();
        for element in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "property_element")
        {
            let name = element
                .child_by_field_name("name")
                .and_then(|name| first_descendant(name, "name"))
                .map(|name| self.text(name))
                .unwrap_or_default();
            if !matches!(name, "listen" | "subscribe") {
                continue;
            }
            let Some(array) = element
                .child_by_field_name("default_value")
                .filter(|value| value.kind() == "array_creation_expression")
                .or_else(|| first_child(element, "array_creation_expression"))
            else {
                continue;
            };
            handled = true;
            let mut array_cursor = array.walk();
            for entry in array
                .children(&mut array_cursor)
                .filter(|child| child.kind() == "array_element_initializer")
            {
                let constants = direct_children(entry, "class_constant_access_expression");
                let listener_array = first_child(entry, "array_creation_expression");
                let Some(event) = constants
                    .first()
                    .and_then(|constant| class_scope(*constant, self.source))
                else {
                    continue;
                };
                let Some(listener_array) = listener_array else {
                    continue;
                };
                let mut listener_cursor = listener_array.walk();
                for listener_entry in listener_array
                    .children(&mut listener_cursor)
                    .filter(|child| child.kind() == "array_element_initializer")
                {
                    if let Some(constant) =
                        first_child(listener_entry, "class_constant_access_expression")
                        && let Some(listener) = class_scope(constant, self.source)
                    {
                        self.pending_listeners.push(Listener {
                            event: event.clone(),
                            listener,
                            line: line(constant),
                        });
                    }
                }
            }
        }
        handled
    }

    fn add_function(&mut self, node: Node<'tree>, parent_class: Option<&str>) {
        let Some(name_node) = node
            .child_by_field_name("name")
            .or_else(|| first_child(node, "name"))
        else {
            return;
        };
        let name = self.text(name_node).to_owned();
        let id = parent_class.map_or_else(
            || make_id(&[&self.stem, &name]),
            |class| make_id(&[class, &name]),
        );
        let label = if parent_class.is_some() {
            format!(".{name}()")
        } else {
            format!("{name}()")
        };
        self.add_node(id.clone(), &label, line(node), true, true);
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
            false,
        );
        if let Some(parameters) = node
            .child_by_field_name("parameters")
            .or_else(|| first_child(node, "formal_parameters"))
        {
            let mut cursor = parameters.walk();
            for parameter in parameters.children(&mut cursor).filter(|parameter| {
                matches!(
                    parameter.kind(),
                    "simple_parameter" | "property_promotion_parameter"
                )
            }) {
                let promoted = parameter.kind() == "property_promotion_parameter";
                let mut parameter_cursor = parameter.walk();
                if let Some(type_node) = parameter
                    .children(&mut parameter_cursor)
                    .find(|child| TYPE_NODES.contains(&child.kind()))
                {
                    let mut references = Vec::new();
                    collect_type_refs(type_node, self.source, false, &mut references);
                    for (reference, generic) in references {
                        let target = self.ensure_reference(&reference);
                        let context = if generic {
                            "generic_arg"
                        } else {
                            "parameter_type"
                        };
                        if target != id {
                            self.add_edge(
                                &id,
                                &target,
                                "references",
                                line(node),
                                Some(context),
                                false,
                            );
                        }
                        if promoted
                            && let Some(class) = parent_class
                            && target != class
                        {
                            self.add_edge(
                                class,
                                &target,
                                "references",
                                line(node),
                                Some(if generic { "generic_arg" } else { "field" }),
                                false,
                            );
                        }
                    }
                }
            }
        }
        if let Some(return_type) = return_type_node(node) {
            let mut references = Vec::new();
            collect_type_refs(return_type, self.source, false, &mut references);
            for (reference, generic) in references {
                let target = self.ensure_reference(&reference);
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
                        false,
                    );
                }
            }
        }
        if let Some(body) = node
            .child_by_field_name("body")
            .or_else(|| first_child(node, "compound_statement"))
        {
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
        let labels_ci: HashMap<_, _> = labels
            .iter()
            .map(|(label, id)| (label.to_ascii_lowercase(), id.clone()))
            .collect();
        let functions = std::mem::take(&mut self.functions);
        let mut seen_calls = HashSet::new();
        let mut seen_special = HashSet::new();
        for function in functions {
            self.walk_calls(
                function.body,
                &function.id,
                &labels,
                &labels_ci,
                &mut seen_calls,
                &mut seen_special,
            );
        }
    }

    fn walk_calls(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        labels: &HashMap<String, String>,
        labels_ci: &HashMap<String, String>,
        seen_calls: &mut HashSet<(String, String)>,
        seen_special: &mut HashSet<(String, String, String)>,
    ) {
        if matches!(node.kind(), "function_definition" | "method_declaration") {
            return;
        }
        if CALL_NODES.contains(&node.kind()) {
            let call = php_call(node, self.source);
            if let Some(call) = &call {
                if let Some(target) = labels
                    .get(&call.name)
                    .filter(|target| target.as_str() != caller)
                {
                    if seen_calls.insert((caller.to_owned(), (*target).clone())) {
                        self.add_edge(caller, target, "calls", line(node), Some("call"), false);
                    }
                } else if !call.name.is_empty() {
                    self.extraction.raw_calls_mut().push(RawCall {
                        caller_nid: caller.to_owned(),
                        callee: call.name.clone(),
                        is_member_call: call.member,
                        source_file: self.source_file.clone(),
                        source_location: format!("L{}", line(node)),
                        receiver: Some(None),
                        receiver_type: None,
                        lang: None,
                    });
                }
                if call.name == "config" {
                    self.add_config_edge(node, caller, labels_ci, seen_special);
                }
                if node.kind() == "member_call_expression"
                    && CONTAINER_METHODS.contains(&call.name.as_str())
                {
                    self.add_container_edge(node, labels_ci, seen_special);
                }
            }
        }
        if node.kind() == "scoped_property_access_expression"
            && let Some(class) = class_scope(node, self.source)
            && let Some(target) = labels_ci.get(&class.to_ascii_lowercase())
            && target != caller
            && seen_special.insert((
                caller.to_owned(),
                target.clone(),
                "uses_static_prop".to_owned(),
            ))
        {
            self.add_edge(caller, target, "uses_static_prop", line(node), None, true);
        }
        if node.kind() == "class_constant_access_expression"
            && let Some(class) = class_scope(node, self.source)
            && let Some(target) = labels_ci.get(&class.to_ascii_lowercase())
            && target != caller
            && seen_special.insert((
                caller.to_owned(),
                target.clone(),
                "references_constant".to_owned(),
            ))
        {
            self.add_edge(
                caller,
                target,
                "references_constant",
                line(node),
                None,
                true,
            );
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, labels, labels_ci, seen_calls, seen_special);
        }
    }

    fn add_config_edge(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        labels_ci: &HashMap<String, String>,
        seen: &mut HashSet<(String, String, String)>,
    ) {
        let Some(arguments) = node
            .child_by_field_name("arguments")
            .or_else(|| first_child(node, "arguments"))
        else {
            return;
        };
        let Some(content) = first_descendant(arguments, "string_content") else {
            return;
        };
        let segment = self
            .text(content)
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let target = labels_ci
            .get(&segment)
            .or_else(|| labels_ci.get(&format!("{segment}.php")));
        if let Some(target) = target
            && target != caller
            && seen.insert((
                caller.to_owned(),
                (*target).clone(),
                "uses_config".to_owned(),
            ))
        {
            self.add_edge(caller, target, "uses_config", line(node), None, true);
        }
    }

    fn add_container_edge(
        &mut self,
        node: Node<'tree>,
        labels_ci: &HashMap<String, String>,
        seen: &mut HashSet<(String, String, String)>,
    ) {
        let Some(arguments) = node
            .child_by_field_name("arguments")
            .or_else(|| first_child(node, "arguments"))
        else {
            return;
        };
        let mut classes = Vec::new();
        let mut cursor = arguments.walk();
        for argument in arguments
            .children(&mut cursor)
            .filter(|child| child.kind() == "argument")
        {
            if let Some(constant) = first_child(argument, "class_constant_access_expression")
                && let Some(class) = class_scope(constant, self.source)
            {
                classes.push(class);
            }
            if classes.len() == 2 {
                break;
            }
        }
        if classes.len() != 2 {
            return;
        }
        let source = labels_ci.get(&classes[0].to_ascii_lowercase());
        let target = labels_ci.get(&classes[1].to_ascii_lowercase());
        if let (Some(source), Some(target)) = (source, target)
            && source != target
            && seen.insert((source.clone(), target.clone(), "bound_to".to_owned()))
        {
            self.add_edge(source, target, "bound_to", line(node), None, true);
        }
    }

    fn add_listener_edges(&mut self) {
        let labels: HashMap<String, String> = self
            .extraction
            .nodes
            .iter()
            .filter_map(|node| {
                node.attributes
                    .get("label")
                    .and_then(Value::as_str)
                    .map(|label| (label.to_ascii_lowercase(), node.id.clone()))
            })
            .collect();
        let listeners = std::mem::take(&mut self.pending_listeners);
        let mut seen = HashSet::new();
        for listener in listeners {
            let source = labels.get(&listener.event.to_ascii_lowercase());
            let target = labels.get(&listener.listener.to_ascii_lowercase());
            if let (Some(source), Some(target)) = (source, target)
                && source != target
                && seen.insert((source.clone(), target.clone()))
            {
                self.add_edge(source, target, "listened_by", listener.line, None, true);
            }
        }
    }

    fn ensure_reference(&mut self, name: &str) -> String {
        let local = make_id(&[&self.stem, name]);
        if self.seen_nodes.contains(&local) {
            return local;
        }
        self.add_stub(name, true)
    }

    fn ensure_base(&mut self, name: &str) -> String {
        let local = make_id(&[&self.stem, name]);
        if self.seen_nodes.contains(&local) {
            return local;
        }
        self.add_stub(name, false)
    }

    fn add_stub(&mut self, name: &str, origin: bool) -> String {
        let id = make_id(&[name]);
        if self.seen_nodes.insert(id.clone()) {
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(name.to_owned()));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert("source_file".to_owned(), Value::String(String::new()));
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            if origin {
                attributes.insert(
                    "origin_file".to_owned(),
                    Value::String(self.source_file.clone()),
                );
            }
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

    fn add_node(&mut self, id: String, label: &str, at_line: usize, sourced: bool, callable: bool) {
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(if sourced {
                self.source_file.clone()
            } else {
                String::new()
            }),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(if sourced {
                format!("L{at_line}")
            } else {
                String::new()
            }),
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
        confidence_score: bool,
    ) {
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        attributes.insert(
            "confidence".to_owned(),
            Value::String("EXTRACTED".to_owned()),
        );
        if confidence_score {
            attributes.insert("confidence_score".to_owned(), json!(1.0));
        }
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
}

fn php_call(node: Node<'_>, source: &[u8]) -> Option<Call> {
    match node.kind() {
        "function_call_expression" => node.child_by_field_name("function").map(|name| Call {
            name: text(name, source).to_owned(),
            member: false,
        }),
        "scoped_call_expression" => node.child_by_field_name("scope").map(|name| Call {
            name: text(name, source).to_owned(),
            member: false,
        }),
        "member_call_expression" => node.child_by_field_name("name").map(|name| Call {
            name: text(name, source).to_owned(),
            member: true,
        }),
        _ => None,
    }
}

fn collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    match node.kind() {
        "primitive_type" => {}
        "named_type" => {
            let mut cursor = node.walk();
            if let Some(name) = node
                .children(&mut cursor)
                .find(|child| matches!(child.kind(), "name" | "qualified_name"))
            {
                output.push((php_name(text(name, source)).to_owned(), generic));
            }
        }
        "name" | "qualified_name" => {
            output.push((php_name(text(node, source)).to_owned(), generic));
        }
        "nullable_type" | "union_type" | "intersection_type" | "optional_type" => {
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

fn return_type_node(node: Node<'_>) -> Option<Node<'_>> {
    let mut saw_parameters = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "formal_parameters" {
            saw_parameters = true;
            continue;
        }
        if saw_parameters && child.is_named() && TYPE_NODES.contains(&child.kind()) {
            return Some(child);
        }
    }
    None
}

fn class_scope(node: Node<'_>, source: &[u8]) -> Option<String> {
    node.child_by_field_name("scope")
        .or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor).find(|child| {
                child.is_named() && matches!(child.kind(), "name" | "qualified_name" | "identifier")
            })
        })
        .map(|scope| text(scope, source).to_owned())
}

fn php_name(value: &str) -> &str {
    value.rsplit('\\').next().unwrap_or_default()
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
