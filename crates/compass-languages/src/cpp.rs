use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use compass_model::{EdgeRecord, NodeRecord};
use serde_json::{Map, Value, json};
use tree_sitter::Node;

use crate::{Extraction, RawCall, file_stem, make_id};

const TYPE_DECLARATIONS: &[&str] = &["class_specifier", "struct_specifier"];
const TYPE_WRAPPERS: &[&str] = &[
    "type_descriptor",
    "pointer_declarator",
    "reference_declarator",
    "array_declarator",
    "type_qualifier",
    "abstract_pointer_declarator",
    "abstract_reference_declarator",
    "abstract_array_declarator",
];

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
        functions: Vec::new(),
        type_table: HashMap::new(),
    };
    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
        false,
    );
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        state.walk(child, None);
    }
    state.add_calls();
    if !state.type_table.is_empty() {
        state.extraction.extensions.insert(
            "cpp_type_table".to_owned(),
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
    type_table: HashMap<String, String>,
}

impl<'tree> State<'_, 'tree> {
    fn walk(&mut self, node: Node<'tree>, parent_type: Option<&str>) {
        let kind = node.kind();
        if kind == "preproc_include" {
            self.add_import(node);
            return;
        }
        if TYPE_DECLARATIONS.contains(&kind) {
            self.add_type(node);
            return;
        }
        if kind == "field_declaration" && parent_type.is_some() {
            self.add_fields(node, parent_type.unwrap_or_default());
            return;
        }
        if kind == "function_definition" {
            self.add_function(node, parent_type);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, None);
        }
    }

    fn add_import(&mut self, node: Node<'tree>) {
        let Some(path) = node.child_by_field_name("path") else {
            return;
        };
        let raw = self.text(path);
        let clean = raw.trim_matches(['<', '>', '\'', '"']);
        let target_id = if path.kind() != "system_lib_string" {
            Path::new(&self.source_file)
                .parent()
                .and_then(|parent| fs::canonicalize(parent.join(clean)).ok())
                .filter(|candidate| candidate.is_file())
                .map(|candidate| make_id(&[&candidate.to_string_lossy()]))
        } else {
            None
        }
        .unwrap_or_else(|| {
            let target = Path::new(clean)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(clean);
            make_id(&[target])
        });
        if !target_id.is_empty() {
            self.add_edge(
                &self.file_id.clone(),
                &target_id,
                "imports",
                line(node),
                Some("import"),
            );
        }
    }

    fn add_type(&mut self, node: Node<'tree>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_owned();
        if name.is_empty() {
            return;
        }
        let id = make_id(&[&self.stem, &name]);
        self.add_node(id.clone(), &name, line(node), true);
        self.add_edge(&self.file_id.clone(), &id, "contains", line(node), None);
        self.add_bases(node, &id);
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                self.walk(child, Some(&id));
            }
        }
    }

    fn add_bases(&mut self, node: Node<'tree>, class_id: &str) {
        let mut cursor = node.walk();
        for clause in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "base_class_clause")
        {
            let mut clause_cursor = clause.walk();
            for base in clause.children(&mut clause_cursor) {
                let (name, arguments) = match base.kind() {
                    "type_identifier" => (self.text(base).to_owned(), None),
                    "qualified_identifier" => {
                        let name = base
                            .child_by_field_name("name")
                            .map_or_else(|| self.text(base), |name| self.text(name));
                        (name.to_owned(), None)
                    }
                    "template_type" => {
                        let name = base
                            .child_by_field_name("name")
                            .map_or_else(|| self.text(base), |name| self.text(name));
                        (name.to_owned(), base.child_by_field_name("arguments"))
                    }
                    _ => continue,
                };
                if name.is_empty() {
                    continue;
                }
                let target = self.ensure_named(&name);
                self.add_edge(class_id, &target, "inherits", line(node), None);
                if let Some(arguments) = arguments {
                    let mut references = Vec::new();
                    let mut arguments_cursor = arguments.walk();
                    for argument in arguments
                        .children(&mut arguments_cursor)
                        .filter(|argument| argument.is_named())
                    {
                        collect_type_refs(argument, self.source, true, &mut references);
                    }
                    for (reference, _) in references {
                        let target = self.ensure_named(&reference);
                        if target != class_id {
                            self.add_edge(
                                class_id,
                                &target,
                                "references",
                                line(node),
                                Some("generic_arg"),
                            );
                        }
                    }
                }
            }
        }
    }

    fn add_fields(&mut self, node: Node<'tree>, class_id: &str) {
        let declarators = children_by_field_name(node, "declarator");
        let is_method = declarators.iter().copied().any(is_function_declarator);
        if !is_method && let Some(type_node) = node.child_by_field_name("type") {
            let mut references = Vec::new();
            collect_type_refs(type_node, self.source, false, &mut references);
            for (reference, generic) in references {
                let target = self.ensure_named(&reference);
                if target != class_id {
                    self.add_edge(
                        class_id,
                        &target,
                        "references",
                        line(node),
                        Some(if generic { "generic_arg" } else { "field" }),
                    );
                }
            }
        }
        for declarator in declarators {
            let Some(name) = cpp_name(declarator, self.source) else {
                continue;
            };
            let id = make_id(&[class_id, &name]);
            self.add_node(id.clone(), &name, line(declarator), false);
            self.add_edge(class_id, &id, "defines", line(declarator), Some("field"));
        }
    }

    fn add_function(&mut self, node: Node<'tree>, parent_type: Option<&str>) {
        let Some(declarator) = node.child_by_field_name("declarator") else {
            return;
        };
        let Some(name) = cpp_name(declarator, self.source) else {
            return;
        };
        if make_id(&[&name]).is_empty() {
            return;
        }
        let id = parent_type.map_or_else(
            || make_id(&[&self.stem, &name]),
            |parent| make_id(&[parent, &name]),
        );
        let label = if parent_type.is_some() {
            format!(".{name}()")
        } else {
            format!("{name}()")
        };
        self.add_node(id.clone(), &label, line(node), true);
        let owner = parent_type.unwrap_or(&self.file_id).to_owned();
        self.add_edge(
            &owner,
            &id,
            if parent_type.is_some() {
                "method"
            } else {
                "contains"
            },
            line(node),
            None,
        );
        if let Some(return_type) = node.child_by_field_name("type") {
            self.add_function_type_references(return_type, &id, "return_type", line(node));
        }
        let mut function_declarator = Some(declarator);
        while function_declarator.is_some_and(|declarator| {
            matches!(
                declarator.kind(),
                "pointer_declarator" | "reference_declarator"
            )
        }) {
            function_declarator = function_declarator
                .and_then(|declarator| declarator.child_by_field_name("declarator"));
        }
        if let Some(function_declarator) = function_declarator
            && function_declarator.kind() == "function_declarator"
            && let Some(parameters) = function_declarator.child_by_field_name("parameters")
        {
            let mut cursor = parameters.walk();
            for parameter in parameters
                .children(&mut cursor)
                .filter(|parameter| parameter.kind() == "parameter_declaration")
            {
                if let Some(parameter_type) = parameter.child_by_field_name("type") {
                    self.add_function_type_references(
                        parameter_type,
                        &id,
                        "parameter_type",
                        line(node),
                    );
                }
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            collect_local_types(body, self.source, &mut self.type_table);
            self.functions.push(FunctionBody { id, body });
        }
    }

    fn add_function_type_references(
        &mut self,
        node: Node<'tree>,
        function_id: &str,
        context: &str,
        at_line: usize,
    ) {
        let mut references = Vec::new();
        collect_type_refs(node, self.source, false, &mut references);
        for (reference, generic) in references {
            let target = self.ensure_named(&reference);
            if target != function_id {
                self.add_edge(
                    function_id,
                    &target,
                    "references",
                    at_line,
                    Some(if generic { "generic_arg" } else { context }),
                );
            }
        }
    }

    fn add_calls(&mut self) {
        let labels: HashMap<String, String> = self
            .extraction
            .nodes
            .iter()
            .filter(|node| node.attributes.get("type").and_then(Value::as_str) != Some("namespace"))
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
        if node.kind() == "function_definition" {
            return;
        }
        if node.kind() == "call_expression"
            && let Some(call) = cpp_call(node, self.source)
        {
            let target = labels
                .get(&call.name)
                .filter(|target| target.as_str() != caller);
            if let Some(target) = target {
                if seen.insert((caller.to_owned(), (*target).clone())) {
                    self.add_edge(caller, target, "calls", line(node), Some("call"));
                }
            } else if !call.name.is_empty() {
                self.extraction.raw_calls_mut().push(RawCall {
                    caller_nid: caller.to_owned(),
                    callee: call.name,
                    is_member_call: Some(call.member),
                    source_file: self.source_file.clone(),
                    source_location: format!("L{}", line(node)),
                    receiver: Some(call.receiver),
                    receiver_type: None,
                    lang: Some("cpp".to_owned()),
                    extensions: Map::new(),
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

    fn add_node(&mut self, id: String, label: &str, at_line: usize, callable: bool) {
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

fn cpp_call(node: Node<'_>, source: &[u8]) -> Option<Call> {
    let function = node.child_by_field_name("function")?;
    match function.kind() {
        "identifier" => Some(Call {
            name: text(function, source).to_owned(),
            member: false,
            receiver: None,
        }),
        "field_expression" => {
            let name = function.child_by_field_name("field")?;
            let object = function.child_by_field_name("argument");
            let receiver = object.and_then(|object| match object.kind() {
                "identifier" => Some(text(object, source).to_owned()),
                "this" => Some("this".to_owned()),
                _ => None,
            });
            Some(Call {
                name: text(name, source).to_owned(),
                member: true,
                receiver,
            })
        }
        "qualified_identifier" => {
            let name = function.child_by_field_name("name")?;
            let receiver = function
                .child_by_field_name("scope")
                .map(|scope| text(scope, source).to_owned());
            Some(Call {
                name: text(name, source).to_owned(),
                member: true,
                receiver,
            })
        }
        _ => None,
    }
}

fn cpp_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if matches!(
        node.kind(),
        "identifier" | "field_identifier" | "destructor_name" | "operator_name"
    ) {
        return Some(text(node, source).to_owned());
    }
    if node.kind() == "qualified_identifier" {
        return Some(text(node, source).to_owned());
    }
    if let Some(declarator) = node.child_by_field_name("declarator") {
        return cpp_name(declarator, source);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "identifier")
        .map(|child| text(child, source).to_owned())
}

fn collect_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    if matches!(
        node.kind(),
        "primitive_type" | "sized_type_specifier" | "auto" | "placeholder_type_specifier"
    ) {
        return;
    }
    match node.kind() {
        "type_identifier" => output.push((text(node, source).to_owned(), generic)),
        "qualified_identifier" => {
            if let Some(name) = node.child_by_field_name("name") {
                collect_type_refs(name, source, generic, output);
            }
        }
        "template_type" => {
            if let Some(name) = node.child_by_field_name("name") {
                output.push((text(name, source).to_owned(), generic));
            }
            if let Some(arguments) = node.child_by_field_name("arguments") {
                let mut cursor = arguments.walk();
                for argument in arguments
                    .children(&mut cursor)
                    .filter(|argument| argument.is_named())
                {
                    collect_type_refs(argument, source, true, output);
                }
            }
        }
        kind if TYPE_WRAPPERS.contains(&kind) => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                collect_type_refs(child, source, generic, output);
            }
        }
        _ => {}
    }
}

fn collect_local_types(node: Node<'_>, source: &[u8], table: &mut HashMap<String, String>) {
    if node.kind() == "function_definition" {
        return;
    }
    if node.kind() == "declaration"
        && let Some(type_node) = node.child_by_field_name("type")
        && matches!(type_node.kind(), "type_identifier" | "qualified_identifier")
    {
        let type_name = text(type_node, source)
            .rsplit("::")
            .next()
            .unwrap_or_default()
            .trim();
        let declarators: Vec<_> = {
            let mut cursor = node.walk();
            node.children(&mut cursor)
                .filter(|child| {
                    matches!(
                        child.kind(),
                        "identifier"
                            | "pointer_declarator"
                            | "reference_declarator"
                            | "init_declarator"
                    )
                })
                .collect()
        };
        if type_name.starts_with(char::is_uppercase)
            && declarators.len() == 1
            && let Some(name) = plain_declarator_name(declarators[0], source)
        {
            table.entry(name).or_insert_with(|| type_name.to_owned());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_local_types(child, source, table);
    }
}

fn plain_declarator_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(text(node, source).to_owned());
    }
    if matches!(
        node.kind(),
        "pointer_declarator" | "reference_declarator" | "init_declarator"
    ) {
        return node
            .child_by_field_name("declarator")
            .or_else(|| {
                let mut cursor = node.walk();
                node.children(&mut cursor).find(|child| {
                    matches!(
                        child.kind(),
                        "identifier" | "pointer_declarator" | "reference_declarator"
                    )
                })
            })
            .and_then(|declarator| plain_declarator_name(declarator, source));
    }
    None
}

fn is_function_declarator(node: Node<'_>) -> bool {
    if node.kind() == "function_declarator" {
        return true;
    }
    if matches!(node.kind(), "pointer_declarator" | "reference_declarator") {
        let mut cursor = node.walk();
        return node
            .children(&mut cursor)
            .any(|child| child.kind() == "function_declarator");
    }
    false
}

fn children_by_field_name<'tree>(node: Node<'tree>, field: &str) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.children_by_field_name(field, &mut cursor).collect()
}

fn text<'source>(node: Node<'_>, source: &'source [u8]) -> &'source str {
    node.utf8_text(source).unwrap_or_default()
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}
