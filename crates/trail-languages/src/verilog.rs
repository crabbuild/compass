use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use trail_model::{EdgeRecord, NodeRecord};

use crate::{Extraction, file_stem, make_id};

const BUILTIN_TYPES: &[&str] = &[
    "bit",
    "logic",
    "reg",
    "wire",
    "int",
    "integer",
    "shortint",
    "longint",
    "byte",
    "time",
    "real",
    "shortreal",
    "void",
    "string",
    "type",
    "event",
    "mailbox",
    "semaphore",
    "process",
    "chandle",
];
const NON_TYPE_WORDS: &[&str] = &[
    "return",
    "if",
    "else",
    "for",
    "foreach",
    "while",
    "case",
    "begin",
    "end",
    "function",
    "task",
    "class",
    "endclass",
    "endfunction",
    "endtask",
];

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    State::new(path, source).run()
}

struct State<'a> {
    path: &'a Path,
    source: &'a [u8],
    text: &'a str,
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
}

impl<'a> State<'a> {
    fn new(path: &'a Path, source: &'a [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        Self {
            path,
            source,
            text: std::str::from_utf8(source).unwrap_or_default(),
            stem: file_stem(path),
            file_id: make_id(&[&source_file]),
            source_file,
            extraction: Extraction {
                raw_calls: None,
                ..Extraction::default()
            },
            seen: HashSet::new(),
        }
    }

    fn run(mut self) -> Extraction {
        let label = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        self.add_node(&self.file_id.clone(), label, 1);
        self.add_modules();
        self.add_classes();
        self.extraction
    }

    fn add_modules(&mut self) {
        let Ok(modules) = Regex::new(r"(?s)\bmodule\s+([A-Za-z_]\w*)[^;]*;(.*?)\bendmodule\b")
        else {
            return;
        };
        for module in modules.captures_iter(self.text) {
            let (Some(full), Some(name_match), Some(body)) =
                (module.get(0), module.get(1), module.get(2))
            else {
                continue;
            };
            let name = name_match.as_str();
            let id = make_id(&[&self.stem, name]);
            let at = self.line_at(full.start());
            self.add_node(&id, name, at);
            self.add_edge(&self.file_id.clone(), &id, "defines", at, None);

            let mut events = module_events(body.as_str(), body.start());
            events.sort_by_key(|event| event.offset());
            for event in events {
                match event {
                    ModuleEvent::Import { offset, package } => {
                        let target = make_id(&[&package]);
                        let at = self.line_at(offset);
                        self.add_node(&target, &package, at);
                        self.add_edge(&id, &target, "imports_from", at, None);
                    }
                    ModuleEvent::Function { offset, name } => {
                        let function = make_id(&[&id, &name]);
                        let at = self.line_at(offset);
                        self.add_node(&function, &format!("{name}()"), at);
                        self.add_edge(&id, &function, "contains", at, None);
                    }
                    ModuleEvent::Task { offset, name } => {
                        let task = make_id(&[&id, &name]);
                        let at = self.line_at(offset);
                        self.add_node(&task, &name, at);
                        self.add_edge(&id, &task, "contains", at, None);
                    }
                    ModuleEvent::Instantiation { offset, type_name } => {
                        let target = make_id(&[&type_name]);
                        let at = self.line_at(offset);
                        self.add_node(&target, &type_name, at);
                        self.add_edge(&id, &target, "instantiates", at, None);
                    }
                }
            }
        }
    }

    fn add_classes(&mut self) {
        let stripped = strip_comments(self.text);
        let Ok(classes) = Regex::new(
            r"(?s)\b(?:(interface)\s+)?class\s+([A-Za-z_]\w*)([^;{]*)\s*;(.*?)\bendclass\b",
        ) else {
            return;
        };
        let mut label_to_id: HashMap<String, String> = self
            .extraction
            .nodes
            .iter()
            .filter_map(|node| {
                node.attributes
                    .get("label")
                    .and_then(Value::as_str)
                    .map(|label| (label.to_owned(), node.id.clone()))
            })
            .collect();
        for class in classes.captures_iter(&stripped) {
            let (Some(full), Some(name_match), Some(body_match)) =
                (class.get(0), class.get(2), class.get(4))
            else {
                continue;
            };
            let name = name_match.as_str();
            let header = class.get(3).map_or("", |header| header.as_str());
            let body = body_match.as_str();
            let at = line_at(stripped.as_bytes(), full.start());
            let id = make_id(&[&self.stem, name]);
            self.add_semantic_node(&id, name, at, &mut label_to_id);
            self.add_semantic_edge(&self.file_id.clone(), &id, "defines", at, None);

            let type_parameters = type_parameters(header);
            if let Some(base) = capture_one(header, r"\bextends\s+([A-Za-z_]\w*)") {
                let target = self.ensure_type(&base, at, &mut label_to_id);
                self.add_semantic_edge(&id, &target, "inherits", at, None);
            }
            if let Some(interfaces) = capture_one(header, r"\bimplements\s+([^;{]+)") {
                for interface in split_type_list(&interfaces) {
                    let label = interface.split('#').next().unwrap_or_default().trim();
                    if !label.is_empty() {
                        let target = self.ensure_type(label, at, &mut label_to_id);
                        self.add_semantic_edge(&id, &target, "implements", at, None);
                    }
                }
            }

            let without_functions = remove_function_bodies(body);
            for field in fields(&without_functions) {
                let field_line = at + without_functions[..field.offset].matches('\n').count();
                for (reference, role) in
                    collect_type_refs(&field.type_name, false, &type_parameters)
                {
                    let target = self.ensure_type(&reference, field_line, &mut label_to_id);
                    let context = if role == "generic_arg" {
                        "generic_arg"
                    } else {
                        "field"
                    };
                    self.add_semantic_edge(&id, &target, "references", field_line, Some(context));
                }
            }

            for function in class_functions(body) {
                let function_line = at + body[..function.offset].matches('\n').count();
                let function_id = make_id(&[&id, &function.name]);
                self.add_semantic_node(
                    &function_id,
                    &function.name,
                    function_line,
                    &mut label_to_id,
                );
                self.add_semantic_edge(&id, &function_id, "method", function_line, None);
                for (reference, role) in
                    collect_type_refs(&function.return_type, false, &type_parameters)
                {
                    let target = self.ensure_type(&reference, function_line, &mut label_to_id);
                    let context = if role == "generic_arg" {
                        "generic_arg"
                    } else {
                        "return_type"
                    };
                    self.add_semantic_edge(
                        &function_id,
                        &target,
                        "references",
                        function_line,
                        Some(context),
                    );
                }
                for parameter in split_type_list(&function.parameters) {
                    if let Some(type_name) = parameter_type(&parameter) {
                        for (reference, role) in
                            collect_type_refs(&type_name, false, &type_parameters)
                        {
                            let target =
                                self.ensure_type(&reference, function_line, &mut label_to_id);
                            let context = if role == "generic_arg" {
                                "generic_arg"
                            } else {
                                "parameter_type"
                            };
                            self.add_semantic_edge(
                                &function_id,
                                &target,
                                "references",
                                function_line,
                                Some(context),
                            );
                        }
                    }
                }
            }
        }
    }

    fn ensure_type(
        &mut self,
        label: &str,
        at: usize,
        labels: &mut HashMap<String, String>,
    ) -> String {
        if let Some(id) = labels.get(label) {
            return id.clone();
        }
        let id = make_id(&[&self.stem, label]);
        self.add_semantic_node(&id, label, at, labels);
        id
    }

    fn add_node(&mut self, id: &str, label: &str, at: usize) {
        if !self.seen.insert(id.to_owned()) {
            return;
        }
        let mut attributes = node_attributes(label, &self.source_file, at);
        attributes.insert("confidence_score".into(), Value::from(1.0));
        self.extraction.nodes.push(NodeRecord {
            id: id.to_owned(),
            attributes,
        });
    }

    fn add_semantic_node(
        &mut self,
        id: &str,
        label: &str,
        at: usize,
        labels: &mut HashMap<String, String>,
    ) {
        self.add_node(id, label, at);
        labels.insert(label.to_owned(), id.to_owned());
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        at: usize,
        context: Option<&str>,
    ) {
        let mut attributes = edge_attributes(relation, &self.source_file, at, context);
        attributes.insert("confidence_score".into(), Value::from(1.0));
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn add_semantic_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        at: usize,
        context: Option<&str>,
    ) {
        self.add_edge(source, target, relation, at, context);
    }

    fn line_at(&self, offset: usize) -> usize {
        line_at(self.source, offset)
    }
}

enum ModuleEvent {
    Import { offset: usize, package: String },
    Function { offset: usize, name: String },
    Task { offset: usize, name: String },
    Instantiation { offset: usize, type_name: String },
}

impl ModuleEvent {
    fn offset(&self) -> usize {
        match self {
            Self::Import { offset, .. }
            | Self::Function { offset, .. }
            | Self::Task { offset, .. }
            | Self::Instantiation { offset, .. } => *offset,
        }
    }
}

fn module_events(body: &str, base: usize) -> Vec<ModuleEvent> {
    let mut events = Vec::new();
    if let Ok(pattern) = Regex::new(r"\bimport\s+([A-Za-z_]\w*)\s*::[^;]*;") {
        for capture in pattern.captures_iter(body) {
            if let (Some(full), Some(package)) = (capture.get(0), capture.get(1)) {
                events.push(ModuleEvent::Import {
                    offset: base + full.start(),
                    package: package.as_str().to_owned(),
                });
            }
        }
    }
    if let Ok(pattern) = Regex::new(
        r"(?m)\bfunction\s+(?:automatic\s+)?[A-Za-z_]\w*(?:\s*#\s*\([^;]*?\))?\s+([A-Za-z_]\w*)\s*\([^;]*\)\s*;",
    ) {
        for capture in pattern.captures_iter(body) {
            if let (Some(full), Some(name)) = (capture.get(0), capture.get(1)) {
                events.push(ModuleEvent::Function {
                    offset: base + full.start(),
                    name: name.as_str().to_owned(),
                });
            }
        }
    }
    if let Ok(pattern) =
        Regex::new(r"(?m)\btask\s+(?:automatic\s+)?([A-Za-z_]\w*)\s*(?:\([^;]*\))?\s*;")
    {
        for capture in pattern.captures_iter(body) {
            if let (Some(full), Some(name)) = (capture.get(0), capture.get(1)) {
                events.push(ModuleEvent::Task {
                    offset: base + full.start(),
                    name: name.as_str().to_owned(),
                });
            }
        }
    }
    if let Ok(pattern) =
        Regex::new(r"(?m)^[ \t]*([A-Za-z_]\w*)\s+(?:#\s*\([^;]*\)\s*)?[A-Za-z_]\w*\s*\([^;]*\)\s*;")
    {
        for capture in pattern.captures_iter(body) {
            if let (Some(full), Some(type_name)) = (capture.get(0), capture.get(1)) {
                events.push(ModuleEvent::Instantiation {
                    offset: base + full.start(),
                    type_name: type_name.as_str().to_owned(),
                });
            }
        }
    }
    events
}

struct Field {
    offset: usize,
    type_name: String,
}

fn fields(body: &str) -> Vec<Field> {
    let Ok(pattern) = Regex::new(
        r"(?m)^[ \t\r\n]*(?:(?:rand|randc|local|protected|static|const|automatic|var)\s+)*([A-Za-z_]\w*(?:\s*#\s*\([^;]+?\))?)\s+\w+\s*;",
    ) else {
        return Vec::new();
    };
    pattern
        .captures_iter(body)
        .filter_map(|capture| {
            capture.get(1).map(|type_name| Field {
                offset: type_name.start(),
                type_name: type_name.as_str().to_owned(),
            })
        })
        .collect()
}

struct ClassFunction {
    offset: usize,
    return_type: String,
    name: String,
    parameters: String,
}

fn class_functions(body: &str) -> Vec<ClassFunction> {
    let Ok(pattern) = Regex::new(
        r"(?m)\bfunction\s+([A-Za-z_]\w*(?:\s*#\s*\((?:[^()]|\([^()]*\))*\))?)\s+(\w+)\s*\(((?:[^()]|\([^()]*\))*)\)\s*;",
    ) else {
        return Vec::new();
    };
    pattern
        .captures_iter(body)
        .filter_map(|capture| {
            Some(ClassFunction {
                offset: capture.get(0)?.start(),
                return_type: capture.get(1)?.as_str().to_owned(),
                name: capture.get(2)?.as_str().to_owned(),
                parameters: capture.get(3)?.as_str().to_owned(),
            })
        })
        .collect()
}

fn parameter_type(parameter: &str) -> Option<String> {
    let pattern = Regex::new(
        r"^\s*(?:(?:input|output|inout|ref|const\s+ref)\s+)?([A-Za-z_]\w*(?:\s*#\s*\((?:[^()]|\([^()]*\))*\))?)\s+\w+",
    )
    .ok()?;
    pattern
        .captures(parameter)
        .and_then(|capture| capture.get(1))
        .map(|value| value.as_str().to_owned())
}

fn collect_type_refs(
    type_name: &str,
    generic: bool,
    skip: &HashSet<String>,
) -> Vec<(String, &'static str)> {
    let mut references = Vec::new();
    let trimmed = type_name.trim();
    if let Ok(head) = Regex::new(r"^([A-Za-z_]\w*)")
        && let Some(name) = head
            .captures(trimmed)
            .and_then(|capture| capture.get(1))
            .map(|name| name.as_str())
        && !BUILTIN_TYPES.contains(&name)
        && !NON_TYPE_WORDS.contains(&name)
        && !skip.contains(name)
    {
        references.push((
            name.to_owned(),
            if generic { "generic_arg" } else { "type" },
        ));
    }
    if let Some(open) = trimmed.find("#(")
        && let Some(close) = matching_paren(trimmed.as_bytes(), open + 1)
    {
        for argument in split_type_list(&trimmed[open + 2..close]) {
            references.extend(collect_type_refs(&argument, true, skip));
        }
    }
    references
}

fn split_type_list(value: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        if character == '(' {
            depth += 1;
        } else if character == ')' {
            depth = depth.saturating_sub(1);
        } else if character == ',' && depth == 0 {
            let item = value[start..index].trim();
            if !item.is_empty() {
                values.push(item.to_owned());
            }
            start = index + character.len_utf8();
        }
    }
    let item = value[start..].trim();
    if !item.is_empty() {
        values.push(item.to_owned());
    }
    values
}

fn type_parameters(header: &str) -> HashSet<String> {
    Regex::new(r"\btype\s+(\w+)")
        .ok()
        .map(|pattern| {
            pattern
                .captures_iter(header)
                .filter_map(|capture| capture.get(1).map(|name| name.as_str().to_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn capture_one(value: &str, pattern: &str) -> Option<String> {
    Regex::new(pattern)
        .ok()?
        .captures(value)
        .and_then(|capture| capture.get(1))
        .map(|capture| capture.as_str().to_owned())
}

fn remove_function_bodies(body: &str) -> String {
    let Ok(pattern) = Regex::new(r"(?s)\bfunction\b.*?\bendfunction\b") else {
        return body.to_owned();
    };
    pattern
        .replace_all(body, |captures: &regex::Captures<'_>| {
            "\n".repeat(
                captures
                    .get(0)
                    .map_or(0, |value| value.as_str().matches('\n').count()),
            )
        })
        .into_owned()
}

fn strip_comments(value: &str) -> String {
    let without_blocks = Regex::new(r"(?s)/\*.*?\*/").ok().map_or_else(
        || value.to_owned(),
        |pattern| pattern.replace_all(value, "").into_owned(),
    );
    Regex::new(r"//.*")
        .ok()
        .map_or(without_blocks.clone(), |pattern| {
            pattern.replace_all(&without_blocks, "").into_owned()
        })
}

fn matching_paren(value: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    for (index, byte) in value.iter().enumerate().skip(open) {
        if *byte == b'(' {
            depth += 1;
        } else if *byte == b')' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn node_attributes(label: &str, source_file: &str, at: usize) -> Map<String, Value> {
    let mut attributes = Map::new();
    attributes.insert("label".into(), Value::String(label.to_owned()));
    attributes.insert("file_type".into(), Value::String("code".into()));
    attributes.insert("source_file".into(), Value::String(source_file.to_owned()));
    attributes.insert("source_location".into(), Value::String(format!("L{at}")));
    attributes
}

fn edge_attributes(
    relation: &str,
    source_file: &str,
    at: usize,
    context: Option<&str>,
) -> Map<String, Value> {
    let mut attributes = Map::new();
    attributes.insert("relation".into(), Value::String(relation.to_owned()));
    attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
    attributes.insert("source_file".into(), Value::String(source_file.to_owned()));
    attributes.insert("source_location".into(), Value::String(format!("L{at}")));
    attributes.insert("weight".into(), Value::from(1.0));
    if let Some(context) = context {
        attributes.insert("context".into(), Value::String(context.to_owned()));
    }
    attributes
}

fn line_at(source: &[u8], offset: usize) -> usize {
    source[..offset.min(source.len())]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1
}
