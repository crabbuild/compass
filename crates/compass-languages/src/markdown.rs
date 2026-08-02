use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::facts::stamp_source_range;
use crate::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value, json};
use tree_sitter::{Node, Parser};
use tree_sitter_md::{INLINE_LANGUAGE, LANGUAGE};

const FRONTMATTER_MAX_BYTES: usize = 64 * 1024;
const MAX_BLOCKS: usize = 100_000;
const MAX_LINKS: usize = 100_000;
const MAX_DIAGNOSTICS: usize = 256;
const MAX_METADATA_KEYS: usize = 256;
const MAX_METADATA_STRING_BYTES: usize = 16 * 1024;
const MAX_METADATA_ARRAY_ITEMS: usize = 256;
const MAX_LABEL_CHARS: usize = 512;

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

    let (metadata, frontmatter_diagnostic) = parse_frontmatter(source);
    let stem = crate::file_stem(path);
    let file_id = crate::make_id(&[source_file]);
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
        heading_targets: HashMap::new(),
        reference_definitions: HashMap::new(),
        pending_links: Vec::new(),
        unresolved_links: Vec::new(),
        external_links: Vec::new(),
        diagnostics: Vec::new(),
        inline_parser,
        next_block_index: 1,
    };

    state.add_root(file_id, metadata);
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
    state.finalize_links();

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
    heading_targets: HashMap<String, Vec<String>>,
    reference_definitions: HashMap<String, String>,
    pending_links: Vec<PendingLink>,
    unresolved_links: Vec<Value>,
    external_links: Vec<Value>,
    diagnostics: Vec<String>,
    inline_parser: Parser,
    next_block_index: usize,
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

impl State<'_, '_> {
    fn add_root(&mut self, id: String, metadata: Option<Map<String, Value>>) {
        self.seen_nodes.insert(id.clone());
        let mut attributes = Map::new();
        attributes.insert(
            "label".to_owned(),
            Value::String(
                self.path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_owned(),
            ),
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
        extra.insert(
            "anchor_slug".to_owned(),
            Value::String(slugify(explicit_id.as_deref().unwrap_or(title))),
        );
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
        self.register_heading_target(title, explicit_id.as_deref(), &id);
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
        if kind == "pipe_table_header" {
            extra.insert("table_role".to_owned(), Value::String("header".to_owned()));
        } else if kind == "pipe_table_row" {
            extra.insert("table_role".to_owned(), Value::String("row".to_owned()));
        } else if kind == "pipe_table_cell" {
            extra.insert("table_role".to_owned(), Value::String("cell".to_owned()));
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
        stamp_source_range(&mut extra, self.source, range.start, range.end);
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

    fn register_heading_target(&mut self, title: &str, explicit_id: Option<&str>, id: &str) {
        let mut keys = vec![slugify(title)];
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
            let target_path = if path_part.is_empty() {
                lexical_normalize(self.path)
            } else {
                let mut target = PathBuf::from(path_part);
                if target.extension().is_none() {
                    target.set_extension("md");
                }
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
            let same_file = lexical_normalize(self.path) == target_path;
            if same_file {
                if let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) {
                    let key = fragment.to_ascii_lowercase();
                    let candidates = self
                        .heading_targets
                        .get(&key)
                        .or_else(|| self.heading_targets.get(&slugify(fragment)));
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
                        Some(_) => self.add_unresolved(&pending, "ambiguous_fragment", fragment),
                        None => self.add_unresolved(&pending, "missing_fragment", fragment),
                    }
                }
                continue;
            }
            if is_documentable_source(&target_path) && !target_path.is_file() {
                continue;
            }
            if !is_supported_local_link(&target_path) {
                continue;
            }
            let target_id = crate::make_id(&[&target_path.to_string_lossy()]);
            let relation = if is_documentable_source(&target_path) {
                "documents"
            } else {
                "references"
            };
            self.add_link_edge(&pending, target_id, Some((relation, fragment)));
        }
    }

    fn add_link_edge(
        &mut self,
        pending: &PendingLink,
        target: String,
        relation: Option<(&str, Option<&str>)>,
    ) {
        let (relation, fragment) = relation.unwrap_or(("references", None));
        self.add_relation_with_site(
            &pending.owner_id,
            &target,
            relation,
            pending.site,
            pending.kind,
            fragment,
        );
    }

    fn add_unresolved(&mut self, pending: &PendingLink, reason: &str, target: &str) {
        if self.unresolved_links.len() >= MAX_DIAGNOSTICS {
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
        self.source[..offset.min(self.source.len())]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            .saturating_add(1)
    }

    fn line_start(&self, offset: usize) -> usize {
        let prefix = &self.source[..offset.min(self.source.len())];
        prefix
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |newline| newline.saturating_add(1))
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

fn parse_frontmatter(source: &[u8]) -> (Option<Map<String, Value>>, Option<String>) {
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
            let yaml = String::from_utf8_lossy(yaml);
            return match serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&yaml) {
                Ok(value) => match yaml_metadata(&value) {
                    Ok(metadata) => (Some(metadata), None),
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

fn yaml_metadata(value: &serde_yaml_ng::Value) -> Result<Map<String, Value>, &'static str> {
    let serde_yaml_ng::Value::Mapping(mapping) = value else {
        return Err("Markdown frontmatter must be a mapping");
    };
    if mapping.len() > MAX_METADATA_KEYS {
        return Err("Markdown frontmatter has too many keys");
    }
    let mut entries = Vec::with_capacity(mapping.len());
    for (key, value) in mapping {
        let serde_yaml_ng::Value::String(key) = key else {
            return Err("Markdown frontmatter keys must be strings");
        };
        if key.len() > MAX_METADATA_STRING_BYTES {
            return Err("Markdown frontmatter key exceeds the byte limit");
        }
        let json_value = match value {
            serde_yaml_ng::Value::Null => Value::Null,
            serde_yaml_ng::Value::Bool(value) => Value::Bool(*value),
            serde_yaml_ng::Value::Number(value) => {
                serde_json::to_value(value).map_err(|_| "Markdown frontmatter number is invalid")?
            }
            serde_yaml_ng::Value::String(value) => {
                if value.len() > MAX_METADATA_STRING_BYTES {
                    return Err("Markdown frontmatter value exceeds the byte limit");
                }
                Value::String(value.clone())
            }
            serde_yaml_ng::Value::Sequence(values) => {
                if values.len() > MAX_METADATA_ARRAY_ITEMS {
                    return Err("Markdown frontmatter array exceeds the item limit");
                }
                let mut output = Vec::with_capacity(values.len());
                for value in values {
                    let scalar = yaml_scalar(value)
                        .ok_or("Markdown frontmatter arrays must contain scalars")?;
                    output.push(scalar);
                }
                Value::Array(output)
            }
            serde_yaml_ng::Value::Mapping(_) | serde_yaml_ng::Value::Tagged(_) => {
                return Err("Markdown frontmatter nested values are not supported");
            }
        };
        entries.push((key.clone(), json_value));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut output = Map::new();
    for (key, value) in entries {
        output.insert(key, value);
    }
    Ok(output)
}

fn yaml_scalar(value: &serde_yaml_ng::Value) -> Option<Value> {
    match value {
        serde_yaml_ng::Value::Null => Some(Value::Null),
        serde_yaml_ng::Value::Bool(value) => Some(Value::Bool(*value)),
        serde_yaml_ng::Value::Number(value) => serde_json::to_value(value).ok(),
        serde_yaml_ng::Value::String(value) if value.len() <= MAX_METADATA_STRING_BYTES => {
            Some(Value::String(value.clone()))
        }
        _ => None,
    }
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
