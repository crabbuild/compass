use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};

use crate::{Extraction, RawCall, file_stem, make_id};

static IMPORT: LazyLock<Regex> = LazyLock::new(|| regex(r"^\s*import\s+(?:static\s+)?([\w.]+)"));
static TYPE: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"^\s*(?:[\w@]+\s+)*(class|interface)\s+(\w+)(?:\s+extends\s+([\w.]+))?(?:\s+implements\s+([^\{]+))?",
    )
});
static METHOD: LazyLock<Regex> = LazyLock::new(|| {
    regex(
        r"^\s*(?:(?:public|protected|private|static|final|abstract|synchronized)\s+)*(?:def|[\w<>\[\].?]+)\s+(\w+)\s*\(",
    )
});
static CONSTRUCTOR: LazyLock<Regex> =
    LazyLock::new(|| regex(r"^\s*(?:(?:public|protected|private)\s+)?([A-Z]\w*)\s*\("));
static MEMBER_CALL: LazyLock<Regex> =
    LazyLock::new(|| regex(r"([A-Za-z_]\w*)\s*\.\s*([A-Za-z_]\w*)\s*\("));
static SPOCK_FEATURE: LazyLock<Regex> =
    LazyLock::new(|| regex(r#"^\s*def\s+(?:\"([^\"]+)\"|'([^']+)')\s*\("#));

pub(crate) fn extract(path: &Path, source: &[u8]) -> Extraction {
    let text = String::from_utf8_lossy(source);
    if text.contains("spock.lang.Specification")
        && text.lines().any(|line| SPOCK_FEATURE.is_match(line))
    {
        return extract_spock(path, &text);
    }
    extract_regular(path, &text)
}

fn extract_regular(path: &Path, source: &str) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = State::new(source_file, stem, file_id.clone());
    state.add_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
        false,
    );
    let lines: Vec<_> = source.lines().collect();
    for (index, text) in lines.iter().enumerate() {
        if let Some(captures) = IMPORT.captures(text) {
            let target = captures
                .get(1)
                .map(|value| value.as_str())
                .unwrap_or_default()
                .rsplit('.')
                .next()
                .unwrap_or_default();
            if !target.is_empty() {
                state.add_edge(
                    &file_id,
                    &make_id(&[target]),
                    "imports",
                    index + 1,
                    Some("import"),
                );
            }
        }
    }

    let mut classes = Vec::new();
    let mut depth = 0_i32;
    let mut active: Option<ActiveClass> = None;
    for (index, text) in lines.iter().enumerate() {
        let at_line = index + 1;
        if active.is_none()
            && let Some(captures) = TYPE.captures(text)
        {
            let name = capture(&captures, 2);
            if !name.is_empty() {
                let id = make_id(&[&state.stem, name]);
                state.add_node(id.clone(), name, at_line, true);
                state.add_edge(&file_id, &id, "contains", at_line, None);
                if let Some(base) = captures.get(3).map(|value| value.as_str()) {
                    let base = base.rsplit('.').next().unwrap_or(base);
                    let target = state.ensure_base(base);
                    state.add_edge(&id, &target, "inherits", at_line, None);
                }
                if let Some(interfaces) = captures.get(4).map(|value| value.as_str()) {
                    for interface in interfaces
                        .split(',')
                        .map(str::trim)
                        .filter(|name| !name.is_empty())
                    {
                        let interface = interface.rsplit('.').next().unwrap_or(interface);
                        let target = state.ensure_base(interface);
                        state.add_edge(&id, &target, "implements", at_line, None);
                    }
                }
                depth += brace_delta(text);
                active = Some(ActiveClass {
                    id,
                    name: name.to_owned(),
                    start_depth: depth - brace_delta(text),
                    current_method: None,
                });
                continue;
            }
        }
        if let Some(class) = &mut active {
            let before = depth;
            let method_name = CONSTRUCTOR
                .captures(text)
                .filter(|captures| capture(captures, 1) == class.name)
                .map(|captures| (capture(&captures, 1).to_owned(), true))
                .or_else(|| {
                    METHOD
                        .captures(text)
                        .map(|captures| (capture(&captures, 1).to_owned(), false))
                });
            if class.current_method.is_none()
                && let Some((name, constructor)) = method_name
            {
                let id = make_id(&[&class.id, &name]);
                let declaration_line = if constructor {
                    groovy_constructor_line(&lines, index)
                } else {
                    at_line
                };
                state.add_node(id.clone(), &format!(".{name}()"), declaration_line, true);
                state.add_edge(&class.id, &id, "method", declaration_line, None);
                class.current_method = Some(ActiveMethod {
                    start_depth: before,
                });
            }
            depth += brace_delta(text);
            if let Some(method) = &class.current_method
                && depth <= method.start_depth
            {
                class.current_method = None;
            }
            if depth <= class.start_depth {
                classes.push(active.take().unwrap_or_else(|| unreachable!()));
            }
        } else {
            depth += brace_delta(text);
        }
    }
    if let Some(class) = active {
        classes.push(class);
    }

    let call_targets: HashMap<String, String> = state
        .extraction
        .nodes
        .iter()
        .filter_map(|node| {
            node.attributes
                .get("label")
                .and_then(Value::as_str)
                .map(|label| {
                    (
                        label
                            .trim_matches(['(', ')'])
                            .trim_start_matches('.')
                            .to_owned(),
                        node.id.clone(),
                    )
                })
        })
        .collect();
    let mut seen_calls = HashSet::new();
    depth = 0;
    let mut active_class: Option<(String, i32)> = None;
    let mut active_method: Option<(String, i32)> = None;
    for (index, text) in lines.iter().enumerate() {
        if active_class.is_none()
            && let Some(captures) = TYPE.captures(text)
        {
            let name = capture(&captures, 2);
            active_class = Some((make_id(&[&state.stem, name]), depth));
            depth += brace_delta(text);
            continue;
        }
        if let Some((class_id, class_depth)) = active_class.clone() {
            let before = depth;
            if active_method.is_none() {
                let class_name = state
                    .extraction
                    .nodes
                    .iter()
                    .find(|node| node.id == class_id)
                    .and_then(|node| node.attributes.get("label"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let name = CONSTRUCTOR
                    .captures(text)
                    .filter(|captures| capture(captures, 1) == class_name)
                    .map(|captures| capture(&captures, 1).to_owned())
                    .or_else(|| {
                        METHOD
                            .captures(text)
                            .map(|captures| capture(&captures, 1).to_owned())
                    });
                if let Some(name) = name {
                    active_method = Some((make_id(&[&class_id, &name]), before));
                }
            }
            if let Some((caller, _)) = &active_method {
                for captures in MEMBER_CALL.captures_iter(text) {
                    let callee = capture(&captures, 2);
                    if callee.is_empty() {
                        continue;
                    }
                    if let Some(target) = call_targets
                        .get(callee)
                        .filter(|target| target.as_str() != caller)
                    {
                        if seen_calls.insert((caller.clone(), target.clone())) {
                            state.add_edge(caller, target, "calls", index + 1, Some("call"));
                        }
                    } else {
                        state.extraction.raw_calls_mut().push(RawCall {
                            caller_nid: caller.clone(),
                            callee: callee.to_owned(),
                            is_member_call: Some(false),
                            source_file: state.source_file.clone(),
                            source_location: format!("L{}", index + 1),
                            receiver: Some(None),
                            receiver_type: None,
                            lang: None,
                        });
                    }
                }
            }
            depth += brace_delta(text);
            if active_method
                .as_ref()
                .is_some_and(|(_, method_depth)| depth <= *method_depth)
            {
                active_method = None;
            }
            if depth <= class_depth {
                active_class = None;
            }
        } else {
            depth += brace_delta(text);
        }
    }
    state.extraction
}

fn extract_spock(path: &Path, source: &str) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = State::new(source_file, stem, file_id.clone());
    state.extraction.raw_calls = None;
    state.add_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
        false,
    );
    let mut class: Option<String> = None;
    for (index, text) in source.lines().enumerate() {
        if let Some(captures) = IMPORT.captures(text) {
            let target = capture(&captures, 1).rsplit('.').next().unwrap_or_default();
            state.add_edge(
                &file_id,
                &make_id(&[target]),
                "imports",
                index + 1,
                Some("import"),
            );
            continue;
        }
        if let Some(captures) = TYPE.captures(text) {
            let name = capture(&captures, 2);
            let id = make_id(&[&state.stem, name]);
            state.add_node(id.clone(), name, index + 1, false);
            state.add_edge(&file_id, &id, "contains", index + 1, None);
            class = Some(id);
            continue;
        }
        let Some(class_id) = &class else {
            continue;
        };
        if let Some(captures) = SPOCK_FEATURE.captures(text) {
            let name = captures
                .get(1)
                .or_else(|| captures.get(2))
                .map(|value| value.as_str())
                .unwrap_or_default();
            let id = make_id(&[class_id, name]);
            state.add_node(id.clone(), &format!("\"{name}\""), index + 1, false);
            state.add_edge(class_id, &id, "method", index + 1, None);
        } else if let Some(captures) = METHOD.captures(text) {
            let name = capture(&captures, 1);
            let id = make_id(&[class_id, name]);
            state.add_node(id.clone(), &format!(".{name}()"), index + 1, false);
            state.add_edge(class_id, &id, "method", index + 1, None);
        }
    }
    state.extraction
}

struct ActiveClass {
    id: String,
    name: String,
    start_depth: i32,
    current_method: Option<ActiveMethod>,
}

struct ActiveMethod {
    start_depth: i32,
}

struct State {
    source_file: String,
    stem: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
}

impl State {
    fn new(source_file: String, stem: String, _file_id: String) -> Self {
        Self {
            source_file,
            stem,
            extraction: Extraction::default(),
            seen_nodes: HashSet::new(),
        }
    }

    fn ensure_base(&mut self, name: &str) -> String {
        let local = make_id(&[&self.stem, name]);
        if self.seen_nodes.contains(&local) {
            return local;
        }
        let id = make_id(&[name]);
        if self.seen_nodes.insert(id.clone()) {
            let mut attributes = Map::new();
            attributes.insert("label".to_owned(), Value::String(name.to_owned()));
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert("source_file".to_owned(), Value::String(String::new()));
            attributes.insert("source_location".to_owned(), Value::String(String::new()));
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
        }
        id
    }

    fn add_node(&mut self, id: String, label: &str, line: usize, callable: bool) {
        if !self.seen_nodes.insert(id.clone()) {
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
        self.extraction.nodes.push(NodeRecord { id, attributes });
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
}

fn brace_delta(line: &str) -> i32 {
    line.bytes().fold(0, |depth, byte| match byte {
        b'{' => depth + 1,
        b'}' => depth - 1,
        _ => depth,
    })
}

fn groovy_constructor_line(lines: &[&str], index: usize) -> usize {
    for previous in (0..index).rev() {
        let text = lines[previous].trim();
        if text.is_empty() {
            continue;
        }
        if !text.contains('(') && !text.contains('{') && !text.contains('}') {
            return previous + 1;
        }
        break;
    }
    index + 1
}

fn capture<'capture>(captures: &'capture regex::Captures<'_>, index: usize) -> &'capture str {
    captures
        .get(index)
        .map(|value| value.as_str())
        .unwrap_or_default()
}

fn regex(pattern: &str) -> Regex {
    Regex::new(pattern)
        .unwrap_or_else(|error| unreachable!("static Groovy regex is invalid: {error}"))
}
