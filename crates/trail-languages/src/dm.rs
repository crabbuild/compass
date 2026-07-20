use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use flate2::read::ZlibDecoder;
use regex::Regex;
use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};
use tree_sitter::Node;

use crate::{ExtractError, Extraction, RawCall, file_stem, make_id};

pub(crate) fn extract_source(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = SourceState {
        path,
        source,
        source_file,
        stem,
        file_id: file_id.clone(),
        extraction: Extraction::default(),
        seen_nodes: HashSet::new(),
        function_bodies: Vec::new(),
    };
    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
    );
    state.walk(root, None, None);
    state.walk_calls();
    state.extraction
}

struct FunctionBody<'tree> {
    id: String,
    body: Node<'tree>,
}

struct SourceState<'path, 'source, 'tree> {
    path: &'path Path,
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    function_bodies: Vec<FunctionBody<'tree>>,
}

impl<'tree> SourceState<'_, '_, 'tree> {
    fn walk(
        &mut self,
        node: Node<'tree>,
        parent_type_path: Option<&str>,
        parent_type_id: Option<&str>,
    ) {
        let line = node.start_position().row + 1;
        match node.kind() {
            "preproc_include" => {
                self.add_include(node, line);
                return;
            }
            "type_definition" => {
                let Some(type_path_node) = child_of_kind(node, "type_path") else {
                    return;
                };
                let type_path = self.text(type_path_node).trim().to_owned();
                let type_id = self.ensure_type(&type_path, line);
                self.add_edge(&self.file_id.clone(), &type_id, "contains", line, None);
                if let Some(body) = child_of_kind(node, "type_body") {
                    let mut cursor = body.walk();
                    for child in body.children(&mut cursor) {
                        self.walk(child, Some(&type_path), Some(&type_id));
                    }
                }
                return;
            }
            "type_body_intended" | "type_body_braced" => {
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    self.walk(child, parent_type_path, parent_type_id);
                }
                return;
            }
            "type_proc_definition" | "type_proc_override" => {
                let (Some(type_path), Some(type_id)) = (parent_type_path, parent_type_id) else {
                    return;
                };
                let Some(name_node) = node.child_by_field_name("name") else {
                    return;
                };
                let name = self.text(name_node).to_owned();
                let id = make_id(&[&self.stem, type_path, &name]);
                self.add_node(id.clone(), &format!("{type_path}/{name}()"), line);
                self.add_edge(type_id, &id, "method", line, None);
                if let Some(body) = child_of_kind(node, "block") {
                    self.function_bodies.push(FunctionBody { id, body });
                }
                return;
            }
            "proc_definition" | "proc_override" => {
                self.add_proc(node, line);
                return;
            }
            "operator_override" | "type_operator_override" => return,
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, parent_type_path, parent_type_id);
        }
    }

    fn add_include(&mut self, node: Node<'tree>, line: usize) {
        let Some(file_node) = node.child_by_field_name("file") else {
            return;
        };
        let raw = if file_node.kind() == "string_literal" {
            let mut parts = String::new();
            let mut cursor = file_node.walk();
            for child in file_node.children(&mut cursor) {
                if child.kind() == "string_content" {
                    parts.push_str(self.text(child));
                }
            }
            parts
        } else {
            self.text(file_node).trim_matches(['\'', '"']).to_owned()
        };
        if raw.is_empty() {
            return;
        }
        let normalized = raw.replace('\\', "/");
        let normalized = normalized.strip_prefix("./").unwrap_or(&normalized);
        let candidate = lexical_normalize(
            &self
                .path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(normalized),
        );
        let exists = candidate.exists();
        let resolved = if exists {
            fs::canonicalize(&candidate).unwrap_or(candidate)
        } else {
            candidate
        };
        let target = if exists {
            make_id(&[&resolved.to_string_lossy()])
        } else {
            make_id(&[normalized])
        };
        let mut attributes = edge_attributes(
            &self.source_file,
            if exists { "imports_from" } else { "imports" },
            line,
            Some("import"),
        );
        if !exists {
            attributes.insert("external".to_owned(), Value::Bool(true));
        }
        self.extraction.edges.push(EdgeRecord {
            source: self.file_id.clone(),
            target,
            attributes,
        });
    }

    fn add_proc(&mut self, node: Node<'tree>, line: usize) {
        let owner_path = child_of_kind(node, "type_path")
            .map(|type_path| self.text(type_path).trim().to_owned());
        let owner_id = owner_path
            .as_deref()
            .map(|owner_path| self.ensure_type(owner_path, line));
        if let Some(owner_id) = &owner_id {
            self.add_edge(&self.file_id.clone(), owner_id, "contains", line, None);
        }
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_owned();
        let (id, label) = owner_path.as_ref().map_or_else(
            || (make_id(&[&self.stem, &name]), format!("{name}()")),
            |owner| {
                (
                    make_id(&[&self.stem, owner, &name]),
                    format!("{owner}/{name}()"),
                )
            },
        );
        self.add_node(id.clone(), &label, line);
        self.add_edge(
            owner_id.as_deref().unwrap_or(&self.file_id.clone()),
            &id,
            if owner_id.is_some() {
                "method"
            } else {
                "contains"
            },
            line,
            None,
        );
        if let Some(body) = child_of_kind(node, "block") {
            self.function_bodies.push(FunctionBody { id, body });
        }
    }

    fn walk_calls(&mut self) {
        let mut labels: HashMap<String, Vec<String>> = HashMap::new();
        let mut paths: HashMap<String, Vec<String>> = HashMap::new();
        for node in &self.extraction.nodes {
            let label = node.label().trim_matches(['(', ')']);
            let last = label.rsplit('/').next().unwrap_or(label);
            if !last.is_empty() {
                labels
                    .entry(last.to_lowercase())
                    .or_default()
                    .push(node.id.clone());
            }
            if label.starts_with('/') {
                paths
                    .entry(label.to_lowercase())
                    .or_default()
                    .push(node.id.clone());
            }
        }
        let mut seen_calls = HashSet::new();
        for function in self.function_bodies.clone() {
            self.walk_call_node(
                function.body,
                &function.id,
                &labels,
                &paths,
                &mut seen_calls,
            );
        }
    }

    fn walk_call_node(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        labels: &HashMap<String, Vec<String>>,
        paths: &HashMap<String, Vec<String>>,
        seen_calls: &mut HashSet<(String, String)>,
    ) {
        if matches!(
            node.kind(),
            "proc_definition"
                | "proc_override"
                | "type_proc_definition"
                | "type_proc_override"
                | "type_definition"
        ) {
            return;
        }
        match node.kind() {
            "call_expression" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let callee = self.text(name).to_owned();
                    if !callee.is_empty() && callee != ".." {
                        self.emit_call(
                            caller,
                            &callee,
                            node.start_position().row + 1,
                            false,
                            labels,
                            seen_calls,
                        );
                    }
                }
            }
            "field_proc_expression" => {
                if let Some(name) = node.child_by_field_name("proc") {
                    let callee = self.text(name).to_owned();
                    if !callee.is_empty() {
                        self.emit_call(
                            caller,
                            &callee,
                            node.start_position().row + 1,
                            true,
                            labels,
                            seen_calls,
                        );
                    }
                }
            }
            "new_expression" => {
                if let Some(type_path) = child_of_kind(node, "type_path") {
                    let target_text = self.text(type_path).trim().to_lowercase();
                    if let Some(candidates) = paths.get(&target_text)
                        && candidates.len() == 1
                        && candidates[0] != caller
                    {
                        let target = candidates[0].clone();
                        let pair = (caller.to_owned(), target.clone());
                        if seen_calls.insert(pair) {
                            self.extraction.edges.push(EdgeRecord {
                                source: caller.to_owned(),
                                target,
                                attributes: edge_attributes(
                                    &self.source_file,
                                    "instantiates",
                                    node.start_position().row + 1,
                                    Some("call"),
                                ),
                            });
                        }
                    }
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_call_node(child, caller, labels, paths, seen_calls);
        }
    }

    fn emit_call(
        &mut self,
        caller: &str,
        callee: &str,
        line: usize,
        is_member: bool,
        labels: &HashMap<String, Vec<String>>,
        seen_calls: &mut HashSet<(String, String)>,
    ) {
        let target = labels
            .get(&callee.to_lowercase())
            .filter(|candidates| candidates.len() == 1)
            .and_then(|candidates| candidates.first())
            .filter(|target| target.as_str() != caller)
            .cloned();
        if let Some(target) = target {
            let pair = (caller.to_owned(), target.clone());
            if seen_calls.insert(pair) {
                self.extraction.edges.push(EdgeRecord {
                    source: caller.to_owned(),
                    target,
                    attributes: edge_attributes(&self.source_file, "calls", line, Some("call")),
                });
            }
        } else {
            self.extraction.raw_calls_mut().push(RawCall {
                caller_nid: caller.to_owned(),
                callee: callee.to_owned(),
                is_member_call: Some(is_member),
                source_file: self.source_file.clone(),
                source_location: format!("L{line}"),
                receiver: None,
                receiver_type: None,
                lang: None,
            });
        }
    }

    fn ensure_type(&mut self, path: &str, line: usize) -> String {
        let id = make_id(&[&self.stem, path]);
        self.add_node(id.clone(), path, line);
        id
    }

    fn add_node(&mut self, id: String, label: &str, line: usize) {
        if id.is_empty() || !self.seen_nodes.insert(id.clone()) {
            return;
        }
        self.extraction
            .nodes
            .push(code_node(id, label, &self.source_file, &format!("L{line}")));
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        line: usize,
        context: Option<&str>,
    ) {
        if source.is_empty() || target.is_empty() || source == target {
            return;
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes: edge_attributes(&self.source_file, relation, line, context),
        });
    }

    fn text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.source).unwrap_or_default()
    }
}

impl Clone for FunctionBody<'_> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            body: self.body,
        }
    }
}

pub(crate) fn extract_asset(path: &Path) -> Result<Extraction, ExtractError> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "dmi" => extract_dmi(path),
        "dmm" => extract_dmm(path),
        "dmf" => extract_dmf(path),
        _ => Err(ExtractError::Unsupported(path.to_path_buf())),
    }
}

fn extract_dmi(path: &Path) -> Result<Extraction, ExtractError> {
    let data = fs::read(path).map_err(|source| trail_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut extraction = empty();
    extraction.nodes.push(code_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        &source_file,
        "L1",
    ));
    let description = dmi_description(&data);
    if description.is_empty() {
        return Ok(extraction);
    }
    let mut seen = HashSet::from([file_id.clone()]);
    for (index, raw_line) in description.lines().enumerate() {
        let line = index + 1;
        let stripped = raw_line.trim();
        let Some(value) = stripped.strip_prefix("state =") else {
            continue;
        };
        let value = value.trim();
        let state = if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            &value[1..value.len() - 1]
        } else {
            value
        };
        if state.is_empty() {
            continue;
        }
        let id = make_id(&[&stem, "state", state]);
        if !seen.insert(id.clone()) {
            continue;
        }
        extraction.nodes.push(code_node(
            id.clone(),
            &format!("\"{state}\""),
            &source_file,
            &format!("L{line}"),
        ));
        extraction.edges.push(EdgeRecord {
            source: file_id.clone(),
            target: id,
            attributes: edge_attributes(&source_file, "contains", line, None),
        });
    }
    Ok(extraction)
}

fn dmi_description(data: &[u8]) -> String {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return String::new();
    }
    let mut index = 8_usize;
    while index.saturating_add(8) <= data.len() {
        let Some(length_bytes) = data.get(index..index + 4) else {
            return String::new();
        };
        let length = u32::from_be_bytes([
            length_bytes[0],
            length_bytes[1],
            length_bytes[2],
            length_bytes[3],
        ]) as usize;
        let Some(chunk_type) = data.get(index + 4..index + 8) else {
            return String::new();
        };
        let payload_start = index + 8;
        let Some(payload_end) = payload_start.checked_add(length) else {
            return String::new();
        };
        let Some(payload) = data.get(payload_start..payload_end) else {
            return String::new();
        };
        if matches!(chunk_type, b"tEXt" | b"zTXt") {
            let Some(separator) = payload.iter().position(|byte| *byte == 0) else {
                return String::new();
            };
            if payload.get(..separator) == Some(b"Description".as_slice()) {
                if chunk_type == b"zTXt" {
                    let Some(compressed) = payload.get(separator + 2..) else {
                        return String::new();
                    };
                    let mut output = Vec::new();
                    if ZlibDecoder::new(compressed)
                        .take(1024 * 1024)
                        .read_to_end(&mut output)
                        .is_err()
                    {
                        return String::new();
                    }
                    return String::from_utf8_lossy(&output).into_owned();
                }
                return payload
                    .get(separator + 1..)
                    .map_or_else(String::new, |text| {
                        String::from_utf8_lossy(text).into_owned()
                    });
            }
        }
        let Some(next) = payload_end.checked_add(4) else {
            return String::new();
        };
        index = next;
    }
    String::new()
}

static DMM_GRID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\(\s*\d+\s*,\s*\d+\s*,\s*\d+\s*\)\s*=")
        .unwrap_or_else(|error| unreachable!("static DMM grid regex is invalid: {error}"))
});

fn extract_dmm(path: &Path) -> Result<Extraction, ExtractError> {
    const MAX_BYTES: u64 = 50 * 1024 * 1024;
    let mut data = Vec::new();
    File::open(path)
        .map_err(|source| trail_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .take(MAX_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|source| trail_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if data.len() > MAX_BYTES as usize {
        return Ok(failure("file too large (>50 MB)"));
    }
    let text = String::from_utf8_lossy(&data);
    let source_file = path.to_string_lossy().into_owned();
    let file_id = make_id(&[&source_file]);
    let mut extraction = empty();
    extraction.nodes.push(code_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        &source_file,
        "L1",
    ));
    let dictionary = DMM_GRID
        .find(&text)
        .map_or(text.as_ref(), |matched| &text[..matched.start()]);
    let mut seen_targets = HashSet::new();
    let mut buffer = String::new();
    let mut open_line = 0;
    let mut depth = 0_i64;
    let mut in_string = false;
    let mut escape = false;
    for (index, line) in dictionary.lines().enumerate() {
        let line_number = index + 1;
        for character in line.chars() {
            if escape {
                escape = false;
            } else if in_string {
                if character == '\\' {
                    escape = true;
                } else if character == '"' {
                    in_string = false;
                }
            } else if character == '"' {
                in_string = true;
            } else if character == '(' {
                if depth == 0 {
                    open_line = line_number;
                }
                depth += 1;
            } else if character == ')' {
                depth -= 1;
            }
            buffer.push(character);
        }
        buffer.push('\n');
        if depth != 0 || buffer.is_empty() {
            continue;
        }
        let chunk = std::mem::take(&mut buffer);
        let Some(left) = chunk.find('(') else {
            continue;
        };
        let Some(right) = chunk.rfind(')') else {
            continue;
        };
        if right <= left {
            continue;
        }
        for entry in split_dmm_tile(&chunk[left + 1..right]) {
            let path = entry
                .split_once('{')
                .map_or(entry.as_str(), |(head, _)| head)
                .trim();
            if !path.starts_with('/') {
                continue;
            }
            let target = make_id(&[path]);
            if !seen_targets.insert(target.clone()) {
                continue;
            }
            extraction.edges.push(EdgeRecord {
                source: file_id.clone(),
                target,
                attributes: edge_attributes_with_context(&source_file, "uses", open_line, "map"),
            });
        }
    }
    Ok(extraction)
}

fn split_dmm_tile(body: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut buffer = String::new();
    let mut depth = 0_i64;
    let mut in_string = false;
    let mut escape = false;
    for character in body.chars() {
        if escape {
            buffer.push(character);
            escape = false;
            continue;
        }
        if in_string {
            buffer.push(character);
            if character == '\\' {
                escape = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                buffer.push(character);
            }
            '(' | '{' | '[' => {
                depth += 1;
                buffer.push(character);
            }
            ')' | '}' | ']' => {
                depth -= 1;
                buffer.push(character);
            }
            ',' if depth == 0 => output.push(std::mem::take(&mut buffer).trim().to_owned()),
            _ => buffer.push(character),
        }
    }
    let tail = buffer.trim();
    if !tail.is_empty() {
        output.push(tail.to_owned());
    }
    output
}

static DMF_WINDOW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*window\s+"([^"]+)"\s*$"#)
        .unwrap_or_else(|error| unreachable!("static DMF window regex is invalid: {error}"))
});
static DMF_ELEMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*elem\s+"([^"]+)"\s*$"#)
        .unwrap_or_else(|error| unreachable!("static DMF element regex is invalid: {error}"))
});
static DMF_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*type\s*=\s*(\S+)\s*$")
        .unwrap_or_else(|error| unreachable!("static DMF type regex is invalid: {error}"))
});

fn extract_dmf(path: &Path) -> Result<Extraction, ExtractError> {
    let data = fs::read(path).map_err(|source| trail_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8_lossy(&data);
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut extraction = empty();
    extraction.nodes.push(code_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        &source_file,
        "L1",
    ));
    let mut seen = HashSet::from([file_id.clone()]);
    let mut current_window = None;
    let mut current_element = None;
    let mut current_element_name = None;
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if let Some(name) = DMF_WINDOW
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str())
        {
            let id = make_id(&[&stem, "window", name]);
            if seen.insert(id.clone()) {
                extraction.nodes.push(code_node(
                    id.clone(),
                    &format!("window \"{name}\""),
                    &source_file,
                    &format!("L{line_number}"),
                ));
                extraction.edges.push(EdgeRecord {
                    source: file_id.clone(),
                    target: id.clone(),
                    attributes: edge_attributes(&source_file, "contains", line_number, None),
                });
            }
            current_window = Some(id);
            current_element = None;
            current_element_name = None;
            continue;
        }
        if let Some(name) = DMF_ELEMENT
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str())
            && let Some(window) = &current_window
        {
            let id = make_id(&[&stem, "elem", window, name]);
            if seen.insert(id.clone()) {
                extraction.nodes.push(code_node(
                    id.clone(),
                    &format!("elem \"{name}\""),
                    &source_file,
                    &format!("L{line_number}"),
                ));
                extraction.edges.push(EdgeRecord {
                    source: window.clone(),
                    target: id.clone(),
                    attributes: edge_attributes(&source_file, "contains", line_number, None),
                });
            }
            current_element = Some(id);
            current_element_name = Some(name.to_owned());
            continue;
        }
        if let Some(component_type) = DMF_TYPE
            .captures(line)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str())
            && let (Some(element), Some(name)) = (&current_element, &current_element_name)
            && let Some(node) = extraction.nodes.iter_mut().find(|node| node.id == *element)
            && !node.label().contains(" [")
        {
            node.attributes.insert(
                "label".to_owned(),
                Value::String(format!("elem \"{name}\" [{component_type}]")),
            );
        }
    }
    Ok(extraction)
}

fn child_of_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn code_node(id: String, label: &str, source_file: &str, location: &str) -> NodeRecord {
    let mut attributes = Map::new();
    attributes.insert("label".to_owned(), Value::String(label.to_owned()));
    attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
    attributes.insert(
        "source_file".to_owned(),
        Value::String(source_file.to_owned()),
    );
    attributes.insert(
        "source_location".to_owned(),
        Value::String(location.to_owned()),
    );
    NodeRecord { id, attributes }
}

fn edge_attributes(
    source_file: &str,
    relation: &str,
    line: usize,
    context: Option<&str>,
) -> Map<String, Value> {
    let mut attributes = Map::new();
    attributes.insert("relation".to_owned(), Value::String(relation.to_owned()));
    attributes.insert(
        "confidence".to_owned(),
        Value::String("EXTRACTED".to_owned()),
    );
    attributes.insert(
        "source_file".to_owned(),
        Value::String(source_file.to_owned()),
    );
    attributes.insert(
        "source_location".to_owned(),
        Value::String(format!("L{line}")),
    );
    attributes.insert("weight".to_owned(), json!(1.0));
    if let Some(context) = context {
        attributes.insert("context".to_owned(), Value::String(context.to_owned()));
    }
    attributes
}

fn edge_attributes_with_context(
    source_file: &str,
    relation: &str,
    line: usize,
    context: &str,
) -> Map<String, Value> {
    edge_attributes(source_file, relation, line, Some(context))
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

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !output.pop() {
                    output.push(component.as_os_str());
                }
            }
            _ => output.push(component.as_os_str()),
        }
    }
    output
}
