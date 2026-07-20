use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use serde_json::{Map, Value};
use trail_model::{EdgeRecord, NodeRecord};

use crate::{Extraction, RawCall, file_stem, make_id};

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    State::new(path, source).run()
}

struct FunctionBody {
    id: String,
    start: usize,
    end: usize,
}

#[derive(Clone)]
enum Declaration {
    Import(String),
    Container {
        name: String,
        kind: String,
        body_start: usize,
        body_end: usize,
    },
    Function {
        name: String,
        body_start: usize,
        body_end: usize,
    },
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
    function_bodies: Vec<FunctionBody>,
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
            extraction: Extraction::default(),
            seen: HashSet::new(),
            function_bodies: Vec::new(),
        }
    }

    fn run(mut self) -> Extraction {
        let label = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        self.add_node(&self.file_id.clone(), label, 1);

        for (offset, declaration) in declarations(self.text) {
            match declaration {
                Declaration::Import(module) => self.add_edge(
                    &self.file_id.clone(),
                    &make_id(&[&module]),
                    "imports_from",
                    self.line_at(offset),
                ),
                Declaration::Container {
                    name,
                    kind,
                    body_start,
                    body_end,
                } => {
                    let id = make_id(&[&self.stem, &name]);
                    let at = self.line_at(offset);
                    self.add_node(&id, &name, at);
                    self.add_edge(&self.file_id.clone(), &id, "contains", at);
                    if kind == "struct" {
                        for (name, declaration_start, start, end) in
                            functions(self.text, body_start, body_end)
                        {
                            let function = make_id(&[&id, &name]);
                            let at = self.line_at(declaration_start);
                            self.add_node(&function, &format!(".{name}()"), at);
                            self.add_edge(&id, &function, "method", at);
                            self.function_bodies.push(FunctionBody {
                                id: function,
                                start,
                                end,
                            });
                        }
                    }
                }
                Declaration::Function {
                    name,
                    body_start,
                    body_end,
                } => {
                    let id = make_id(&[&self.stem, &name]);
                    let at = self.line_at(offset);
                    self.add_node(&id, &format!("{name}()"), at);
                    self.add_edge(&self.file_id.clone(), &id, "contains", at);
                    self.function_bodies.push(FunctionBody {
                        id,
                        start: body_start,
                        end: body_end,
                    });
                }
            }
        }

        let mut labels = HashMap::new();
        for node in &self.extraction.nodes {
            if let Some(label) = node.attributes.get("label").and_then(Value::as_str)
                && label.ends_with("()")
            {
                labels
                    .entry(
                        label
                            .trim_matches(['(', ')'])
                            .trim_start_matches('.')
                            .to_owned(),
                    )
                    .or_insert_with(|| node.id.clone());
            }
        }
        let mut seen_calls = HashSet::new();
        for body in std::mem::take(&mut self.function_bodies) {
            self.add_calls(&body, &labels, &mut seen_calls);
        }
        self.extraction.edges.retain(|edge| {
            self.seen.contains(&edge.source)
                && (self.seen.contains(&edge.target)
                    || edge.attributes.get("relation").and_then(Value::as_str)
                        == Some("imports_from"))
        });
        self.extraction
    }

    fn add_calls(
        &mut self,
        body: &FunctionBody,
        labels: &HashMap<String, String>,
        seen: &mut HashSet<(String, String)>,
    ) {
        let Ok(calls) =
            Regex::new(r"(?m)([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)\s*\(")
        else {
            return;
        };
        let body_text = &self.text[body.start..body.end];
        for capture in calls.captures_iter(body_text) {
            let Some(raw_match) = capture.get(1) else {
                continue;
            };
            let absolute = body.start + raw_match.start();
            if absolute > 0 && self.source[absolute - 1] == b'@' {
                continue;
            }
            let raw = raw_match.as_str();
            let callee = raw.rsplit('.').next().unwrap_or_default();
            if matches!(callee, "if" | "while" | "for" | "switch" | "catch") {
                continue;
            }
            let at = self.line_at(absolute);
            if let Some(target) = labels
                .get(callee)
                .filter(|target| target.as_str() != body.id)
            {
                if seen.insert((body.id.clone(), target.clone())) {
                    self.add_edge(&body.id, target, "calls", at);
                }
            } else if !callee.is_empty() {
                self.extraction.raw_calls_mut().push(RawCall {
                    caller_nid: body.id.clone(),
                    callee: callee.to_owned(),
                    is_member_call: raw.contains('.'),
                    source_file: self.source_file.clone(),
                    source_location: format!("L{at}"),
                    receiver: None,
                    receiver_type: None,
                    lang: None,
                });
            }
        }
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

    fn add_edge(&mut self, source: &str, target: &str, relation: &str, at: usize) {
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
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

    fn line_at(&self, offset: usize) -> usize {
        self.source[..offset.min(self.source.len())]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + 1
    }
}

fn declarations(source: &str) -> Vec<(usize, Declaration)> {
    let Ok(containers) = Regex::new(
        r"(?m)^[ \t]*(?:pub[ \t]+)?(?:const|var)[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*=[ \t]*(struct|enum|union)(?:\([^\n{]*\))?[ \t]*\{",
    ) else {
        return Vec::new();
    };
    let Ok(imports) = Regex::new(
        r#"(?m)^[ \t]*(?:pub[ \t]+)?(?:const|var)[ \t]+[A-Za-z_][A-Za-z0-9_]*[ \t]*=[^;\n]*@(?:import|cImport)\("([^"]+)"\)"#,
    ) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    let mut ranges = Vec::new();
    for capture in containers.captures_iter(source) {
        let Some(full) = capture.get(0) else {
            continue;
        };
        let Some(open) = source[full.start()..full.end()]
            .rfind('{')
            .map(|index| full.start() + index)
        else {
            continue;
        };
        let end = matching_brace(source.as_bytes(), open).unwrap_or(source.len());
        ranges.push((full.start(), end));
        events.push((
            full.start(),
            Declaration::Container {
                name: capture.get(1).map_or("", |value| value.as_str()).to_owned(),
                kind: capture.get(2).map_or("", |value| value.as_str()).to_owned(),
                body_start: open + 1,
                body_end: end.saturating_sub(1),
            },
        ));
    }
    for capture in imports.captures_iter(source) {
        if let (Some(full), Some(module)) = (capture.get(0), capture.get(1)) {
            events.push((
                full.start(),
                Declaration::Import(module.as_str().to_owned()),
            ));
        }
    }
    for (name, declaration_start, body_start, end) in functions(source, 0, source.len()) {
        if !ranges.iter().any(|(range_start, range_end)| {
            declaration_start >= *range_start && declaration_start < *range_end
        }) {
            events.push((
                declaration_start,
                Declaration::Function {
                    name,
                    body_start,
                    body_end: end,
                },
            ));
        }
    }
    events.sort_by_key(|(offset, _)| *offset);
    events
}

fn functions(source: &str, start: usize, end: usize) -> Vec<(String, usize, usize, usize)> {
    let Ok(pattern) =
        Regex::new(r"(?m)^[ \t]*(?:pub[ \t]+)?fn[ \t]+([A-Za-z_][A-Za-z0-9_]*)[^\n{]*\{")
    else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for capture in pattern.captures_iter(&source[start..end]) {
        let Some(full) = capture.get(0) else {
            continue;
        };
        let absolute = start + full.start();
        let Some(open) = source[start + full.start()..start + full.end()]
            .rfind('{')
            .map(|index| absolute + index)
        else {
            continue;
        };
        let close = matching_brace(source.as_bytes(), open).unwrap_or(end);
        values.push((
            capture.get(1).map_or("", |name| name.as_str()).to_owned(),
            absolute,
            open + 1,
            close.min(end),
        ));
    }
    values
}

fn matching_brace(source: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    let mut line_comment = false;
    let mut index = open;
    while index < source.len() {
        let byte = source[index];
        if line_comment {
            if byte == b'\n' {
                line_comment = false;
            }
            index += 1;
            continue;
        }
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        if byte == b'/' && source.get(index + 1) == Some(&b'/') {
            line_comment = true;
            index += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if byte == b'{' {
            depth += 1;
        } else if byte == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index + 1);
            }
        }
        index += 1;
    }
    None
}
