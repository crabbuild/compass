use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::builtins::is_language_builtin_global;
use crate::{Extraction, RawCall, make_id};

const PREDECLARED_TYPES: &[&str] = &[
    "bool",
    "byte",
    "complex64",
    "complex128",
    "error",
    "float32",
    "float64",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "rune",
    "string",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "any",
    "comparable",
];

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    GoState::new(path, source).run(root)
}

#[derive(Clone)]
struct LexicalBinding {
    name: String,
    active_from: usize,
    active_until: usize,
}

struct GoState<'source, 'tree> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    package_scope: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
    function_bodies: Vec<(String, Node<'tree>, Vec<LexicalBinding>)>,
    imported_packages: HashSet<String>,
}

impl<'source, 'tree> GoState<'source, 'tree> {
    fn new(path: &Path, source: &'source [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let stem = crate::file_stem(path);
        let package_scope = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or(&stem)
            .to_owned();
        let file_id = make_id(&[&source_file]);
        let mut state = Self {
            source,
            source_file,
            stem,
            package_scope,
            file_id,
            extraction: Extraction::default(),
            seen: HashSet::new(),
            function_bodies: Vec::new(),
            imported_packages: HashSet::new(),
        };
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        state.add_node(&state.file_id.clone(), label, 1);
        state
    }

    fn run(mut self, root: Node<'tree>) -> Extraction {
        self.walk_type_declarations(root);
        self.walk(root);
        self.walk_calls();
        let valid = &self.seen;
        self.extraction.edges.retain(|edge| {
            valid.contains(&edge.source)
                && (valid.contains(&edge.target)
                    || matches!(
                        edge.attributes.get("relation").and_then(Value::as_str),
                        Some("imports" | "imports_from")
                    ))
        });
        self.extraction
    }

    fn walk_type_declarations(&mut self, node: Node<'tree>) {
        if node.kind() == "type_declaration" {
            self.add_types(node);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_type_declarations(child);
        }
    }

    fn walk(&mut self, node: Node<'tree>) {
        match node.kind() {
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node);
                    let at = line(node);
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &format!("{name}()"), at);
                    self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
                    self.add_function_references(node, &id, at);
                    if let Some(body) = node.child_by_field_name("body") {
                        self.function_bodies
                            .push((id, body, lexical_bindings(node, self.source)));
                    }
                }
                return;
            }
            "method_declaration" => {
                self.add_method(node);
                return;
            }
            "type_declaration" => {
                return;
            }
            "import_declaration" => {
                self.add_imports(node);
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child);
        }
    }

    fn add_method(&mut self, node: Node<'tree>) {
        let receiver_type = node.child_by_field_name("receiver").and_then(|receiver| {
            let mut cursor = receiver.walk();
            receiver.children(&mut cursor).find_map(|parameter| {
                (parameter.kind() == "parameter_declaration")
                    .then(|| parameter.child_by_field_name("type"))
                    .flatten()
                    .map(|kind| self.text(kind).trim_start_matches('*').trim().to_owned())
            })
        });
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let at = line(node);
        let id = if let Some(receiver) = receiver_type {
            let parent = make_id(&[&self.package_scope, &receiver]);
            self.add_node(&parent, &receiver, at);
            let id = make_id(&[&parent, &name]);
            self.add_node(&id, &format!(".{name}()"), at);
            self.add_edge(&parent, &id, "method", at, None);
            id
        } else {
            let id = make_id(&[&self.stem, &name]);
            self.add_node(&id, &format!("{name}()"), at);
            self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
            id
        };
        self.add_function_references(node, &id, at);
        if let Some(body) = node.child_by_field_name("body") {
            self.function_bodies
                .push((id, body, lexical_bindings(node, self.source)));
        }
    }

    fn add_types(&mut self, node: Node<'tree>) {
        self.add_type_specs(node);
    }

    fn add_type_specs(&mut self, node: Node<'tree>) {
        if node.kind() == "type_spec" {
            self.add_type_spec(node);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.add_type_specs(child);
        }
    }

    fn add_type_spec(&mut self, node: Node<'tree>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let at = line(node);
        let id = make_id(&[&self.package_scope, &name]);
        self.add_node(&id, &name, at);
        let symbol_kind = if has_descendant_kind(node, "struct_type") {
            "struct"
        } else if has_descendant_kind(node, "interface_type") {
            "interface"
        } else {
            "type_alias"
        };
        if let Some(record) = self
            .extraction
            .nodes
            .iter_mut()
            .find(|record| record.id == id)
        {
            record
                .attributes
                .insert("symbol_kind".into(), Value::String(symbol_kind.to_owned()));
        }
        self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
        let mut body_cursor = node.walk();
        for body in node.children(&mut body_cursor) {
            match body.kind() {
                "struct_type" => self.add_struct_references(body, &id),
                "interface_type" => self.add_interface_references(body, &id),
                _ => {}
            }
        }
    }

    fn add_struct_references(&mut self, body: Node<'tree>, type_id: &str) {
        let mut cursor = body.walk();
        for list in body.children(&mut cursor) {
            if list.kind() != "field_declaration_list" {
                continue;
            }
            let mut list_cursor = list.walk();
            for field in list.children(&mut list_cursor) {
                if field.kind() != "field_declaration" {
                    continue;
                }
                let mut field_cursor = field.walk();
                let children: Vec<_> = field.children(&mut field_cursor).collect();
                let has_name = children
                    .iter()
                    .any(|child| child.kind() == "field_identifier");
                let type_node = field.child_by_field_name("type").or_else(|| {
                    children
                        .iter()
                        .copied()
                        .find(|child| child.is_named() && child.kind() != "field_identifier")
                });
                let mut refs = Vec::new();
                collect_type_refs(type_node, self.source, false, &mut refs);
                for (name, generic) in refs {
                    let target = self.ensure_named_node(&name);
                    if target == type_id {
                        continue;
                    }
                    if !has_name && !generic {
                        self.add_edge(type_id, &target, "embeds", line(field), None);
                    } else {
                        let context = if generic { "generic_arg" } else { "field" };
                        self.add_edge(type_id, &target, "references", line(field), Some(context));
                    }
                }
            }
        }
    }

    fn add_interface_references(&mut self, body: Node<'tree>, type_id: &str) {
        let mut cursor = body.walk();
        for element in body.children(&mut cursor) {
            if element.kind() != "type_elem" {
                continue;
            }
            let mut refs = Vec::new();
            let mut element_cursor = element.walk();
            for child in element
                .children(&mut element_cursor)
                .filter(|child| child.is_named())
            {
                collect_type_refs(Some(child), self.source, false, &mut refs);
            }
            for (name, generic) in refs {
                let target = self.ensure_named_node(&name);
                if target == type_id {
                    continue;
                }
                if generic {
                    self.add_edge(
                        type_id,
                        &target,
                        "references",
                        line(element),
                        Some("generic_arg"),
                    );
                } else {
                    self.add_edge(type_id, &target, "embeds", line(element), None);
                }
            }
        }
    }

    fn add_function_references(&mut self, node: Node<'tree>, id: &str, at: usize) {
        if let Some(parameters) = node.child_by_field_name("parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters.children(&mut cursor) {
                if parameter.kind() != "parameter_declaration" {
                    continue;
                }
                self.add_type_references(
                    parameter.child_by_field_name("type"),
                    id,
                    at,
                    "parameter_type",
                );
            }
        }
        if let Some(result) = node.child_by_field_name("result") {
            if result.kind() == "parameter_list" {
                let mut cursor = result.walk();
                for parameter in result.children(&mut cursor) {
                    if parameter.kind() != "parameter_declaration" {
                        continue;
                    }
                    let type_node = parameter.child_by_field_name("type").or_else(|| {
                        let mut inner = parameter.walk();
                        parameter
                            .children(&mut inner)
                            .find(|child| child.is_named())
                    });
                    self.add_type_references(type_node, id, at, "return_type");
                }
            } else {
                self.add_type_references(Some(result), id, at, "return_type");
            }
        }
    }

    fn add_type_references(
        &mut self,
        node: Option<Node<'tree>>,
        id: &str,
        at: usize,
        context: &str,
    ) {
        let mut refs = Vec::new();
        collect_type_refs(node, self.source, false, &mut refs);
        for (name, generic) in refs {
            let target = self.ensure_named_node(&name);
            if target != id {
                self.add_edge(
                    id,
                    &target,
                    "references",
                    at,
                    Some(if generic { "generic_arg" } else { context }),
                );
            }
        }
    }

    fn add_imports(&mut self, node: Node<'tree>) {
        let mut specs = Vec::new();
        collect_kind(node, "import_spec", &mut specs);
        for spec in specs {
            let Some(path) = spec.child_by_field_name("path") else {
                continue;
            };
            let raw = self.text(path).trim_matches('"').to_owned();
            let target = make_id(&["go", "pkg", &raw]);
            self.add_edge(
                &self.file_id.clone(),
                &target,
                "imports_from",
                line(spec),
                Some("import"),
            );
            let local = spec
                .child_by_field_name("name")
                .map(|name| self.text(name))
                .unwrap_or_else(|| raw.rsplit('/').next().unwrap_or_default().to_owned());
            if !matches!(local.as_str(), "" | "_" | ".") {
                self.imported_packages.insert(local);
            }
        }
    }

    fn walk_calls(&mut self) {
        let mut labels = HashMap::new();
        for node in &self.extraction.nodes {
            let label = node
                .attributes
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            labels.insert(
                label
                    .trim_matches(|character| character == '(' || character == ')')
                    .trim_start_matches('.')
                    .to_owned(),
                node.id.clone(),
            );
        }
        let mut seen_pairs = HashSet::new();
        for (caller, body, bindings) in self.function_bodies.clone() {
            self.walk_calls_in(body, &caller, &labels, &bindings, &mut seen_pairs);
        }
    }

    fn walk_calls_in(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        labels: &HashMap<String, String>,
        lexical_bindings: &[LexicalBinding],
        seen_pairs: &mut HashSet<(String, String, usize, usize)>,
    ) {
        if matches!(node.kind(), "function_declaration" | "method_declaration") {
            return;
        }
        if node.kind() == "call_expression"
            && let Some(function) = node.child_by_field_name("function")
        {
            let (callee, member) = if function.kind() == "identifier" {
                (Some(self.text(function)), false)
            } else if function.kind() == "selector_expression" {
                let callee = function
                    .child_by_field_name("field")
                    .map(|field| self.text(field));
                let receiver = function
                    .child_by_field_name("operand")
                    .map(|operand| self.text(operand))
                    .unwrap_or_default();
                (callee, !self.imported_packages.contains(&receiver))
            } else {
                (None, false)
            };
            if let Some(callee) = callee {
                let is_lexically_bound = function.kind() == "identifier"
                    && lexical_bindings.iter().any(|binding| {
                        binding.name == callee
                            && binding.active_from <= function.start_byte()
                            && function.start_byte() < binding.active_until
                    });
                if is_lexically_bound {
                    // A local callback or function value shadows repository-level symbols.
                } else if let Some(target) = labels.get(&callee).filter(|target| *target != caller)
                {
                    let pair = (
                        caller.to_owned(),
                        target.clone(),
                        node.start_byte(),
                        node.end_byte(),
                    );
                    if seen_pairs.insert(pair) {
                        self.add_edge(caller, target, "calls", line(node), Some("call"));
                        crate::facts::stamp_last_edge_range(&mut self.extraction, node);
                    }
                } else if !is_language_builtin_global("go", &callee) {
                    self.extraction.raw_calls_mut().push(RawCall {
                        caller_nid: caller.to_owned(),
                        callee,
                        is_member_call: Some(member),
                        source_file: self.source_file.clone(),
                        source_location: format!("L{}", line(node)),
                        receiver: None,
                        receiver_type: None,
                        lang: None,
                        extensions: crate::facts::node_range(node),
                    });
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls_in(child, caller, labels, lexical_bindings, seen_pairs);
        }
    }

    fn ensure_named_node(&mut self, name: &str) -> String {
        let local = make_id(&[&self.package_scope, name]);
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

    fn text(&self, node: Node<'_>) -> String {
        node.utf8_text(self.source).unwrap_or_default().to_owned()
    }
}

fn lexical_bindings(callable: Node<'_>, source: &[u8]) -> Vec<LexicalBinding> {
    fn collect_binding_identifiers(node: Node<'_>, source: &[u8], names: &mut Vec<String>) {
        if node.kind() == "identifier"
            && let Ok(name) = node.utf8_text(source)
            && !name.is_empty()
        {
            names.push(name.to_owned());
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            collect_binding_identifiers(child, source, names);
        }
    }

    fn push_bindings(
        names_node: Node<'_>,
        source: &[u8],
        active_from: usize,
        active_until: usize,
        bindings: &mut Vec<LexicalBinding>,
    ) {
        let mut names = Vec::new();
        collect_binding_identifiers(names_node, source, &mut names);
        names.sort();
        names.dedup();
        bindings.extend(names.into_iter().map(|name| LexicalBinding {
            name,
            active_from,
            active_until,
        }));
    }

    fn walk(node: Node<'_>, source: &[u8], scope_end: usize, bindings: &mut Vec<LexicalBinding>) {
        if node.kind() == "func_literal"
            && let Some(body) = node.child_by_field_name("body")
        {
            if let Some(parameters) = node.child_by_field_name("parameters") {
                push_bindings(
                    parameters,
                    source,
                    body.start_byte(),
                    body.end_byte(),
                    bindings,
                );
            }
            walk(body, source, body.end_byte(), bindings);
            return;
        }
        let scope_end = if node.kind() == "block" {
            node.end_byte()
        } else {
            scope_end
        };
        match node.kind() {
            "var_spec" => {
                let type_node = node.child_by_field_name("type");
                let value_node = node.child_by_field_name("value");
                let mut cursor = node.walk();
                for child in node.children(&mut cursor).filter(|child| {
                    child.is_named() && Some(*child) != type_node && Some(*child) != value_node
                }) {
                    if child.kind() == "identifier" {
                        push_bindings(child, source, node.end_byte(), scope_end, bindings);
                    }
                }
            }
            "short_var_declaration" => {
                if let Some(left) = node.child_by_field_name("left") {
                    push_bindings(left, source, node.end_byte(), scope_end, bindings);
                }
            }
            "for_statement" => {
                let body = node.child_by_field_name("body");
                let mut cursor = node.walk();
                if let Some(range) = node
                    .children(&mut cursor)
                    .find(|child| child.kind() == "range_clause")
                    && range.utf8_text(source).unwrap_or_default().contains(":=")
                    && let Some(left) = range.child_by_field_name("left")
                {
                    push_bindings(
                        left,
                        source,
                        range.end_byte(),
                        body.map_or(node.end_byte(), |body| body.end_byte()),
                        bindings,
                    );
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            walk(child, source, scope_end, bindings);
        }
    }

    let Some(body) = callable.child_by_field_name("body") else {
        return Vec::new();
    };
    let mut bindings = Vec::new();
    for field in ["receiver", "parameters"] {
        if let Some(parameters) = callable.child_by_field_name(field) {
            push_bindings(
                parameters,
                source,
                body.start_byte(),
                body.end_byte(),
                &mut bindings,
            );
        }
    }
    walk(body, source, body.end_byte(), &mut bindings);
    bindings
}

fn collect_type_refs(
    node: Option<Node<'_>>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    let Some(node) = node else { return };
    match node.kind() {
        "type_identifier" => {
            let name = node.utf8_text(source).unwrap_or_default();
            if !name.is_empty() && !PREDECLARED_TYPES.contains(&name) {
                output.push((name.to_owned(), generic));
            }
            return;
        }
        "qualified_type" => {
            let name = node
                .utf8_text(source)
                .unwrap_or_default()
                .rsplit('.')
                .next()
                .unwrap_or_default();
            if !name.is_empty() && !PREDECLARED_TYPES.contains(&name) {
                output.push((name.to_owned(), generic));
            }
            return;
        }
        "generic_type" => {
            collect_type_refs(node.child_by_field_name("type"), source, generic, output);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_arguments" {
                    let mut args = child.walk();
                    for argument in child
                        .children(&mut args)
                        .filter(|argument| argument.is_named())
                    {
                        collect_type_refs(Some(argument), source, true, output);
                    }
                }
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_type_refs(Some(child), source, generic, output);
    }
}

fn collect_kind<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            output.push(child);
        } else {
            collect_kind(child, kind, output);
        }
    }
}

fn has_descendant_kind(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == kind || has_descendant_kind(child, kind))
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
