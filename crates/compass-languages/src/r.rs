use std::collections::{HashMap, HashSet};
use std::path::Path;

use compass_model::{EdgeRecord, NodeRecord};
use regex::Regex;
use serde_json::{Map, Value};

use crate::{Extraction, RawCall, file_stem, make_id};

const NON_CALLS: &[&str] = &[
    "function", "if", "for", "while", "repeat", "switch", "return",
];

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    State::new(path, source).run()
}

#[derive(Clone)]
struct Function {
    start: usize,
    end: usize,
    id: String,
    name: String,
    parent: Option<String>,
}

struct State<'a> {
    path: &'a Path,
    text: String,
    masked: Vec<u8>,
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
}

impl<'a> State<'a> {
    fn new(path: &'a Path, source: &'a [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let text = String::from_utf8_lossy(source).into_owned();
        let masked = mask_non_code(text.as_bytes());
        Self {
            path,
            text,
            masked,
            stem: file_stem(path),
            file_id: make_id(&[&source_file]),
            source_file,
            extraction: Extraction::default(),
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
        self.add_imports();

        let functions = self.functions();
        for function in &functions {
            let at = self.line_at(function.start);
            self.add_node(&function.id, &format!("{}()", function.name), at);
            let container = function
                .parent
                .as_deref()
                .unwrap_or(&self.file_id)
                .to_owned();
            self.add_edge(&container, &function.id, "contains", at);
        }

        let mut labels: HashMap<String, Vec<Function>> = HashMap::new();
        for function in &functions {
            labels
                .entry(function.name.clone())
                .or_default()
                .push(function.clone());
        }
        let mut seen_calls = HashSet::new();
        for function in &functions {
            self.add_calls(function, &functions, &labels, &mut seen_calls);
        }

        self.extraction.edges.retain(|edge| {
            self.seen.contains(&edge.source)
                && (self.seen.contains(&edge.target)
                    || edge.attributes.get("relation").and_then(Value::as_str) == Some("imports"))
        });
        self.extraction
    }

    fn add_imports(&mut self) {
        let Ok(imports) = Regex::new(r"(?m)^[ \t]*(?:library|require|requireNamespace)[ \t]*\(")
        else {
            return;
        };
        let Ok(package) = Regex::new(r#"^\s*(?:package\s*=\s*)?["']?([A-Za-z][A-Za-z0-9._-]*)"#)
        else {
            return;
        };
        let masked = String::from_utf8_lossy(&self.masked);
        let mut found = Vec::new();
        for matched in imports.find_iter(&masked) {
            let Some(arguments_end) = matching_delimiter(&self.masked, matched.end(), b'(', b')')
            else {
                continue;
            };
            if let Some(capture) = package.captures(&self.text[matched.end()..arguments_end])
                && let Some(name) = capture.get(1)
            {
                found.push((matched.start(), name.as_str().to_owned()));
            }
        }
        for (offset, name) in found {
            self.add_edge(
                &self.file_id.clone(),
                &make_id(&[&name]),
                "imports",
                self.line_at(offset),
            );
        }
    }

    fn functions(&self) -> Vec<Function> {
        let Ok(pattern) = Regex::new(
            r"(?m)^[ \t]*([A-Za-z.][A-Za-z0-9._]*)[ \t]*(?:<<-|<-|=)[ \t]*function[ \t]*\(",
        ) else {
            return Vec::new();
        };
        let masked = String::from_utf8_lossy(&self.masked);
        let specs = pattern
            .captures_iter(&masked)
            .filter_map(|capture| {
                let whole = capture.get(0)?;
                let name = capture.get(1)?.as_str().to_owned();
                let parameters_end = matching_delimiter(&self.masked, whole.end(), b'(', b')')?;
                let mut body_start = parameters_end + 1;
                while self
                    .masked
                    .get(body_start)
                    .is_some_and(|byte| byte.is_ascii_whitespace())
                {
                    body_start += 1;
                }
                let end = if self.masked.get(body_start) == Some(&b'{') {
                    matching_delimiter(&self.masked, body_start + 1, b'{', b'}')?
                } else {
                    self.masked[body_start..]
                        .iter()
                        .position(|byte| *byte == b'\n')
                        .map_or_else(
                            || self.masked.len().saturating_sub(1),
                            |relative| body_start + relative,
                        )
                };
                Some((whole.start(), end, name))
            })
            .collect::<Vec<_>>();
        let mut functions: Vec<Function> = Vec::new();
        for (start, end, name) in specs {
            let parent = functions
                .iter()
                .filter(|function| function.start < start && end <= function.end)
                .min_by_key(|function| function.end - function.start)
                .map(|function| function.id.clone());
            let id = make_id(&[parent.as_deref().unwrap_or(&self.stem), &name]);
            functions.push(Function {
                start,
                end,
                id,
                name,
                parent,
            });
        }
        functions
    }

    fn add_calls(
        &mut self,
        function: &Function,
        functions: &[Function],
        labels: &HashMap<String, Vec<Function>>,
        seen: &mut HashSet<(String, String)>,
    ) {
        let Ok(calls) =
            Regex::new(r"([A-Za-z.][A-Za-z0-9._]*(?:(?:::|\$)[A-Za-z.][A-Za-z0-9._]*)?)[ \t]*\(")
        else {
            return;
        };
        let mut body = self.masked[function.start..=function.end].to_vec();
        for nested in functions {
            if function.start < nested.start && nested.end <= function.end {
                for index in nested.start..=nested.end {
                    if body[index - function.start] != b'\n' {
                        body[index - function.start] = b' ';
                    }
                }
            }
        }
        let body = String::from_utf8_lossy(&body);
        for capture in calls.captures_iter(&body) {
            let Some(raw) = capture.get(1) else {
                continue;
            };
            let expression = raw.as_str();
            let callee = expression
                .rsplit_once("::")
                .or_else(|| expression.rsplit_once('$'))
                .map_or(expression, |(_, name)| name);
            if NON_CALLS.contains(&callee) {
                continue;
            }
            let absolute = function.start + raw.start();
            if let Some(target) = call_target(callee, function, labels) {
                if seen.insert((function.id.clone(), target.clone())) {
                    self.add_edge(&function.id, &target, "calls", self.line_at(absolute));
                }
            } else {
                let source_location = format!("L{}", self.line_at(absolute));
                self.extraction.raw_calls_mut().push(RawCall {
                    caller_nid: function.id.clone(),
                    callee: callee.to_owned(),
                    is_member_call: Some(expression.contains('$')),
                    source_file: self.source_file.clone(),
                    source_location,
                    receiver: None,
                    receiver_type: None,
                    lang: None,
                    extensions: Map::new(),
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
        self.text.as_bytes()[..offset.min(self.text.len())]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + 1
    }
}

fn call_target(
    callee: &str,
    caller: &Function,
    labels: &HashMap<String, Vec<Function>>,
) -> Option<String> {
    let candidates = labels
        .get(callee)?
        .iter()
        .filter(|candidate| candidate.id != caller.id)
        .collect::<Vec<_>>();
    let children = candidates
        .iter()
        .filter(|candidate| candidate.parent.as_deref() == Some(caller.id.as_str()))
        .collect::<Vec<_>>();
    if children.len() == 1 {
        return Some(children[0].id.clone());
    }
    let same_scope = candidates
        .iter()
        .filter(|candidate| candidate.parent == caller.parent)
        .collect::<Vec<_>>();
    if same_scope.len() == 1 {
        return Some(same_scope[0].id.clone());
    }
    (candidates.len() == 1).then(|| candidates[0].id.clone())
}

fn mask_non_code(source: &[u8]) -> Vec<u8> {
    let mut masked = source.to_vec();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    for (index, byte) in source.iter().copied().enumerate() {
        if comment {
            if byte == b'\n' {
                comment = false;
            } else {
                masked[index] = b' ';
            }
            continue;
        }
        if let Some(active) = quote {
            if byte == b'\n' {
                continue;
            } else if escaped {
                escaped = false;
                masked[index] = b' ';
            } else if byte == b'\\' {
                escaped = true;
                masked[index] = b' ';
            } else if byte == active {
                quote = None;
            } else {
                masked[index] = b' ';
            }
            continue;
        }
        if byte == b'#' {
            comment = true;
            masked[index] = b' ';
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        }
    }
    masked
}

fn matching_delimiter(source: &[u8], start: usize, opening: u8, closing: u8) -> Option<usize> {
    let mut depth = 1_u32;
    for (index, byte) in source.iter().copied().enumerate().skip(start) {
        if byte == opening {
            depth += 1;
        } else if byte == closing {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}
