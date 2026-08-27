use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::facts::stamp_source_range;
use crate::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value, json};
use tree_sitter::{Node, Parser};
use tree_sitter_language_pack::{DataNode, DataNodeKind, ProcessConfig};
use tree_sitter_md::{INLINE_LANGUAGE, LANGUAGE};

const FRONTMATTER_MAX_BYTES: usize = 64 * 1024;
const MAX_BLOCKS: usize = 100_000;
const MAX_LINKS: usize = 100_000;
const MAX_DIAGNOSTICS: usize = 256;
const MAX_METADATA_KEYS: usize = 256;
const MAX_METADATA_STRING_BYTES: usize = 16 * 1024;
const MAX_METADATA_ARRAY_ITEMS: usize = 256;
const MAX_METADATA_DEPTH: usize = 12;
const MAX_METADATA_GRAPH_NODES: usize = 512;
const MAX_LABEL_CHARS: usize = 512;
const MAX_TABLE_COLUMNS: usize = 128;
const MAX_TABLE_ROWS: usize = 10_000;
const MAX_TABLE_CELL_BYTES: usize = 4 * 1024;
const MAX_TABLE_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_TABLE_CELLS: usize = 100_000;
const MAX_TABLE_NODES_PER_TABLE: usize = 20_000;
const MAX_TABLE_CELLS_PER_TABLE: usize = 16_384;
const MAX_TABLE_TEXT_BYTES_PER_TABLE: usize = 512 * 1024;
const MAX_DOCUMENT_REFERENCES: usize = 128;
const MAX_REFERENCE_SPELLING_BYTES: usize = 4 * 1024;

/// Extract Markdown from bytes supplied by the caller.
///
/// The engine uses this path after its bounded source read. Keeping the source
/// buffer here avoids a second filesystem read and lets every location remain
/// relative to the exact bytes that were parsed.
pub(crate) fn extract_source(
    path: &Path,
    source_file: &str,
    source: &[u8],
) -> Result<crate::Extraction, crate::ExtractError> {
    let mut block_parser = Parser::new();
    let block_language = LANGUAGE.into();
    block_parser
        .set_language(&block_language)
        .map_err(|error| crate::ExtractError::MissingGrammar {
            language: "markdown".to_owned(),
            detail: error.to_string(),
        })?;
    let block_tree = block_parser
        .parse(source, None)
        .ok_or_else(|| crate::ExtractError::ParseCancelled(path.to_path_buf()))?;

    let mut inline_parser = Parser::new();
    let inline_language = INLINE_LANGUAGE.into();
    inline_parser
        .set_language(&inline_language)
        .map_err(|error| crate::ExtractError::MissingGrammar {
            language: "markdown_inline".to_owned(),
            detail: error.to_string(),
        })?;

    let (frontmatter, frontmatter_diagnostic) = parse_frontmatter(source);
    let stem = crate::file_stem(path);
    let file_id = crate::make_id(&[source_file]);
    let line_starts = newline_offsets(source);
    let mut state = State {
        path,
        source,
        source_file: source_file.to_owned(),
        stem,
        file_id: file_id.clone(),
        extraction: crate::Extraction {
            raw_calls: None,
            ..crate::Extraction::default()
        },
        seen_nodes: HashSet::new(),
        heading_stack: Vec::new(),
        heading_occurrences: HashMap::new(),
        heading_slug_occurrences: HashMap::new(),
        heading_targets: HashMap::new(),
        reference_definitions: HashMap::new(),
        footnote_definitions: HashMap::new(),
        pending_links: Vec::new(),
        unresolved_links: Vec::new(),
        external_links: Vec::new(),
        document_references: HashMap::new(),
        diagnostics: Vec::new(),
        inline_parser,
        line_starts,
        next_block_index: 1,
        other_count: 0,
        table_occurrences: HashMap::new(),
        table_cells_retained: 0,
        table_text_bytes: 0,
        table_limit_diagnostics: HashSet::new(),
        document_reference_limit_reported: false,
    };

    state.add_root(
        file_id,
        frontmatter
            .as_ref()
            .map(|frontmatter| frontmatter.metadata.clone()),
    );
    if let Some(frontmatter) = frontmatter {
        state.add_frontmatter_nodes(frontmatter.facts);
    }
    if let Some(diagnostic) = frontmatter_diagnostic {
        state.add_diagnostic(diagnostic);
    }
    if block_tree.root_node().has_error() {
        state.add_diagnostic("Markdown parser recovered from malformed syntax".to_owned());
        state.extraction.extensions.insert(
            crate::EXTRACTION_QUALITY_EXTENSION.to_owned(),
            json!(crate::EXTRACTION_QUALITY_PARTIAL),
        );
        state.extraction.extensions.insert(
            crate::EXTRACTION_QUALITY_REASON_EXTENSION.to_owned(),
            json!("markdown_parser_recovery"),
        );
    }

    let root = block_tree.root_node();
    let root_id = state.file_id.clone();
    let mut cursor = root.walk();
    for child in root.children(&mut cursor).filter(Node::is_named) {
        match child.kind() {
            "minus_metadata" | "plus_metadata" => {}
            "section" => state.walk_section(child, Some(&root_id)),
            _ => state.emit_block(child, Some(&root_id)),
        }
    }
    state.scan_reference_definitions();
    state.scan_footnotes();
    state.scan_other_constructs();
    state.finalize_links();
    state.publish_document_references();

    state
        .extraction
        .extensions
        .insert("input_tokens".to_owned(), json!(0));
    state
        .extraction
        .extensions
        .insert("output_tokens".to_owned(), json!(0));
    state.extraction.extensions.insert(
        "markdown_block_count".to_owned(),
        json!(state.next_block_index.saturating_sub(1)),
    );
    state.extraction.extensions.insert(
        "markdown_link_count".to_owned(),
        json!(state.pending_link_count()),
    );
    state.extraction.extensions.insert(
        "markdown_footnote_count".to_owned(),
        json!(
            state
                .footnote_definitions
                .values()
                .map(Vec::len)
                .sum::<usize>()
        ),
    );
    state
        .extraction
        .extensions
        .insert("markdown_other_count".to_owned(), json!(state.other_count));
    if !state.diagnostics.is_empty() {
        state.extraction.extensions.insert(
            "markdown_diagnostics".to_owned(),
            Value::Array(state.diagnostics.into_iter().map(Value::String).collect()),
        );
    }
    if !state.unresolved_links.is_empty() {
        state.extraction.extensions.insert(
            "markdown_unresolved_links".to_owned(),
            Value::Array(state.unresolved_links),
        );
    }
    if !state.external_links.is_empty() {
        state.extraction.extensions.insert(
            "markdown_external_links".to_owned(),
            Value::Array(state.external_links),
        );
    }
    Ok(state.extraction)
}

struct State<'source, 'path> {
    path: &'path Path,
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: crate::Extraction,
    seen_nodes: HashSet<String>,
    heading_stack: Vec<HeadingFrame>,
    heading_occurrences: HashMap<String, usize>,
    heading_slug_occurrences: HashMap<String, usize>,
    heading_targets: HashMap<String, Vec<String>>,
    reference_definitions: HashMap<String, String>,
    footnote_definitions: HashMap<String, Vec<String>>,
    pending_links: Vec<PendingLink>,
    unresolved_links: Vec<Value>,
    external_links: Vec<Value>,
    document_references: HashMap<String, Vec<Value>>,
    diagnostics: Vec<String>,
    inline_parser: Parser,
    line_starts: Vec<usize>,
    next_block_index: usize,
    other_count: usize,
    table_occurrences: HashMap<String, usize>,
    table_cells_retained: usize,
    table_text_bytes: usize,
    table_limit_diagnostics: HashSet<&'static str>,
    document_reference_limit_reported: bool,
}

#[derive(Clone)]
struct HeadingFrame {
    level: usize,
    id: String,
    qualified_name: String,
}

#[derive(Clone)]
struct PendingLink {
    raw: String,
    reference_label: Option<String>,
    owner_id: String,
    site: LinkSite,
    kind: &'static str,
}

#[derive(Clone, Copy)]
struct LinkSite {
    start_byte: usize,
    end_byte: usize,
    line: usize,
    line_start: usize,
}

struct DocumentTargetHint {
    path: String,
    extension_inferred: bool,
    root_relative: bool,
}

struct FrontmatterExtraction {
    metadata: Map<String, Value>,
    facts: Vec<FrontmatterFact>,
}

struct FrontmatterFact {
    key: String,
    key_path: String,
    parent_path: Option<String>,
    value: Value,
    start_byte: usize,
    end_byte: usize,
}

#[derive(Default)]
struct MetadataBudget {
    keys: usize,
    array_items: usize,
}

#[derive(Clone)]
struct TableCellFact {
    text: String,
    raw_start: usize,
    raw_end: usize,
}

#[derive(Clone, Copy)]
enum TableAlignment {
    Left,
    Center,
    Right,
    Unspecified,
}

impl TableAlignment {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Center => "center",
            Self::Right => "right",
            Self::Unspecified => "unspecified",
        }
    }
}

fn table_children<'tree>(
    node: Node<'tree>,
) -> (Option<Node<'tree>>, Option<Node<'tree>>, Vec<Node<'tree>>) {
    let mut header = None;
    let mut delimiter = None;
    let mut rows = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(Node::is_named) {
        match child.kind() {
            "pipe_table_header" => header = Some(child),
            "pipe_table_delimiter_row" => delimiter = Some(child),
            "pipe_table_row" => rows.push(child),
            _ => {}
        }
    }
    (header, delimiter, rows)
}

fn table_cells(node: Node<'_>, source: &[u8]) -> Vec<TableCellFact> {
    let mut cells = Vec::new();
    let mut cursor = node.walk();
    for child in node
        .children(&mut cursor)
        .filter(|child| child.kind() == "pipe_table_cell")
    {
        cells.push(TableCellFact::empty(
            child.start_byte(),
            child.end_byte(),
            node_text(source, child.start_byte(), child.end_byte()),
        ));
    }
    cells
}

fn table_alignments(node: Node<'_>, source: &[u8]) -> Vec<TableAlignment> {
    let mut alignments = Vec::new();
    let mut cursor = node.walk();
    for cell in node
        .children(&mut cursor)
        .filter(|child| child.kind() == "pipe_table_delimiter_cell")
    {
        let text = node_text(source, cell.start_byte(), cell.end_byte());
        let text = text.trim();
        let alignment = match (text.starts_with(':'), text.ends_with(':')) {
            (true, true) => TableAlignment::Center,
            (true, false) => TableAlignment::Left,
            (false, true) => TableAlignment::Right,
            (false, false) => TableAlignment::Unspecified,
        };
        alignments.push(alignment);
    }
    alignments
}

fn normalize_table_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn table_label(section: &str, headers: &[String]) -> String {
    let header = headers
        .iter()
        .filter(|header| !header.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" | ");
    let title = if header.is_empty() {
        "table".to_owned()
    } else {
        format!("table: {header}")
    };
    if section.is_empty() {
        bounded_label(&title)
    } else {
        bounded_label(&format!("{section} — {title}"))
    }
}

fn row_label(headers: &[String], cells: &[String]) -> String {
    let values = cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| !cell.is_empty())
        .take(4)
        .map(
            |(index, cell)| match headers.get(index).filter(|header| !header.is_empty()) {
                Some(header) => format!("{header}={cell}"),
                None => cell.clone(),
            },
        )
        .collect::<Vec<_>>();
    if values.is_empty() {
        "table row".to_owned()
    } else {
        bounded_label(&values.join(" · "))
    }
}

fn table_cell_label(header: Option<&str>, text: &str, column_index: usize) -> String {
    let value = if text.is_empty() { "(empty)" } else { text };
    let column = header
        .filter(|header| !header.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("column {}", column_index.saturating_add(1)));
    bounded_label(&format!("{column}: {value}"))
}

fn compact_identity(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if output.len() >= 64 {
            break;
        }
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-') {
            output.push(character.to_ascii_lowercase());
        } else if !output.ends_with('-') {
            output.push('-');
        }
    }
    let trimmed = output.trim_matches('-');
    if trimmed.is_empty() {
        "row".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn truncate_utf8(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text[..end].to_owned()
}

fn is_inline_reference_candidate(value: &str) -> bool {
    if value.is_empty()
        || value.len() > MAX_REFERENCE_SPELLING_BYTES
        || value.chars().any(char::is_whitespace)
        || value.contains('`')
    {
        return false;
    }
    let valid = value.chars().all(|character| {
        character.is_ascii_alphanumeric()
            || matches!(character, '_' | '-' | '/' | '.' | ':' | '$' | '#' | '@')
    });
    if !valid {
        return false;
    }
    // A path, qualified symbol, or identifier with an explicit separator is
    // useful evidence. Keep ordinary single-word spans too: the backticks are
    // the syntax proof that the author intended a literal identifier.
    value
        .chars()
        .any(|character| character.is_ascii_alphanumeric())
}

/// Encode a nested source anchor using the same camel-case contract as the
/// strict graph model. Raw extraction attributes otherwise use snake case,
/// but table columns/cells are typed payloads and must be round-trippable.
fn source_anchor_json(
    source_file: &str,
    source: &[u8],
    line_starts: &[usize],
    start: usize,
    end: usize,
) -> Value {
    let start = start.min(source.len());
    let end = end.clamp(start, source.len());
    let (start_line, start_column) = indexed_source_point(line_starts, start);
    let (end_line, end_column) = indexed_source_point(line_starts, end);
    json!({
        "file": source_file,
        "startByte": start as u64,
        "endByte": end as u64,
        "startLine": start_line as u64,
        "startColumn": start_column as u64,
        "endLine": end_line as u64,
        "endColumn": end_column as u64,
    })
}

fn newline_offsets(source: &[u8]) -> Vec<usize> {
    let mut offsets = vec![0];
    offsets.extend(
        source
            .iter()
            .enumerate()
            .filter_map(|(index, byte)| (*byte == b'\n').then_some(index.saturating_add(1))),
    );
    offsets
}

fn indexed_source_point(line_starts: &[usize], offset: usize) -> (usize, usize) {
    let line_index = match line_starts.binary_search(&offset) {
        Ok(index) => index,
        Err(index) => index.saturating_sub(1),
    };
    let line_start = line_starts.get(line_index).copied().unwrap_or(0);
    (
        line_index.saturating_add(1),
        offset.saturating_sub(line_start),
    )
}

fn stamp_source_range_indexed(
    attributes: &mut Map<String, Value>,
    source: &[u8],
    line_starts: &[usize],
    start: usize,
    end: usize,
) {
    let start = start.min(source.len());
    let end = end.clamp(start, source.len());
    let (start_line, start_column) = indexed_source_point(line_starts, start);
    let (end_line, end_column) = indexed_source_point(line_starts, end);
    attributes.insert("start_byte".to_owned(), Value::from(start as u64));
    attributes.insert("end_byte".to_owned(), Value::from(end as u64));
    attributes.insert("line_start".to_owned(), Value::from(start_line as u64));
    attributes.insert("line_end".to_owned(), Value::from(end_line as u64));
    attributes.insert("column_start".to_owned(), Value::from(start_column as u64));
    attributes.insert("column_end".to_owned(), Value::from(end_column as u64));
}

impl TableCellFact {
    fn empty(start: usize, end: usize, text: String) -> Self {
        Self {
            text,
            raw_start: start,
            raw_end: end,
        }
    }
}

impl State<'_, '_> {
    fn add_root(&mut self, id: String, metadata: Option<Map<String, Value>>) {
        self.seen_nodes.insert(id.clone());
        let mut attributes = Map::new();
        let source_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let label = metadata
            .as_ref()
            .and_then(|metadata| metadata.get("title"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(bounded_label)
            .unwrap_or_else(|| source_name.to_owned());
        attributes.insert("label".to_owned(), Value::String(label));
        attributes.insert(
            "qualified_name".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("file_type".to_owned(), Value::String("document".to_owned()));
        attributes.insert(
            "document_kind".to_owned(),
            Value::String("document".to_owned()),
        );
        attributes.insert(
            "document_format".to_owned(),
            Value::String("markdown".to_owned()),
        );
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".to_owned(), Value::String("L1".to_owned()));
        attributes.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        stamp_source_range(&mut attributes, self.source, 0, self.source.len());
        if let Some(metadata) = metadata {
            attributes.insert("document_metadata".to_owned(), Value::Object(metadata));
        }
        self.extraction.nodes.push(NodeRecord {
            id: id.clone(),
            attributes,
        });
        self.file_id = id;
    }

    fn add_frontmatter_nodes(&mut self, facts: Vec<FrontmatterFact>) {
        let mut ids = HashMap::new();
        for fact in facts {
            let id = crate::make_id(&[&self.source_file, "markdown_frontmatter", &fact.key_path]);
            let parent = fact
                .parent_path
                .as_ref()
                .and_then(|path| ids.get(path))
                .cloned()
                .unwrap_or_else(|| self.file_id.clone());
            let mut attributes = Map::new();
            attributes.insert(
                "symbol_kind".to_owned(),
                Value::String("config_key".to_owned()),
            );
            attributes.insert("file_type".to_owned(), Value::String("code".to_owned()));
            attributes.insert(
                "label".to_owned(),
                Value::String(frontmatter_fact_label(&fact.key, &fact.value)),
            );
            attributes.insert(
                "qualified_name".to_owned(),
                Value::String(format!("frontmatter{}", fact.key_path)),
            );
            attributes.insert("key_path".to_owned(), Value::String(fact.key_path.clone()));
            attributes.insert(
                "format".to_owned(),
                Value::String("yaml_frontmatter".to_owned()),
            );
            attributes.insert(
                "namespace".to_owned(),
                Value::String(self.source_file.clone()),
            );
            attributes.insert(
                "source_file".to_owned(),
                Value::String(self.source_file.clone()),
            );
            attributes.insert(
                "source_location".to_owned(),
                Value::String(format!("L{}", self.line_at(fact.start_byte))),
            );
            attributes.insert("_origin".to_owned(), Value::String("config".to_owned()));
            attributes.insert(
                "rule".to_owned(),
                Value::String("markdown-frontmatter-key".to_owned()),
            );
            stamp_source_range_indexed(
                &mut attributes,
                self.source,
                &self.line_starts,
                fact.start_byte,
                fact.end_byte,
            );
            self.seen_nodes.insert(id.clone());
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
            self.add_frontmatter_relation(&parent, &id, fact.start_byte, fact.end_byte);
            ids.insert(fact.key_path, id);
        }
    }

    fn add_frontmatter_relation(&mut self, source: &str, target: &str, start: usize, end: usize) {
        let mut attributes = Map::new();
        attributes.insert("relation".to_owned(), Value::String("contains".to_owned()));
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
            Value::String(format!("L{}", self.line_at(start))),
        );
        attributes.insert("_origin".to_owned(), Value::String("config".to_owned()));
        attributes.insert(
            "rule".to_owned(),
            Value::String("markdown-frontmatter-containment".to_owned()),
        );
        stamp_source_range_indexed(&mut attributes, self.source, &self.line_starts, start, end);
        attributes.insert("weight".to_owned(), json!(1.0));
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn add_diagnostic(&mut self, diagnostic: String) {
        if self.diagnostics.len() < MAX_DIAGNOSTICS {
            self.diagnostics.push(diagnostic);
        }
    }

    fn walk_section(&mut self, node: Node<'_>, inherited_parent: Option<&str>) {
        let stack_len = self.heading_stack.len();
        let mut parent = inherited_parent.map(str::to_owned);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            match child.kind() {
                "atx_heading" | "setext_heading" => {
                    parent = Some(self.emit_heading(child, parent.as_deref()));
                }
                "section" => self.walk_section(child, parent.as_deref()),
                "minus_metadata" | "plus_metadata" => {}
                _ => self.emit_block(child, parent.as_deref()),
            }
        }
        self.heading_stack.truncate(stack_len);
    }

    fn emit_heading(&mut self, node: Node<'_>, inherited_parent: Option<&str>) -> String {
        let level = heading_level(node, self.source);
        while self
            .heading_stack
            .last()
            .is_some_and(|frame| frame.level >= level)
        {
            self.heading_stack.pop();
        }
        let raw_title = heading_title(node, self.source);
        let (title, explicit_id) = split_explicit_heading_id(&raw_title);
        let title = title.trim();
        let qualified_base = self.heading_stack.last().map_or_else(
            || title.to_owned(),
            |parent| format!("{}::{title}", parent.qualified_name),
        );
        let occurrence = self
            .heading_occurrences
            .entry(qualified_base.clone())
            .or_default();
        *occurrence += 1;
        let mut qualified_name = if *occurrence == 1 {
            qualified_base
        } else {
            format!("{qualified_base}#{occurrence}")
        };
        if self.heading_stack.is_empty()
            && self
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == qualified_name)
        {
            qualified_name.push_str("::heading");
        }
        let mut id = crate::make_id(&[&self.stem, &qualified_name]);
        if id == self.file_id {
            id = crate::make_id(&[&self.source_file, "heading", &qualified_name]);
        }
        let parent = self
            .heading_stack
            .last()
            .map(|frame| frame.id.clone())
            .or_else(|| inherited_parent.map(str::to_owned))
            .or_else(|| Some(self.file_id.clone()));
        let mut extra = Map::new();
        extra.insert(
            "qualified_name".to_owned(),
            Value::String(qualified_name.clone()),
        );
        extra.insert("heading_level".to_owned(), json!(level));
        extra.insert(
            "heading_style".to_owned(),
            Value::String(
                if node.kind() == "setext_heading" {
                    "setext"
                } else {
                    "atx"
                }
                .to_owned(),
            ),
        );
        let anchor_slug = self.heading_anchor_slug(title, explicit_id.as_deref());
        extra.insert("anchor_slug".to_owned(), Value::String(anchor_slug.clone()));
        if let Some(explicit_id) = explicit_id.as_deref() {
            extra.insert(
                "explicit_id".to_owned(),
                Value::String(explicit_id.to_owned()),
            );
        }
        let id = self.add_block_node(
            id,
            title,
            "heading",
            node.start_byte()..node.end_byte(),
            parent.as_deref(),
            extra,
        );
        self.register_heading_target(&anchor_slug, explicit_id.as_deref(), &id);
        self.collect_inline_descendants(node, &id);
        self.heading_stack.push(HeadingFrame {
            level,
            id: id.clone(),
            qualified_name,
        });
        id
    }

    fn emit_block(&mut self, node: Node<'_>, parent: Option<&str>) {
        let kind = node.kind();
        match kind {
            "section" => {
                self.walk_section(node, parent);
                return;
            }
            "atx_heading" | "setext_heading" => {
                self.emit_heading(node, parent);
                return;
            }
            "pipe_table" => {
                self.emit_pipe_table(node, parent);
                return;
            }
            "inline" | "block_continuation" | "minus_metadata" | "plus_metadata" => return,
            _ => {}
        }
        if self.next_block_index > MAX_BLOCKS {
            self.add_diagnostic("Markdown block limit exceeded".to_owned());
            return;
        }
        let text = node_text(self.source, node.start_byte(), node.end_byte());
        let label = block_label(kind, &text);
        let id = crate::make_id(&[
            &self.source_file,
            "markdown_block",
            kind,
            &self.next_block_index.to_string(),
            &node.start_byte().to_string(),
        ]);
        let mut extra = Map::new();
        extra.insert("block_index".to_owned(), json!(self.next_block_index));
        if (kind == "fenced_code_block" || kind == "indented_code_block")
            && let Some(info) = child_text(node, self.source, "info_string")
        {
            let info = compact_label(&info);
            if !info.is_empty() {
                extra.insert("language".to_owned(), Value::String(info));
            }
        }
        if kind == "list_item" {
            let checked = has_child_kind(node, "task_list_marker_checked");
            let unchecked = has_child_kind(node, "task_list_marker_unchecked");
            if checked || unchecked {
                extra.insert("task".to_owned(), Value::Bool(true));
                extra.insert("task_checked".to_owned(), Value::Bool(checked));
            }
        }
        if let Some(section) = self.heading_stack.last() {
            extra.insert(
                "document_section".to_owned(),
                Value::String(section.qualified_name.clone()),
            );
            extra.insert(
                "qualified_name".to_owned(),
                Value::String(format!(
                    "{}::{kind}#{}",
                    section.qualified_name, self.next_block_index
                )),
            );
        } else {
            extra.insert(
                "qualified_name".to_owned(),
                Value::String(format!("{}::{kind}#{}", self.stem, self.next_block_index)),
            );
        }
        if !matches!(kind, "table" | "table_row") {
            let content = truncate_utf8(&normalize_table_text(&text), MAX_TABLE_CELL_BYTES);
            if !content.is_empty() {
                extra.insert("document_content".to_owned(), Value::String(content));
            }
        }
        let id = self.add_block_node(
            id,
            &label,
            kind,
            node.start_byte()..node.end_byte(),
            parent,
            extra,
        );
        if kind == "link_reference_definition" {
            self.record_reference_definition(node, &id);
        }
        self.collect_inline_descendants(node, &id);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            if !is_nested_block(child.kind()) {
                continue;
            }
            self.emit_block(child, Some(&id));
        }
    }

    fn emit_pipe_table(&mut self, node: Node<'_>, parent: Option<&str>) {
        if self.next_block_index > MAX_BLOCKS {
            self.add_table_limit_diagnostic("blocks");
            return;
        }

        let (header_node, delimiter_node, rows) = table_children(node);
        let headers = header_node
            .map(|header| table_cells(header, self.source))
            .unwrap_or_default();
        let delimiter_alignments = delimiter_node
            .map(|delimiter| table_alignments(delimiter, self.source))
            .unwrap_or_default();
        let row_facts = rows
            .iter()
            .map(|row| table_cells(*row, self.source))
            .collect::<Vec<_>>();

        let source_section = self
            .heading_stack
            .last()
            .map(|heading| heading.qualified_name.clone())
            .unwrap_or_else(|| self.stem.clone());
        let header_signature = headers
            .iter()
            .take(MAX_TABLE_COLUMNS)
            .map(|cell| truncate_utf8(&normalize_table_text(&cell.text), MAX_TABLE_CELL_BYTES))
            .collect::<Vec<_>>()
            .join("\u{1f}");
        let table_key = format!("{source_section}\u{1e}{header_signature}");
        let occurrence = self.table_occurrences.entry(table_key).or_default();
        *occurrence = occurrence.saturating_add(1);
        let table_ordinal = *occurrence;
        let table_qualified_name = format!("{source_section}::pipe_table#{table_ordinal}");
        let table_id = crate::make_id(&[
            &self.source_file,
            "markdown_table",
            &source_section,
            &header_signature,
            &table_ordinal.to_string(),
        ]);

        let column_count = headers
            .len()
            .max(row_facts.iter().map(Vec::len).max().unwrap_or(0));
        let retained_columns = column_count.min(MAX_TABLE_COLUMNS);
        let omitted_columns = column_count.saturating_sub(retained_columns);
        if omitted_columns > 0 {
            self.add_table_limit_diagnostic("columns");
        }
        let mut retained_headers = Vec::with_capacity(retained_columns);
        let mut header_truncated = false;
        for cell in headers.iter().take(retained_columns) {
            let mut header = normalize_table_text(&cell.text);
            if header.len() > MAX_TABLE_CELL_BYTES {
                header = truncate_utf8(&header, MAX_TABLE_CELL_BYTES);
                header_truncated = true;
                self.add_table_limit_diagnostic("cell_text");
            }
            if self.table_text_bytes.saturating_add(header.len()) > MAX_TABLE_TEXT_BYTES {
                header.clear();
                header_truncated = true;
                self.add_table_limit_diagnostic("text");
            } else {
                self.table_text_bytes = self.table_text_bytes.saturating_add(header.len());
            }
            retained_headers.push(header);
        }
        let table_initially_truncated = header_truncated || omitted_columns > 0;
        let table_label = table_label(&source_section, &retained_headers);
        let mut table_extra = Map::new();
        table_extra.insert(
            "qualified_name".to_owned(),
            Value::String(table_qualified_name.clone()),
        );
        table_extra.insert("table_role".to_owned(), Value::String("table".to_owned()));
        table_extra.insert(
            "table_headers".to_owned(),
            Value::Array(
                retained_headers
                    .iter()
                    .map(|header| Value::String(header.clone()))
                    .collect(),
            ),
        );
        table_extra.insert(
            "table_alignments".to_owned(),
            Value::Array(
                (0..retained_columns)
                    .map(|index| {
                        Value::String(
                            delimiter_alignments
                                .get(index)
                                .copied()
                                .unwrap_or(TableAlignment::Unspecified)
                                .as_str()
                                .to_owned(),
                        )
                    })
                    .collect(),
            ),
        );
        table_extra.insert(
            "table_columns".to_owned(),
            Value::Array(
                retained_headers
                    .iter()
                    .enumerate()
                    .map(|(index, header)| {
                        let mut column = Map::new();
                        column.insert("index".to_owned(), json!(index));
                        column.insert("header".to_owned(), Value::String(header.clone()));
                        column.insert(
                            "alignment".to_owned(),
                            Value::String(
                                delimiter_alignments
                                    .get(index)
                                    .copied()
                                    .unwrap_or(TableAlignment::Unspecified)
                                    .as_str()
                                    .to_owned(),
                            ),
                        );
                        if let Some(cell) = headers.get(index) {
                            column.insert(
                                "source".to_owned(),
                                source_anchor_json(
                                    &self.source_file,
                                    self.source,
                                    &self.line_starts,
                                    cell.raw_start,
                                    cell.raw_end,
                                ),
                            );
                        }
                        Value::Object(column)
                    })
                    .collect(),
            ),
        );
        table_extra.insert(
            "table_body_row_count".to_owned(),
            json!(rows.len().min(MAX_TABLE_ROWS)),
        );
        table_extra.insert("table_omitted_row_count".to_owned(), json!(0));
        table_extra.insert(
            "table_omitted_column_count".to_owned(),
            json!(omitted_columns),
        );
        table_extra.insert(
            "table_truncated".to_owned(),
            Value::Bool(table_initially_truncated),
        );
        let table_id = self.add_block_node(
            table_id,
            &table_label,
            "pipe_table",
            node.start_byte()..node.end_byte(),
            parent,
            table_extra,
        );

        // Keep the complete parser-backed table hierarchy in graph/1, but
        // give each record bounded semantic labels and stable structural
        // identities. Consumers can navigate exact header/cell evidence while
        // architecture analysis treats these containment edges as zero-weight.
        let mut table_node_count = 1usize;
        let mut table_cell_count = 0usize;
        let mut table_text_bytes = retained_headers.iter().map(String::len).sum::<usize>();
        if let Some(header) = header_node
            && self.next_block_index <= MAX_BLOCKS
            && table_node_count < MAX_TABLE_NODES_PER_TABLE
        {
            let header_cells = table_cells(header, self.source);
            let header_qualified_name = format!("{table_qualified_name}::pipe_table_header#1");
            let header_id = crate::make_id(&[&table_id, "markdown_table_header"]);
            let mut header_extra = Map::new();
            header_extra.insert(
                "qualified_name".to_owned(),
                Value::String(header_qualified_name.clone()),
            );
            header_extra.insert("table_role".to_owned(), Value::String("header".to_owned()));
            let header_content = retained_headers.join(" | ");
            if !header_content.is_empty() {
                header_extra.insert(
                    "document_content".to_owned(),
                    Value::String(header_content.clone()),
                );
            }
            let header_id = self.add_block_node(
                header_id,
                &bounded_label(&format!("header: {header_content}")),
                "pipe_table_header",
                header.start_byte()..header.end_byte(),
                Some(&table_id),
                header_extra,
            );
            table_node_count = table_node_count.saturating_add(1);

            for (column_index, cell) in header_cells.iter().take(retained_columns).enumerate() {
                if self.next_block_index > MAX_BLOCKS
                    || table_node_count >= MAX_TABLE_NODES_PER_TABLE
                    || table_cell_count >= MAX_TABLE_CELLS_PER_TABLE
                    || self.table_cells_retained >= MAX_TABLE_CELLS
                {
                    header_truncated = true;
                    self.add_table_limit_diagnostic("cells");
                    break;
                }
                let text = retained_headers
                    .get(column_index)
                    .cloned()
                    .unwrap_or_default();
                let cell_id = crate::make_id(&[
                    &table_id,
                    "markdown_table_header_cell",
                    &column_index.to_string(),
                ]);
                let mut cell_extra = Map::new();
                cell_extra.insert(
                    "qualified_name".to_owned(),
                    Value::String(format!(
                        "{header_qualified_name}::pipe_table_cell#{}",
                        column_index.saturating_add(1)
                    )),
                );
                cell_extra.insert(
                    "table_role".to_owned(),
                    Value::String("header_cell".to_owned()),
                );
                cell_extra.insert("table_column_index".to_owned(), json!(column_index));
                cell_extra.insert(
                    "table_alignment".to_owned(),
                    Value::String(
                        delimiter_alignments
                            .get(column_index)
                            .copied()
                            .unwrap_or(TableAlignment::Unspecified)
                            .as_str()
                            .to_owned(),
                    ),
                );
                cell_extra.insert(
                    "table_cell_state".to_owned(),
                    Value::String(if text.is_empty() { "empty" } else { "present" }.to_owned()),
                );
                if !text.is_empty() {
                    cell_extra.insert("document_content".to_owned(), Value::String(text.clone()));
                }
                let cell_id = self.add_block_node(
                    cell_id,
                    &table_cell_label(None, &text, column_index),
                    "pipe_table_cell",
                    cell.raw_start..cell.raw_end,
                    Some(&header_id),
                    cell_extra,
                );
                self.collect_inline_text_range(cell.raw_start, cell.raw_end, &cell_id);
                table_node_count = table_node_count.saturating_add(1);
                table_cell_count = table_cell_count.saturating_add(1);
                self.table_cells_retained = self.table_cells_retained.saturating_add(1);
            }
        }

        let mut row_occurrences = HashMap::<String, usize>::new();
        let mut retained_rows = 0usize;
        let mut table_truncated = table_initially_truncated || header_truncated;
        for (row_index, row) in rows.iter().enumerate() {
            if row_index >= MAX_TABLE_ROWS {
                table_truncated = true;
                self.add_table_limit_diagnostic("rows");
                break;
            }
            if self.next_block_index > MAX_BLOCKS {
                table_truncated = true;
                self.add_table_limit_diagnostic("blocks");
                break;
            }
            if table_node_count >= MAX_TABLE_NODES_PER_TABLE {
                table_truncated = true;
                self.add_table_limit_diagnostic("per_table_nodes");
                break;
            }
            if table_cell_count >= MAX_TABLE_CELLS_PER_TABLE
                || self.table_cells_retained >= MAX_TABLE_CELLS
            {
                table_truncated = true;
                self.add_table_limit_diagnostic("cells");
                break;
            }
            let cells = table_cells(*row, self.source);
            let normalized_cells = cells
                .iter()
                .take(retained_columns)
                .map(|cell| truncate_utf8(&normalize_table_text(&cell.text), MAX_TABLE_CELL_BYTES))
                .collect::<Vec<_>>();
            let identity = normalized_cells
                .iter()
                .enumerate()
                .find(|(_, value)| !value.is_empty())
                .map(|(index, value)| (index, value.clone()));
            let identity_key = identity.as_ref().map_or_else(
                || format!("ordinal:{row_index}"),
                |(index, value)| format!("column:{index}:{value}"),
            );
            let identity_occurrence = row_occurrences.entry(identity_key.clone()).or_default();
            *identity_occurrence = identity_occurrence.saturating_add(1);
            let row_qualified_name = format!(
                "{table_qualified_name}::pipe_table_row#{}-{}",
                compact_identity(&identity_key),
                *identity_occurrence
            );
            let row_id = crate::make_id(&[
                &table_id,
                "markdown_table_row",
                &identity_key,
                &identity_occurrence.to_string(),
            ]);
            let mut row_extra = Map::new();
            row_extra.insert(
                "qualified_name".to_owned(),
                Value::String(row_qualified_name.clone()),
            );
            row_extra.insert("table_role".to_owned(), Value::String("row".to_owned()));
            row_extra.insert("table_row_index".to_owned(), json!(row_index));
            if let Some((index, _)) = identity.as_ref() {
                row_extra.insert("table_identity_cell_index".to_owned(), json!(*index));
            }
            let mut row_truncated = omitted_columns > 0;
            let mut serialized_cells = Vec::with_capacity(retained_columns);
            let mut emitted_cells = Vec::<(usize, String, &'static str, usize, usize)>::new();
            for column_index in 0..retained_columns {
                if self.next_block_index.saturating_add(emitted_cells.len()) > MAX_BLOCKS
                    || table_node_count
                        .saturating_add(1)
                        .saturating_add(emitted_cells.len())
                        >= MAX_TABLE_NODES_PER_TABLE
                    || table_cell_count.saturating_add(emitted_cells.len())
                        >= MAX_TABLE_CELLS_PER_TABLE
                    || self
                        .table_cells_retained
                        .saturating_add(emitted_cells.len())
                        >= MAX_TABLE_CELLS
                {
                    row_truncated = true;
                    table_truncated = true;
                    self.add_table_limit_diagnostic("cells");
                    break;
                }
                let Some(cell) = cells.get(column_index) else {
                    serialized_cells.push(json!({
                        "columnIndex": column_index,
                        "state": "missing",
                        "text": ""
                    }));
                    continue;
                };
                let mut text = normalize_table_text(&cell.text);
                let mut state = if text.is_empty() { "empty" } else { "present" };
                if text.len() > MAX_TABLE_CELL_BYTES {
                    text = truncate_utf8(&text, MAX_TABLE_CELL_BYTES);
                    row_truncated = true;
                    self.add_table_limit_diagnostic("cell_text");
                }
                let text_bytes = text.len();
                if self.table_text_bytes.saturating_add(text_bytes) > MAX_TABLE_TEXT_BYTES
                    || table_text_bytes.saturating_add(text_bytes) > MAX_TABLE_TEXT_BYTES_PER_TABLE
                {
                    text.clear();
                    state = "limited";
                    row_truncated = true;
                    table_truncated = true;
                    self.add_table_limit_diagnostic("text");
                } else {
                    self.table_text_bytes = self.table_text_bytes.saturating_add(text_bytes);
                    table_text_bytes = table_text_bytes.saturating_add(text_bytes);
                }
                serialized_cells.push(json!({
                    "columnIndex": column_index,
                    "state": state,
                    "text": text,
                    "source": source_anchor_json(
                        &self.source_file,
                        self.source,
                        &self.line_starts,
                        cell.raw_start,
                        cell.raw_end,
                    )
                }));
                emitted_cells.push((column_index, text, state, cell.raw_start, cell.raw_end));
            }
            if cells.len() > retained_columns {
                row_truncated = true;
            }
            row_extra.insert("table_cells".to_owned(), Value::Array(serialized_cells));
            row_extra.insert("table_truncated".to_owned(), Value::Bool(row_truncated));
            let row_label = row_label(&retained_headers, &normalized_cells);
            let row_id = self.add_block_node(
                row_id,
                &row_label,
                "pipe_table_row",
                row.start_byte()..row.end_byte(),
                Some(&table_id),
                row_extra,
            );
            table_node_count = table_node_count.saturating_add(1);

            for (column_index, text, state, start, end) in emitted_cells {
                let cell_id =
                    crate::make_id(&[&row_id, "markdown_table_cell", &column_index.to_string()]);
                let mut cell_extra = Map::new();
                cell_extra.insert(
                    "qualified_name".to_owned(),
                    Value::String(format!(
                        "{}::pipe_table_cell#{}",
                        row_qualified_name,
                        column_index.saturating_add(1)
                    )),
                );
                cell_extra.insert(
                    "table_role".to_owned(),
                    Value::String("body_cell".to_owned()),
                );
                cell_extra.insert("table_column_index".to_owned(), json!(column_index));
                cell_extra.insert(
                    "table_header".to_owned(),
                    Value::String(
                        retained_headers
                            .get(column_index)
                            .cloned()
                            .unwrap_or_default(),
                    ),
                );
                cell_extra.insert(
                    "table_cell_state".to_owned(),
                    Value::String(state.to_owned()),
                );
                if !text.is_empty() {
                    cell_extra.insert("document_content".to_owned(), Value::String(text.clone()));
                }
                let label = if state == "limited" {
                    table_cell_label(
                        retained_headers.get(column_index).map(String::as_str),
                        "(limited)",
                        column_index,
                    )
                } else {
                    table_cell_label(
                        retained_headers.get(column_index).map(String::as_str),
                        &text,
                        column_index,
                    )
                };
                let cell_id = self.add_block_node(
                    cell_id,
                    &label,
                    "pipe_table_cell",
                    start..end,
                    Some(&row_id),
                    cell_extra,
                );
                if state != "limited" {
                    self.collect_inline_text_range(start, end, &cell_id);
                }
                table_node_count = table_node_count.saturating_add(1);
                table_cell_count = table_cell_count.saturating_add(1);
                self.table_cells_retained = self.table_cells_retained.saturating_add(1);
            }
            retained_rows = retained_rows.saturating_add(1);
        }
        let omitted_rows = rows.len().saturating_sub(retained_rows);
        table_truncated |= omitted_rows > 0;
        self.update_table_metadata(&table_id, retained_rows, omitted_rows, table_truncated);
    }

    fn update_table_metadata(
        &mut self,
        table_id: &str,
        retained_rows: usize,
        omitted_rows: usize,
        truncated: bool,
    ) {
        if let Some(table) = self
            .extraction
            .nodes
            .iter_mut()
            .find(|node| node.id == table_id)
        {
            table
                .attributes
                .insert("table_body_row_count".to_owned(), json!(retained_rows));
            table
                .attributes
                .insert("table_omitted_row_count".to_owned(), json!(omitted_rows));
            table
                .attributes
                .insert("table_truncated".to_owned(), Value::Bool(truncated));
        }
    }

    fn collect_inline_text_range(&mut self, start: usize, end: usize, owner_id: &str) {
        let start = start.min(self.source.len());
        let end = end.clamp(start, self.source.len());
        if start >= end {
            return;
        }
        let inline_source = &self.source[start..end];
        let Some(tree) = self.inline_parser.parse(inline_source, None) else {
            self.add_diagnostic("Markdown inline parser was cancelled".to_owned());
            return;
        };
        self.walk_inline_node(tree.root_node(), start, owner_id);
        self.scan_reference_links(start, inline_source, owner_id);
        self.scan_wikilinks(start, inline_source, owner_id);
        self.scan_inline_code_references(start, inline_source, owner_id);
    }

    fn add_table_limit_diagnostic(&mut self, class: &'static str) {
        if self.table_limit_diagnostics.insert(class) {
            self.add_diagnostic(format!("Markdown table {class} limit exceeded"));
            self.extraction.extensions.insert(
                crate::EXTRACTION_QUALITY_EXTENSION.to_owned(),
                json!(crate::EXTRACTION_QUALITY_PARTIAL),
            );
            self.extraction.extensions.insert(
                crate::EXTRACTION_QUALITY_REASON_EXTENSION.to_owned(),
                json!("markdown_table_limit"),
            );
        }
    }

    fn add_block_node(
        &mut self,
        id: String,
        label: &str,
        kind: &str,
        range: std::ops::Range<usize>,
        parent: Option<&str>,
        mut extra: Map<String, Value>,
    ) -> String {
        if !self.seen_nodes.insert(id.clone()) {
            return id;
        }
        let block_index = self.next_block_index;
        self.next_block_index = self.next_block_index.saturating_add(1);
        extra.insert("label".to_owned(), Value::String(bounded_label(label)));
        extra.insert("file_type".to_owned(), Value::String("document".to_owned()));
        extra.insert(
            "document_format".to_owned(),
            Value::String("markdown".to_owned()),
        );
        extra.insert("document_kind".to_owned(), Value::String(kind.to_owned()));
        extra.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        extra.insert(
            "source_location".to_owned(),
            Value::String(format!("L{}", self.line_at(range.start))),
        );
        extra.insert("block_index".to_owned(), json!(block_index));
        extra.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        stamp_source_range_indexed(
            &mut extra,
            self.source,
            &self.line_starts,
            range.start,
            range.end,
        );
        self.extraction.nodes.push(NodeRecord {
            id: id.clone(),
            attributes: extra,
        });
        if let Some(parent) = parent {
            self.add_relation(parent, &id, "contains", range.start, range.end);
        }
        id
    }

    fn collect_inline_descendants(&mut self, node: Node<'_>, owner_id: &str) {
        if node.kind() == "inline" {
            self.collect_inline_node(node, owner_id);
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            // A nested structural block gets its own owner when `emit_block`
            // visits it. Do not walk through it here, or a link in a list item
            // would be emitted once for every containing list/table/quote.
            if is_structural_block_kind(child.kind()) {
                continue;
            }
            self.collect_inline_descendants(child, owner_id);
        }
    }

    fn collect_inline_node(&mut self, node: Node<'_>, owner_id: &str) {
        let start = node.start_byte();
        let end = node.end_byte().min(self.source.len());
        if start >= end || end > self.source.len() {
            return;
        }
        let inline_source = &self.source[start..end];
        let Some(tree) = self.inline_parser.parse(inline_source, None) else {
            self.add_diagnostic("Markdown inline parser was cancelled".to_owned());
            return;
        };
        self.walk_inline_node(tree.root_node(), start, owner_id);
        self.scan_reference_links(start, inline_source, owner_id);
        self.scan_wikilinks(start, inline_source, owner_id);
        self.scan_inline_code_references(start, inline_source, owner_id);
    }

    /// Retain only explicitly delimited code spans that have a shape useful
    /// to a deterministic repository resolver. Ordinary prose remains prose;
    /// a backtick span is evidence of intentional reference syntax, but it is
    /// still resolved fail-closed later by the graph publisher.
    fn scan_inline_code_references(&mut self, base: usize, source: &[u8], owner_id: &str) {
        let mut offset = 0usize;
        while offset < source.len() {
            if source[offset] != b'`' {
                offset = offset.saturating_add(1);
                continue;
            }
            let mut run = 1usize;
            while offset.saturating_add(run) < source.len() && source[offset + run] == b'`' {
                run = run.saturating_add(1);
            }
            let body_start = offset.saturating_add(run);
            let mut search = body_start;
            let mut close = None;
            while search < source.len() {
                if source[search] != b'`' {
                    search = search.saturating_add(1);
                    continue;
                }
                let mut close_run = 1usize;
                while search.saturating_add(close_run) < source.len()
                    && source[search + close_run] == b'`'
                {
                    close_run = close_run.saturating_add(1);
                }
                if close_run == run {
                    close = Some(search);
                    break;
                }
                search = search.saturating_add(close_run);
            }
            let Some(close) = close else {
                break;
            };
            let absolute_start = base.saturating_add(offset);
            let absolute_end = base
                .saturating_add(close.saturating_add(run))
                .min(self.source.len());
            let body = String::from_utf8_lossy(&source[body_start..close]);
            let spelling = body.trim();
            if is_inline_reference_candidate(spelling) {
                self.record_document_reference(
                    owner_id,
                    spelling,
                    "inline_code",
                    self.link_site(absolute_start, absolute_end),
                    "unresolved",
                    None,
                    Vec::new(),
                );
            }
            offset = close.saturating_add(run);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record_document_reference(
        &mut self,
        owner_id: &str,
        spelling: &str,
        kind: &str,
        site: LinkSite,
        resolution: &str,
        target: Option<&str>,
        mut candidates: Vec<Value>,
    ) {
        let spelling = truncate_utf8(spelling.trim(), MAX_REFERENCE_SPELLING_BYTES);
        if spelling.is_empty() {
            return;
        }
        let references = self
            .document_references
            .entry(owner_id.to_owned())
            .or_default();
        if references.len() >= MAX_DOCUMENT_REFERENCES {
            if !self.document_reference_limit_reported {
                self.document_reference_limit_reported = true;
                self.add_diagnostic("Markdown document reference limit exceeded".to_owned());
                self.extraction.extensions.insert(
                    crate::EXTRACTION_QUALITY_EXTENSION.to_owned(),
                    json!(crate::EXTRACTION_QUALITY_PARTIAL),
                );
                self.extraction.extensions.insert(
                    crate::EXTRACTION_QUALITY_REASON_EXTENSION.to_owned(),
                    json!("markdown_reference_limit"),
                );
            }
            return;
        }
        let duplicate = references.iter().any(|value| {
            value.as_object().is_some_and(|object| {
                object.get("kind").and_then(Value::as_str) == Some(kind)
                    && object.get("spelling").and_then(Value::as_str) == Some(&spelling)
                    && object
                        .get("site")
                        .and_then(Value::as_object)
                        .and_then(|site| site.get("startByte"))
                        .and_then(Value::as_u64)
                        == Some(site.start_byte as u64)
            })
        });
        if duplicate {
            return;
        }
        candidates.sort_by_cached_key(|value| {
            let object = value.as_object();
            (
                object
                    .and_then(|object| object.get("nodeId"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                object
                    .and_then(|object| object.get("reason"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                object
                    .and_then(|object| object.get("confidence"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            )
        });
        candidates.dedup();
        let mut reference = Map::new();
        reference.insert("spelling".to_owned(), Value::String(spelling));
        reference.insert("kind".to_owned(), Value::String(kind.to_owned()));
        reference.insert(
            "site".to_owned(),
            source_anchor_json(
                &self.source_file,
                self.source,
                &self.line_starts,
                site.start_byte,
                site.end_byte,
            ),
        );
        reference.insert(
            "resolution".to_owned(),
            Value::String(resolution.to_owned()),
        );
        if let Some(target) = target.filter(|target| !target.is_empty()) {
            reference.insert("target".to_owned(), Value::String(target.to_owned()));
        }
        if !candidates.is_empty() {
            reference.insert("candidates".to_owned(), Value::Array(candidates));
        }
        references.push(Value::Object(reference));
    }

    fn publish_document_references(&mut self) {
        for node in &mut self.extraction.nodes {
            let Some(mut references) = self.document_references.remove(&node.id) else {
                continue;
            };
            references.sort_by_cached_key(|value| {
                let object = value.as_object();
                (
                    object
                        .and_then(|object| object.get("site"))
                        .and_then(Value::as_object)
                        .and_then(|site| site.get("startByte"))
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                    object
                        .and_then(|object| object.get("kind"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    object
                        .and_then(|object| object.get("spelling"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                )
            });
            node.attributes
                .insert("document_references".to_owned(), Value::Array(references));
        }
    }

    fn walk_inline_node(&mut self, node: Node<'_>, base: usize, owner_id: &str) {
        let kind = node.kind();
        if kind == "image" {
            return;
        }
        let link = match kind {
            "inline_link" => child_text_at(node, self.source, base, "link_destination")
                .map(|raw| (raw, None, "inline")),
            "full_reference_link" => {
                child_text_at(node, self.source, base, "link_label").map(|label| {
                    (
                        String::new(),
                        Some(normalize_reference_label(&label)),
                        "reference",
                    )
                })
            }
            "collapsed_reference_link" | "shortcut_link" => {
                child_text_at(node, self.source, base, "link_text").map(|label| {
                    (
                        String::new(),
                        Some(normalize_reference_label(&label)),
                        "reference",
                    )
                })
            }
            "uri_autolink" | "email_autolink" => Some((
                node_text(
                    self.source,
                    base + node.start_byte(),
                    base + node.end_byte(),
                ),
                None,
                "autolink",
            )),
            _ => None,
        };
        if let Some((raw, reference_label, link_kind)) = link {
            let absolute_start = base.saturating_add(node.start_byte());
            if link_kind == "reference" && self.is_reference_definition_at(absolute_start) {
                return;
            }
            if self.pending_links.len() >= MAX_LINKS {
                self.add_diagnostic("Markdown link limit exceeded".to_owned());
                return;
            }
            let start = absolute_start;
            let end = base.saturating_add(node.end_byte()).min(self.source.len());
            self.pending_links.push(PendingLink {
                raw,
                reference_label,
                owner_id: owner_id.to_owned(),
                site: self.link_site(start, end),
                kind: link_kind,
            });
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            self.walk_inline_node(child, base, owner_id);
        }
    }

    fn is_reference_definition_at(&self, offset: usize) -> bool {
        let line_start = self.line_start(offset);
        let line_end = next_line(self.source, line_start).map_or(self.source.len(), |(_, line)| {
            line_start.saturating_add(line.len())
        });
        parse_reference_definition(&self.source[line_start..line_end]).is_some()
    }

    fn scan_wikilinks(&mut self, base: usize, source: &[u8], owner_id: &str) {
        let mut offset = 0;
        while let Some(relative_start) = source[offset..].windows(2).position(|pair| pair == b"[[")
        {
            let start = offset + relative_start;
            if start > 0 && source[start - 1] == b'!' {
                offset = start.saturating_add(2);
                continue;
            }
            let Some(relative_end) = source[start + 2..]
                .windows(2)
                .position(|pair| pair == b"]]")
            else {
                break;
            };
            let end = start + 2 + relative_end + 2;
            let body = String::from_utf8_lossy(&source[start + 2..end - 2]);
            let target = body
                .split_once('|')
                .map_or(body.as_ref(), |(target, _)| target)
                .trim();
            if !target.is_empty() && self.pending_links.len() < MAX_LINKS {
                let absolute_start = base.saturating_add(start);
                let absolute_end = base.saturating_add(end).min(self.source.len());
                self.pending_links.push(PendingLink {
                    raw: target.to_owned(),
                    reference_label: None,
                    owner_id: owner_id.to_owned(),
                    site: self.link_site(absolute_start, absolute_end),
                    kind: "wikilink",
                });
            }
            offset = end;
        }
    }

    fn scan_reference_links(&mut self, base: usize, source: &[u8], owner_id: &str) {
        let mut offset = 0;
        while let Some(relative_start) = source[offset..].iter().position(|byte| *byte == b'[') {
            let start = offset.saturating_add(relative_start);
            if start > 0 && source[start - 1] == b'!' {
                offset = start.saturating_add(1);
                continue;
            }
            let Some(relative_text_end) = source[start + 1..].iter().position(|byte| *byte == b']')
            else {
                break;
            };
            let text_end = start.saturating_add(1).saturating_add(relative_text_end);
            let label_start = text_end.saturating_add(1);
            if source.get(label_start) != Some(&b'[') {
                offset = label_start;
                continue;
            }
            let Some(relative_label_end) = source[label_start + 1..]
                .iter()
                .position(|byte| *byte == b']')
            else {
                break;
            };
            let label_end = label_start
                .saturating_add(1)
                .saturating_add(relative_label_end);
            let text = String::from_utf8_lossy(&source[start + 1..text_end]);
            let label = String::from_utf8_lossy(&source[label_start + 1..label_end]);
            let label = if label.trim().is_empty() {
                text.to_string()
            } else {
                label.to_string()
            };
            let absolute_start = base.saturating_add(start);
            let absolute_end = base
                .saturating_add(label_end.saturating_add(1))
                .min(self.source.len());
            if !label.trim().is_empty()
                && !self.has_pending_site(absolute_start, absolute_end)
                && self.pending_links.len() < MAX_LINKS
            {
                self.pending_links.push(PendingLink {
                    raw: String::new(),
                    reference_label: Some(normalize_reference_label(&label)),
                    owner_id: owner_id.to_owned(),
                    site: self.link_site(absolute_start, absolute_end),
                    kind: "reference",
                });
            }
            offset = label_end.saturating_add(1);
        }
    }

    fn has_pending_site(&self, start: usize, end: usize) -> bool {
        self.pending_links
            .iter()
            .any(|pending| pending.site.start_byte == start && pending.site.end_byte == end)
    }

    fn record_reference_definition(&mut self, node: Node<'_>, owner_id: &str) {
        let Some(label) = child_text(node, self.source, "link_label") else {
            return;
        };
        let Some(target) = child_text(node, self.source, "link_destination") else {
            return;
        };
        let label = normalize_reference_label(&label);
        if label.is_empty() {
            return;
        }
        self.reference_definitions.insert(label, target.clone());
        if self.pending_links.len() < MAX_LINKS {
            self.pending_links.push(PendingLink {
                raw: target,
                reference_label: None,
                owner_id: owner_id.to_owned(),
                site: self.link_site(node.start_byte(), node.end_byte()),
                kind: "reference_definition",
            });
        }
    }

    fn scan_reference_definitions(&mut self) {
        let mut offset = 0;
        let mut fence = None;
        while let Some((line_end, line)) = next_line(self.source, offset) {
            if let Some(open) = fence {
                if closes_fence(line, open) {
                    fence = None;
                }
                offset = line_end;
                continue;
            }
            if let Some(open) = opens_fence(line) {
                fence = Some(open);
                offset = line_end;
                continue;
            }
            if let Some((label, target, relative_start, relative_end)) =
                parse_reference_definition(line)
            {
                let label = normalize_reference_label(&label);
                if !label.is_empty() && !self.reference_definitions.contains_key(&label) {
                    self.reference_definitions.insert(label, target.clone());
                    if self.pending_links.len() < MAX_LINKS {
                        let start = offset.saturating_add(relative_start);
                        let end = offset.saturating_add(relative_end).min(self.source.len());
                        let owner = self.owner_for_offset(start);
                        self.pending_links.push(PendingLink {
                            raw: target,
                            reference_label: None,
                            owner_id: owner,
                            site: self.link_site(start, end),
                            kind: "reference_definition",
                        });
                    }
                }
            }
            offset = line_end;
        }
    }

    fn scan_footnotes(&mut self) {
        let mut offset = 0;
        let mut fence = None;
        while let Some((line_end, line)) = next_line(self.source, offset) {
            if let Some(open) = fence {
                if closes_fence(line, open) {
                    fence = None;
                }
                offset = line_end;
                continue;
            }
            if let Some(open) = opens_fence(line) {
                fence = Some(open);
                offset = line_end;
                continue;
            }
            if let Some((label, relative_start, relative_end)) = parse_footnote_definition(line)
                && self
                    .footnote_definitions
                    .values()
                    .map(Vec::len)
                    .sum::<usize>()
                    < MAX_LINKS
            {
                let start = offset.saturating_add(relative_start);
                let end = offset.saturating_add(relative_end).min(self.source.len());
                let id = crate::make_id(&[
                    &self.source_file,
                    "markdown_footnote",
                    &label,
                    &start.to_string(),
                ]);
                let parent = self.owner_for_offset(start);
                let mut extra = Map::new();
                extra.insert("footnote_label".to_owned(), Value::String(label.clone()));
                extra.insert(
                    "footnote_role".to_owned(),
                    Value::String("definition".to_owned()),
                );
                self.add_block_node(
                    id.clone(),
                    &format!("Footnote {label}"),
                    "footnote_definition",
                    start..end,
                    Some(&parent),
                    extra,
                );
                self.footnote_definitions.entry(label).or_default().push(id);
            }
            self.scan_footnote_references_in_line(offset, line);
            offset = line_end;
        }
    }

    fn scan_footnote_references_in_line(&mut self, offset: usize, line: &[u8]) {
        let mut cursor = 0;
        while let Some(relative_start) = line[cursor..].windows(2).position(|pair| pair == b"[^") {
            let start = cursor.saturating_add(relative_start);
            let Some(relative_end) = line[start.saturating_add(2)..]
                .iter()
                .position(|byte| *byte == b']')
            else {
                break;
            };
            let end = start
                .saturating_add(2)
                .saturating_add(relative_end)
                .saturating_add(1);
            if line.get(end) == Some(&b':') {
                cursor = end.saturating_add(1);
                continue;
            }
            let label =
                String::from_utf8_lossy(&line[start.saturating_add(2)..end.saturating_sub(1)])
                    .trim()
                    .to_owned();
            if !label.is_empty() && label.len() <= MAX_LABEL_CHARS {
                let absolute_start = offset.saturating_add(start);
                let absolute_end = offset.saturating_add(end).min(self.source.len());
                if self.pending_links.len() < MAX_LINKS {
                    self.pending_links.push(PendingLink {
                        raw: label,
                        reference_label: None,
                        owner_id: self.owner_for_offset(absolute_start),
                        site: self.link_site(absolute_start, absolute_end),
                        kind: "footnote",
                    });
                }
            }
            cursor = end;
        }
    }

    fn scan_other_constructs(&mut self) {
        let extension = self
            .path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if extension != "mdx" && extension != "qmd" {
            return;
        }
        let mut offset = 0;
        let mut fence = None;
        while let Some((line_end, line)) = next_line(self.source, offset) {
            if let Some(open) = fence {
                if closes_fence(line, open) {
                    fence = None;
                }
                offset = line_end;
                continue;
            }
            if let Some(open) = opens_fence(line) {
                fence = Some(open);
                offset = line_end;
                continue;
            }
            let trimmed = line.trim_ascii();
            let (other_kind, should_emit) = if extension == "qmd" {
                (
                    "quarto_directive",
                    trimmed.starts_with(b":::") || trimmed.starts_with(b"{{<"),
                )
            } else {
                (
                    "mdx_construct",
                    trimmed.starts_with(b"import ")
                        || trimmed.starts_with(b"export ")
                        || trimmed.starts_with(b"{")
                        || (trimmed.starts_with(b"<")
                            && trimmed.get(1).is_some_and(u8::is_ascii_uppercase)),
                )
            };
            if should_emit && self.other_count < MAX_DIAGNOSTICS {
                let start = offset;
                let end = offset.saturating_add(line.len()).min(self.source.len());
                let id = crate::make_id(&[
                    &self.source_file,
                    "markdown_other",
                    other_kind,
                    &start.to_string(),
                ]);
                let parent = self.owner_for_offset(start);
                let mut extra = Map::new();
                extra.insert(
                    "other_kind".to_owned(),
                    Value::String(other_kind.to_owned()),
                );
                extra.insert("source_syntax".to_owned(), Value::String(extension.clone()));
                self.add_block_node(
                    id,
                    &compact_label(&String::from_utf8_lossy(line)),
                    "other",
                    start..end,
                    Some(&parent),
                    extra,
                );
                self.other_count = self.other_count.saturating_add(1);
            }
            offset = line_end;
        }
    }

    fn owner_for_offset(&self, offset: usize) -> String {
        let mut owner = self.file_id.clone();
        let mut smallest_span = usize::MAX;
        for node in &self.extraction.nodes {
            if node.id == self.file_id {
                continue;
            }
            let Some(start) = node
                .attributes
                .get("start_byte")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
            else {
                continue;
            };
            let Some(end) = node
                .attributes
                .get("end_byte")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
            else {
                continue;
            };
            if start <= offset && offset < end {
                let span = end.saturating_sub(start);
                if span < smallest_span {
                    smallest_span = span;
                    owner = node.id.clone();
                }
            }
        }
        owner
    }

    fn heading_anchor_slug(&mut self, title: &str, explicit_id: Option<&str>) -> String {
        let base = slugify(explicit_id.unwrap_or(title));
        if explicit_id.is_some() || base.is_empty() {
            return base;
        }
        let occurrence = self
            .heading_slug_occurrences
            .entry(base.clone())
            .or_default();
        let slug = if *occurrence == 0 {
            base
        } else {
            format!("{base}-{occurrence}")
        };
        *occurrence = occurrence.saturating_add(1);
        slug
    }

    fn register_heading_target(&mut self, anchor_slug: &str, explicit_id: Option<&str>, id: &str) {
        let mut keys = vec![anchor_slug.to_ascii_lowercase()];
        if let Some(explicit_id) = explicit_id {
            keys.push(explicit_id.to_ascii_lowercase());
            keys.push(slugify(explicit_id));
        }
        for key in keys.into_iter().filter(|key| !key.is_empty()) {
            let targets = self.heading_targets.entry(key).or_default();
            if !targets.contains(&id.to_owned()) {
                targets.push(id.to_owned());
            }
        }
    }

    fn finalize_links(&mut self) {
        let pending_links = std::mem::take(&mut self.pending_links);
        for pending in pending_links {
            if pending.kind == "footnote" {
                match self.footnote_definitions.get(&pending.raw) {
                    Some(candidates) if candidates.len() == 1 => {
                        if let Some(target) = candidates.first() {
                            self.add_link_edge(
                                &pending,
                                target.clone(),
                                Some(("references", Some(&pending.raw))),
                            );
                        }
                    }
                    Some(candidates) => self.add_unresolved_with_candidates(
                        &pending,
                        "ambiguous_footnote",
                        &pending.raw,
                        candidates
                            .iter()
                            .map(|target| {
                                json!({
                                    "nodeId": target,
                                    "reason": "multiple footnote definitions",
                                    "confidence": "ambiguous"
                                })
                            })
                            .collect(),
                    ),
                    None => {
                        self.add_unresolved(&pending, "missing_footnote_definition", &pending.raw)
                    }
                }
                continue;
            }
            let raw = pending
                .reference_label
                .as_ref()
                .and_then(|label| self.reference_definitions.get(label))
                .cloned()
                .or_else(|| pending.reference_label.as_ref().map(|_| String::new()))
                .unwrap_or(pending.raw.clone());
            if raw.is_empty() {
                self.add_unresolved(
                    &pending,
                    "missing_reference_definition",
                    pending.reference_label.as_deref().unwrap_or_default(),
                );
                continue;
            }
            let raw = raw.trim().trim_matches('<').trim_matches('>');
            if is_external_target(raw) {
                self.external_links.push(json!({
                    "source": pending.owner_id,
                    "target": raw,
                    "kind": pending.kind,
                    "source_file": self.source_file,
                    "start_byte": pending.site.start_byte,
                    "end_byte": pending.site.end_byte,
                    "start_line": pending.site.line,
                    "end_line": self.line_at(pending.site.end_byte),
                    "line_start": pending.site.line,
                    "line_end": self.line_at(pending.site.end_byte),
                    "column_start": pending.site.start_byte.saturating_sub(pending.site.line_start),
                    "column_end": pending.site.end_byte.saturating_sub(self.line_start(pending.site.end_byte)),
                }));
                self.record_document_reference(
                    &pending.owner_id,
                    raw,
                    pending.kind,
                    pending.site,
                    "unresolved",
                    None,
                    Vec::new(),
                );
                continue;
            }
            let (path_part, fragment) =
                raw.split_once('#').map_or((raw, None), |(path, fragment)| {
                    (path, Some(fragment.trim()))
                });
            let path_part = path_part
                .split_once('?')
                .map_or(path_part, |(path, _)| path)
                .trim();
            let root_relative = path_part.starts_with('/');
            let unresolved_path = if path_part.is_empty() {
                lexical_normalize(self.path)
            } else if root_relative {
                lexical_normalize(Path::new(path_part.trim_start_matches('/')))
            } else {
                let target = PathBuf::from(path_part);
                if target.is_absolute() {
                    target
                } else {
                    lexical_normalize(
                        &self
                            .path
                            .parent()
                            .unwrap_or_else(|| Path::new(""))
                            .join(target),
                    )
                }
            };
            let extension_inferred = unresolved_path.extension().is_none();
            let mut target_path = unresolved_path.clone();
            if extension_inferred {
                target_path.set_extension("md");
            }
            let same_file = lexical_normalize(self.path) == target_path;
            if same_file {
                if let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) {
                    let fragment_key = decode_fragment(fragment);
                    let key = fragment_key.to_ascii_lowercase();
                    let candidates = self
                        .heading_targets
                        .get(&key)
                        .or_else(|| self.heading_targets.get(&slugify(&fragment_key)));
                    match candidates {
                        Some(candidates) if candidates.len() == 1 => {
                            if let Some(target_id) = candidates.first() {
                                self.add_link_edge(
                                    &pending,
                                    target_id.clone(),
                                    Some(("references", Some(fragment))),
                                );
                            }
                        }
                        Some(candidates) => self.add_unresolved_with_candidates(
                            &pending,
                            "ambiguous_fragment",
                            fragment,
                            candidates
                                .iter()
                                .map(|target| {
                                    json!({
                                        "nodeId": target,
                                        "reason": "multiple matching heading anchors",
                                        "confidence": "ambiguous"
                                    })
                                })
                                .collect(),
                        ),
                        None => self.add_unresolved(&pending, "missing_fragment", fragment),
                    }
                } else {
                    self.add_unresolved(&pending, "same_file_without_fragment", raw);
                }
                continue;
            }
            if !is_supported_local_link(&target_path) {
                self.add_unresolved(&pending, "unsupported_local_target", raw);
                continue;
            }
            let target_id = crate::make_id(&[&target_path.to_string_lossy()]);
            let relation = if is_documentable_source(&target_path) {
                "documents"
            } else {
                "references"
            };
            self.add_link_edge_with_hint(
                &pending,
                target_id,
                Some((relation, fragment)),
                DocumentTargetHint {
                    path: unresolved_path.to_string_lossy().replace('\\', "/"),
                    extension_inferred,
                    root_relative,
                },
            );
        }
    }

    fn add_link_edge(
        &mut self,
        pending: &PendingLink,
        target: String,
        relation: Option<(&str, Option<&str>)>,
    ) {
        let (relation, fragment) = relation.unwrap_or(("references", None));
        let spelling = pending
            .reference_label
            .as_deref()
            .unwrap_or(pending.raw.as_str());
        self.record_document_reference(
            &pending.owner_id,
            spelling,
            pending.kind,
            pending.site,
            "exact",
            Some(&target),
            Vec::new(),
        );
        self.add_relation_with_site(
            &pending.owner_id,
            &target,
            relation,
            pending.site,
            pending.kind,
            fragment,
        );
    }

    fn add_link_edge_with_hint(
        &mut self,
        pending: &PendingLink,
        target: String,
        relation: Option<(&str, Option<&str>)>,
        hint: DocumentTargetHint,
    ) {
        self.add_link_edge(pending, target, relation);
        if let Some(edge) = self.extraction.edges.last_mut() {
            edge.attributes
                .insert("_document_target_path".to_owned(), Value::String(hint.path));
            edge.attributes.insert(
                "_document_target_extension_inferred".to_owned(),
                Value::Bool(hint.extension_inferred),
            );
            edge.attributes.insert(
                "_document_target_root_relative".to_owned(),
                Value::Bool(hint.root_relative),
            );
        }
    }

    fn add_unresolved(&mut self, pending: &PendingLink, reason: &str, target: &str) {
        self.add_unresolved_with_candidates(pending, reason, target, Vec::new());
    }

    fn add_unresolved_with_candidates(
        &mut self,
        pending: &PendingLink,
        reason: &str,
        target: &str,
        candidates: Vec<Value>,
    ) {
        if self.unresolved_links.len() >= MAX_DIAGNOSTICS {
            // Keep the typed evidence bounded independently of the diagnostic
            // list. A limit is still an observable partial result.
            self.record_document_reference(
                &pending.owner_id,
                target,
                pending.kind,
                pending.site,
                "limited",
                None,
                Vec::new(),
            );
            return;
        }
        self.unresolved_links.push(json!({
            "source": pending.owner_id,
            "target": target,
            "kind": pending.kind,
            "reason": reason,
            "source_file": self.source_file,
            "start_byte": pending.site.start_byte,
            "end_byte": pending.site.end_byte,
            "start_line": pending.site.line,
            "end_line": self.line_at(pending.site.end_byte),
            "line_start": pending.site.line,
            "line_end": self.line_at(pending.site.end_byte),
            "column_start": pending.site.start_byte.saturating_sub(pending.site.line_start),
            "column_end": pending.site.end_byte.saturating_sub(self.line_start(pending.site.end_byte)),
        }));
        let spelling = pending
            .reference_label
            .as_deref()
            .unwrap_or(if target.is_empty() {
                pending.raw.as_str()
            } else {
                target
            });
        let resolution = if candidates.is_empty() {
            "unresolved"
        } else {
            "ambiguous"
        };
        self.record_document_reference(
            &pending.owner_id,
            spelling,
            pending.kind,
            pending.site,
            resolution,
            None,
            candidates,
        );
    }

    fn add_relation(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        start: usize,
        end: usize,
    ) {
        self.add_relation_with_site(
            source,
            target,
            relation,
            self.link_site(start, end),
            "structural",
            None,
        );
    }

    fn add_relation_with_site(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        site: LinkSite,
        link_kind: &str,
        fragment: Option<&str>,
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
            Value::String(format!("L{}", site.line)),
        );
        attributes.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        stamp_source_range(&mut attributes, self.source, site.start_byte, site.end_byte);
        attributes.insert("start_line".to_owned(), json!(site.line));
        attributes.insert("end_line".to_owned(), json!(self.line_at(site.end_byte)));
        attributes.insert("weight".to_owned(), json!(1.0));
        if link_kind != "structural" {
            attributes.insert("link_kind".to_owned(), Value::String(link_kind.to_owned()));
        }
        if let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) {
            attributes.insert("fragment".to_owned(), Value::String(fragment.to_owned()));
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn link_site(&self, start: usize, end: usize) -> LinkSite {
        let start = start.min(self.source.len());
        let end = end.clamp(start, self.source.len());
        LinkSite {
            start_byte: start,
            end_byte: end,
            line: self.line_at(start),
            line_start: self.line_start(start),
        }
    }

    fn line_at(&self, offset: usize) -> usize {
        indexed_source_point(&self.line_starts, offset.min(self.source.len())).0
    }

    fn line_start(&self, offset: usize) -> usize {
        let offset = offset.min(self.source.len());
        let line_index = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        self.line_starts.get(line_index).copied().unwrap_or(0)
    }

    fn pending_link_count(&self) -> usize {
        self.extraction
            .edges
            .iter()
            .filter(|edge| edge.attributes.get("link_kind").is_some())
            .count()
            .saturating_add(self.external_links.len())
            .saturating_add(self.unresolved_links.len())
    }
}

fn parse_frontmatter(source: &[u8]) -> (Option<FrontmatterExtraction>, Option<String>) {
    let Some((first_end, first_line)) = next_line(source, 0) else {
        return (None, None);
    };
    if !is_frontmatter_delimiter(first_line, true) {
        return (None, None);
    }
    if first_end > FRONTMATTER_MAX_BYTES {
        return (
            None,
            Some("Markdown frontmatter exceeds the byte limit".to_owned()),
        );
    }
    let mut offset = first_end;
    while offset < source.len() && offset <= FRONTMATTER_MAX_BYTES {
        let Some((line_end, line)) = next_line(source, offset) else {
            break;
        };
        if is_frontmatter_delimiter(line, false) {
            let yaml = &source[first_end..offset];
            if yaml.len() > FRONTMATTER_MAX_BYTES {
                return (
                    None,
                    Some("Markdown frontmatter exceeds the byte limit".to_owned()),
                );
            }
            if yaml_contains_alias_or_tag(yaml) {
                return (
                    None,
                    Some("Markdown frontmatter aliases and tags are not supported".to_owned()),
                );
            }
            let Ok(yaml) = std::str::from_utf8(yaml) else {
                return (
                    None,
                    Some("Markdown frontmatter must be valid UTF-8".to_owned()),
                );
            };
            return match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(yaml) {
                Ok(value) => match yaml_metadata(&value) {
                    Ok(metadata) => match frontmatter_facts(yaml, first_end, &metadata) {
                        Ok(facts) => (Some(FrontmatterExtraction { metadata, facts }), None),
                        Err(diagnostic) => (None, Some(diagnostic)),
                    },
                    Err(diagnostic) => (None, Some(diagnostic.to_owned())),
                },
                Err(_) => (
                    None,
                    Some("Markdown frontmatter is not valid YAML".to_owned()),
                ),
            };
        }
        offset = line_end;
    }
    (
        None,
        Some("Markdown frontmatter has no bounded closing delimiter".to_owned()),
    )
}

fn yaml_contains_alias_or_tag(source: &[u8]) -> bool {
    let mut quoted = None;
    let mut escaped = false;
    let mut index = 0;
    while index < source.len() {
        let byte = source[index];
        if let Some(quote) = quoted {
            if quote == b'"' && escaped {
                escaped = false;
            } else if quote == b'"' && byte == b'\\' {
                escaped = true;
            } else if quote == b'\'' && byte == b'\'' && source.get(index + 1) == Some(&b'\'') {
                index = index.saturating_add(2);
                continue;
            } else if byte == quote {
                quoted = None;
            }
            index = index.saturating_add(1);
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quoted = Some(byte);
            index = index.saturating_add(1);
            continue;
        }
        if byte == b'#' && (index == 0 || source[index - 1].is_ascii_whitespace()) {
            index = source[index..]
                .iter()
                .position(|value| *value == b'\n')
                .map_or(source.len(), |offset| index.saturating_add(offset));
            continue;
        }
        if matches!(byte, b'&' | b'*' | b'!') {
            let token_start = index == 0
                || source[index - 1].is_ascii_whitespace()
                || matches!(
                    source[index - 1],
                    b':' | b'[' | b']' | b',' | b'{' | b'}' | b'-'
                );
            if token_start {
                return true;
            }
        }
        index = index.saturating_add(1);
    }
    false
}

fn yaml_metadata(value: &serde_yaml_ng::Value) -> Result<Map<String, Value>, &'static str> {
    let serde_yaml_ng::Value::Mapping(mapping) = value else {
        return Err("Markdown frontmatter must be a mapping");
    };
    let mut budget = MetadataBudget::default();
    yaml_mapping(mapping, 0, &mut budget)
}

fn yaml_mapping(
    mapping: &serde_yaml_ng::Mapping,
    depth: usize,
    budget: &mut MetadataBudget,
) -> Result<Map<String, Value>, &'static str> {
    if depth > MAX_METADATA_DEPTH {
        return Err("Markdown frontmatter exceeds the nesting-depth limit");
    }
    let mut entries = Vec::with_capacity(mapping.len());
    for (key, value) in mapping {
        let serde_yaml_ng::Value::String(key) = key else {
            return Err("Markdown frontmatter keys must be strings");
        };
        budget.keys = budget.keys.saturating_add(1);
        if budget.keys > MAX_METADATA_KEYS {
            return Err("Markdown frontmatter has too many keys");
        }
        if key.len() > MAX_METADATA_STRING_BYTES {
            return Err("Markdown frontmatter key exceeds the byte limit");
        }
        let json_value = yaml_value(value, depth.saturating_add(1), budget)?;
        entries.push((key.clone(), json_value));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut output = Map::new();
    for (key, value) in entries {
        output.insert(key, value);
    }
    Ok(output)
}

fn yaml_value(
    value: &serde_yaml_ng::Value,
    depth: usize,
    budget: &mut MetadataBudget,
) -> Result<Value, &'static str> {
    if depth > MAX_METADATA_DEPTH {
        return Err("Markdown frontmatter exceeds the nesting-depth limit");
    }
    match value {
        serde_yaml_ng::Value::Null => Ok(Value::Null),
        serde_yaml_ng::Value::Bool(value) => Ok(Value::Bool(*value)),
        serde_yaml_ng::Value::Number(value) => {
            serde_json::to_value(value).map_err(|_| "Markdown frontmatter number is invalid")
        }
        serde_yaml_ng::Value::String(value) => {
            if value.len() > MAX_METADATA_STRING_BYTES {
                return Err("Markdown frontmatter value exceeds the byte limit");
            }
            Ok(Value::String(value.clone()))
        }
        serde_yaml_ng::Value::Sequence(values) => {
            budget.array_items = budget.array_items.saturating_add(values.len());
            if values.len() > MAX_METADATA_ARRAY_ITEMS
                || budget.array_items > MAX_METADATA_ARRAY_ITEMS
            {
                return Err("Markdown frontmatter array exceeds the item limit");
            }
            values
                .iter()
                .map(|value| yaml_value(value, depth.saturating_add(1), budget))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        serde_yaml_ng::Value::Mapping(mapping) => {
            yaml_mapping(mapping, depth.saturating_add(1), budget).map(Value::Object)
        }
        serde_yaml_ng::Value::Tagged(_) => Err("Markdown frontmatter YAML tags are not supported"),
    }
}

fn frontmatter_facts(
    yaml: &str,
    source_offset: usize,
    metadata: &Map<String, Value>,
) -> Result<Vec<FrontmatterFact>, String> {
    let config = ProcessConfig::new("yaml")
        .minimal()
        .with_data_extraction(true);
    let parsed = tree_sitter_language_pack::process(yaml, &config)
        .map_err(|error| format!("Markdown frontmatter source anchoring failed: {error}"))?;
    let root = parsed
        .data
        .ok_or_else(|| "Markdown frontmatter source anchoring produced no data".to_owned())?;
    let root_value = Value::Object(metadata.clone());
    let mut facts = Vec::new();
    let mut seen_paths = HashSet::new();
    collect_frontmatter_facts(
        &root.children,
        "",
        None,
        &root_value,
        source_offset,
        yaml.len(),
        &mut seen_paths,
        &mut facts,
    )?;
    if !metadata.is_empty() && facts.is_empty() {
        return Err("Markdown frontmatter keys could not be source-anchored".to_owned());
    }
    Ok(facts)
}

#[allow(clippy::too_many_arguments)]
fn collect_frontmatter_facts(
    nodes: &[DataNode],
    parent_path: &str,
    parent_fact_path: Option<&str>,
    root: &Value,
    source_offset: usize,
    yaml_len: usize,
    seen_paths: &mut HashSet<String>,
    facts: &mut Vec<FrontmatterFact>,
) -> Result<(), &'static str> {
    for node in nodes {
        let Some(segment) = node.key.as_deref() else {
            continue;
        };
        let key_path = frontmatter_key_path(parent_path, segment);
        let Some(value) = json_pointer_value(root, &key_path) else {
            return Err("Markdown frontmatter syntax and normalized keys disagree");
        };
        if !seen_paths.insert(key_path.clone()) {
            return Err("Markdown frontmatter contains a duplicate key path");
        }
        let scalar_sequence_item = node.kind == DataNodeKind::Sequence
            && node.children.is_empty()
            && !matches!(value, Value::Array(_) | Value::Object(_));
        let emitted_path = if scalar_sequence_item {
            parent_fact_path.map(str::to_owned)
        } else {
            if facts.len() >= MAX_METADATA_GRAPH_NODES {
                return Err("Markdown frontmatter graph-node limit exceeded");
            }
            if node.span.start_byte >= node.span.end_byte || node.span.end_byte > yaml_len {
                return Err("Markdown frontmatter contains an invalid source range");
            }
            facts.push(FrontmatterFact {
                key: if node.kind == DataNodeKind::Sequence {
                    segment
                        .parse::<usize>()
                        .ok()
                        .map_or_else(|| segment.to_owned(), |index| format!("item {}", index + 1))
                } else {
                    segment.to_owned()
                },
                key_path: key_path.clone(),
                parent_path: parent_fact_path.map(str::to_owned),
                value: value.clone(),
                start_byte: source_offset.saturating_add(node.span.start_byte),
                end_byte: source_offset.saturating_add(node.span.end_byte),
            });
            Some(key_path.clone())
        };
        collect_frontmatter_facts(
            &node.children,
            &key_path,
            emitted_path.as_deref(),
            root,
            source_offset,
            yaml_len,
            seen_paths,
            facts,
        )?;
    }
    Ok(())
}

fn frontmatter_key_path(parent: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    if parent.is_empty() {
        format!("/{escaped}")
    } else {
        format!("{parent}/{escaped}")
    }
}

fn json_pointer_value<'value>(root: &'value Value, pointer: &str) -> Option<&'value Value> {
    root.pointer(pointer)
}

fn frontmatter_fact_label(key: &str, value: &Value) -> String {
    let semantic_key = key.to_ascii_lowercase().replace(['-', '_'], "");
    let show_value = matches!(
        semantic_key.as_str(),
        "title"
            | "tag"
            | "tags"
            | "alias"
            | "aliases"
            | "author"
            | "authors"
            | "description"
            | "summary"
            | "category"
            | "categories"
            | "layout"
            | "status"
            | "draft"
            | "date"
            | "published"
            | "updated"
            | "slug"
            | "permalink"
            | "navlabel"
            | "contenttype"
            | "audience"
            | "owner"
            | "owners"
    );
    let Some(summary) = show_value
        .then(|| frontmatter_value_summary(value))
        .flatten()
    else {
        return bounded_label(key);
    };
    bounded_label(&format!("{key}: {summary}"))
}

fn frontmatter_value_summary(value: &Value) -> Option<String> {
    let summary = match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => compact_label(value),
        Value::Array(values) if values.len() <= 16 => {
            let values = values
                .iter()
                .map(|value| match value {
                    Value::Null => Some("null".to_owned()),
                    Value::Bool(value) => Some(value.to_string()),
                    Value::Number(value) => Some(value.to_string()),
                    Value::String(value) => Some(compact_label(value)),
                    Value::Array(_) | Value::Object(_) => None,
                })
                .collect::<Option<Vec<_>>>()?;
            values.join(", ")
        }
        Value::Array(_) | Value::Object(_) => return None,
    };
    (!summary.is_empty()).then(|| truncate_utf8(&summary, MAX_LABEL_CHARS))
}

fn next_line(source: &[u8], start: usize) -> Option<(usize, &[u8])> {
    if start >= source.len() {
        return None;
    }
    let relative_end = source[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(source.len().saturating_sub(start), |offset| {
            offset.saturating_add(1)
        });
    let end = start.saturating_add(relative_end);
    Some((end, &source[start..end]))
}

fn opens_fence(line: &[u8]) -> Option<(u8, usize)> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let indent = line.iter().take_while(|byte| **byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let marker = *line.get(indent)?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = line[indent..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    if length < 3 || (marker == b'`' && line[indent + length..].contains(&b'`')) {
        return None;
    }
    Some((marker, length))
}

fn closes_fence(line: &[u8], fence: (u8, usize)) -> bool {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let indent = line.iter().take_while(|byte| **byte == b' ').count();
    if indent > 3 || line.get(indent) != Some(&fence.0) {
        return false;
    }
    let length = line[indent..]
        .iter()
        .take_while(|byte| **byte == fence.0)
        .count();
    length >= fence.1
        && line[indent + length..]
            .iter()
            .all(|byte| byte.is_ascii_whitespace())
}

fn parse_reference_definition(line: &[u8]) -> Option<(String, String, usize, usize)> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let mut start = 0;
    while start < line.len() && line[start] == b' ' && start < 3 {
        start += 1;
    }
    if line.get(start) != Some(&b'[') {
        return None;
    }
    let label_end = line[start + 1..].iter().position(|byte| *byte == b']')? + start + 1;
    let colon = label_end.saturating_add(1);
    if line.get(colon) != Some(&b':') {
        return None;
    }
    let mut target_start = colon.saturating_add(1);
    while target_start < line.len() && line[target_start].is_ascii_whitespace() {
        target_start += 1;
    }
    if target_start >= line.len() {
        return None;
    }
    let (target_end, target) = if line[target_start] == b'<' {
        let end = line[target_start + 1..]
            .iter()
            .position(|byte| *byte == b'>')?
            + target_start
            + 1;
        (end.saturating_add(1), &line[target_start + 1..end])
    } else {
        let end = line[target_start..]
            .iter()
            .position(|byte| byte.is_ascii_whitespace())
            .map_or(line.len(), |relative| target_start.saturating_add(relative));
        (end, &line[target_start..end])
    };
    let label = String::from_utf8_lossy(&line[start + 1..label_end]).into_owned();
    let target = String::from_utf8_lossy(target).into_owned();
    Some((label, target, start, target_end))
}

fn parse_footnote_definition(line: &[u8]) -> Option<(String, usize, usize)> {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let mut start = 0;
    while start < line.len() && line[start] == b' ' && start < 3 {
        start += 1;
    }
    if line.get(start..start.saturating_add(2)) != Some(b"[^") {
        return None;
    }
    let label_end = line[start.saturating_add(2)..]
        .iter()
        .position(|byte| *byte == b']')?
        .saturating_add(start)
        .saturating_add(2);
    if line.get(label_end.saturating_add(1)) != Some(&b':') {
        return None;
    }
    let label = String::from_utf8_lossy(&line[start.saturating_add(2)..label_end])
        .trim()
        .to_owned();
    if label.is_empty() || label.len() > MAX_LABEL_CHARS {
        return None;
    }
    Some((label, start, line.len()))
}

fn is_frontmatter_delimiter(line: &[u8], allow_bom: bool) -> bool {
    let line = if allow_bom {
        line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(line)
    } else {
        line
    };
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let mut end = line.len();
    while end > 0 && matches!(line[end - 1], b' ' | b'\t') {
        end = end.saturating_sub(1);
    }
    &line[..end] == b"---"
}

fn heading_level(node: Node<'_>, source: &[u8]) -> usize {
    if node.kind() == "setext_heading" {
        let mut cursor = node.walk();
        return if node
            .children(&mut cursor)
            .any(|child| child.kind() == "setext_h1_underline")
        {
            1
        } else {
            2
        };
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| child.kind().strip_prefix("atx_h")?.strip_suffix("_marker"))
        .and_then(|level| level.parse::<usize>().ok())
        .filter(|level| (1..=6).contains(level))
        .unwrap_or_else(|| {
            let line = node_text(source, node.start_byte(), node.end_byte());
            line.trim_start()
                .chars()
                .take_while(|char| *char == '#')
                .count()
                .clamp(1, 6)
        })
}

fn heading_title(node: Node<'_>, source: &[u8]) -> String {
    if let Some(inline) = find_descendant_kind(node, "inline") {
        return node_text(source, inline.start_byte(), inline.end_byte());
    }
    node_text(source, node.start_byte(), node.end_byte())
}

fn find_descendant_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(Node::is_named) {
        if child.kind() == kind {
            return Some(child);
        }
        if let Some(found) = find_descendant_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

fn child_text(node: Node<'_>, source: &[u8], kind: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
        .map(|child| node_text(source, child.start_byte(), child.end_byte()))
}

fn child_text_at(node: Node<'_>, source: &[u8], base: usize, kind: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
        .map(|child| {
            node_text(
                source,
                base.saturating_add(child.start_byte()),
                base.saturating_add(child.end_byte()),
            )
        })
}

fn has_child_kind(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| child.kind() == kind)
}

fn node_text(source: &[u8], start: usize, end: usize) -> String {
    let start = start.min(source.len());
    let end = end.clamp(start, source.len());
    String::from_utf8_lossy(&source[start..end]).into_owned()
}

fn block_label(kind: &str, text: &str) -> String {
    match kind {
        "list" => "list".to_owned(),
        "list_item" => compact_label(text),
        "pipe_table" | "pipe_table_header" | "pipe_table_row" | "pipe_table_cell" => {
            kind.replace('_', " ")
        }
        "fenced_code_block" | "indented_code_block" => "code".to_owned(),
        "block_quote" => "blockquote".to_owned(),
        "thematic_break" => "thematic break".to_owned(),
        "link_reference_definition" => "link definition".to_owned(),
        "footnote_definition" => "footnote".to_owned(),
        "other" => "other Markdown construct".to_owned(),
        "html_block" => "HTML block".to_owned(),
        _ => compact_label(text),
    }
}

fn compact_label(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    bounded_label(&normalized)
}

fn bounded_label(text: &str) -> String {
    let mut output = String::new();
    for character in text.chars().take(MAX_LABEL_CHARS) {
        output.push(character);
    }
    if text.chars().count() > MAX_LABEL_CHARS {
        output.push('…');
    }
    output
}

fn split_explicit_heading_id(title: &str) -> (String, Option<String>) {
    let Some(start) = title.rfind("{#") else {
        return (title.to_owned(), None);
    };
    if !title.ends_with('}') || start + 3 >= title.len() {
        return (title.to_owned(), None);
    }
    let candidate = &title[start + 2..title.len().saturating_sub(1)];
    if candidate.is_empty()
        || !candidate.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | ':' | '.')
        })
    {
        return (title.to_owned(), None);
    }
    (
        title[..start].trim_end().to_owned(),
        Some(candidate.to_owned()),
    )
}

fn slugify(text: &str) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for character in text.trim().chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(character);
        } else if matches!(character, ' ' | '\t' | '-' | '_' | '.') {
            pending_dash = true;
        }
    }
    slug
}

fn decode_fragment(fragment: &str) -> String {
    let bytes = fragment.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len().min(512));
    let mut index = 0;
    while index < bytes.len() && decoded.len() < 512 {
        if bytes[index] == b'%'
            && let (Some(high), Some(low)) = (bytes.get(index + 1), bytes.get(index + 2))
            && let (Some(high), Some(low)) = (hex_digit(*high), hex_digit(*low))
        {
            decoded.push(high.saturating_mul(16).saturating_add(low));
            index = index.saturating_add(3);
            continue;
        }
        decoded.push(bytes[index]);
        index = index.saturating_add(1);
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

const fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn normalize_reference_label(label: &str) -> String {
    label
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn is_nested_block(kind: &str) -> bool {
    is_structural_block_kind(kind)
}

fn is_structural_block_kind(kind: &str) -> bool {
    matches!(
        kind,
        "atx_heading"
            | "block_quote"
            | "fenced_code_block"
            | "footnote_definition"
            | "html_block"
            | "indented_code_block"
            | "link_reference_definition"
            | "list"
            | "list_item"
            | "paragraph"
            | "pipe_table"
            | "pipe_table_cell"
            | "pipe_table_header"
            | "pipe_table_row"
            | "section"
            | "setext_heading"
            | "thematic_break"
            | "other"
    )
}

fn is_external_target(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    target.contains("://")
        || lower.starts_with("mailto:")
        || lower.starts_with("tel:")
        || lower.starts_with("data:")
        || lower.starts_with("//")
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

fn is_supported_local_link(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "markdown"
                    | "mdx"
                    | "qmd"
                    | "skill"
                    | "rst"
                    | "txt"
                    | "html"
                    | "htm"
                    | "yaml"
                    | "yml"
                    | "json"
                    | "docx"
                    | "xlsx"
                    | "pptx"
                    | "rtf"
                    | "pdf"
                    | "rs"
                    | "py"
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
