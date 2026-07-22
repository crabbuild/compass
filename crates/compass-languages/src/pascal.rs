use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use compass_model::{EdgeRecord, NodeRecord};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::{Extraction, RawCall, file_stem, make_id};

const KEYWORDS: &[&str] = &[
    "begin",
    "end",
    "if",
    "then",
    "else",
    "while",
    "do",
    "for",
    "to",
    "downto",
    "repeat",
    "until",
    "case",
    "of",
    "try",
    "finally",
    "except",
    "with",
    "inherited",
    "result",
    "var",
    "const",
    "type",
    "nil",
    "true",
    "false",
    "exit",
    "break",
    "continue",
    "uses",
    "unit",
    "program",
    "library",
    "interface",
    "implementation",
    "initialization",
    "finalization",
    "procedure",
    "function",
    "constructor",
    "destructor",
    "class",
    "record",
    "object",
    "array",
    "string",
    "integer",
    "boolean",
    "real",
    "char",
    "writeln",
    "write",
    "readln",
    "read",
    "assigned",
    "length",
    "high",
    "low",
    "inc",
    "dec",
    "new",
    "dispose",
    "setlength",
    "copy",
    "pos",
    "trim",
    "format",
    "inttostr",
    "strtoint",
    "ord",
    "chr",
    "sizeof",
    "create",
    "free",
    "destroy",
];

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    State::new(path, source).run()
}

struct Procedure {
    id: String,
    line: usize,
    body: String,
    container: String,
    name: String,
}

struct State<'a> {
    path: &'a Path,
    source: &'a [u8],
    text: String,
    source_file: String,
    stem: String,
    file_id: String,
    module_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<(String, String, String)>,
    procedures: Vec<Procedure>,
}

impl<'a> State<'a> {
    fn new(path: &'a Path, source: &'a [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let file_id = make_id(&[&source_file]);
        Self {
            path,
            source,
            text: strip_comments(std::str::from_utf8(source).unwrap_or_default()),
            stem: file_stem(path),
            file_id: file_id.clone(),
            module_id: file_id,
            source_file,
            extraction: Extraction::default(),
            seen_nodes: HashSet::new(),
            seen_edges: HashSet::new(),
            procedures: Vec::new(),
        }
    }

    fn run(mut self) -> Extraction {
        let label = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        self.add_node(&self.file_id.clone(), &label, 1);
        self.add_module();
        let sections = sections(&self.text);
        self.add_uses(&sections);
        self.add_types(&sections);
        self.add_implementations(&sections);
        self.add_calls();
        self.extraction
            .extensions
            .insert("input_tokens".to_owned(), json!(0));
        self.extraction
            .extensions
            .insert("output_tokens".to_owned(), json!(0));
        self.extraction
    }

    fn add_module(&mut self) {
        let Ok(pattern) =
            Regex::new(r"(?i)\b(?:unit|program|library)\s+([A-Za-z_][A-Za-z0-9_.]*)\s*;")
        else {
            return;
        };
        let Some(capture) = pattern.captures(&self.text) else {
            return;
        };
        let (Some(full), Some(name_match)) = (capture.get(0), capture.get(1)) else {
            return;
        };
        let name = name_match.as_str().to_owned();
        let id = make_id(&[&self.stem, &name]);
        let at = self.line_at(full.start());
        self.add_node(&id, &name, at);
        self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
        self.module_id = id;
    }

    fn add_uses(&mut self, sections: &Sections) {
        let Ok(pattern) = Regex::new(r"(?is)\buses\b\s*([^;]+);") else {
            return;
        };
        for (section, offset) in [
            (sections.interface.as_str(), sections.interface_offset),
            (
                sections.implementation.as_str(),
                sections.implementation_offset,
            ),
        ] {
            for capture in pattern.captures_iter(section) {
                let (Some(full), Some(list)) = (capture.get(0), capture.get(1)) else {
                    continue;
                };
                let at = self.line_at(offset + full.start());
                for unit in split_uses(list.as_str()) {
                    let target = resolve_unit(self.path, &unit);
                    self.add_edge(
                        &self.module_id.clone(),
                        &target,
                        "imports",
                        at,
                        Some("import"),
                    );
                }
            }
        }
    }

    fn add_types(&mut self, sections: &Sections) {
        let (search, offset) = if sections.interface.is_empty() {
            (self.text.clone(), 0)
        } else {
            (sections.interface.clone(), sections.interface_offset)
        };
        let Ok(headers) = Regex::new(
            r"(?i)\b([A-Za-z_]\w*)(?:\s*<[^>]+>)?\s*=\s*(?:packed\s+)?(?:class|interface)\b(?:\s*\(\s*([^)]*)\s*\))?",
        ) else {
            return;
        };
        let Ok(methods) = Regex::new(
            r"(?i)\b(?:procedure|function|constructor|destructor)\s+([A-Za-z_]\w*)(?:\s*\([^)]*\))?(?:\s*:\s*[\w<>,\s.]+)?\s*;",
        ) else {
            return;
        };
        let Ok(end_pattern) = Regex::new(r"(?i)\bend\s*;") else {
            return;
        };
        let mut position = 0;
        while let Some(capture) = headers.captures(&search[position..]) {
            let Some(full_relative) = capture.get(0) else {
                break;
            };
            let start = position + full_relative.start();
            let end = position + full_relative.end();
            let name = capture.get(1).map_or("", |value| value.as_str());
            let bases = capture.get(2).map_or("", |value| value.as_str());
            let at = self.line_at(offset + start);
            let id = make_id(&[&self.stem, name]);
            self.add_node(&id, name, at);
            self.add_edge(&self.module_id.clone(), &id, "contains", at, None);

            for base in split_bases(bases) {
                let same_file = make_id(&[&self.stem, &base]);
                let target = if self.seen_nodes.contains(&same_file) {
                    same_file
                } else if let Some(resolved) = resolve_class(self.path, &base) {
                    resolved
                } else {
                    let stub = make_id(&[&base]);
                    self.add_node(&stub, &base, at);
                    stub
                };
                self.add_edge(&id, &target, "inherits", at, None);
            }

            let body_end = end_pattern
                .find(&search[end..])
                .map(|value| end + value.start());
            if let Some(body_end) = body_end {
                let body = &search[end..body_end];
                for method in methods.captures_iter(body) {
                    let (Some(method_full), Some(method_name)) = (method.get(0), method.get(1))
                    else {
                        continue;
                    };
                    let method_name = method_name.as_str();
                    let method_id = make_id(&[&id, method_name]);
                    let method_line = self.line_at(offset + end + method_full.start());
                    self.add_node(&method_id, &format!("{method_name}()"), method_line);
                    self.add_edge(&id, &method_id, "method", method_line, None);
                }
                position = end_pattern
                    .find(&search[end..])
                    .map_or(search.len(), |value| end + value.end());
            } else {
                break;
            }
        }
    }

    fn add_implementations(&mut self, sections: &Sections) {
        let Ok(pattern) = Regex::new(
            r"(?i)\b(?:procedure|function|constructor|destructor)\s+([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)?)(?:\s*<[^>]+>)?(?:\s*\([^)]*\))?(?:\s*:\s*[\w<>,\s.]+)?\s*;",
        ) else {
            return;
        };
        for capture in pattern.captures_iter(&sections.implementation) {
            let (Some(full), Some(qualified_match)) = (capture.get(0), capture.get(1)) else {
                continue;
            };
            let qualified = qualified_match.as_str();
            let at = self.line_at(sections.implementation_offset + full.start());
            let (container, relation, label, name) =
                if let Some((class, method)) = qualified.split_once('.') {
                    let class_id = make_id(&[&self.stem, class]);
                    if self.seen_nodes.contains(&class_id) {
                        (
                            class_id,
                            "method",
                            format!("{method}()"),
                            method.to_ascii_lowercase(),
                        )
                    } else {
                        (
                            self.module_id.clone(),
                            "contains",
                            format!("{method}()"),
                            method.to_ascii_lowercase(),
                        )
                    }
                } else {
                    (
                        self.module_id.clone(),
                        "contains",
                        format!("{qualified}()"),
                        qualified.to_ascii_lowercase(),
                    )
                };
            let id = make_id(&[&self.stem, qualified]);
            self.add_node(&id, &label, at);
            self.add_edge(&container, &id, relation, at, None);
            let (body_start, body_end) = find_body(&sections.implementation, full.end());
            let body = if body_start == 0 {
                String::new()
            } else {
                sections.implementation[body_start..body_end].to_owned()
            };
            self.procedures.push(Procedure {
                id,
                line: at,
                body,
                container,
                name,
            });
        }
    }

    fn add_calls(&mut self) {
        let resolver = CallResolver::new(&self.procedures, &self.extraction.edges, &self.module_id);
        let Ok(calls) = Regex::new(r"\b([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)\s*[(;]") else {
            return;
        };
        let mut seen = HashSet::new();
        let procedures = std::mem::take(&mut self.procedures);
        for procedure in procedures {
            for call in calls.captures_iter(&procedure.body) {
                let (Some(full), Some(name_match)) = (call.get(0), call.get(1)) else {
                    continue;
                };
                let name = name_match
                    .as_str()
                    .rsplit('.')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if KEYWORDS.contains(&name.as_str()) {
                    continue;
                }
                let at = procedure.line + procedure.body[..full.start()].matches('\n').count();
                let target = resolver.resolve(&procedure.id, &name);
                if target.as_deref() == Some(procedure.id.as_str()) {
                    continue;
                }
                if let Some(target) = target {
                    if seen.insert((procedure.id.clone(), target.clone())) {
                        self.add_edge(&procedure.id, &target, "calls", at, Some("call"));
                    }
                } else {
                    self.extraction.raw_calls_mut().push(RawCall {
                        caller_nid: procedure.id.clone(),
                        callee: name,
                        is_member_call: None,
                        source_file: self.source_file.clone(),
                        source_location: format!("L{at}"),
                        receiver: None,
                        receiver_type: None,
                        lang: None,
                        extensions: Map::new(),
                    });
                }
            }
        }
    }

    fn add_node(&mut self, id: &str, label: &str, at: usize) {
        if !self.seen_nodes.insert(id.to_owned()) {
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
        context: Option<&str>,
    ) {
        let key = (source.to_owned(), target.to_owned(), relation.to_owned());
        if !self.seen_edges.insert(key) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("weight".into(), Value::from(1.0));
        if let Some(context) = context {
            attributes.insert("context".into(), Value::String(context.to_owned()));
        }
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

struct Sections {
    interface: String,
    interface_offset: usize,
    implementation: String,
    implementation_offset: usize,
}

fn sections(text: &str) -> Sections {
    let interface = keyword(text, "interface");
    let implementation = keyword(text, "implementation");
    if let (Some(interface), Some(implementation)) = (interface, implementation) {
        let implementation_end = keyword(&text[implementation.1..], "initialization")
            .or_else(|| keyword(&text[implementation.1..], "finalization"))
            .map_or(text.len(), |value| implementation.1 + value.0);
        Sections {
            interface: text[interface.1..implementation.0].to_owned(),
            interface_offset: interface.1,
            implementation: text[implementation.1..implementation_end].to_owned(),
            implementation_offset: implementation.1,
        }
    } else {
        Sections {
            interface: String::new(),
            interface_offset: 0,
            implementation: text.to_owned(),
            implementation_offset: 0,
        }
    }
}

fn keyword(text: &str, keyword: &str) -> Option<(usize, usize)> {
    let pattern = Regex::new(&format!(r"(?i)\b{keyword}\b")).ok()?;
    pattern.find(text).map(|value| (value.start(), value.end()))
}

fn strip_comments(text: &str) -> String {
    let Ok(pattern) = Regex::new(r"(?s)'(?:''|[^'])*'|\{[^}]*\}|\(\*.*?\*\)|//[^\n]*") else {
        return text.to_owned();
    };
    pattern
        .replace_all(text, |captures: &regex::Captures<'_>| {
            let token = captures.get(0).map_or("", |value| value.as_str());
            if token.starts_with('\'') {
                token.to_owned()
            } else {
                token
                    .chars()
                    .map(|character| if character == '\n' { '\n' } else { ' ' })
                    .collect()
            }
        })
        .into_owned()
}

fn split_uses(value: &str) -> Vec<String> {
    let in_pattern = Regex::new(r"(?i)\s+in\s+").ok();
    value
        .split(',')
        .filter_map(|chunk| {
            let trimmed = chunk.trim();
            let name = in_pattern
                .as_ref()
                .and_then(|pattern| pattern.find(trimmed))
                .map_or(trimmed, |matched| &trimmed[..matched.start()])
                .trim()
                .trim_matches(';');
            valid_identifier(name, true).then(|| name.to_owned())
        })
        .collect()
}

fn split_bases(value: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut depth = 0_u32;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        if character == '<' {
            depth += 1;
        } else if character == '>' {
            depth = depth.saturating_sub(1);
        } else if character == ',' && depth == 0 {
            push_base(&value[start..index], &mut values);
            start = index + 1;
        }
    }
    push_base(&value[start..], &mut values);
    values
}

fn push_base(value: &str, values: &mut Vec<String>) {
    let name = value.split('<').next().unwrap_or_default().trim();
    if valid_identifier(name, false) {
        values.push(name.to_owned());
    }
}

fn valid_identifier(value: &str, dotted: bool) -> bool {
    !value.is_empty()
        && value.split(if dotted { '.' } else { '\0' }).all(|part| {
            part.as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

fn find_body(text: &str, start: usize) -> (usize, usize) {
    let Ok(begin) = Regex::new(r"(?i)\bbegin\b") else {
        return (0, 0);
    };
    let Some(begin) = begin.find(&text[start..]) else {
        return (0, 0);
    };
    let body_start = start + begin.end();
    let Ok(tokens) = Regex::new(r"(?i)\b(begin|end|case|try|asm|record)\b") else {
        return (body_start, text.len());
    };
    let mut depth = 1_u32;
    for token in tokens.captures_iter(&text[body_start..]) {
        let (Some(full), Some(keyword)) = (token.get(0), token.get(1)) else {
            continue;
        };
        if keyword.as_str().eq_ignore_ascii_case("end") {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return (body_start, body_start + full.start());
            }
        } else {
            depth += 1;
        }
    }
    (body_start, text.len())
}

struct CallResolver {
    class_bases: HashMap<String, Vec<String>>,
    class_procedures: HashMap<String, HashMap<String, Vec<String>>>,
    module_procedures: HashMap<String, Vec<String>>,
    global_procedures: HashMap<String, Vec<String>>,
    owners: HashMap<String, String>,
}

impl CallResolver {
    fn new(procedures: &[Procedure], edges: &[EdgeRecord], module: &str) -> Self {
        let mut resolver = Self {
            class_bases: HashMap::new(),
            class_procedures: HashMap::new(),
            module_procedures: HashMap::new(),
            global_procedures: HashMap::new(),
            owners: HashMap::new(),
        };
        for edge in edges {
            if edge.attributes.get("relation").and_then(Value::as_str) == Some("inherits") {
                resolver
                    .class_bases
                    .entry(edge.source.clone())
                    .or_default()
                    .push(edge.target.clone());
            }
        }
        for procedure in procedures {
            resolver
                .owners
                .insert(procedure.id.clone(), procedure.container.clone());
            resolver
                .global_procedures
                .entry(procedure.name.clone())
                .or_default()
                .push(procedure.id.clone());
            if procedure.container == module {
                resolver
                    .module_procedures
                    .entry(procedure.name.clone())
                    .or_default()
                    .push(procedure.id.clone());
            } else {
                resolver
                    .class_procedures
                    .entry(procedure.container.clone())
                    .or_default()
                    .entry(procedure.name.clone())
                    .or_default()
                    .push(procedure.id.clone());
            }
        }
        resolver
    }

    fn resolve(&self, caller: &str, name: &str) -> Option<String> {
        if let Some(owner) = self.owners.get(caller) {
            if let Some(candidates) = self
                .class_procedures
                .get(owner)
                .and_then(|procedures| procedures.get(name))
            {
                return unique(candidates);
            }
            let mut seen = HashSet::new();
            let mut queue: VecDeque<String> = self
                .class_bases
                .get(owner)
                .cloned()
                .unwrap_or_default()
                .into();
            while let Some(base) = queue.pop_front() {
                if !seen.insert(base.clone()) {
                    continue;
                }
                if let Some(candidates) = self
                    .class_procedures
                    .get(&base)
                    .and_then(|procedures| procedures.get(name))
                {
                    return unique(candidates);
                }
                queue.extend(self.class_bases.get(&base).cloned().unwrap_or_default());
            }
        }
        if let Some(candidates) = self.module_procedures.get(name) {
            return unique(candidates);
        }
        self.global_procedures
            .get(name)
            .and_then(|values| unique(values))
    }
}

fn unique(values: &[String]) -> Option<String> {
    (values.len() == 1).then(|| values[0].clone())
}

fn resolve_unit(path: &Path, unit: &str) -> String {
    let root = project_root(path);
    pascal_files(&root)
        .into_iter()
        .find(|candidate| {
            candidate
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(unit))
        })
        .map_or_else(
            || make_id(&[unit]),
            |candidate| make_id(&[&candidate.to_string_lossy()]),
        )
}

fn resolve_class(path: &Path, class: &str) -> Option<String> {
    let unit = class.strip_prefix(['T', 'I']).unwrap_or(class);
    pascal_files(&project_root(path))
        .into_iter()
        .find(|candidate| {
            candidate
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem.eq_ignore_ascii_case(unit))
        })
        .map(|candidate| make_id(&[&file_stem(&candidate), class]))
}

fn project_root(path: &Path) -> PathBuf {
    let mut best = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let mut current = best.clone();
    for _ in 0..12 {
        if current.components().count() <= 1 {
            break;
        }
        let mut pas = 0;
        let mut dpr = 0;
        if let Ok(entries) = fs::read_dir(&current) {
            for entry in entries.flatten() {
                match entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                {
                    Some(extension) if extension.eq_ignore_ascii_case("pas") => pas += 1,
                    Some(extension) if extension.eq_ignore_ascii_case("dpr") => dpr += 1,
                    _ => {}
                }
            }
        }
        if pas >= 2 || dpr >= 1 {
            best = current.clone();
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }
        current = parent.to_path_buf();
    }
    best
}

fn pascal_files(root: &Path) -> Vec<PathBuf> {
    fn walk(directory: &Path, values: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, values);
            } else if path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "pas" | "pp" | "dpr" | "dpk" | "inc"
                    )
                })
            {
                values.push(path);
            }
        }
    }
    let mut values = Vec::new();
    walk(root, &mut values);
    values
}
