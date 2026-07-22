use std::collections::HashSet;
use std::path::Path;

use compass_model::{EdgeRecord, NodeRecord};
use regex::Regex;
use serde_json::{Map, Value};

use crate::{Extraction, file_stem, make_id};

const CONTROL_FLOW: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "try",
    "catch",
    "finally",
    "return",
    "throw",
    "new",
    "void",
    "null",
    "true",
    "false",
    "this",
    "super",
    "class",
    "interface",
    "enum",
    "trigger",
    "on",
];

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    State::new(path, source).run()
}

struct State<'a> {
    path: &'a Path,
    text: &'a str,
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
    current_class: Option<String>,
    annotations: Vec<String>,
}

impl<'a> State<'a> {
    fn new(path: &'a Path, source: &'a [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        Self {
            path,
            text: std::str::from_utf8(source).unwrap_or_default(),
            stem: file_stem(path),
            file_id: make_id(&[&source_file]),
            source_file,
            extraction: Extraction {
                raw_calls: None,
                ..Extraction::default()
            },
            seen: HashSet::new(),
            current_class: None,
            annotations: Vec::new(),
        }
    }

    fn run(mut self) -> Extraction {
        let label = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        self.add_node(&self.file_id.clone(), label, 1);
        let patterns = Patterns::new();
        for (index, line) in self.text.lines().enumerate() {
            self.add_line(index + 1, line, &patterns);
        }
        self.extraction
    }

    fn add_line(&mut self, at: usize, line: &str, patterns: &Patterns) {
        let trimmed = line.trim();
        if trimmed.starts_with('@') {
            if let Some(annotation) = &patterns.annotation {
                self.annotations.extend(
                    annotation
                        .captures_iter(trimmed)
                        .filter_map(|capture| capture.get(1))
                        .map(|name| name.as_str().to_ascii_lowercase()),
                );
            }
            return;
        }
        if let Some(capture) = patterns
            .trigger
            .as_ref()
            .and_then(|pattern| pattern.captures(trimmed))
        {
            let (Some(name), Some(object)) = (capture.get(1), capture.get(2)) else {
                return;
            };
            let name = name.as_str();
            let object = object.as_str();
            let id = make_id(&[&self.stem, name]);
            self.add_node(&id, name, at);
            self.add_edge(&self.file_id.clone(), &id, "contains", at, "EXTRACTED");
            let object_id = make_id(&[object]);
            self.add_node(&object_id, object, at);
            self.add_edge(&id, &object_id, "uses", at, "INFERRED");
            self.current_class = Some(id);
            self.annotations.clear();
            return;
        }
        if let Some(capture) = patterns
            .class
            .as_ref()
            .and_then(|pattern| pattern.captures(trimmed))
        {
            let Some(name) = capture.get(1).map(|name| name.as_str()) else {
                return;
            };
            if CONTROL_FLOW.contains(&name.to_ascii_lowercase().as_str()) {
                self.annotations.clear();
                return;
            }
            let id = make_id(&[&self.stem, name]);
            self.add_node(&id, name, at);
            self.add_edge(&self.file_id.clone(), &id, "contains", at, "EXTRACTED");
            if let Some(base) = capture.get(2).map(|value| value.as_str().trim()) {
                let target = self.local_or_stub(base, at);
                self.add_edge(&id, &target, "extends", at, "INFERRED");
            }
            if let Some(interfaces) = capture.get(3) {
                for interface in interfaces.as_str().split(',').map(str::trim) {
                    if !interface.is_empty() {
                        let target = self.local_or_stub(interface, at);
                        self.add_edge(&id, &target, "implements", at, "INFERRED");
                    }
                }
            }
            self.current_class = Some(id);
            self.annotations.clear();
            return;
        }
        if let Some(capture) = patterns
            .interface
            .as_ref()
            .and_then(|pattern| pattern.captures(trimmed))
        {
            let Some(name) = capture.get(1).map(|name| name.as_str()) else {
                return;
            };
            if CONTROL_FLOW.contains(&name.to_ascii_lowercase().as_str()) {
                self.annotations.clear();
                return;
            }
            let id = make_id(&[&self.stem, name]);
            self.add_node(&id, name, at);
            let parent = self
                .current_class
                .clone()
                .unwrap_or_else(|| self.file_id.clone());
            self.add_edge(&parent, &id, "contains", at, "EXTRACTED");
            if let Some(parents) = capture.get(2) {
                for parent in parents.as_str().split(',').map(str::trim) {
                    if !parent.is_empty() {
                        let target = self.local_or_stub(parent, at);
                        self.add_edge(&id, &target, "extends", at, "INFERRED");
                    }
                }
            }
            self.annotations.clear();
            return;
        }
        if let Some(capture) = patterns
            .enumeration
            .as_ref()
            .and_then(|pattern| pattern.captures(trimmed))
        {
            let Some(name) = capture.get(1).map(|name| name.as_str()) else {
                return;
            };
            if CONTROL_FLOW.contains(&name.to_ascii_lowercase().as_str()) {
                self.annotations.clear();
                return;
            }
            let id = make_id(&[&self.stem, name]);
            self.add_node(&id, name, at);
            let parent = self
                .current_class
                .clone()
                .unwrap_or_else(|| self.file_id.clone());
            self.add_edge(&parent, &id, "contains", at, "EXTRACTED");
            self.annotations.clear();
            return;
        }
        if let Some(class) = self.current_class.clone()
            && let Some(capture) = patterns
                .method
                .as_ref()
                .and_then(|pattern| pattern.captures(trimmed))
            && let Some(name) = capture.get(1).map(|name| name.as_str())
            && !CONTROL_FLOW.contains(&name.to_ascii_lowercase().as_str())
        {
            let id = make_id(&[&class, name]);
            self.add_node(&id, &format!(".{name}()"), at);
            self.add_edge(&class, &id, "method", at, "EXTRACTED");
            if self
                .annotations
                .iter()
                .any(|annotation| matches!(annotation.as_str(), "auraenabled" | "invocablemethod"))
            {
                self.add_edge(&self.file_id.clone(), &id, "contains", at, "INFERRED");
            }
            self.annotations.clear();
            return;
        }
        self.annotations.clear();

        if let Some(pattern) = &patterns.soql {
            for capture in pattern.captures_iter(line) {
                if let Some(object) = capture.get(1).map(|value| value.as_str()) {
                    let id = make_id(&[object]);
                    self.add_node(&id, object, at);
                    let source = self
                        .current_class
                        .clone()
                        .unwrap_or_else(|| self.file_id.clone());
                    self.add_edge(&source, &id, "uses", at, "INFERRED");
                }
            }
        }
        if let Some(pattern) = &patterns.dml {
            for capture in pattern.captures_iter(line) {
                if let Some(operation) = capture
                    .get(1)
                    .map(|value| value.as_str().to_ascii_lowercase())
                {
                    let id = make_id(&[&format!("dml_{operation}")]);
                    self.add_node(&id, &operation, at);
                    let source = self
                        .current_class
                        .clone()
                        .unwrap_or_else(|| self.file_id.clone());
                    self.add_edge(&source, &id, "uses", at, "INFERRED");
                }
            }
        }
    }

    fn local_or_stub(&mut self, name: &str, at: usize) -> String {
        let local = make_id(&[&self.stem, name]);
        if self.seen.contains(&local) {
            return local;
        }
        let id = make_id(&[name]);
        self.add_node(&id, name, at);
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
        confidence: &str,
    ) {
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String(confidence.to_owned()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("weight".into(), Value::from(1.0));
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }
}

struct Patterns {
    class: Option<Regex>,
    interface: Option<Regex>,
    enumeration: Option<Regex>,
    trigger: Option<Regex>,
    method: Option<Regex>,
    annotation: Option<Regex>,
    soql: Option<Regex>,
    dml: Option<Regex>,
}

impl Patterns {
    fn new() -> Self {
        let annotation = r"(?:\s*@\w+(?:\s*\([^)]*\))?\s*)*";
        let access = r"(?:public|private|protected|global|webService)?";
        let sharing = r"(?:\s+(?:with|without|inherited)\s+sharing)?";
        let modifiers = r"(?:\s+(?:abstract|virtual|override|static|final|transient|testMethod))?";
        Self {
            class: Regex::new(&format!(
                r"(?i)^{annotation}\s*{access}{sharing}{modifiers}\s*class\s+(\w+)(?:\s+extends\s+(\w+))?(?:\s+implements\s+([\w,\s]+))?\s*\{{?"
            ))
            .ok(),
            interface: Regex::new(&format!(
                r"(?i)^{annotation}\s*{access}{sharing}{modifiers}\s*interface\s+(\w+)(?:\s+extends\s+([\w,\s]+))?\s*\{{?"
            ))
            .ok(),
            enumeration: Regex::new(&format!(
                r"(?i)^{annotation}\s*{access}{sharing}{modifiers}\s*enum\s+(\w+)\s*\{{?"
            ))
            .ok(),
            trigger: Regex::new(r"(?i)^\s*trigger\s+(\w+)\s+on\s+(\w+)\s*\(").ok(),
            method: Regex::new(&format!(
                r"(?i)^{annotation}\s*{access}{modifiers}\s*(?:static\s+)?[\w<>\[\]]+\s+(\w+)\s*\([^)]*\)\s*(?:throws\s+\w+\s*)?\{{?"
            ))
            .ok(),
            annotation: Regex::new(r"(?i)@(\w+)").ok(),
            soql: Regex::new(r"(?i)\[\s*SELECT\b[^\]]+FROM\s+(\w+)").ok(),
            dml: Regex::new(r"(?i)\b(insert|update|delete|upsert|merge|undelete)\s+\w").ok(),
        }
    }
}
