use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde_json::{Map, Value};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::{Node, Parser, Tree};

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
            ExtractorKind::Markdown => crate::markdown::extract(path),
            ExtractorKind::JsonConfig => self.extract_json(path, spec),
            ExtractorKind::McpConfig => crate::mcp::extract(path),
            ExtractorKind::PackageManifest => crate::package_manifest::extract(path),
            ExtractorKind::Terraform => self.extract_terraform(path, spec),
            ExtractorKind::PascalForm => crate::pascal_forms::extract_form(path),
            ExtractorKind::LazarusPackage => crate::pascal_forms::extract_package(path),
            ExtractorKind::DreamMaker => self.extract_dreammaker(path),
            ExtractorKind::Solution => crate::dotnet_project::extract_solution(path),
            ExtractorKind::ProjectXml => crate::dotnet_project::extract_project(path),
            ExtractorKind::Xaml => crate::xaml::extract(self, path),
            _ => Err(ExtractError::Unsupported(path.to_path_buf())),
        }
    }

    fn extract_generic(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        let mut source = fs::read(path).map_err(|source| trail_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if spec.name == "groovy" {
            return Ok(crate::groovy::extract(path, &source));
        }
        if spec.name == "objc" {
            crate::objc::mask_annotation_macros(&mut source);
        }
        let tree = self.parse(path, spec, &source)?;
        let config = generic_config(spec);
        let root = tree.root_node();
        if spec.name == "go" {
            return Ok(crate::go::extract(path, &source, root));
        }
        if spec.name == "rust" {
            return Ok(crate::rust_lang::extract(path, &source, root));
        }
        if spec.name == "bash" {
            return Ok(crate::bash::extract(path, &source, root));
        }
        if spec.name == "csharp" {
            return Ok(crate::csharp::extract(path, &source, root));
        }
        if spec.name == "cpp" {
            return Ok(crate::cpp::extract(path, &source, root));
        }
        if spec.name == "php" {
            return Ok(crate::php::extract(path, &source, root));
        }
        if spec.name == "swift" {
            return Ok(crate::swift::extract(path, &source, root));
        }
        if spec.name == "objc" {
            return Ok(crate::objc::extract(path, &source, root));
        }
        Ok(extract_tree(path, &source, root, &config, spec.name))
    }

    fn extract_json(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        const MAX_BYTES: u64 = 1_048_576;
        let mut source = Vec::new();
        File::open(path)
            .map_err(|source| trail_files::FileError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut source)
            .map_err(|source| trail_files::FileError::Io {
                path: path.to_path_buf(),
                source,
            })?;
        if source.len() > MAX_BYTES as usize {
            return Ok(crate::json_config::error("json file too large to index"));
        }
        let tree = self.parse(path, spec, &source)?;
        Ok(crate::json_config::extract(path, &source, tree.root_node()))
    }

    fn extract_terraform(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        let source = fs::read(path).map_err(|source| trail_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let tree = self.parse(path, spec, &source)?;
        Ok(crate::terraform::extract(path, &source, tree.root_node()))
    }

    fn extract_dreammaker(&mut self, path: &Path) -> Result<Extraction, ExtractError> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "dm" | "dme") {
            return crate::dm::extract_asset(path);
        }
        let source = fs::read(path).map_err(|source| trail_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let parser = if let Some(parser) = self.parsers.get_mut("dm") {
            parser
        } else {
            let language = tree_sitter_dm::LANGUAGE.into();
            let mut parser = Parser::new();
            parser
                .set_language(&language)
                .map_err(|error| ExtractError::MissingGrammar {
                    language: "dm".to_owned(),
                    detail: error.to_string(),
                })?;
            self.parsers.entry("dm").or_insert(parser)
        };
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| ExtractError::ParseCancelled(path.to_path_buf()))?;
        Ok(crate::dm::extract_source(path, &source, tree.root_node()))
    }

    fn parse(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
        source: &[u8],
    ) -> Result<Tree, ExtractError> {
        let grammar = spec
            .grammar
            .ok_or_else(|| ExtractError::Unsupported(path.to_path_buf()))?;
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
            .parse(source, None)
            .ok_or_else(|| ExtractError::ParseCancelled(path.to_path_buf()))?;
        Ok(tree)
    }
}

struct FunctionBody<'tree> {
    id: String,
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
    types: HashMap<String, String>,
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
        types: HashMap::new(),
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
        if self.config.import_types.contains(&kind) && !matches!(self.language, "kotlin" | "lua") {
            self.add_import(node);
        }

        if self.config.class_types.contains(&kind)
            && let Some(name) = self.declaration_name(node)
        {
            let id = make_id(&[&self.stem, &name]);
            self.add_node(&id, &name, line(node), true, None);
            self.types.insert(name.clone(), id.clone());
            self.callables.entry(name).or_default().push(id.clone());
            let source = parent_class.unwrap_or(&self.file_id).to_owned();
            self.add_edge(&source, &id, "contains", line(node), None);
            if self.language == "java" {
                self.add_java_parent_edges(node, &id);
                if kind == "enum_declaration" {
                    self.add_java_enum_constants(node, &id);
                    let mut constructors = Vec::new();
                    collect_nodes_of_kind(node, "constructor_declaration", &mut constructors);
                    let duplicate_line =
                        constructors.first().map_or(line(node), |node| line(*node));
                    self.add_edge(&self.file_id.clone(), &id, "contains", duplicate_line, None);
                    return;
                }
            } else if self.language == "ruby" {
                self.add_ruby_parent_edge(node, &id);
            } else if self.language == "kotlin" {
                self.add_kotlin_parent_edges(node, &id);
            } else if self.language == "scala" {
                self.add_scala_class_references(node, &id);
            }
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
            if self.language == "java" {
                self.add_java_function_references(node, &id);
            } else if self.language == "c" {
                self.add_c_function_references(node, &id);
            } else if self.language == "kotlin" {
                self.add_kotlin_function_references(node, &id);
            } else if self.language == "scala" {
                self.add_scala_function_references(node, &id);
            }
            self.callables.entry(name).or_default().push(id.clone());
            self.functions.push(FunctionBody { id, node });
            return;
        }

        if self.language == "kotlin" && kind == "enum_entry" {
            if let Some(class_id) = parent_class
                && let Some(name_node) = first_descendant(node, "simple_identifier")
                    .or_else(|| first_descendant(node, "identifier"))
                && let Some(name) = self.node_text(name_node).map(clean_name)
            {
                let id = make_id(&[class_id, &name]);
                self.add_node(&id, &name, line(node), false, None);
                self.add_edge(class_id, &id, "case_of", line(node), None);
            }
            return;
        }

        if self.language == "kotlin" && kind == "property_declaration" {
            if let Some(class_id) = parent_class {
                self.add_kotlin_property_reference(node, class_id);
            }
            return;
        }

        if self.language == "scala"
            && matches!(kind, "val_definition" | "var_definition")
            && let Some(class_id) = parent_class
        {
            self.add_scala_field_reference(node, class_id);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_declarations(child, parent_class);
        }
    }

    fn walk_function_calls(&mut self) {
        let functions = std::mem::take(&mut self.functions);
        for function in functions {
            self.walk_calls(function.node, &function.id, true);
        }
    }

    fn walk_calls(&mut self, node: Node<'tree>, caller: &str, is_root: bool) {
        let kind = node.kind();
        if !is_root && self.config.function_boundaries.contains(&kind) {
            return;
        }
        if self.config.call_types.contains(&kind)
            && let Some(call) = self.call_name(node)
            && !BUILTIN_GLOBALS.contains(&call.name.as_str())
        {
            let candidates = self.callables.get(&call.name).cloned().unwrap_or_default();
            let defer_member = call.member
                && (self.language == "java"
                    || call
                        .receiver
                        .as_deref()
                        .is_some_and(|receiver| receiver.starts_with(char::is_uppercase)));
            let target = (!defer_member)
                .then(|| candidates.last().cloned())
                .flatten();
            if let Some(target) = target.as_ref().filter(|target| target.as_str() != caller) {
                self.add_edge(caller, target, "calls", line(node), Some("call"));
            } else if target.is_none()
                && !(self.language == "lua" && (call.member || call.name.contains('.')))
            {
                self.extraction.raw_calls_mut().push(RawCall {
                    caller_nid: caller.to_owned(),
                    callee: call.name,
                    is_member_call: call.member,
                    source_file: self.source_file.clone(),
                    source_location: format!("L{}", line(node)),
                    receiver: Some(call.receiver),
                    receiver_type: (self.language == "ruby" && call.member).then_some(None),
                    lang: (self.language == "java").then(|| "java".to_owned()),
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, false);
        }
    }

    fn declaration_name(&self, node: Node<'tree>) -> Option<String> {
        if self.language == "kotlin"
            && self.config.class_types.contains(&node.kind())
            && let Some(name) = first_descendant(node, "type_identifier")
                .and_then(|name| self.node_text(name))
                .map(clean_name)
        {
            return Some(name);
        }
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
        if self.language == "ruby" {
            let name = node
                .child_by_field_name("method")
                .and_then(|method| self.node_text(method))
                .map(clean_name)?;
            let receiver = node
                .child_by_field_name("receiver")
                .and_then(|receiver| self.node_text(receiver))
                .map(|receiver| receiver.rsplit("::").next().unwrap_or_default().to_owned());
            return Some(CallName {
                name,
                member: receiver.is_some(),
                receiver,
            });
        }
        if self.language == "java" {
            if node.kind() == "method_invocation" {
                let name = node
                    .child_by_field_name("name")
                    .and_then(|name| self.node_text(name))
                    .map(clean_name)?;
                let receiver = node
                    .child_by_field_name("object")
                    .and_then(|receiver| self.node_text(receiver))
                    .map(clean_name);
                return Some(CallName {
                    name,
                    member: receiver.is_some(),
                    receiver,
                });
            }
            if node.kind() == "object_creation_expression" {
                let type_node = node.child_by_field_name("type")?;
                let name_node = first_identifier(type_node).unwrap_or(type_node);
                let name = self.node_text(name_node).map(clean_name)?;
                return Some(CallName {
                    name,
                    member: false,
                    receiver: None,
                });
            }
        }
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
        if self.language == "scala" {
            let mut cursor = node.walk();
            if let Some(target_node) = node
                .children(&mut cursor)
                .find(|child| matches!(child.kind(), "stable_id" | "identifier"))
            {
                let raw = self.node_text(target_node).unwrap_or_default();
                let target = raw
                    .rsplit('.')
                    .next()
                    .unwrap_or_default()
                    .trim_matches(['{', '}', ' ']);
                if !target.is_empty() && target != "_" {
                    self.add_edge(
                        &self.file_id.clone(),
                        &make_id(&[target]),
                        "imports",
                        line(node),
                        Some("import"),
                    );
                }
            }
            return;
        }
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

    fn add_java_parent_edges(&mut self, node: Node<'tree>, class_id: &str) {
        if let Some(superclass) = node.child_by_field_name("superclass")
            && let Some(name_node) = first_identifier(superclass)
            && let Some(name) = self.node_text(name_node).map(clean_name)
        {
            let target = self.ensure_type_node(&name, false);
            self.add_edge(class_id, &target, "inherits", line(node), None);
        }
        if let Some(interfaces) = node.child_by_field_name("interfaces") {
            let mut names = Vec::new();
            collect_type_names(interfaces, self.source, &mut names);
            for name in names {
                if java_builtin_type(&name) {
                    continue;
                }
                let target = self.ensure_type_node(&name, false);
                self.add_edge(class_id, &target, "implements", line(node), None);
            }
        }
    }

    fn add_ruby_parent_edge(&mut self, node: Node<'tree>, class_id: &str) {
        let Some(superclass) = node.child_by_field_name("superclass") else {
            return;
        };
        let Some(name_node) = first_descendant(superclass, "constant") else {
            return;
        };
        let Some(name) = self.node_text(name_node).map(clean_name) else {
            return;
        };
        let target = self.ensure_type_node(&name, true);
        self.add_edge(class_id, &target, "inherits", line(node), None);
    }

    fn add_kotlin_parent_edges(&mut self, node: Node<'tree>, class_id: &str) {
        let mut specifiers = Vec::new();
        collect_nodes_of_kind(node, "delegation_specifier", &mut specifiers);
        for specifier in specifiers {
            let relation = if first_descendant(specifier, "constructor_invocation").is_some() {
                "inherits"
            } else {
                "implements"
            };
            let Some(user_type) = first_descendant(specifier, "user_type") else {
                continue;
            };
            let Some(name_node) = first_descendant(user_type, "type_identifier")
                .or_else(|| first_descendant(user_type, "simple_identifier"))
                .or_else(|| first_descendant(user_type, "identifier"))
            else {
                continue;
            };
            let Some(name) = self.node_text(name_node).map(clean_name) else {
                continue;
            };
            let target = self.ensure_type_node(&name, true);
            self.add_edge(class_id, &target, relation, line(node), None);

            let mut arguments = Vec::new();
            collect_nodes_of_kind(user_type, "type_projection", &mut arguments);
            for argument in arguments {
                let mut refs = Vec::new();
                collect_kotlin_type_refs(argument, self.source, true, &mut refs);
                self.add_kotlin_type_references(class_id, &refs, "generic_arg", line(node));
            }
        }
    }

    fn add_kotlin_property_reference(&mut self, node: Node<'tree>, class_id: &str) {
        let Some(type_node) = first_descendant(node, "user_type")
            .or_else(|| first_descendant(node, "nullable_type"))
            .or_else(|| first_descendant(node, "type_reference"))
        else {
            return;
        };
        let mut refs = Vec::new();
        collect_kotlin_type_refs(type_node, self.source, false, &mut refs);
        for (name, generic) in refs {
            let target = self.ensure_type_node(&name, true);
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

    fn add_kotlin_function_references(&mut self, node: Node<'tree>, function_id: &str) {
        if let Some(parameters) = first_descendant(node, "function_value_parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters
                .children(&mut cursor)
                .filter(|child| child.kind() == "parameter")
            {
                let Some(type_node) = first_descendant(parameter, "user_type")
                    .or_else(|| first_descendant(parameter, "nullable_type"))
                    .or_else(|| first_descendant(parameter, "type_reference"))
                else {
                    continue;
                };
                let mut refs = Vec::new();
                collect_kotlin_type_refs(type_node, self.source, false, &mut refs);
                self.add_kotlin_type_references(function_id, &refs, "parameter_type", line(node));
            }
        }

        let mut saw_parameters = false;
        let mut saw_colon = false;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "function_value_parameters" {
                saw_parameters = true;
                continue;
            }
            if saw_parameters && child.kind() == ":" {
                saw_colon = true;
                continue;
            }
            if saw_colon && child.is_named() {
                let mut refs = Vec::new();
                collect_kotlin_type_refs(child, self.source, false, &mut refs);
                self.add_kotlin_type_references(function_id, &refs, "return_type", line(node));
                break;
            }
        }
    }

    fn add_kotlin_type_references(
        &mut self,
        source: &str,
        refs: &[(String, bool)],
        context: &str,
        at: usize,
    ) {
        for (name, generic) in refs {
            let target = self.ensure_type_node(name, true);
            if target != source {
                self.add_edge(
                    source,
                    &target,
                    "references",
                    at,
                    Some(if *generic { "generic_arg" } else { context }),
                );
            }
        }
    }

    fn add_scala_class_references(&mut self, node: Node<'tree>, class_id: &str) {
        let extends = node
            .child_by_field_name("extend")
            .or_else(|| first_descendant(node, "extends_clause"));
        if let Some(extends) = extends {
            let mut bases = Vec::new();
            let mut cursor = extends.walk();
            for child in extends.children(&mut cursor) {
                let name_node = if child.kind() == "type_identifier" {
                    Some(child)
                } else if child.kind() == "generic_type" {
                    child
                        .child_by_field_name("type")
                        .or_else(|| first_descendant(child, "type_identifier"))
                } else {
                    None
                };
                if let Some(name) = name_node
                    .and_then(|name| self.node_text(name))
                    .map(clean_name)
                {
                    bases.push((name, line(child)));
                }
            }
            for (index, (name, at)) in bases.into_iter().enumerate() {
                let target = self.ensure_type_node(&name, true);
                if target != class_id {
                    self.add_edge(
                        class_id,
                        &target,
                        if index == 0 { "inherits" } else { "mixes_in" },
                        at,
                        None,
                    );
                }
            }
        }

        let mut parameters = Vec::new();
        collect_nodes_of_kind(node, "class_parameter", &mut parameters);
        for parameter in parameters {
            if let Some(type_node) = parameter.child_by_field_name("type") {
                let mut refs = Vec::new();
                collect_scala_type_refs(type_node, self.source, false, &mut refs);
                self.add_scala_type_references(class_id, &refs, "field", line(parameter));
            }
        }
    }

    fn add_scala_field_reference(&mut self, node: Node<'tree>, class_id: &str) {
        let Some(type_node) = node.child_by_field_name("type") else {
            return;
        };
        let mut refs = Vec::new();
        collect_scala_type_refs(type_node, self.source, false, &mut refs);
        self.add_scala_type_references(class_id, &refs, "field", line(node));
    }

    fn add_scala_function_references(&mut self, node: Node<'tree>, function_id: &str) {
        if let Some(parameters) = first_descendant(node, "parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters
                .children(&mut cursor)
                .filter(|child| child.kind() == "parameter")
            {
                if let Some(type_node) = parameter.child_by_field_name("type") {
                    let mut refs = Vec::new();
                    collect_scala_type_refs(type_node, self.source, false, &mut refs);
                    self.add_scala_type_references(
                        function_id,
                        &refs,
                        "parameter_type",
                        line(node),
                    );
                }
            }
        }
        if let Some(return_type) = node.child_by_field_name("return_type") {
            let mut refs = Vec::new();
            collect_scala_type_refs(return_type, self.source, false, &mut refs);
            self.add_scala_type_references(function_id, &refs, "return_type", line(node));
        }
    }

    fn add_scala_type_references(
        &mut self,
        source: &str,
        refs: &[(String, bool)],
        context: &str,
        at: usize,
    ) {
        for (name, generic) in refs {
            let target = self.ensure_type_node(name, true);
            if target != source {
                self.add_edge(
                    source,
                    &target,
                    "references",
                    at,
                    Some(if *generic { "generic_arg" } else { context }),
                );
            }
        }
    }

    fn add_java_enum_constants(&mut self, node: Node<'tree>, enum_id: &str) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.add_java_enum_constants_recursive(child, enum_id);
        }
    }

    fn add_java_enum_constants_recursive(&mut self, node: Node<'tree>, enum_id: &str) {
        if node.kind() == "enum_constant" {
            if let Some(name) = node
                .child_by_field_name("name")
                .and_then(|name| self.node_text(name))
                .map(clean_name)
            {
                let id = make_id(&[enum_id, &name]);
                self.add_node(&id, &name, line(node), false, None);
                self.add_edge(enum_id, &id, "case_of", line(node), None);
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.add_java_enum_constants_recursive(child, enum_id);
        }
    }

    fn add_java_function_references(&mut self, node: Node<'tree>, function_id: &str) {
        let mut parameters = Vec::new();
        collect_nodes_of_kind(node, "formal_parameter", &mut parameters);
        for parameter in parameters {
            if let Some(type_node) = parameter.child_by_field_name("type") {
                let mut names = Vec::new();
                collect_type_names(type_node, self.source, &mut names);
                self.add_java_type_references(function_id, &names, "parameter_type", line(node));
            }
        }

        if let Some(return_type) = node.child_by_field_name("type") {
            let mut names = Vec::new();
            collect_type_names(return_type, self.source, &mut names);
            if let Some((base, generic)) = names.split_first() {
                if !java_builtin_type(base) {
                    let target = self.ensure_type_node(base, true);
                    self.add_edge(
                        function_id,
                        &target,
                        "references",
                        line(node),
                        Some("return_type"),
                    );
                }
                self.add_java_type_references(function_id, generic, "generic_arg", line(node));
            }
        }

        let mut annotations = Vec::new();
        collect_nodes_of_kind(node, "marker_annotation", &mut annotations);
        for annotation in annotations {
            if let Some(name) = annotation
                .child_by_field_name("name")
                .and_then(|name| self.node_text(name))
                .map(clean_name)
            {
                let target = self.ensure_type_node(&name, true);
                self.add_edge(
                    function_id,
                    &target,
                    "references",
                    line(node),
                    Some("attribute"),
                );
            }
        }
    }

    fn add_c_function_references(&mut self, node: Node<'tree>, function_id: &str) {
        if let Some(return_type) = node.child_by_field_name("type") {
            let mut names = Vec::new();
            collect_c_type_names(return_type, self.source, &mut names);
            self.add_c_type_references(function_id, &names, "return_type", line(node));
        }
        let mut parameters = Vec::new();
        collect_nodes_of_kind(node, "parameter_declaration", &mut parameters);
        for parameter in parameters {
            if let Some(type_node) = parameter.child_by_field_name("type") {
                let mut names = Vec::new();
                collect_c_type_names(type_node, self.source, &mut names);
                self.add_c_type_references(function_id, &names, "parameter_type", line(node));
            }
        }
    }

    fn add_c_type_references(
        &mut self,
        function_id: &str,
        names: &[String],
        context: &str,
        line: usize,
    ) {
        for name in names {
            let target = self.ensure_type_node(name, true);
            self.add_edge(function_id, &target, "references", line, Some(context));
        }
    }

    fn add_java_type_references(
        &mut self,
        function_id: &str,
        names: &[String],
        context: &str,
        line: usize,
    ) {
        for name in names {
            if java_builtin_type(name) {
                continue;
            }
            let target = self.ensure_type_node(name, true);
            self.add_edge(function_id, &target, "references", line, Some(context));
        }
    }

    fn ensure_type_node(&mut self, name: &str, origin_file: bool) -> String {
        if let Some(id) = self.types.get(name) {
            return id.clone();
        }
        let local_id = make_id(&[&self.stem, name]);
        if self.seen_nodes.contains(&local_id) {
            return local_id;
        }
        let id = make_id(&[name]);
        if self.seen_nodes.insert(id.clone()) {
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(name.to_owned()));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert("source_file".to_owned(), Value::String(String::new()));
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            if origin_file {
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

fn collect_type_names(node: Node<'_>, source: &[u8], output: &mut Vec<String>) {
    if matches!(node.kind(), "type_identifier" | "scoped_type_identifier")
        && let Ok(text) = node.utf8_text(source)
    {
        output.push(text.to_owned());
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_names(child, source, output);
    }
}

fn collect_c_type_names(node: Node<'_>, source: &[u8], output: &mut Vec<String>) {
    if node.kind() == "type_identifier" {
        if let Ok(text) = node.utf8_text(source) {
            output.push(text.to_owned());
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_c_type_names(child, source, output);
    }
}

fn collect_kotlin_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    if matches!(node.kind(), "integral_literal" | "boolean_literal") {
        return;
    }
    if node.kind() == "user_type" {
        if let Some(name_node) = first_descendant(node, "type_identifier")
            .or_else(|| first_descendant(node, "simple_identifier"))
            .or_else(|| first_descendant(node, "identifier"))
            && let Ok(name) = name_node.utf8_text(source)
            && !kotlin_builtin_type(name)
        {
            output.push((name.to_owned(), generic));
        }
        let mut arguments = Vec::new();
        collect_nodes_of_kind(node, "type_projection", &mut arguments);
        for argument in arguments {
            let mut cursor = argument.walk();
            for child in argument
                .children(&mut cursor)
                .filter(|child| child.is_named())
            {
                collect_kotlin_type_refs(child, source, true, output);
            }
        }
        return;
    }
    if matches!(node.kind(), "identifier" | "type_identifier") {
        if let Ok(name) = node.utf8_text(source)
            && !kotlin_builtin_type(name)
        {
            output.push((name.to_owned(), generic));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_kotlin_type_refs(child, source, generic, output);
    }
}

fn kotlin_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "Int"
            | "Long"
            | "Short"
            | "Byte"
            | "Boolean"
            | "Char"
            | "Float"
            | "Double"
            | "Unit"
            | "Any"
            | "Nothing"
    )
}

fn collect_scala_type_refs(
    node: Node<'_>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    if node.kind() == "type_identifier" {
        if let Ok(name) = node.utf8_text(source)
            && !name.is_empty()
        {
            output.push((name.to_owned(), generic));
        }
        return;
    }
    if node.kind() == "generic_type" {
        let base = node
            .child_by_field_name("type")
            .or_else(|| first_descendant(node, "type_identifier"));
        if let Some(base) = base
            && let Ok(name) = base.utf8_text(source)
            && !name.is_empty()
        {
            output.push((name.to_owned(), generic));
        }
        let mut cursor = node.walk();
        for arguments in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "type_arguments")
        {
            let mut argument_cursor = arguments.walk();
            for argument in arguments
                .children(&mut argument_cursor)
                .filter(|child| child.is_named())
            {
                collect_scala_type_refs(argument, source, true, output);
            }
        }
        return;
    }
    if matches!(
        node.kind(),
        "compound_type"
            | "infix_type"
            | "function_type"
            | "tuple_type"
            | "annotated_type"
            | "projected_type"
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            collect_scala_type_refs(child, source, generic, output);
        }
    }
}

fn collect_nodes_of_kind<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            output.push(child);
        } else {
            collect_nodes_of_kind(child, kind, output);
        }
    }
}

fn java_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "String"
            | "List"
            | "Map"
            | "Set"
            | "ArrayList"
            | "HashMap"
            | "Integer"
            | "Long"
            | "Double"
            | "Float"
            | "Boolean"
            | "Object"
            | "Class"
    )
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
