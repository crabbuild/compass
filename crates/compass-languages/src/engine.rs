use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use compass_model::{EdgeRecord, NodeRecord};
use serde_json::{Map, Value};
use tree_sitter::{Node, Parser, Tree};

use crate::builtins::LANGUAGE_BUILTIN_GLOBALS;
use crate::config::{GenericConfig, generic_config};
use crate::{
    ExtractError, Extraction, ExtractorKind, LanguageSpec, RawCall, Registry, file_stem, make_id,
};

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
            ExtractorKind::Template => crate::templates::extract(self, path, spec.name),
        }
    }

    pub(super) fn extract_embedded_script(
        &mut self,
        path: &Path,
        source: &[u8],
        language: &'static str,
        grammar: &'static str,
    ) -> Result<Extraction, ExtractError> {
        let spec = LanguageSpec {
            name: language,
            grammar: Some(grammar),
            kind: ExtractorKind::Generic,
        };
        let tree = self.parse(path, spec, source)?;
        let mut extraction = extract_tree(
            path,
            source,
            tree.root_node(),
            &generic_config(spec),
            language,
        );
        if language == "python" {
            add_python_rationale(path, source, tree.root_node(), &mut extraction);
        }
        Ok(extraction)
    }

    fn extract_generic(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        let mut source = fs::read(path).map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if spec.name == "groovy" {
            return Ok(crate::groovy::extract(path, &source));
        }
        // These extractors are intentionally source-driven and do not consume a
        // tree-sitter root. Avoid initializing and touching their large static
        // grammar tables only to discard the tree; this materially lowers cold
        // multilingual startup RSS and latency while preserving identical facts.
        match spec.name {
            "zig" => return Ok(crate::zig::extract(path, &source)),
            "verilog" => return Ok(crate::verilog::extract(path, &source)),
            "sql" => return Ok(crate::sql::extract(path, &source)),
            "pascal" => return Ok(crate::pascal::extract(path, &source)),
            "apex" => return Ok(crate::apex::extract(path, &source)),
            "dart" => return Ok(crate::dart::extract(path, &source)),
            _ => {}
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
        if spec.name == "powershell" {
            return Ok(crate::powershell::extract(path, &source, root));
        }
        if spec.name == "elixir" {
            return Ok(crate::elixir::extract(path, &source, root));
        }
        if spec.name == "julia" {
            return Ok(crate::julia::extract(path, &source, root));
        }
        if spec.name == "fortran" {
            return Ok(crate::fortran::extract(path, &source, root));
        }
        let mut extraction = extract_tree(path, &source, root, &config, spec.name);
        if spec.name == "python" {
            add_python_rationale(path, &source, root, &mut extraction);
        }
        Ok(extraction)
    }

    fn extract_json(
        &mut self,
        path: &Path,
        spec: LanguageSpec,
    ) -> Result<Extraction, ExtractError> {
        const MAX_BYTES: u64 = 1_048_576;
        let mut source = Vec::new();
        File::open(path)
            .map_err(|source| compass_files::FileError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut source)
            .map_err(|source| compass_files::FileError::Io {
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
        let source = fs::read(path).map_err(|source| compass_files::FileError::Io {
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
        let source = fs::read(path).map_err(|source| compass_files::FileError::Io {
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
    top_level: bool,
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
    seen_resolved_calls: HashSet<(String, String)>,
    seen_dynamic_imports: HashSet<(String, String)>,
    python_import_aliases: HashMap<String, String>,
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
        seen_resolved_calls: HashSet::new(),
        seen_dynamic_imports: HashSet::new(),
        python_import_aliases: if language == "python" {
            python_import_aliases(root, source)
        } else {
            HashMap::new()
        },
    };
    let file_label = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    state.add_node(&state.file_id.clone(), file_label, 1, false, None);
    state.walk_declarations(root, None);
    if language == "python" {
        let module_bound = python_bound_names(root, source, true);
        state.walk_python_indirect(root, &state.file_id.clone(), true, &module_bound);
    } else if matches!(language, "javascript" | "typescript" | "tsx") {
        let module_bound = js_module_bound_names(root, source);
        state.walk_js_module_indirect(root, true, &module_bound);
    }
    state.walk_function_calls();
    state.extraction
}

fn add_python_rationale(path: &Path, source: &[u8], root: Node<'_>, extraction: &mut Extraction) {
    let stem = file_stem(path);
    let file_id = make_id(&[&path.to_string_lossy()]);
    let mut seen = extraction
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<HashSet<_>>();
    let autogenerated = source.get(..source.len().min(2_048)).is_some_and(|head| {
        let head = String::from_utf8_lossy(head);
        [
            "DO NOT EDIT",
            "@generated",
            "Generated by the protocol buffer",
        ]
        .iter()
        .any(|marker| head.contains(marker))
            || (head.contains("def upgrade(")
                && head.contains("down_revision")
                && (head.contains("revision =") || head.contains("revision:")))
            || (head.contains("class Migration(migrations.Migration)")
                && head.contains("operations"))
    });
    if !autogenerated && let Some((text, line)) = python_docstring(root, source) {
        push_rationale(path, &stem, &file_id, &text, line, extraction, &mut seen);
    }
    walk_python_docstrings(path, &stem, &file_id, root, source, extraction, &mut seen);
    let text = String::from_utf8_lossy(source);
    for (index, line) in text.lines().enumerate() {
        let stripped = line.trim();
        if [
            "# NOTE:",
            "# IMPORTANT:",
            "# HACK:",
            "# WHY:",
            "# RATIONALE:",
            "# TODO:",
            "# FIXME:",
        ]
        .iter()
        .any(|prefix| stripped.starts_with(prefix))
        {
            push_rationale(
                path,
                &stem,
                &file_id,
                stripped,
                index + 1,
                extraction,
                &mut seen,
            );
        }
    }
}

fn walk_python_docstrings(
    path: &Path,
    stem: &str,
    parent_id: &str,
    node: Node<'_>,
    source: &[u8],
    extraction: &mut Extraction,
    seen: &mut HashSet<String>,
) {
    match node.kind() {
        "class_definition" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let Some(body) = node.child_by_field_name("body") else {
                return;
            };
            let name = source_node_text(name_node, source);
            let class_id = make_id(&[stem, &name]);
            if let Some((text, line)) = python_docstring(body, source) {
                push_rationale(path, stem, &class_id, &text, line, extraction, seen);
            }
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                walk_python_docstrings(path, stem, &class_id, child, source, extraction, seen);
            }
            return;
        }
        "function_definition" => {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let Some(body) = node.child_by_field_name("body") else {
                return;
            };
            let name = source_node_text(name_node, source);
            let function_id = if parent_id == make_id(&[&path.to_string_lossy()]) {
                make_id(&[stem, &name])
            } else {
                make_id(&[parent_id, &name])
            };
            if let Some((text, line)) = python_docstring(body, source) {
                push_rationale(path, stem, &function_id, &text, line, extraction, seen);
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_python_docstrings(path, stem, parent_id, child, source, extraction, seen);
    }
}

fn python_docstring(node: Node<'_>, source: &[u8]) -> Option<(String, usize)> {
    let mut cursor = node.walk();
    let first = node.named_children(&mut cursor).next()?;
    let string = if matches!(first.kind(), "string" | "concatenated_string") {
        first
    } else if first.kind() == "expression_statement" {
        let mut inner_cursor = first.walk();
        first
            .named_children(&mut inner_cursor)
            .find(|child| matches!(child.kind(), "string" | "concatenated_string"))?
    } else {
        return None;
    };
    let text = source_node_text(string, source)
        .trim_matches(['\"', '\''])
        .trim()
        .to_owned();
    (text.chars().count() > 20).then(|| (text, line(first)))
}

fn push_rationale(
    path: &Path,
    stem: &str,
    parent_id: &str,
    text: &str,
    line_number: usize,
    extraction: &mut Extraction,
    seen: &mut HashSet<String>,
) {
    let id = make_id(&[stem, "rationale", &line_number.to_string()]);
    let label = text
        .chars()
        .take(80)
        .collect::<String>()
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
        .trim()
        .to_owned();
    let source_file = path.to_string_lossy().into_owned();
    let source_location = format!("L{line_number}");
    if seen.insert(id.clone()) {
        extraction.nodes.push(NodeRecord {
            id: id.clone(),
            attributes: Map::from_iter([
                ("label".to_owned(), Value::String(label)),
                (
                    "file_type".to_owned(),
                    Value::String("rationale".to_owned()),
                ),
                ("source_file".to_owned(), Value::String(source_file.clone())),
                (
                    "source_location".to_owned(),
                    Value::String(source_location.clone()),
                ),
            ]),
        });
    }
    extraction.edges.push(EdgeRecord {
        source: id,
        target: parent_id.to_owned(),
        attributes: Map::from_iter([
            (
                "relation".to_owned(),
                Value::String("rationale_for".to_owned()),
            ),
            (
                "confidence".to_owned(),
                Value::String("EXTRACTED".to_owned()),
            ),
            ("source_file".to_owned(), Value::String(source_file)),
            ("source_location".to_owned(), Value::String(source_location)),
            ("weight".to_owned(), Value::from(1.0)),
        ]),
    });
}

fn source_node_text(node: Node<'_>, source: &[u8]) -> String {
    source
        .get(node.start_byte()..node.end_byte())
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or_default()
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
            if self.language == "python" {
                self.add_python_parent_edges(node, &id);
            } else if self.language == "java" {
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
            if matches!(self.language, "typescript" | "tsx") {
                self.add_ts_class_decorators(node, &id);
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
            if self.language == "python" {
                self.add_python_function_references(node, &id);
            } else if self.language == "java" {
                self.add_java_function_references(node, &id);
            } else if self.language == "c" {
                self.add_c_function_references(node, &id);
            } else if self.language == "kotlin" {
                self.add_kotlin_function_references(node, &id);
            } else if self.language == "scala" {
                self.add_scala_function_references(node, &id);
            }
            self.callables.entry(name).or_default().push(id.clone());
            self.functions.push(FunctionBody {
                id,
                node,
                top_level: parent_class.is_none()
                    && (self.language != "python"
                        || node
                            .parent()
                            .is_some_and(|parent| parent.kind() == "module")),
            });
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

        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && kind == "lexical_declaration"
            && parent_class.is_none()
            && node.parent().is_some_and(|parent| {
                parent.kind() == "program"
                    || (parent.kind() == "export_statement"
                        && parent
                            .parent()
                            .is_some_and(|grandparent| grandparent.kind() == "program"))
            })
        {
            self.add_js_module_bindings(node);
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
            if self.language == "python" {
                if function.top_level {
                    self.walk_python_import_calls(function.node, &function.id);
                }
                let bound = python_bound_names(function.node, self.source, false);
                let body = function
                    .node
                    .child_by_field_name("body")
                    .unwrap_or(function.node);
                self.walk_python_indirect(body, &function.id, true, &bound);
                self.walk_calls(body, &function.id, true);
            } else {
                let body = function
                    .node
                    .child_by_field_name("body")
                    .unwrap_or(function.node);
                self.walk_calls(body, &function.id, true);
            }
        }
    }

    fn walk_python_import_calls(&mut self, node: Node<'tree>, caller: &str) {
        if node.kind() == "call"
            && let Some(function) = node.child_by_field_name("function")
            && let Some(name) = if function.kind() == "identifier" {
                self.node_text(function)
                    .map(clean_name)
                    .and_then(|local_name| self.python_import_aliases.get(&local_name).cloned())
            } else {
                None
            }
        {
            let mut extensions = Map::new();
            extensions.insert("symbol_import_use".to_owned(), Value::Bool(true));
            self.extraction.raw_calls_mut().push(RawCall {
                caller_nid: caller.to_owned(),
                callee: name,
                is_member_call: Some(false),
                source_file: self.source_file.clone(),
                source_location: format!("L{}", line(node)),
                receiver: None,
                receiver_type: None,
                lang: None,
                extensions,
            });
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_python_import_calls(child, caller);
        }
    }

    fn walk_python_indirect(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        is_root: bool,
        bound: &HashSet<String>,
    ) {
        if !is_root && matches!(node.kind(), "function_definition" | "class_definition") {
            return;
        }
        if caller != self.file_id
            && node.kind() == "call"
            && let Some(arguments) = node.child_by_field_name("arguments")
        {
            let mut cursor = arguments.walk();
            for argument in arguments.children(&mut cursor) {
                let candidate = if argument.kind() == "identifier" {
                    Some(argument)
                } else if argument.kind() == "keyword_argument" {
                    argument.child_by_field_name("value")
                } else {
                    None
                };
                if candidate.is_some_and(|candidate| candidate.kind() == "identifier") {
                    self.add_python_indirect(caller, candidate, "argument", bound);
                }
            }
        }
        if matches!(node.kind(), "dictionary" | "list" | "set" | "tuple") {
            let mut identifiers = Vec::new();
            collect_python_collection_values(node, &mut identifiers);
            for identifier in identifiers {
                self.add_python_indirect(caller, Some(identifier), "collection", bound);
            }
        } else if node.kind() == "assignment"
            && let Some(value) = node.child_by_field_name("right")
        {
            let mut identifiers = Vec::new();
            collect_python_reference_values(value, &mut identifiers);
            for identifier in identifiers {
                self.add_python_indirect(caller, Some(identifier), "assignment", bound);
            }
        } else if node.kind() == "return_statement" {
            let mut cursor = node.walk();
            if let Some(value) = node.children(&mut cursor).find(|child| child.is_named()) {
                let mut identifiers = Vec::new();
                collect_python_reference_values(value, &mut identifiers);
                for identifier in identifiers {
                    self.add_python_indirect(caller, Some(identifier), "return", bound);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_python_indirect(child, caller, false, bound);
        }
    }

    fn add_python_indirect(
        &mut self,
        caller: &str,
        node: Option<Node<'tree>>,
        context: &str,
        bound: &HashSet<String>,
    ) {
        let Some(node) = node else {
            return;
        };
        let Some(name) = self.node_text(node).map(clean_name) else {
            return;
        };
        if name.is_empty() || bound.contains(&name) {
            return;
        }
        let mut extensions = Map::new();
        extensions.insert("indirect".to_owned(), Value::Bool(true));
        extensions.insert("context".to_owned(), Value::String(context.to_owned()));
        self.extraction.raw_calls_mut().push(RawCall {
            caller_nid: caller.to_owned(),
            callee: name,
            is_member_call: Some(false),
            source_file: self.source_file.clone(),
            source_location: format!("L{}", line(node)),
            receiver: None,
            receiver_type: None,
            lang: None,
            extensions,
        });
    }

    fn walk_js_module_indirect(
        &mut self,
        node: Node<'tree>,
        is_root: bool,
        bound: &HashSet<String>,
    ) {
        if !is_root
            && matches!(
                node.kind(),
                "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "generator_function_declaration"
                    | "generator_function"
                    | "class_declaration"
                    | "class"
            )
        {
            return;
        }
        if matches!(node.kind(), "object" | "array") {
            let mut identifiers = Vec::new();
            collect_js_collection_values(node, &mut identifiers);
            for identifier in identifiers {
                self.add_js_indirect(identifier, "collection", bound);
            }
        } else if matches!(node.kind(), "call_expression" | "new_expression")
            && let Some(arguments) = node.child_by_field_name("arguments")
        {
            let mut cursor = arguments.walk();
            for argument in arguments.children(&mut cursor) {
                if argument.kind() == "identifier" {
                    self.add_js_indirect(argument, "argument", bound);
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_js_module_indirect(child, false, bound);
        }
    }

    fn add_js_indirect(&mut self, node: Node<'tree>, context: &str, bound: &HashSet<String>) {
        let Some(name) = self.node_text(node).map(clean_name) else {
            return;
        };
        if name.is_empty() || bound.contains(&name) {
            return;
        }
        let Some(target) = self
            .callables
            .get(&name)
            .filter(|candidates| candidates.len() == 1)
            .and_then(|candidates| candidates.first())
            .cloned()
        else {
            return;
        };
        if target == self.file_id
            || !self
                .seen_resolved_calls
                .insert((self.file_id.clone(), target.clone()))
        {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert(
            "relation".to_owned(),
            Value::String("indirect_call".to_owned()),
        );
        attributes.insert("context".to_owned(), Value::String(context.to_owned()));
        attributes.insert(
            "confidence".to_owned(),
            Value::String("INFERRED".to_owned()),
        );
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{}", line(node))),
        );
        attributes.insert("weight".to_owned(), Value::from(1.0));
        self.extraction.edges.push(EdgeRecord {
            source: self.file_id.clone(),
            target,
            attributes,
        });
    }

    fn walk_calls(&mut self, node: Node<'tree>, caller: &str, is_root: bool) {
        let kind = node.kind();
        if !is_root && self.config.function_boundaries.contains(&kind) {
            return;
        }
        if matches!(self.language, "javascript" | "typescript" | "tsx")
            && kind == "call_expression"
            && self.add_js_dynamic_import(node, caller)
        {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk_calls(child, caller, false);
            }
            return;
        }
        if self.config.call_types.contains(&kind)
            && let Some(call) = self.call_name(node)
            && !LANGUAGE_BUILTIN_GLOBALS.contains(&call.name.as_str())
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
                .flatten()
                .or_else(|| {
                    (!call.member || self.language == "python")
                        .then(|| self.types.get(&call.name).cloned())
                        .flatten()
                });
            if let Some(target) = target.as_ref().filter(|target| {
                target.as_str() != caller
                    && self
                        .seen_resolved_calls
                        .insert((caller.to_owned(), (*target).clone()))
            }) {
                self.add_edge(caller, target, "calls", line(node), Some("call"));
            } else if target.is_none()
                && !(self.language == "lua" && (call.member || call.name.contains('.')))
            {
                self.extraction.raw_calls_mut().push(RawCall {
                    caller_nid: caller.to_owned(),
                    callee: call.name,
                    is_member_call: Some(call.member),
                    source_file: self.source_file.clone(),
                    source_location: format!("L{}", line(node)),
                    receiver: Some(call.receiver),
                    receiver_type: (self.language == "ruby" && call.member).then_some(None),
                    lang: (self.language == "java").then(|| "java".to_owned()),
                    extensions: Map::new(),
                });
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls(child, caller, false);
        }
    }

    fn add_js_dynamic_import(&mut self, node: Node<'tree>, caller: &str) -> bool {
        let function = node.child_by_field_name("function").or_else(|| {
            let mut cursor = node.walk();
            node.children(&mut cursor).next()
        });
        if function
            .and_then(|function| self.node_text(function))
            .as_deref()
            != Some("import")
        {
            return false;
        }
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return true;
        };
        let mut cursor = arguments.walk();
        for argument in arguments.children(&mut cursor) {
            let raw = if argument.kind() == "template_string" {
                let mut nested = argument.walk();
                if argument
                    .children(&mut nested)
                    .any(|child| child.kind() == "template_substitution")
                {
                    break;
                }
                self.node_text(argument)
                    .map(|value| value.trim_matches('`').to_owned())
            } else if argument.kind() == "string" {
                self.node_text(argument)
                    .map(|value| value.trim_matches(['\'', '"', ' ']).to_owned())
            } else {
                continue;
            };
            let Some(raw) = raw.filter(|value| !value.is_empty()) else {
                break;
            };
            let source_path = Path::new(&self.source_file);
            let target_path = if raw.starts_with('.') {
                resolve_js_import_path(&lexical_normalize(
                    &source_path
                        .parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(&raw),
                ))
            } else {
                Path::new(&raw).to_path_buf()
            };
            let target = make_id(&[&target_path.to_string_lossy().replace('\\', "/")]);
            if self
                .seen_dynamic_imports
                .insert((caller.to_owned(), target.clone()))
            {
                let mut attributes = Map::new();
                attributes.insert(
                    "relation".to_owned(),
                    Value::String("imports_from".to_owned()),
                );
                attributes.insert("context".to_owned(), Value::String("import".to_owned()));
                attributes.insert("deferred".to_owned(), Value::Bool(true));
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
                    Value::String(format!("L{}", line(node))),
                );
                attributes.insert("weight".to_owned(), Value::from(1.0));
                attributes.insert(
                    "target_file".to_owned(),
                    Value::String(target_path.to_string_lossy().into_owned()),
                );
                self.extraction.edges.push(EdgeRecord {
                    source: caller.to_owned(),
                    target,
                    attributes,
                });
            }
            break;
        }
        true
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
        if self.language == "python" {
            self.add_python_import(node);
            return;
        }
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
            && matches!(node.kind(), "import_statement" | "export_statement")
        {
            self.add_js_import(node);
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

    fn add_python_import(&mut self, node: Node<'tree>) {
        if node.kind() == "import_statement" {
            let mut cursor = node.walk();
            let imports = node
                .children(&mut cursor)
                .filter(|child| matches!(child.kind(), "dotted_name" | "aliased_import"))
                .filter_map(|child| self.node_text(child))
                .map(|raw| {
                    raw.split(" as ")
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .trim_start_matches('.')
                        .to_owned()
                })
                .filter(|module| !module.is_empty())
                .collect::<Vec<_>>();
            for module in imports {
                self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[&module]),
                    "imports",
                    line(node),
                    Some("import"),
                );
            }
            return;
        }
        let Some(module_node) = node.child_by_field_name("module_name") else {
            return;
        };
        let Some(raw) = self.node_text(module_node) else {
            return;
        };
        let target = if raw.starts_with('.') {
            let dots = raw.len().saturating_sub(raw.trim_start_matches('.').len());
            let module = raw.trim_start_matches('.');
            let mut base = Path::new(&self.source_file)
                .parent()
                .unwrap_or_else(|| Path::new("."));
            for _ in 1..dots {
                base = base.parent().unwrap_or(base);
            }
            let relative = if module.is_empty() {
                "__init__.py".to_owned()
            } else {
                format!("{}.py", module.replace('.', "/"))
            };
            make_id(&[&base.join(relative).to_string_lossy()])
        } else {
            make_id(&[&raw])
        };
        self.add_edge(
            &self.file_id.clone(),
            &target,
            "imports_from",
            line(node),
            Some("import"),
        );
    }

    fn add_js_import(&mut self, node: Node<'tree>) {
        let is_reexport = node.kind() == "export_statement";
        let mut cursor = node.walk();
        let Some(module_node) = node
            .children(&mut cursor)
            .find(|child| child.kind() == "string")
        else {
            return;
        };
        let Some(raw_module) = self
            .node_text(module_node)
            .map(|value| value.trim_matches(['\'', '"', '`', ' ']).to_owned())
        else {
            return;
        };
        let source_path = Path::new(&self.source_file);
        let target_path = raw_module.starts_with('.').then(|| {
            resolve_js_import_path(&lexical_normalize(
                &source_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&raw_module),
            ))
        });
        let module_id = if let Some(target_path) = &target_path {
            make_id(&[&target_path.to_string_lossy().replace('\\', "/")])
        } else {
            make_id(&["ref", &raw_module])
        };
        self.add_edge(
            &self.file_id.clone(),
            &module_id,
            "imports_from",
            line(node),
            Some(if is_reexport { "re-export" } else { "import" }),
        );
        if let (Some(edge), Some(target_path)) = (self.extraction.edges.last_mut(), &target_path) {
            edge.attributes.insert(
                "target_file".to_owned(),
                Value::String(target_path.to_string_lossy().into_owned()),
            );
        }
        let Some(target_path) = target_path else {
            return;
        };
        let target_stem = file_stem(&target_path);
        if is_reexport {
            let Some(clause) = first_descendant(node, "export_clause") else {
                return;
            };
            let mut specifiers = Vec::new();
            collect_nodes_of_kind(clause, "export_specifier", &mut specifiers);
            for specifier in specifiers {
                let Some(name) = specifier
                    .child_by_field_name("name")
                    .and_then(|name| self.node_text(name))
                    .map(clean_name)
                else {
                    continue;
                };
                if name.is_empty() || name == "default" {
                    continue;
                }
                self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[&target_stem, &name]),
                    "re_exports",
                    line(node),
                    Some("re-export"),
                );
            }
        } else if let Some(clause) = first_descendant(node, "import_clause") {
            let mut specifiers = Vec::new();
            collect_nodes_of_kind(clause, "import_specifier", &mut specifiers);
            for specifier in specifiers {
                let Some(name) = specifier
                    .child_by_field_name("name")
                    .and_then(|name| self.node_text(name))
                    .map(clean_name)
                else {
                    continue;
                };
                if name.is_empty() {
                    continue;
                }
                self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[&target_stem, &name]),
                    "imports",
                    line(node),
                    Some("import"),
                );
            }
        }
    }

    fn add_js_module_bindings(&mut self, node: Node<'tree>) {
        let mut cursor = node.walk();
        let declarations: Vec<_> = node
            .children(&mut cursor)
            .filter(|child| child.kind() == "variable_declarator")
            .collect();
        for declaration in declarations {
            let Some(name_node) = declaration.child_by_field_name("name") else {
                continue;
            };
            let Some(value) = declaration.child_by_field_name("value") else {
                continue;
            };
            self.add_js_require_import(declaration, name_node, value);
            let Some(name) = self.node_text(name_node).map(clean_name) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            let id = make_id(&[&self.stem, &name]);
            if matches!(
                value.kind(),
                "arrow_function" | "function_expression" | "function"
            ) {
                self.add_node(&id, &format!("{name}()"), line(declaration), true, None);
                self.add_edge(
                    &self.file_id.clone(),
                    &id,
                    "contains",
                    line(declaration),
                    None,
                );
                self.callables.entry(name).or_default().push(id.clone());
                self.functions.push(FunctionBody {
                    id,
                    node: value,
                    top_level: true,
                });
            } else if matches!(
                value.kind(),
                "object" | "array" | "as_expression" | "call_expression" | "new_expression"
            ) {
                self.add_node(&id, &name, line(declaration), false, None);
                self.add_edge(
                    &self.file_id.clone(),
                    &id,
                    "contains",
                    line(declaration),
                    None,
                );
            }
        }
    }

    fn add_js_require_import(
        &mut self,
        declaration: Node<'tree>,
        name_node: Node<'tree>,
        value: Node<'tree>,
    ) -> bool {
        let Some(call) = find_require_call(value, self.source) else {
            return false;
        };
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return false;
        };
        let mut cursor = arguments.walk();
        let Some(module_node) = arguments
            .children(&mut cursor)
            .find(|child| child.kind() == "string")
        else {
            return false;
        };
        let Some(raw_module) = self
            .node_text(module_node)
            .map(|value| value.trim_matches(['\'', '"', '`', ' ']).to_owned())
            .filter(|value| !value.is_empty())
        else {
            return false;
        };
        let source_path = Path::new(&self.source_file);
        let target_path = if raw_module.starts_with('.') {
            resolve_js_import_path(&lexical_normalize(
                &source_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&raw_module),
            ))
        } else {
            Path::new(&raw_module).to_path_buf()
        };
        let module_id = make_id(&[&target_path.to_string_lossy().replace('\\', "/")]);
        self.add_edge(
            &self.file_id.clone(),
            &module_id,
            "imports_from",
            line(declaration),
            Some("import"),
        );
        if let Some(edge) = self.extraction.edges.last_mut() {
            edge.attributes.insert(
                "target_file".to_owned(),
                Value::String(target_path.to_string_lossy().into_owned()),
            );
        }

        let mut symbols = Vec::new();
        if name_node.kind() == "object_pattern" {
            let mut cursor = name_node.walk();
            for property in name_node.children(&mut cursor) {
                let symbol_node = match property.kind() {
                    "shorthand_property_identifier_pattern" => Some(property),
                    "pair_pattern" => property.child_by_field_name("key"),
                    _ => None,
                };
                if let Some(symbol) = symbol_node
                    .and_then(|node| self.node_text(node))
                    .map(clean_name)
                    .filter(|name| !name.is_empty())
                {
                    symbols.push(symbol);
                }
            }
        } else if value.kind() == "member_expression"
            && let Some(symbol) = value
                .child_by_field_name("property")
                .and_then(|node| self.node_text(node))
                .map(clean_name)
                .filter(|name| !name.is_empty())
        {
            symbols.push(symbol);
        }
        let target_stem = file_stem(&target_path);
        for symbol in symbols {
            self.add_edge(
                &self.file_id.clone(),
                &make_id(&[&target_stem, &symbol]),
                "imports",
                line(declaration),
                Some("import"),
            );
        }
        true
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

    fn add_ts_class_decorators(&mut self, node: Node<'tree>, class_id: &str) {
        let mut decorators = Vec::new();
        let mut cursor = node.walk();
        decorators.extend(
            node.children(&mut cursor)
                .filter(|child| child.kind() == "decorator"),
        );
        if let Some(parent) = node
            .parent()
            .filter(|parent| parent.kind() == "export_statement")
        {
            let mut cursor = parent.walk();
            for child in parent.children(&mut cursor) {
                if child.kind() == "decorator" {
                    decorators.push(child);
                } else if matches!(
                    child.kind(),
                    "class_declaration" | "abstract_class_declaration"
                ) {
                    break;
                }
            }
        }
        for decorator in decorators {
            let mut cursor = decorator.walk();
            let Some(mut target) = decorator
                .children(&mut cursor)
                .find(|child| child.is_named())
            else {
                continue;
            };
            if target.kind() == "call_expression" {
                target = target.child_by_field_name("function").unwrap_or(target);
            }
            if target.kind() == "member_expression" {
                let Some(property) = target.child_by_field_name("property") else {
                    continue;
                };
                target = property;
            }
            if target.kind() != "identifier" {
                continue;
            }
            let Some(name) = self
                .node_text(target)
                .map(clean_name)
                .filter(|name| !name.is_empty())
            else {
                continue;
            };
            let target_id = self.ensure_type_node(&name, true);
            if target_id != class_id {
                self.add_edge(
                    class_id,
                    &target_id,
                    "references",
                    line(decorator),
                    Some("decorator"),
                );
            }
        }
    }

    fn add_python_parent_edges(&mut self, node: Node<'tree>, class_id: &str) {
        let Some(superclasses) = node.child_by_field_name("superclasses") else {
            return;
        };
        let mut cursor = superclasses.walk();
        for superclass in superclasses
            .children(&mut cursor)
            .filter(|child| child.kind() == "identifier")
        {
            let Some(text) = self.node_text(superclass) else {
                continue;
            };
            let name = text;
            if name.is_empty() {
                continue;
            }
            let target = self.ensure_type_node(&name, true);
            self.add_edge(class_id, &target, "inherits", line(node), None);
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

    fn add_python_function_references(&mut self, node: Node<'tree>, function_id: &str) {
        if let Some(parameters) = node.child_by_field_name("parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters.children(&mut cursor).filter(|parameter| {
                matches!(
                    parameter.kind(),
                    "typed_parameter" | "typed_default_parameter"
                )
            }) {
                let mut references = Vec::new();
                collect_python_type_references(
                    parameter.child_by_field_name("type"),
                    self.source,
                    false,
                    &mut references,
                );
                self.emit_python_type_references(
                    function_id,
                    references,
                    "parameter_type",
                    line(node),
                );
            }
        }
        let mut references = Vec::new();
        collect_python_type_references(
            node.child_by_field_name("return_type"),
            self.source,
            false,
            &mut references,
        );
        self.emit_python_type_references(function_id, references, "return_type", line(node));
    }

    fn emit_python_type_references(
        &mut self,
        function_id: &str,
        references: Vec<(String, bool)>,
        ordinary_context: &str,
        line: usize,
    ) {
        for (name, generic) in references {
            let target = self.ensure_type_node(&name, true);
            if target != function_id {
                self.add_edge(
                    function_id,
                    &target,
                    "references",
                    line,
                    Some(if generic {
                        "generic_arg"
                    } else {
                        ordinary_context
                    }),
                );
            }
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
        self.types
            .entry(name.to_owned())
            .or_insert_with(|| id.clone());
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

fn collect_python_type_references(
    node: Option<Node<'_>>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    const CONTAINERS: &[&str] = &[
        "list",
        "dict",
        "set",
        "tuple",
        "frozenset",
        "type",
        "List",
        "Dict",
        "Set",
        "Tuple",
        "FrozenSet",
        "Type",
        "Optional",
        "Union",
        "Sequence",
        "Iterable",
        "Mapping",
        "MutableMapping",
        "Iterator",
        "Callable",
        "Awaitable",
        "AsyncIterable",
        "AsyncIterator",
        "Coroutine",
        "Generator",
        "AsyncGenerator",
        "ContextManager",
        "AsyncContextManager",
        "Annotated",
        "ClassVar",
        "Final",
        "Literal",
        "Concatenate",
        "ParamSpec",
        "TypeVar",
        "None",
        "Ellipsis",
    ];
    const NOISE: &[&str] = &[
        "str",
        "int",
        "float",
        "bool",
        "bytes",
        "bytearray",
        "complex",
        "object",
        "True",
        "False",
        "MagicMock",
        "Mock",
        "AsyncMock",
        "NonCallableMock",
        "NonCallableMagicMock",
        "PropertyMock",
        "patch",
        "sentinel",
    ];
    let Some(node) = node else {
        return;
    };
    let accepted =
        |name: &str| !name.is_empty() && !CONTAINERS.contains(&name) && !NOISE.contains(&name);
    match node.kind() {
        "identifier" => {
            if let Ok(name) = node.utf8_text(source)
                && accepted(name)
            {
                output.push((name.to_owned(), generic));
            }
        }
        "attribute" => {
            if let Ok(text) = node.utf8_text(source) {
                let name = text.rsplit('.').next().unwrap_or_default();
                if accepted(name) {
                    output.push((name.to_owned(), generic));
                }
            }
        }
        "generic_type" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    if let Ok(name) = child.utf8_text(source)
                        && accepted(name)
                    {
                        output.push((name.to_owned(), generic));
                    }
                } else if child.kind() == "type_parameter" {
                    let mut nested = child.walk();
                    for argument in child.children(&mut nested).filter(|child| child.is_named()) {
                        collect_python_type_references(Some(argument), source, true, output);
                    }
                }
            }
        }
        "subscript" => {
            let value = node.child_by_field_name("value");
            collect_python_type_references(value, source, generic, output);
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| {
                child.is_named() && value.is_none_or(|value| child.id() != value.id())
            }) {
                collect_python_type_references(Some(child), source, true, output);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                collect_python_type_references(Some(child), source, generic, output);
            }
        }
    }
}

fn python_import_aliases(root: Node<'_>, source: &[u8]) -> HashMap<String, String> {
    fn collect(node: Node<'_>, source: &[u8], output: &mut HashMap<String, String>) {
        if node.kind() == "import_from_statement" {
            let mut past_import = false;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "import" {
                    past_import = true;
                    continue;
                }
                if !past_import {
                    continue;
                }
                let (imported, local) = if child.kind() == "aliased_import" {
                    let imported = child
                        .child_by_field_name("name")
                        .and_then(|name| name.utf8_text(source).ok());
                    let local = child
                        .child_by_field_name("alias")
                        .and_then(|name| name.utf8_text(source).ok());
                    (imported, local)
                } else if matches!(child.kind(), "dotted_name" | "identifier") {
                    let name = child.utf8_text(source).ok();
                    (name, name)
                } else {
                    (None, None)
                };
                if let (Some(imported), Some(local)) = (imported, local) {
                    let imported = imported.rsplit('.').next().unwrap_or_default().trim();
                    let local = local.rsplit('.').next().unwrap_or_default().trim();
                    if !imported.is_empty() && !local.is_empty() && imported != "*" {
                        output.insert(local.to_owned(), imported.to_owned());
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect(child, source, output);
        }
    }
    let mut output = HashMap::new();
    // Graphify's collection-level symbol pass indexes every `from ... import`
    // alias in the file, including function-local imports. It then scans only
    // undecorated top-level function bodies for uses. Preserve that observable
    // ordering here; `FunctionBody::top_level` supplies the matching use gate.
    collect(root, source, &mut output);
    output
}

fn python_bound_names(node: Node<'_>, source: &[u8], module: bool) -> HashSet<String> {
    let mut output = HashSet::new();
    if !module && let Some(parameters) = node.child_by_field_name("parameters") {
        let mut cursor = parameters.walk();
        for parameter in parameters.children(&mut cursor) {
            if parameter.kind() == "identifier" {
                collect_python_assignment_targets(Some(parameter), source, &mut output);
            } else if matches!(
                parameter.kind(),
                "typed_parameter"
                    | "default_parameter"
                    | "typed_default_parameter"
                    | "list_splat_pattern"
                    | "dictionary_splat_pattern"
            ) {
                let name = parameter.child_by_field_name("name").or_else(|| {
                    let mut nested = parameter.walk();
                    parameter
                        .children(&mut nested)
                        .find(|child| child.kind() == "identifier")
                });
                collect_python_assignment_targets(name, source, &mut output);
            }
        }
    }
    fn walk(node: Node<'_>, source: &[u8], root: bool, output: &mut HashSet<String>) {
        if !root && matches!(node.kind(), "function_definition" | "class_definition") {
            return;
        }
        match node.kind() {
            "assignment" | "for_statement" | "for_in_clause" => {
                collect_python_assignment_targets(node.child_by_field_name("left"), source, output);
            }
            "with_statement" => {
                let mut cursor = node.walk();
                for clause in node
                    .children(&mut cursor)
                    .filter(|child| child.kind() == "with_clause")
                {
                    let mut nested = clause.walk();
                    for item in clause
                        .children(&mut nested)
                        .filter(|child| child.kind() == "with_item")
                    {
                        collect_python_assignment_targets(
                            item.child_by_field_name("alias"),
                            source,
                            output,
                        );
                    }
                }
            }
            "named_expression" => {
                collect_python_assignment_targets(node.child_by_field_name("name"), source, output)
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, source, false, output);
        }
    }
    let start = if module {
        Some(node)
    } else {
        node.child_by_field_name("body")
    };
    if let Some(start) = start {
        walk(start, source, true, &mut output);
    }
    output
}

fn collect_python_assignment_targets(
    node: Option<Node<'_>>,
    source: &[u8],
    output: &mut HashSet<String>,
) {
    let Some(node) = node else {
        return;
    };
    if node.kind() == "identifier" {
        if let Ok(name) = node.utf8_text(source) {
            output.insert(name.to_owned());
        }
    } else if matches!(
        node.kind(),
        "pattern_list" | "tuple_pattern" | "list_pattern"
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_python_assignment_targets(Some(child), source, output);
        }
    }
}

fn js_module_bound_names(root: Node<'_>, source: &[u8]) -> HashSet<String> {
    fn collect_pattern(node: Node<'_>, source: &[u8], output: &mut HashSet<String>) {
        if matches!(
            node.kind(),
            "identifier"
                | "shorthand_property_identifier_pattern"
                | "shorthand_property_identifier"
        ) {
            if let Ok(name) = node.utf8_text(source) {
                output.insert(name.to_owned());
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            collect_pattern(child, source, output);
        }
    }

    fn walk(node: Node<'_>, source: &[u8], root: bool, output: &mut HashSet<String>) {
        if !root
            && matches!(
                node.kind(),
                "function_declaration"
                    | "function_expression"
                    | "arrow_function"
                    | "generator_function_declaration"
                    | "generator_function"
                    | "class_declaration"
                    | "class"
            )
        {
            return;
        }
        if node.kind() == "variable_declarator" {
            let value_is_function = node.child_by_field_name("value").is_some_and(|value| {
                matches!(
                    value.kind(),
                    "arrow_function" | "function_expression" | "function" | "generator_function"
                )
            });
            if !value_is_function && let Some(name) = node.child_by_field_name("name") {
                collect_pattern(name, source, output);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            walk(child, source, false, output);
        }
    }

    let mut output = HashSet::new();
    walk(root, source, true, &mut output);
    output
}

fn collect_js_collection_values<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    if node.kind() == "object" {
        for property in node.children(&mut cursor) {
            if property.kind() == "pair" {
                if let Some(value) = property.child_by_field_name("value")
                    && value.kind() == "identifier"
                {
                    output.push(value);
                }
            } else if property.kind() == "shorthand_property_identifier" {
                output.push(property);
            }
        }
        return;
    }
    for element in node.children(&mut cursor).filter(|child| child.is_named()) {
        if element.kind() == "identifier" {
            output.push(element);
        }
    }
}

fn collect_python_collection_values<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    if node.kind() == "dictionary" {
        for pair in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "pair")
        {
            if let Some(value) = pair.child_by_field_name("value")
                && value.kind() == "identifier"
            {
                output.push(value);
            }
        }
        return;
    }
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if child.kind() == "identifier" {
            output.push(child);
        }
    }
}

fn collect_python_reference_values<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    if node.kind() == "identifier" {
        output.push(node);
    } else if node.kind() == "expression_list" {
        let mut cursor = node.walk();
        for child in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "identifier")
        {
            output.push(child);
        }
    }
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
        if let Some(start) = value.find(quote) {
            let rest = &value[start + quote.len_utf8()..];
            if let Some(end) = rest.find(quote) {
                return Some(rest[..end].to_owned());
            }
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

fn find_require_call<'tree>(node: Node<'tree>, source: &[u8]) -> Option<Node<'tree>> {
    if node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .is_some_and(|function| source_node_text(function, source) == "require")
    {
        return Some(node);
    }
    if node.kind() == "member_expression"
        && let Some(object) = node.child_by_field_name("object")
    {
        return find_require_call(object, source);
    }
    None
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

fn resolve_js_import_path(path: &Path) -> std::path::PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    if path.extension().and_then(|value| value.to_str()) == Some("js") {
        let candidate = path.with_extension("ts");
        if candidate.is_file() {
            return candidate;
        }
    } else if path.extension().and_then(|value| value.to_str()) == Some("jsx") {
        let candidate = path.with_extension("tsx");
        if candidate.is_file() {
            return candidate;
        }
    }
    for extension in [
        "ts", "tsx", "mts", "cts", "svelte", "js", "jsx", "mjs", "cjs",
    ] {
        let candidate = path.with_file_name(format!(
            "{}.{extension}",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
        ));
        if candidate.is_file() {
            return candidate;
        }
    }
    if path.is_dir() {
        for name in [
            "index.ts",
            "index.tsx",
            "index.svelte",
            "index.js",
            "index.jsx",
            "index.mjs",
        ] {
            let candidate = path.join(name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    path.to_path_buf()
}

#[cfg(test)]
mod rationale_tests {
    use super::*;

    #[test]
    fn python_import_alias_uses_match_top_level_function_scan()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("aliases.py");
        fs::write(
            &source,
            "from package import top as module_alias\n\
             def first():\n    module_alias()\n\
             @marker\n\
             def decorated():\n    from package import nested as local_alias\n    local_alias()\n\
             def third():\n    local_alias()\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let import_uses = extraction
            .raw_calls
            .unwrap_or_default()
            .into_iter()
            .filter(|call| {
                call.extensions
                    .get("symbol_import_use")
                    .and_then(Value::as_bool)
                    == Some(true)
            })
            .collect::<Vec<_>>();

        assert_eq!(import_uses.len(), 2);
        assert_eq!(import_uses[0].callee, "top");
        assert_eq!(import_uses[0].source_location, "L3");
        assert_eq!(import_uses[1].callee, "nested");
        assert_eq!(import_uses[1].source_location, "L9");
        Ok(())
    }

    #[test]
    fn rust_calls_named_like_shared_builtins_are_suppressed()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("builtins.rs");
        fs::write(
            &source,
            "fn open() {}\nfn list() {}\nfn caller() { open(); list(); }\n",
        )?;

        let extraction = Engine::default().extract(&source)?;
        let caller = extraction
            .nodes
            .iter()
            .find(|node| node.label() == "caller()")
            .map(|node| node.id.as_str())
            .ok_or("missing caller")?;
        assert!(!extraction.edges.iter().any(|edge| {
            edge.source == caller
                && edge.attributes.get("relation").and_then(Value::as_str) == Some("calls")
        }));
        Ok(())
    }

    #[test]
    fn python_imported_module_member_calls_are_deferred_as_resolvable_symbols()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("cli.py");
        fs::write(
            &source,
            "def dispatch():\n    from graphify import querylog\n    querylog.log_query(kind='query')\n",
        )?;

        let extraction = Engine::default().extract(&source)?;

        assert!(extraction.raw_calls.unwrap_or_default().iter().any(|call| {
            call.callee == "log_query"
                && call.is_member_call == Some(true)
                && call.receiver.as_ref().and_then(|value| value.as_deref()) == Some("querylog")
                && call.source_location == "L3"
        }));
        Ok(())
    }

    #[test]
    fn recognizes_python_module_docstring() -> Result<(), Box<dyn std::error::Error>> {
        let source = b"\"\"\"A sufficiently long architectural rationale for the module.\"\"\"\n";
        let language = tree_sitter_language_pack::get_language("python")?;
        let mut parser = Parser::new();
        parser.set_language(&language)?;
        let tree = parser.parse(source, None).ok_or("missing tree")?;
        assert_eq!(
            python_docstring(tree.root_node(), source),
            Some((
                "A sufficiently long architectural rationale for the module.".to_owned(),
                1
            )),
            "{}",
            tree.root_node().to_sexp()
        );
        Ok(())
    }
}
