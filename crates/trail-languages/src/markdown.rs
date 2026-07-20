use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};
use trail_model::{EdgeRecord, NodeRecord};

use crate::{ExtractError, Extraction, file_stem, make_id};

static INLINE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\[[^\]]*\]\(\s*<?([^\)\s>]+)>?(?:\s+[^\)]*)?\)"#)
        .unwrap_or_else(|error| unreachable!("static Markdown link regex is invalid: {error}"))
});
static REFERENCE_DEFINITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s{0,3}\[[^\]]+\]:\s*<?([^\s>]+)>?"#)
        .unwrap_or_else(|error| unreachable!("static Markdown reference regex is invalid: {error}"))
});
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\[\[([^\]|#]+)(?:[#|][^\]]*)?\]\]"#)
        .unwrap_or_else(|error| unreachable!("static Markdown wikilink regex is invalid: {error}"))
});
static HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(#{1,6})\s+(.+)")
        .unwrap_or_else(|error| unreachable!("static Markdown heading regex is invalid: {error}"))
});

pub(crate) fn extract(path: &Path) -> Result<Extraction, ExtractError> {
    let bytes = fs::read(path).map_err(|source| trail_files::FileError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let source = String::from_utf8_lossy(&bytes);
    let source_file = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_id = make_id(&[&source_file]);
    let mut state = State {
        path,
        source_file,
        stem,
        file_id: file_id.clone(),
        extraction: Extraction {
            raw_calls: None,
            ..Extraction::default()
        },
        seen_nodes: HashSet::new(),
        linked_targets: HashSet::new(),
        heading_stack: Vec::new(),
    };

    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
    );

    let mut in_code_block = false;
    for (index, text) in source.lines().enumerate() {
        let line = index + 1;
        if text.trim().starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if in_code_block {
            continue;
        }

        for captures in INLINE_LINK.captures_iter(text) {
            let Some(whole) = captures.get(0) else {
                continue;
            };
            if whole.start() > 0 && text.as_bytes().get(whole.start() - 1) == Some(&b'!') {
                continue;
            }
            if let Some(target) = captures.get(1) {
                state.add_link(target.as_str(), line);
            }
        }
        for captures in WIKILINK.captures_iter(text) {
            let Some(whole) = captures.get(0) else {
                continue;
            };
            if whole.start() > 0 && text.as_bytes().get(whole.start() - 1) == Some(&b'!') {
                continue;
            }
            if let Some(target) = captures.get(1) {
                state.add_link(target.as_str(), line);
            }
        }
        if let Some(captures) = REFERENCE_DEFINITION.captures(text)
            && let Some(target) = captures.get(1)
        {
            state.add_link(target.as_str(), line);
        }

        let Some(captures) = HEADING.captures(text) else {
            continue;
        };
        let (Some(markers), Some(title)) = (captures.get(1), captures.get(2)) else {
            continue;
        };
        let level = markers.as_str().len();
        let title = title.as_str().trim();
        let mut id = make_id(&[&state.stem, title]);
        if state.seen_nodes.contains(&id) {
            id = make_id(&[&state.stem, title, &line.to_string()]);
        }
        state.add_node(id.clone(), title, line);
        while state
            .heading_stack
            .last()
            .is_some_and(|(parent_level, _)| *parent_level >= level)
        {
            state.heading_stack.pop();
        }
        let parent = state
            .heading_stack
            .last()
            .map_or_else(|| state.file_id.clone(), |(_, id)| id.clone());
        state.add_edge(parent, id.clone(), "contains", line);
        state.heading_stack.push((level, id));
    }

    state
        .extraction
        .extensions
        .insert("input_tokens".to_owned(), json!(0));
    state
        .extraction
        .extensions
        .insert("output_tokens".to_owned(), json!(0));
    Ok(state.extraction)
}

struct State<'path> {
    path: &'path Path,
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    linked_targets: HashSet<String>,
    heading_stack: Vec<(usize, String)>,
}

impl State<'_> {
    fn add_node(&mut self, id: String, label: &str, line: usize) {
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        attributes.insert("file_type".to_owned(), Value::String("document".to_owned()));
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

    fn add_edge(&mut self, source: String, target: String, relation: &str, line: usize) {
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
        self.extraction.edges.push(EdgeRecord {
            source,
            target,
            attributes,
        });
    }

    fn add_link(&mut self, raw: &str, line: usize) {
        let Some(target) = resolve_link(raw, self.path.parent().unwrap_or_else(|| Path::new("")))
        else {
            return;
        };
        let target_id = make_id(&[&target.to_string_lossy()]);
        if target_id == self.file_id || !self.linked_targets.insert(target_id.clone()) {
            return;
        }
        self.add_edge(self.file_id.clone(), target_id, "references", line);
    }
}

fn resolve_link(raw: &str, source_directory: &Path) -> Option<PathBuf> {
    let target = raw.trim();
    if target.is_empty() {
        return None;
    }
    let target = target.split_once('#').map_or(target, |(head, _)| head);
    let target = target
        .split_once('?')
        .map_or(target, |(head, _)| head)
        .trim();
    if target.is_empty() {
        return None;
    }
    let lower = target.to_ascii_lowercase();
    if target.contains("://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("//")
        || lower.starts_with("data:")
    {
        return None;
    }
    let mut target = PathBuf::from(target);
    let suffix = target
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(String::new, |extension| {
            format!(".{extension}").to_ascii_lowercase()
        });
    let suffix = if suffix.is_empty() {
        target.set_extension("md");
        ".md"
    } else {
        suffix.as_str()
    };
    if !matches!(
        suffix,
        ".md" | ".mdx" | ".qmd" | ".markdown" | ".rst" | ".txt"
    ) {
        return None;
    }
    if !target.is_absolute() {
        target = source_directory.join(target);
    }
    Some(lexical_normalize(&target))
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
