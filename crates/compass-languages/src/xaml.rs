use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use compass_model::{EdgeRecord, NodeRecord};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::{Engine, ExtractError, Extraction, file_stem, make_id};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const NON_EVENT_ATTRIBUTES: &[&str] = &[
    "Name",
    "Content",
    "Text",
    "Title",
    "Tag",
    "ToolTip",
    "Header",
    "Class",
    "Key",
    "Uid",
    "DataContext",
    "Style",
    "Source",
];

static IDENTIFIER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z_]\w*$")
        .unwrap_or_else(|error| unreachable!("static XAML identifier regex is invalid: {error}"))
});
static EVENT_SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(\s*object\??\s+\w+\s*,\s*[\w.]*EventArgs(?:<[^>]*>)?\s+\w+\s*\)").unwrap_or_else(
        |error| unreachable!("static XAML event signature regex is invalid: {error}"),
    )
});
static DESIGN_INSTANCE_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bType\s*=\s*(?:\{x:Type\s+)?(?P<type>[\w.:+]+)").unwrap_or_else(|error| {
        unreachable!("static XAML design-instance regex is invalid: {error}")
    })
});
static TOOLKIT_FIELD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?P<name>_?m?_?[A-Za-z_]\w*)\s*(?:=.*)?;")
        .unwrap_or_else(|error| unreachable!("static toolkit field regex is invalid: {error}"))
});
static TOOLKIT_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?P<name>[A-Za-z_]\w*)\s*\(")
        .unwrap_or_else(|error| unreachable!("static toolkit method regex is invalid: {error}"))
});

pub(crate) fn extract(engine: &mut Engine, path: &Path) -> Result<Extraction, ExtractError> {
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
        return Ok(failure("xaml file too large"));
    }
    let lower = source.to_ascii_lowercase();
    if lower.windows(9).any(|window| window == b"<!doctype")
        || lower.windows(8).any(|window| window == b"<!entity")
    {
        return Ok(failure("refusing XML with DOCTYPE/ENTITY declaration"));
    }
    let text = String::from_utf8_lossy(&source);
    let document = match roxmltree::Document::parse(&text) {
        Ok(document) => document,
        Err(error) => return Ok(failure(&format!("XML parse error: {error}"))),
    };
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let root = document.root_element();
    let root_type = root.tag_name().name();
    let root_id = make_id(&[&stem, root_type]);
    let mut state = State {
        source_file: source_file.clone(),
        stem,
        lines: text.lines().map(str::to_owned).collect(),
        extraction: empty(),
        seen_nodes: HashSet::new(),
        seen_edges: HashSet::new(),
    };
    state.add_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        Some(1),
        "code",
        &source_file,
    );
    state.add_node(root_id.clone(), root_type, Some(1), "code", &source_file);
    state.add_edge(&file_id, &root_id, "contains", 1, None, "EXTRACTED");

    let class_name = root
        .attributes()
        .find(|attribute| attribute.name() == "Class" && !attribute.value().is_empty())
        .map(|attribute| attribute.value().trim().to_owned());
    let codebehind = codebehind_symbols(engine, path, class_name.as_deref());
    if let Some(class_name) = &class_name {
        let class_id = if let Some(class_node) = &codebehind.class_node {
            state.add_existing_node(class_node);
            class_node.id.clone()
        } else {
            let label = class_name.rsplit('.').next().unwrap_or(class_name);
            let id = make_id(&[&state.stem, label]);
            state.add_node(
                id.clone(),
                label,
                Some(state.line_for(Some(class_name))),
                "code",
                &source_file,
            );
            id
        };
        state.add_edge(
            &root_id,
            &class_id,
            "references",
            state.line_for(Some(class_name)),
            Some("x_class"),
            "EXTRACTED",
        );
    }

    let (has_data_context, mut viewmodels) = explicit_viewmodel_names(root);
    let prism = prism_autowire(root);
    let mut viewmodel_confidence = "EXTRACTED";
    if !has_data_context {
        let view_name = class_name
            .as_deref()
            .and_then(|name| name.rsplit('.').next())
            .or_else(|| {
                prism
                    .then(|| path.file_stem().and_then(|name| name.to_str()))
                    .flatten()
            });
        viewmodels = inferred_viewmodel_names(view_name);
        viewmodel_confidence = "INFERRED";
    }
    let mut generated_members = HashMap::new();
    if !viewmodels.is_empty() {
        let classes = csharp_viewmodels(engine, path);
        let candidates: HashMap<String, NodeRecord> = viewmodels
            .iter()
            .flat_map(|name| classes.get(name).into_iter().flatten())
            .map(|node| (node.id.clone(), node.clone()))
            .collect();
        if candidates.len() == 1
            && let Some(viewmodel) = candidates.values().next()
        {
            state.add_existing_node(viewmodel);
            state.add_edge(
                &root_id,
                &viewmodel.id,
                "references",
                state.line_for(Some(viewmodel.label())),
                Some("view_model"),
                viewmodel_confidence,
            );
            let (members, edges) = community_toolkit_members(viewmodel);
            for member in members.values() {
                state.add_existing_node(member);
            }
            for edge in edges {
                state.add_existing_edge(&edge);
            }
            generated_members = members;
        }
    }

    for element in root.descendants().filter(roxmltree::Node::is_element) {
        let element_type = element.tag_name().name();
        let element_name = element
            .attributes()
            .find(|attribute| attribute.name() == "Name" && !attribute.value().is_empty())
            .map(|attribute| attribute.value().trim());
        let owner_id = if let Some(name) = element_name {
            let id = make_id(&[&state.stem, name]);
            let line = state.line_for(Some(name));
            state.add_node(id.clone(), name, Some(line), "code", &source_file);
            state.add_edge(&root_id, &id, "contains", line, None, "EXTRACTED");
            let type_id = make_id(&["xaml", element_type]);
            state.add_node(
                type_id.clone(),
                element_type,
                Some(line),
                "concept",
                &source_file,
            );
            state.add_edge(&id, &type_id, "references", line, Some("type"), "EXTRACTED");
            id
        } else {
            root_id.clone()
        };

        for attribute in element.attributes() {
            let attribute_name = attribute.name();
            let value = attribute.value();
            if !NON_EVENT_ATTRIBUTES.contains(&attribute_name)
                && IDENTIFIER.is_match(value)
                && let Some(method) = codebehind.methods.get(value)
            {
                state.add_existing_node(method);
                state.add_edge(
                    &owner_id,
                    &method.id,
                    "references",
                    state.line_for(Some(value)),
                    Some("event"),
                    "EXTRACTED",
                );
                if let Some(method_edge) = codebehind
                    .method_edges
                    .iter()
                    .find(|edge| edge.target == method.id)
                {
                    if let Some(class_node) = &codebehind.class_node {
                        state.add_existing_node(class_node);
                    }
                    state.add_existing_edge(method_edge);
                }
            }
            let (binding_path, converter) = binding_references(value);
            if let Some(binding_path) = binding_path {
                let id = make_id(&["binding", &binding_path]);
                let line = state.line_for(Some(value));
                state.add_node(
                    id.clone(),
                    &binding_path,
                    Some(line),
                    "concept",
                    &source_file,
                );
                let context = if attribute_name == "Command" || attribute_name.ends_with(".Command")
                {
                    "binding_command"
                } else {
                    "binding_path"
                };
                state.add_edge(
                    &owner_id,
                    &id,
                    "references",
                    line,
                    Some(context),
                    "EXTRACTED",
                );
                if let Some(member) = generated_members.get(&binding_path) {
                    state.add_existing_node(member);
                    state.add_edge(
                        &owner_id,
                        &member.id,
                        "references",
                        line,
                        Some(context),
                        "INFERRED",
                    );
                }
            }
            if let Some(converter) = converter {
                let id = make_id(&["binding_converter", &converter]);
                let line = state.line_for(Some(value));
                state.add_node(id.clone(), &converter, Some(line), "concept", &source_file);
                state.add_edge(
                    &owner_id,
                    &id,
                    "references",
                    line,
                    Some("binding_converter"),
                    "EXTRACTED",
                );
            }
            if element_type == "Binding" && attribute_name == "Path" {
                let direct = value.trim();
                if !direct.is_empty() && !direct.contains(['{', '}']) {
                    let id = make_id(&["binding", direct]);
                    let line = state.line_for(Some(value));
                    state.add_node(id.clone(), direct, Some(line), "concept", &source_file);
                    state.add_edge(
                        &owner_id,
                        &id,
                        "references",
                        line,
                        Some("binding_path"),
                        "EXTRACTED",
                    );
                }
            }
            if element_type == "Binding"
                && attribute_name == "Converter"
                && let Some(converter) = static_resource_key(value)
            {
                let id = make_id(&["binding_converter", &converter]);
                let line = state.line_for(Some(value));
                state.add_node(id.clone(), &converter, Some(line), "concept", &source_file);
                state.add_edge(
                    &owner_id,
                    &id,
                    "references",
                    line,
                    Some("binding_converter"),
                    "EXTRACTED",
                );
            }
        }
    }
    Ok(state.extraction)
}

struct Codebehind {
    class_node: Option<NodeRecord>,
    methods: HashMap<String, NodeRecord>,
    method_edges: Vec<EdgeRecord>,
}

fn codebehind_symbols(engine: &mut Engine, path: &Path, class_name: Option<&str>) -> Codebehind {
    let Some(codebehind_path) = codebehind_path(path) else {
        return Codebehind {
            class_node: None,
            methods: HashMap::new(),
            method_edges: Vec::new(),
        };
    };
    let Ok(extraction) = engine.extract(&codebehind_path) else {
        return Codebehind {
            class_node: None,
            methods: HashMap::new(),
            method_edges: Vec::new(),
        };
    };
    let simple = class_name.and_then(|name| name.rsplit('.').next());
    let class_node = simple
        .and_then(|name| extraction.nodes.iter().find(|node| node.label() == name))
        .cloned();
    let method_edges: Vec<EdgeRecord> = class_node.as_ref().map_or_else(Vec::new, |class_node| {
        extraction
            .edges
            .iter()
            .filter(|edge| {
                edge.source == class_node.id
                    && edge.attributes.get("relation").and_then(Value::as_str) == Some("method")
            })
            .cloned()
            .collect()
    });
    let method_ids: Option<HashSet<&str>> = class_node.as_ref().map(|_| {
        method_edges
            .iter()
            .map(|edge| edge.target.as_str())
            .collect()
    });
    let lines = fs::read(&codebehind_path).map_or_else(
        |_| Vec::new(),
        |bytes| {
            String::from_utf8_lossy(&bytes)
                .lines()
                .map(str::to_owned)
                .collect()
        },
    );
    let mut methods = HashMap::new();
    for node in extraction.nodes {
        if method_ids
            .as_ref()
            .is_some_and(|ids| !ids.contains(node.id.as_str()))
        {
            continue;
        }
        let label = node.label();
        if !label.starts_with('.') || !label.ends_with("()") || !has_event_signature(&node, &lines)
        {
            continue;
        }
        methods.insert(label.trim_matches(['.', '(', ')']).to_owned(), node);
    }
    Codebehind {
        class_node,
        methods,
        method_edges,
    }
}

fn has_event_signature(node: &NodeRecord, lines: &[String]) -> bool {
    let Some(line) = node
        .attributes
        .get("source_location")
        .and_then(Value::as_str)
        .and_then(|location| location.strip_prefix('L'))
        .and_then(|line| line.parse::<usize>().ok())
    else {
        return false;
    };
    EVENT_SIGNATURE.is_match(
        &lines
            .iter()
            .skip(line.saturating_sub(1))
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn codebehind_path(path: &Path) -> Option<PathBuf> {
    let expected = PathBuf::from(format!("{}.cs", path.to_string_lossy()));
    if expected.exists() {
        return Some(expected);
    }
    let expected_name = expected.file_name()?.to_str()?;
    fs::read_dir(path.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|sibling| {
            sibling
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(expected_name))
        })
}

struct State {
    source_file: String,
    stem: String,
    lines: Vec<String>,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<(String, String, String, Option<String>)>,
}

impl State {
    fn line_for(&self, value: Option<&str>) -> usize {
        value
            .and_then(|value| self.lines.iter().position(|line| line.contains(value)))
            .map_or(1, |index| index + 1)
    }

    fn add_node(
        &mut self,
        id: String,
        label: &str,
        line: Option<usize>,
        file_type: &str,
        source_file: &str,
    ) {
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String(file_type.to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(source_file.to_owned()),
        );
        attributes.insert(
            "source_location".to_owned(),
            line.map_or(Value::Null, |line| Value::String(format!("L{line}"))),
        );
        self.extraction.nodes.push(NodeRecord { id, attributes });
    }

    fn add_existing_node(&mut self, node: &NodeRecord) {
        if self.seen_nodes.insert(node.id.clone()) {
            self.extraction.nodes.push(node.clone());
        }
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        line: usize,
        context: Option<&str>,
        confidence: &str,
    ) {
        let key = (
            source.to_owned(),
            target.to_owned(),
            relation.to_owned(),
            context.map(str::to_owned),
        );
        if !self.seen_edges.insert(key) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
        attributes.insert(
            "confidence".to_owned(),
            Value::String(confidence.to_owned()),
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
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn add_existing_edge(&mut self, edge: &EdgeRecord) {
        let key = (
            edge.source.clone(),
            edge.target.clone(),
            edge.attributes
                .get("relation")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            edge.attributes
                .get("context")
                .and_then(Value::as_str)
                .map(str::to_owned),
        );
        if self.seen_edges.insert(key) {
            self.extraction.edges.push(edge.clone());
        }
    }
}

fn markup_extension(value: &str) -> Option<(&str, &str)> {
    let value = value.trim();
    let inner = value.strip_prefix('{')?.strip_suffix('}')?.trim();
    if inner.is_empty() || inner.starts_with('}') {
        return None;
    }
    Some(
        inner
            .split_once(' ')
            .map_or((inner, ""), |(name, args)| (name, args.trim())),
    )
}

fn split_markup_arguments(arguments: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    for (index, character) in arguments.char_indices() {
        match character {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                output.push(arguments[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = arguments[start..].trim();
    if !tail.is_empty() {
        output.push(tail);
    }
    output
}

fn static_resource_key(value: &str) -> Option<String> {
    let (name, arguments) = markup_extension(value)?;
    if name != "StaticResource" {
        return None;
    }
    for part in split_markup_arguments(arguments) {
        if let Some((key, resource)) = part.split_once('=') {
            if key.trim() == "ResourceKey" && !resource.trim().is_empty() {
                return Some(resource.trim().to_owned());
            }
        } else if !part.is_empty() {
            return Some(part.to_owned());
        }
    }
    None
}

fn binding_references(value: &str) -> (Option<String>, Option<String>) {
    let Some((name, arguments)) = markup_extension(value) else {
        return (None, None);
    };
    if name != "Binding" {
        return (None, None);
    }
    let mut path = None;
    let mut converter = None;
    for part in split_markup_arguments(arguments) {
        if let Some((key, value)) = part.split_once('=') {
            match key.trim() {
                "Path" => path = Some(value.trim().to_owned()),
                "Converter" => converter = static_resource_key(value.trim()),
                _ => {}
            }
        } else if path.is_none() && !part.is_empty() {
            path = Some(part.to_owned());
        }
    }
    if path.as_ref().is_some_and(|path| path.contains(['{', '}'])) {
        path = None;
    }
    (path.filter(|path| !path.is_empty()), converter)
}

fn explicit_viewmodel_names(root: roxmltree::Node<'_, '_>) -> (bool, Vec<String>) {
    let mut has_data_context = false;
    let mut names = Vec::new();
    for element in root.descendants().filter(roxmltree::Node::is_element) {
        let element_type = element.tag_name().name();
        if element_type.ends_with(".DataContext") || element_type == "DataContext" {
            has_data_context = true;
            for child in element.children().filter(roxmltree::Node::is_element) {
                if let Some(name) = simple_type_name(child.tag_name().name())
                    && !names.contains(&name)
                {
                    names.push(name);
                }
            }
        }
        for attribute in element
            .attributes()
            .filter(|attribute| attribute.name() == "DataContext" && !attribute.value().is_empty())
        {
            has_data_context = true;
            if let Some(captures) = DESIGN_INSTANCE_TYPE.captures(attribute.value())
                && let Some(name) = captures
                    .name("type")
                    .and_then(|name| simple_type_name(name.as_str()))
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
    }
    (has_data_context, names)
}

fn prism_autowire(root: roxmltree::Node<'_, '_>) -> bool {
    root.descendants()
        .filter(roxmltree::Node::is_element)
        .any(|element| {
            element.attributes().any(|attribute| {
                attribute
                    .name()
                    .ends_with("ViewModelLocator.AutoWireViewModel")
                    && attribute.value().trim().eq_ignore_ascii_case("true")
            })
        })
}

fn simple_type_name(reference: &str) -> Option<String> {
    let mut reference = reference.trim().trim_matches(['{', '}']);
    reference = reference.split(',').next().unwrap_or(reference).trim();
    reference = reference
        .strip_prefix("x:Type ")
        .unwrap_or(reference)
        .trim();
    reference = reference
        .rsplit([':', '.', '+'])
        .next()
        .unwrap_or(reference);
    IDENTIFIER.is_match(reference).then(|| reference.to_owned())
}

fn inferred_viewmodel_names(view: Option<&str>) -> Vec<String> {
    let Some(view) = view else { return Vec::new() };
    let mut names = Vec::new();
    if view == "MainWindow" {
        names.push("MainWindowViewModel".to_owned());
        names.push("MainViewModel".to_owned());
    }
    for suffix in ["UserControl", "View", "Page", "Control"] {
        if let Some(prefix) = view.strip_suffix(suffix)
            && !prefix.is_empty()
        {
            let name = format!("{prefix}ViewModel");
            if !names.contains(&name) {
                names.push(name);
            }
            break;
        }
    }
    names
}

fn csharp_viewmodels(engine: &mut Engine, path: &Path) -> HashMap<String, Vec<NodeRecord>> {
    let root = project_root(path);
    let mut files = Vec::new();
    collect_csharp_files(&root, &mut files);
    files.sort();
    let mut classes: HashMap<String, Vec<NodeRecord>> = HashMap::new();
    for file in files {
        let Ok(extraction) = engine.extract(&file) else {
            continue;
        };
        for node in extraction.nodes {
            let label = node.label();
            if label.ends_with("ViewModel")
                && IDENTIFIER.is_match(label)
                && !node.string("source_file").is_empty()
            {
                classes.entry(label.to_owned()).or_default().push(node);
            }
        }
    }
    classes
}

fn project_root(path: &Path) -> PathBuf {
    for directory in path
        .parent()
        .into_iter()
        .flat_map(|parent| parent.ancestors())
    {
        if fs::read_dir(directory).is_ok_and(|entries| {
            entries.filter_map(Result::ok).any(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "csproj" | "fsproj" | "vbproj" | "sln" | "slnx"
                        )
                    })
            })
        }) {
            return directory.to_path_buf();
        }
    }
    path.parent().unwrap_or_else(|| Path::new("")).to_path_buf()
}

fn collect_csharp_files(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    matches!(
                        name,
                        ".git" | "node_modules" | "target" | "bin" | "obj" | ".venv" | "venv"
                    )
                })
            {
                continue;
            }
            collect_csharp_files(&path, output);
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cs"))
        {
            output.push(path);
        }
    }
}

fn community_toolkit_members(
    viewmodel: &NodeRecord,
) -> (HashMap<String, NodeRecord>, Vec<EdgeRecord>) {
    let source_file = viewmodel.string("source_file");
    if source_file.is_empty() {
        return (HashMap::new(), Vec::new());
    }
    let Ok(bytes) = fs::read(&source_file) else {
        return (HashMap::new(), Vec::new());
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut members = HashMap::new();
    let mut edges = Vec::new();
    let mut pending: Option<(&str, usize)> = None;
    for (index, original) in text.lines().enumerate() {
        let line_number = index + 1;
        let remainder = original.split_once(']').map_or("", |(_, rest)| rest.trim());
        let mut line = original;
        if original.contains('[') && original.contains("ObservableProperty") {
            pending = Some(("property", line_number));
            if remainder.is_empty() {
                continue;
            }
            line = remainder;
        }
        if original.contains('[') && original.contains("RelayCommand") {
            pending = Some(("command", line_number));
            if remainder.is_empty() {
                continue;
            }
            line = remainder;
        }
        if pending.is_none() || line.trim().is_empty() || line.trim_start().starts_with('[') {
            continue;
        }
        let Some((kind, attribute_line)) = pending.take() else {
            continue;
        };
        let label = if kind == "property" {
            TOOLKIT_FIELD
                .captures(line)
                .and_then(|captures| captures.name("name"))
                .and_then(|name| pascal_name(name.as_str()))
        } else {
            TOOLKIT_METHOD
                .captures(line)
                .and_then(|captures| captures.name("name"))
                .map(|name| {
                    format!(
                        "{}Command",
                        name.as_str().strip_suffix("Async").unwrap_or(name.as_str())
                    )
                })
        };
        let Some(label) = label else { continue };
        let id = make_id(&[&viewmodel.id, &label]);
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.clone()));
        attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
        attributes.insert("source_file".to_owned(), Value::String(source_file.clone()));
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{attribute_line}")),
        );
        members.insert(
            label.clone(),
            NodeRecord {
                id: id.clone(),
                attributes,
            },
        );
        let mut edge_attributes = Map::new();
        edge_attributes.insert("relation".to_owned(), Value::String("defines".to_owned()));
        edge_attributes.insert(
            "confidence".to_owned(),
            Value::String("INFERRED".to_owned()),
        );
        edge_attributes.insert("source_file".to_owned(), Value::String(source_file.clone()));
        edge_attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{attribute_line}")),
        );
        edge_attributes.insert("weight".to_owned(), json!(1.0));
        edge_attributes.insert(
            "context".to_owned(),
            Value::String(
                if kind == "property" {
                    "communitytoolkit_observable_property"
                } else {
                    "communitytoolkit_relay_command"
                }
                .to_owned(),
            ),
        );
        edges.push(EdgeRecord {
            source: viewmodel.id.clone(),
            target: id,
            attributes: edge_attributes,
        });
    }
    (members, edges)
}

fn pascal_name(name: &str) -> Option<String> {
    let mut name = name.trim().trim_start_matches('_');
    name = name.strip_prefix("m_").unwrap_or(name);
    if !IDENTIFIER.is_match(name) {
        return None;
    }
    let mut characters = name.chars();
    let first = characters.next()?;
    Some(first.to_uppercase().chain(characters).collect())
}

fn empty() -> Extraction {
    Extraction {
        raw_calls: None,
        ..Extraction::default()
    }
}

fn failure(message: &str) -> Extraction {
    let mut extraction = empty();
    extraction.error = Some(message.to_owned());
    extraction
}
