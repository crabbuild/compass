use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use compass_model::{EdgeRecord, NodeRecord};
use regex::Regex;
use serde_json::{Map, Value, json};

use crate::{ExtractError, Extraction, file_stem, make_id};

const MAX_PACKAGE_BYTES: u64 = 2 * 1024 * 1024;

static OBJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*object\s+\w+\s*:\s*(\w+)")
        .unwrap_or_else(|error| unreachable!("static Pascal form object regex is invalid: {error}"))
});
static EVENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*On\w+\s*=\s*(\w+)")
        .unwrap_or_else(|error| unreachable!("static Pascal form event regex is invalid: {error}"))
});
static END: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*end\s*$")
        .unwrap_or_else(|error| unreachable!("static Pascal form end regex is invalid: {error}"))
});

pub(crate) fn extract_form(path: &Path) -> Result<Extraction, ExtractError> {
    let raw = fs::read(path).map_err(|source| compass_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dfm"))
        && raw.starts_with(&[0xff, 0x0a])
    {
        return Ok(failure(&format!(
            "binary DFM (convert to text in Delphi IDE to index): {}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        )));
    }
    let text = String::from_utf8_lossy(&raw);
    Ok(parse_form(path, &text))
}

fn parse_form(path: &Path, text: &str) -> Extraction {
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = State::new(source_file);
    state.add_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
    );
    let mut stack = vec![file_id];
    for (index, line_text) in text.lines().enumerate() {
        let line = index + 1;
        if let Some(class_name) = OBJECT
            .captures(line_text)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str())
        {
            let id = make_id(&[&stem, class_name]);
            state.add_node(id.clone(), class_name, line);
            if let Some(parent) = stack.last() {
                state.add_edge(parent, &id, "contains", line, None);
            }
            stack.push(id);
            continue;
        }
        if let Some(handler) = EVENT
            .captures(line_text)
            .and_then(|captures| captures.get(1))
            .map(|capture| capture.as_str())
            && stack.len() > 1
        {
            let id = make_id(&[&stem, handler]);
            state.add_node(id.clone(), &format!("{handler}()"), line);
            if let Some(parent) = stack.last() {
                state.add_edge(parent, &id, "references", line, Some("event"));
            }
            continue;
        }
        if END.is_match(line_text) && stack.len() > 1 {
            stack.pop();
        }
    }
    state.finish_with_tokens()
}

pub(crate) fn extract_package(path: &Path) -> Result<Extraction, ExtractError> {
    let mut source = Vec::new();
    File::open(path)
        .map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .take(MAX_PACKAGE_BYTES + 1)
        .read_to_end(&mut source)
        .map_err(|source| compass_files::FileError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if source.len() > MAX_PACKAGE_BYTES as usize {
        return Ok(failure("package file too large"));
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
        Err(error) => return Ok(failure(&error.to_string())),
    };

    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = State::new(source_file);
    state.add_node(
        file_id.clone(),
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
    );
    let package_name = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Package")
        .and_then(|package| {
            package
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "Name")
        })
        .and_then(|name| name.attribute("Value"))
        .map_or_else(
            || {
                path.file_stem()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned()
            },
            str::to_owned,
        );
    let package_id = make_id(&[&stem, &package_name]);
    state.add_node(package_id.clone(), &package_name, 1);
    state.add_edge(&file_id, &package_id, "contains", 1, None);

    for dependency in child_items(&document, "RequiredPkgs") {
        if let Some(name) = child_value(dependency, "PackageName")
            && !name.is_empty()
        {
            let id = make_id(&[name]);
            state.add_node(id.clone(), name, 1);
            state.add_edge(&package_id, &id, "imports", 1, Some("import"));
        }
    }
    for unit in child_items(&document, "Files") {
        if let Some(name) = child_value(unit, "UnitName")
            && !name.is_empty()
        {
            let id = resolve_unit(path, name);
            state.add_node(id.clone(), name, 1);
            state.add_edge(&package_id, &id, "contains", 1, None);
        }
    }
    Ok(state.finish_with_tokens())
}

fn child_items<'document>(
    document: &'document roxmltree::Document<'document>,
    container_name: &str,
) -> Vec<roxmltree::Node<'document, 'document>> {
    document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == container_name)
        .flat_map(|container| container.children().filter(roxmltree::Node::is_element))
        .collect()
}

fn child_value<'node>(node: roxmltree::Node<'node, '_>, name: &str) -> Option<&'node str> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
        .and_then(|child| child.attribute("Value"))
}

fn resolve_unit(from_path: &Path, unit_name: &str) -> String {
    let root = pascal_project_root(from_path);
    let mut units = HashMap::new();
    collect_pascal_units(&root, &mut units);
    units
        .get(&unit_name.to_lowercase())
        .map_or_else(|| make_id(&[unit_name]), |path| make_id(&[path]))
}

fn pascal_project_root(from_path: &Path) -> PathBuf {
    let mut best = from_path
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .to_path_buf();
    let mut current = best.clone();
    for _ in 0..12 {
        if current.components().count() <= 1 {
            break;
        }
        let (pascal_count, project_count) = fs::read_dir(&current).map_or((0, 0), |entries| {
            entries
                .filter_map(Result::ok)
                .fold((0, 0), |mut counts, entry| {
                    let extension = entry
                        .path()
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .map(str::to_ascii_lowercase);
                    if extension.as_deref() == Some("pas") {
                        counts.0 += 1;
                    } else if extension.as_deref() == Some("dpr") {
                        counts.1 += 1;
                    }
                    counts
                })
        });
        if pascal_count >= 2 || project_count >= 1 {
            best.clone_from(&current);
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

fn collect_pascal_units(directory: &Path, output: &mut HashMap<String, String>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_pascal_units(&path, output);
            continue;
        }
        let supported = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "pas" | "pp" | "dpr" | "dpk" | "inc"
                )
            });
        if supported && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
            output.insert(stem.to_lowercase(), path.to_string_lossy().into_owned());
        }
    }
}

struct State {
    source_file: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    seen_edges: HashSet<(String, String, String)>,
}

impl State {
    fn new(source_file: String) -> Self {
        Self {
            source_file,
            extraction: Extraction {
                raw_calls: None,
                ..Extraction::default()
            },
            seen_nodes: HashSet::new(),
            seen_edges: HashSet::new(),
        }
    }

    fn add_node(&mut self, id: String, label: &str, line: usize) {
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
        let key = (source.to_owned(), target.to_owned(), relation.to_owned());
        if !self.seen_edges.insert(key) {
            return;
        }
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

    fn finish_with_tokens(mut self) -> Extraction {
        self.extraction
            .extensions
            .insert("input_tokens".to_owned(), json!(0));
        self.extraction
            .extensions
            .insert("output_tokens".to_owned(), json!(0));
        self.extraction
    }
}

fn failure(message: &str) -> Extraction {
    let mut extraction = Extraction {
        raw_calls: None,
        ..Extraction::default()
    };
    extraction.error = Some(message.to_owned());
    extraction
}
