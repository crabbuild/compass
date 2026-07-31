use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::{Extraction, make_id};

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

struct GoTypeRef {
    name: String,
    qualifier: Option<String>,
    generic: bool,
}

struct GoState<'source> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    package_scope: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
    imported_packages: HashMap<String, String>,
}

impl<'source> GoState<'source> {
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
            imported_packages: HashMap::new(),
        };
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        state.add_node(&state.file_id.clone(), label, 1);
        state
    }

    fn run(mut self, root: Node<'_>) -> Extraction {
        self.prescan_imports(root);
        self.walk_type_declarations(root);
        self.walk(root);
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

    fn prescan_imports(&mut self, node: Node<'_>) {
        if node.kind() == "import_declaration" {
            let mut specs = Vec::new();
            collect_kind(node, "import_spec", &mut specs);
            for spec in specs {
                if let Some((local, raw)) = go_import_binding(spec, self.source) {
                    self.imported_packages.insert(local, raw);
                }
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.prescan_imports(child);
        }
    }

    fn walk_type_declarations(&mut self, node: Node<'_>) {
        if node.kind() == "type_declaration" {
            self.add_types(node);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_type_declarations(child);
        }
    }

    fn walk(&mut self, node: Node<'_>) {
        match node.kind() {
            "function_declaration" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node);
                    let at = line(node);
                    let id = make_id(&[&self.stem, &name]);
                    self.add_node(&id, &format!("{name}()"), at);
                    self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
                    self.add_function_references(node, &id, at);
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

    fn add_method(&mut self, node: Node<'_>) {
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
    }

    fn add_types(&mut self, node: Node<'_>) {
        self.add_type_specs(node);
    }

    fn add_type_specs(&mut self, node: Node<'_>) {
        if node.kind() == "type_spec" {
            self.add_type_spec(node);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.add_type_specs(child);
        }
    }

    fn add_type_spec(&mut self, node: Node<'_>) {
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

    fn add_struct_references(&mut self, body: Node<'_>, type_id: &str) {
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
                for reference in refs {
                    let target =
                        self.ensure_named_node(&reference.name, reference.qualifier.as_deref());
                    if target == type_id {
                        continue;
                    }
                    if !has_name && !reference.generic {
                        self.add_edge(type_id, &target, "embeds", line(field), None);
                    } else {
                        let context = if reference.generic {
                            "generic_arg"
                        } else {
                            "field"
                        };
                        self.add_edge(type_id, &target, "references", line(field), Some(context));
                    }
                }
            }
        }
    }

    fn add_interface_references(&mut self, body: Node<'_>, type_id: &str) {
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
            for reference in refs {
                let target =
                    self.ensure_named_node(&reference.name, reference.qualifier.as_deref());
                if target == type_id {
                    continue;
                }
                if reference.generic {
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

    fn add_function_references(&mut self, node: Node<'_>, id: &str, at: usize) {
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

    fn add_type_references(&mut self, node: Option<Node<'_>>, id: &str, at: usize, context: &str) {
        let mut refs = Vec::new();
        collect_type_refs(node, self.source, false, &mut refs);
        for reference in refs {
            let target = self.ensure_named_node(&reference.name, reference.qualifier.as_deref());
            if target != id {
                self.add_edge(
                    id,
                    &target,
                    "references",
                    at,
                    Some(if reference.generic {
                        "generic_arg"
                    } else {
                        context
                    }),
                );
            }
        }
    }

    fn add_imports(&mut self, node: Node<'_>) {
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
            if let Some((local, _)) = go_import_binding(spec, self.source) {
                self.imported_packages.insert(local, raw);
            }
        }
    }

    fn ensure_named_node(&mut self, name: &str, qualifier: Option<&str>) -> String {
        if let Some(qualifier) = qualifier {
            let imported = self
                .imported_packages
                .get(qualifier)
                .map_or(qualifier, String::as_str);
            let target_package = imported.rsplit('/').next().unwrap_or(qualifier);
            let id = make_id(&[imported, name]);
            if self.seen.insert(id.clone()) {
                let mut attributes = Map::new();
                attributes.insert("label".into(), Value::String(name.to_owned()));
                attributes.insert("file_type".into(), Value::String("code".into()));
                attributes.insert("symbol_kind".into(), Value::String("symbol".into()));
                attributes.insert("source_file".into(), Value::String(String::new()));
                attributes.insert("source_location".into(), Value::String(String::new()));
                attributes.insert(
                    "origin_file".into(),
                    Value::String(self.source_file.clone()),
                );
                attributes.insert(
                    "qualified_name".into(),
                    Value::String(format!("{imported}.{name}")),
                );
                attributes.insert("package".into(), Value::String(imported.to_owned()));
                attributes.insert("go_import_path".into(), Value::String(imported.to_owned()));
                attributes.insert(
                    "go_target_package".into(),
                    Value::String(target_package.to_owned()),
                );
                self.extraction.nodes.push(NodeRecord {
                    id: id.clone(),
                    attributes,
                });
            }
            return id;
        }
        let local = make_id(&[&self.package_scope, name]);
        if self.seen.contains(&local) {
            return local;
        }
        let id = make_id(&[name]);
        if self.seen.insert(id.clone()) {
            let mut attributes = Map::new();
            attributes.insert("label".into(), Value::String(name.to_owned()));
            attributes.insert("file_type".into(), Value::String("code".into()));
            attributes.insert("symbol_kind".into(), Value::String("symbol".into()));
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
        attributes.insert("package".into(), Value::String(self.package_scope.clone()));
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

fn collect_type_refs(
    node: Option<Node<'_>>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<GoTypeRef>,
) {
    let Some(node) = node else { return };
    match node.kind() {
        "type_identifier" => {
            let name = node.utf8_text(source).unwrap_or_default();
            if !name.is_empty() && !PREDECLARED_TYPES.contains(&name) {
                output.push(GoTypeRef {
                    name: name.to_owned(),
                    qualifier: None,
                    generic,
                });
            }
            return;
        }
        "qualified_type" => {
            let raw = node.utf8_text(source).unwrap_or_default();
            let (qualifier, name) = raw
                .rsplit_once('.')
                .map_or((None, raw), |(qualifier, name)| (Some(qualifier), name));
            if !name.is_empty() && !PREDECLARED_TYPES.contains(&name) {
                output.push(GoTypeRef {
                    name: name.to_owned(),
                    qualifier: qualifier.map(str::to_owned),
                    generic,
                });
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

fn go_import_binding(spec: Node<'_>, source: &[u8]) -> Option<(String, String)> {
    let raw = spec
        .child_by_field_name("path")?
        .utf8_text(source)
        .ok()?
        .trim_matches('"')
        .to_owned();
    let local = spec
        .child_by_field_name("name")
        .and_then(|name| name.utf8_text(source).ok())
        .map(str::to_owned)
        .unwrap_or_else(|| raw.rsplit('/').next().unwrap_or_default().to_owned());
    (!raw.is_empty() && !matches!(local.as_str(), "" | "_" | ".")).then_some((local, raw))
}

fn has_descendant_kind(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == kind || has_descendant_kind(child, kind))
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
