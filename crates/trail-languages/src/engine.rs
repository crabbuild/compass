use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::{Node, Parser};

use crate::config::{GenericConfig, generic_config};
use crate::{
    ExtractError, Extraction, ExtractorKind, LanguageSpec, RawCall, Registry, file_stem, make_id,
};

const BUILTIN_GLOBALS: &[&str] = &[
    "String",
    "Number",
    "Boolean",
    "Object",
    "Array",
    "Symbol",
    "BigInt",
    "Date",
    "RegExp",
    "Error",
    "Promise",
    "Map",
    "Set",
    "JSON",
    "Math",
    "Reflect",
    "Proxy",
    "URL",
    "console",
    "parseInt",
    "parseFloat",
    "isNaN",
    "str",
    "int",
    "float",
    "bool",
    "list",
    "dict",
    "set",
    "tuple",
    "bytes",
    "len",
    "range",
    "enumerate",
    "zip",
    "map",
    "filter",
    "sum",
    "min",
    "max",
    "print",
    "open",
    "isinstance",
    "type",
    "super",
    "sorted",
    "any",
    "all",
    "abs",
    "round",
];

#[derive(Default)]
pub struct Engine {
    parsers: HashMap<&'static str, Parser>,
}

impl Engine {
    pub fn extract(&mut self, path: &Path) -> Result<Extraction, ExtractError> {
        let spec =
            Registry::resolve(path).ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
        match spec.kind {
            ExtractorKind::Generic => self.extract_generic(path, spec),
            _ => Err(ExtractError::Unsupported(path.to_path_buf())),
        }
    }

    fn extract_generic(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        let grammar = spec
            .grammar
            .ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
        let source = fs::read(path).map_err(|source| trail_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let parser = if let Some(parser) = self.parsers.get_mut(grammar) {
            parser
        } else {
            let language = tree_sitter_language_pack::get_language(grammar).map_err(|error| {
                ExtractError::MissingGrammar {
                    language: grammar.to_owned(),
                    detail: error.to_string(),
                }
            })?;
            let mut parser = Parser::new();
            parser
                .set_language(&language)
                .map_err(|error| ExtractError::MissingGrammar {
                    language: grammar.to_owned(),
                    detail: error.to_string(),
                })?;
            self.parsers.entry(grammar).or_insert(parser)
        };
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| ExtractError::ParseCancelled(path.to_path_buf()))?;
        let config = generic_config(spec);
        Ok(extract_tree(
            path,
            &source,
            tree.root_node(),
            &config,
            spec.name,
        ))
    }
}

struct FunctionBody<'tree> {
    id: String,
    class_id: Option<String>,
    node: Node<'tree>,
}

struct ExtractState<'source, 'tree> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    config: &'source GenericConfig,
    language: &'static str,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    functions: Vec<FunctionBody<'tree>>,
    callables: HashMap<String, Vec<String>>,
}

fn extract_tree(
    path: &Path,
    source: &[u8],
    root: Node<'_>,
    config: &GenericConfig,
    language: &'static str,
) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = ExtractState {
        source,
        source_file,
        stem,
        file_id,
        config,
        language,
        extraction: Extraction::default(),
        seen_nodes: HashSet::new(),
        functions: Vec::new(),
        callables: HashMap::new(),
    };
    let file_label = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    state.add_node(&state.file_id.clone(), file_label, 1, false, None);
    state.walk_declarations(root, None);
    state.walk_function_calls();
    state.extraction
}

impl<'source, 'tree> ExtractState<'source, 'tree> {
    fn walk_declarations(&mut self, node: Node<'tree>, parent_class: Option<&str>) {
        let kind = node.kind();
        if self.config.import_types.contains(&kind) {
            self.add_import(node);
        }

        if self.config.class_types.contains(&kind)
            && let Some(name) = self.declaration_name(node)
        {
            let id = make_id(&[&self.stem, &name]);
            self.add_node(&id, &name, line(node), true, None);
            let source = parent_class.unwrap_or(&self.file_id).to_owned();
            self.add_edge(&source, &id, "contains", line(node), None);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk_declarations(child, Some(&id));
            }
            return;
        }

        if self.config.function_types.contains(&kind)
            && let Some(name) = self.function_name(node)
        {
            let id = parent_class.map_or_else(
                || make_id(&[&self.stem, &name]),
                |class| make_id(&[class, &name]),
            );
            let label = if parent_class.is_some() {
                format!(".{name}()")
            } else {
                format!("{name}()")
            };
            self.add_node(&id, &label, line(node), true, None);
            let source = parent_class.unwrap_or(&self.file_id).to_owned();
            self.add_edge(
                &source,
                &id,
                if parent_class.is_some() {
                    "method"
                } else {
                    "contains"
                },
                line(node),
                None,
            );
            self.callables.entry(name).or_default().push(id.clone());
            self.functions.push(FunctionBody {
                id,
                class_id: parent_class.map(str::to_owned),
                node,
            });
            return;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_declarations(child, parent_class);
        }
    }

    fn walk_function_calls(&mut self) {
        let functions = std::mem::take(&mut self.functions);
        for function in functions {
            self.walk_calls(
                function.node,
                &function.id,
                function.class_id.as_deref(),
                true,
            );
        }
    }

    fn walk_calls(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        class_id: Option<&str>,
        is_root: bool,
    ) {
        let kind = node.kind();
        if !is_root && self.config.function_boundaries.contains(&kind) {
            return;
        }
        if self.config.call_types.contains(&kind)
            && let Some(call) = self.call_name(node)
            && !BUILTIN_GLOBALS.contains(&call.name.as_str())
        {
            let candidates = self.callables.get(&call.name).cloned().unwrap_or_default();
            let target = if let Some(class) = class_id {
                candidates
                    .iter()
                    .find(|candidate| candidate.starts_with(class))
                    .cloned()
                    .or_else(|| (candidates.len() == 1).then(|| candidates[0].clone()))
            } else {
                (candidates.len() == 1).then(|| candidates[0].clone())
            };
            if let Some(target) = target {
                self.add_edge(caller, &target, "calls", line(node), Some("call"));
            } else {
                self.extraction.raw_calls.push(RawCall {
                    caller_nid: caller.to_owned(),
                    callee: call.name,
                    is_member_call: call.member,
                    source_file: self.source_file.clone(),
                    source_location: format!("L{}", line(node)),
                    receiver: call.receiver,
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, class_id, false);
        }
    }

    fn declaration_name(&self, node: Node<'tree>) -> Option<String> {
        node.child_by_field_name("name")
            .and_then(|name| self.node_text(name))
            .or_else(|| {
                self.config.name_fallbacks.iter().find_map(|kind| {
                    first_descendant(node, kind).and_then(|name| self.node_text(name))
                })
            })
            .or_else(|| first_identifier(node).and_then(|name| self.node_text(name)))
            .map(clean_name)
            .filter(|name| !name.is_empty())
    }

    fn function_name(&self, node: Node<'tree>) -> Option<String> {
        self.declaration_name(node).or_else(|| {
            node.child_by_field_name("declarator")
                .and_then(first_identifier)
                .and_then(|name| self.node_text(name))
                .map(clean_name)
        })
    }

    fn call_name(&self, node: Node<'tree>) -> Option<CallName> {
        let function = if self.config.call_function_field.is_empty() {
            None
        } else {
            node.child_by_field_name(self.config.call_function_field)
        }
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| node.child_by_field_name("type"))
        .or_else(|| first_identifier(node))?;
        let function_kind = function.kind();
        let member = self.config.accessor_types.contains(&function_kind);
        let name_node = if member && !self.config.accessor_name_field.is_empty() {
            function
                .child_by_field_name(self.config.accessor_name_field)
                .or_else(|| last_identifier(function))
                .unwrap_or(function)
        } else if member {
            last_identifier(function).unwrap_or(function)
        } else {
            function
        };
        let name = self.node_text(name_node).map(clean_name)?;
        if name.is_empty() {
            return None;
        }
        let receiver = if member && !self.config.accessor_object_field.is_empty() {
            function
                .child_by_field_name(self.config.accessor_object_field)
                .and_then(|receiver| self.node_text(receiver))
                .map(clean_name)
        } else {
            None
        };
        Some(CallName {
            name,
            member,
            receiver,
        })
    }

    fn add_import(&mut self, node: Node<'tree>) {
        let text = self.node_text(node).unwrap_or_default();
        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && node.kind() == "import_statement"
        {
            self.add_js_import(node, &text);
            return;
        }
        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && node.kind() == "export_statement"
            && quoted_value(&text).is_none()
        {
            return;
        }
        let target = quoted_value(&text)
            .or_else(|| angle_value(&text))
            .or_else(|| {
                last_identifier(node)
                    .and_then(|identifier| self.node_text(identifier))
                    .map(clean_name)
            })
            .unwrap_or_default();
        let target = target
            .rsplit(['/', ':'])
            .next()
            .unwrap_or_default()
            .trim_matches(['\'', '"', '>', '<', ';'])
            .to_owned();
        let target = if matches!(self.language, "c" | "cpp" | "objc") {
            Path::new(&target)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(&target)
                .to_owned()
        } else {
            target.rsplit('.').next().unwrap_or(&target).to_owned()
        };
        if !target.is_empty() {
            let target_id = make_id(&[&target]);
            self.add_edge(
                &self.file_id.clone(),
                &target_id,
                "imports",
                line(node),
                Some("import"),
            );
        }
    }

    fn add_js_import(&mut self, node: Node<'tree>, text: &str) {
        let Some(raw_module) = quoted_value(text) else {
            return;
        };
        let source_path = Path::new(&self.source_file);
        let target_path = if raw_module.starts_with('.') {
            lexical_normalize(
                &source_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&raw_module),
            )
        } else {
            Path::new(&raw_module).to_path_buf()
        };
        let target_stem = target_path.to_string_lossy().replace('\\', "/");
        let module_id = make_id(&[&target_stem]);
        self.add_edge(
            &self.file_id.clone(),
            &module_id,
            "imports_from",
            line(node),
            Some("import"),
        );
        if let Some(edge) = self.extraction.edges.last_mut() {
            edge.attributes.insert(
                "target_file".to_owned(),
                Value::String(target_path.to_string_lossy().into_owned()),
            );
        }
        let Some(clause) = first_descendant(node, "import_clause") else {
            return;
        };
        let mut identifiers = Vec::new();
        collect_identifiers(clause, self.source, &mut identifiers);
        identifiers.dedup();
        for identifier in identifiers {
            let target = make_id(&[&target_stem, &identifier]);
            self.add_edge(
                &self.file_id.clone(),
                &target,
                "imports",
                line(node),
                Some("import"),
            );
        }
    }

    fn node_text(&self, node: Node<'tree>) -> Option<String> {
        node.utf8_text(self.source).ok().map(str::to_owned)
    }

    fn add_node(
        &mut self,
        id: &str,
        label: &str,
        line: usize,
        callable: bool,
        node_type: Option<&str>,
    ) {
        if !self.seen_nodes.insert(id.to_owned()) {
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
            Value::String(format!("L{line}")),
        );
        if callable {
            attributes.insert("_callable".to_owned(), Value::Bool(true));
        }
        if let Some(node_type) = node_type {
            attributes.insert("type".to_owned(), Value::String(node_type.to_owned()));
        }
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
        line: usize,
        context: Option<&str>,
    ) {
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        if let Some(context) = context {
            attributes.insert("context".to_owned(), Value::String(context.to_owned()));
        }
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
        attributes.insert("weight".to_owned(), Value::from(1.0));
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }
}

struct CallName {
    name: String,
    member: bool,
    receiver: Option<String>,
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
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

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    [
        "identifier",
        "type_identifier",
        "simple_identifier",
        "name",
        "word",
    ]
    .iter()
    .find_map(|kind| first_descendant(node, kind))
}

fn last_identifier(node: Node<'_>) -> Option<Node<'_>> {
    let mut result = None;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(
            child.kind(),
            "identifier" | "type_identifier" | "simple_identifier" | "name" | "word"
        ) {
            result = Some(child);
        }
        if let Some(found) = last_identifier(child) {
            result = Some(found);
        }
    }
    result
}

fn clean_name(value: String) -> String {
    value
        .trim()
        .trim_matches(['\'', '"', '`', '&', '*', '$', '@'])
        .trim_end_matches(['!', '?'])
        .to_owned()
}

fn quoted_value(value: &str) -> Option<String> {
    for quote in ['\'', '"'] {
        let start = value.find(quote)?;
        let rest = &value[start + quote.len_utf8()..];
        if let Some(end) = rest.find(quote) {
            return Some(rest[..end].to_owned());
        }
    }
    None
}

fn angle_value(value: &str) -> Option<String> {
    let start = value.find('<')?;
    let rest = &value[start + 1..];
    let end = rest.find('>')?;
    Some(rest[..end].to_owned())
}

fn collect_identifiers(node: Node<'_>, source: &[u8], output: &mut Vec<String>) {
    if matches!(node.kind(), "identifier" | "type_identifier")
        && let Ok(text) = node.utf8_text(source)
    {
        output.push(text.to_owned());
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers(child, source, output);
    }
}

fn lexical_normalize(path: &Path) -> std::path::PathBuf {
    use std::path::Component;

    let mut output = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}
