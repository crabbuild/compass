use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use crate::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use regex::Regex;
use serde_json::{Map, Value, json};

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
    let bytes = fs::read(path).map_err(|source| compass_files::FileError::Io {
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
        heading_stack: Vec::new(),
        heading_occurrences: HashMap::new(),
    };

    state.add_node(
        file_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default(),
        1,
        None,
    );

    let mut fence = None;
    let mut byte_offset = 0;
    for (index, line_text) in source.split_inclusive('\n').enumerate() {
        let text = line_text
            .strip_suffix('\n')
            .unwrap_or(line_text)
            .strip_suffix('\r')
            .unwrap_or_else(|| line_text.strip_suffix('\n').unwrap_or(line_text));
        let line = index + 1;
        if let Some(open) = fence {
            if closes_fence(text, open) {
                fence = None;
            }
            byte_offset += line_text.len();
            continue;
        }
        if let Some(open) = opens_fence(text) {
            fence = Some(open);
            byte_offset += line_text.len();
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
                state.add_link(
                    target.as_str(),
                    LinkSite {
                        line,
                        start_byte: byte_offset + whole.start(),
                        end_byte: byte_offset + whole.end(),
                        line_start: byte_offset,
                    },
                );
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
                state.add_link(
                    target.as_str(),
                    LinkSite {
                        line,
                        start_byte: byte_offset + whole.start(),
                        end_byte: byte_offset + whole.end(),
                        line_start: byte_offset,
                    },
                );
            }
        }
        if let Some(captures) = REFERENCE_DEFINITION.captures(text)
            && let (Some(whole), Some(target)) = (captures.get(0), captures.get(1))
        {
            state.add_link(
                target.as_str(),
                LinkSite {
                    line,
                    start_byte: byte_offset + whole.start(),
                    end_byte: byte_offset + whole.end(),
                    line_start: byte_offset,
                },
            );
        }

        let Some(captures) = HEADING.captures(text) else {
            byte_offset += line_text.len();
            continue;
        };
        let (Some(markers), Some(title)) = (captures.get(1), captures.get(2)) else {
            continue;
        };
        let level = markers.as_str().len();
        let title = title.as_str().trim();
        while state
            .heading_stack
            .last()
            .is_some_and(|(parent_level, _, _)| *parent_level >= level)
        {
            state.heading_stack.pop();
        }
        let qualified_base = state.heading_stack.last().map_or_else(
            || title.to_owned(),
            |(_, _, parent_scope)| format!("{parent_scope}::{title}"),
        );
        let occurrence = state
            .heading_occurrences
            .entry(qualified_base.clone())
            .or_default();
        *occurrence += 1;
        let mut qualified_name = if *occurrence == 1 {
            qualified_base
        } else {
            format!("{qualified_base}#{occurrence}")
        };
        if state.heading_stack.is_empty()
            && path_file_name(state.path).is_some_and(|name| name == qualified_name)
        {
            qualified_name.push_str("::heading");
        }
        let mut id = make_id(&[&state.stem, &qualified_name]);
        if id == state.file_id {
            id = make_id(&[&state.source_file, "heading", &qualified_name]);
        }
        state.add_node(id.clone(), title, line, Some(&qualified_name));
        let parent = state
            .heading_stack
            .last()
            .map_or_else(|| state.file_id.clone(), |(_, id, _)| id.clone());
        state.add_edge(parent, id.clone(), "contains", line);
        state.heading_stack.push((level, id, qualified_name));
        byte_offset += line_text.len();
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

fn path_file_name(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

struct State<'path> {
    path: &'path Path,
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen_nodes: HashSet<String>,
    heading_stack: Vec<(usize, String, String)>,
    heading_occurrences: HashMap<String, usize>,
}

#[derive(Clone, Copy)]
struct LinkSite {
    line: usize,
    start_byte: usize,
    end_byte: usize,
    line_start: usize,
}

impl State<'_> {
    fn add_node(&mut self, id: String, label: &str, line: usize, qualified_name: Option<&str>) {
        if !self.seen_nodes.insert(id.clone()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), Value::String(label.to_owned()));
        if let Some(qualified_name) = qualified_name {
            attributes.insert(
                "qualified_name".to_owned(),
                Value::String(qualified_name.to_owned()),
            );
        }
        attributes.insert("file_type".to_owned(), Value::String("document".to_owned()));
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".to_owned(),
            Value::String(format!("L{line}")),
        );
        attributes.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
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
        attributes.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        attributes.insert("weight".to_owned(), json!(1.0));
        self.extraction.edges.push(EdgeRecord {
            source,
            target,
            attributes,
        });
    }

    fn add_link(&mut self, raw: &str, site: LinkSite) {
        let Some(target) = resolve_link(raw, self.path.parent().unwrap_or_else(|| Path::new("")))
        else {
            return;
        };
        let target_id = make_id(&[&target.to_string_lossy()]);
        if target_id == self.file_id {
            return;
        }
        let relation = if is_documentable_source(&target) {
            if !target.is_file() {
                return;
            }
            "documents"
        } else {
            "references"
        };
        self.add_edge_range(self.file_id.clone(), target_id, relation, site);
    }

    fn add_edge_range(&mut self, source: String, target: String, relation: &str, site: LinkSite) {
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
            Value::String(format!("L{}", site.line)),
        );
        attributes.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        attributes.insert("start_byte".to_owned(), json!(site.start_byte));
        attributes.insert("end_byte".to_owned(), json!(site.end_byte));
        attributes.insert("start_line".to_owned(), json!(site.line));
        attributes.insert("end_line".to_owned(), json!(site.line));
        attributes.insert(
            "column_start".to_owned(),
            json!(site.start_byte.saturating_sub(site.line_start)),
        );
        attributes.insert(
            "column_end".to_owned(),
            json!(site.end_byte.saturating_sub(site.line_start)),
        );
        attributes.insert("weight".to_owned(), json!(1.0));
        self.extraction.edges.push(EdgeRecord {
            source,
            target,
            attributes,
        });
    }
}

fn opens_fence(line: &str) -> Option<(u8, usize)> {
    let bytes = line.as_bytes();
    let indent = bytes.iter().take_while(|byte| **byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let marker = *bytes.get(indent)?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = bytes[indent..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    if length < 3 || (marker == b'`' && bytes[indent + length..].contains(&b'`')) {
        return None;
    }
    Some((marker, length))
}

fn closes_fence(line: &str, fence: (u8, usize)) -> bool {
    let bytes = line.as_bytes();
    let indent = bytes.iter().take_while(|byte| **byte == b' ').count();
    if indent > 3 || bytes.get(indent) != Some(&fence.0) {
        return false;
    }
    let length = bytes[indent..]
        .iter()
        .take_while(|byte| **byte == fence.0)
        .count();
    length >= fence.1
        && bytes[indent + length..]
            .iter()
            .all(|byte| byte.is_ascii_whitespace())
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
    if !is_supported_local_link(suffix) {
        return None;
    }
    if !target.is_absolute() {
        target = source_directory.join(target);
    }
    Some(lexical_normalize(&target))
}

fn is_documentable_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "rs" | "py"
                    | "js"
                    | "jsx"
                    | "ts"
                    | "tsx"
                    | "java"
                    | "cs"
                    | "go"
                    | "c"
                    | "cc"
                    | "cpp"
                    | "h"
                    | "hpp"
                    | "rb"
                    | "php"
                    | "swift"
                    | "kt"
                    | "kts"
                    | "scala"
                    | "dart"
                    | "ex"
                    | "exs"
                    | "sql"
            )
        })
}

fn is_supported_local_link(suffix: &str) -> bool {
    matches!(
        suffix,
        ".md"
            | ".mdx"
            | ".qmd"
            | ".markdown"
            | ".rst"
            | ".txt"
            | ".rs"
            | ".py"
            | ".js"
            | ".jsx"
            | ".ts"
            | ".tsx"
            | ".java"
            | ".cs"
            | ".go"
            | ".c"
            | ".cc"
            | ".cpp"
            | ".h"
            | ".hpp"
            | ".rb"
            | ".php"
            | ".swift"
            | ".kt"
            | ".kts"
            | ".scala"
            | ".dart"
            | ".ex"
            | ".exs"
            | ".sql"
    )
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
