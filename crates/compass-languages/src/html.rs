//! Bounded structural HTML extraction and the shared HTML-to-text renderer.
//!
//! HTML is deliberately parsed with the pinned, statically linked
//! `tree-sitter-html` grammar.  The renderer in this module is also used by URL
//! ingestion so ingestion and repository extraction agree about what is
//! visible, what is metadata, and which elements are ignored.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use crate::facts::stamp_source_range;
use crate::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value, json};
use tree_sitter::{Node, Parser, Tree};
use tree_sitter_html::LANGUAGE;
use url::Url;

const MAX_NODES: usize = 100_000;
const MAX_LINKS: usize = 100_000;
const MAX_DIAGNOSTICS: usize = 256;
const MAX_ATTRIBUTES: usize = 256;
const MAX_ATTRIBUTE_BYTES: usize = 16 * 1024;
const MAX_VISIBLE_TEXT_BYTES: usize = 1_048_576;
const MAX_RENDERED_BYTES: usize = 2 * 1_048_576;

/// Bounded, normalized HTML suitable for URL ingestion and other text-only
/// consumers.  `markdown` is intentionally conservative: it preserves block
/// boundaries and links without pretending that HTML presentation is Markdown
/// semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlNormalization {
    pub title: String,
    pub markdown: String,
    pub metadata: Map<String, Value>,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum HtmlError {
    #[error("HTML grammar setup failed: {0}")]
    Grammar(String),
    #[error("HTML parser was cancelled")]
    ParseCancelled,
}

/// Normalize HTML using the same pinned parser and visibility rules as graph
/// extraction.  The function never fetches a URL and treats malformed markup
/// as recoverable input, returning diagnostics alongside the partial text.
pub fn normalize_html(source_file: &str, source: &[u8]) -> Result<HtmlNormalization, HtmlError> {
    let tree = parse(source)?;
    let mut renderer = Renderer {
        source,
        source_file,
        diagnostics: Vec::new(),
        metadata: Map::new(),
        title: String::new(),
        base_href: None,
        rendered_bytes: 0,
    };
    let markdown = renderer.render_document(tree.root_node());
    if tree.root_node().has_error() {
        renderer.add_diagnostic("HTML parser recovered from malformed syntax");
    }
    let visible_text = visible_text_for_node(tree.root_node(), source);
    renderer.metadata.insert(
        "visible_text".to_owned(),
        Value::String(truncate_bytes(&visible_text, MAX_VISIBLE_TEXT_BYTES)),
    );
    if let Some(base_href) = renderer.base_href {
        renderer
            .metadata
            .insert("base_href".to_owned(), Value::String(base_href));
    }
    Ok(HtmlNormalization {
        title: bounded_string(&renderer.title),
        markdown: truncate_bytes(&markdown, MAX_RENDERED_BYTES),
        metadata: renderer.metadata,
        diagnostics: renderer.diagnostics,
    })
}

/// Extract structural HTML graph facts from caller-supplied bytes.
pub(crate) fn extract_source(
    path: &Path,
    source_file: &str,
    source: &[u8],
) -> Result<crate::Extraction, crate::ExtractError> {
    let tree = parse(source).map_err(|error| match error {
        HtmlError::Grammar(detail) => crate::ExtractError::MissingGrammar {
            language: "html".to_owned(),
            detail,
        },
        HtmlError::ParseCancelled => crate::ExtractError::ParseCancelled(path.to_path_buf()),
    })?;
    let file_id = crate::make_id(&[source_file]);
    let mut state = State {
        path,
        source,
        source_file: source_file.to_owned(),
        file_id: file_id.clone(),
        extraction: crate::Extraction {
            raw_calls: None,
            ..crate::Extraction::default()
        },
        seen_nodes: HashSet::new(),
        anchor_targets: HashMap::new(),
        pending_links: Vec::new(),
        unresolved_links: Vec::new(),
        external_links: Vec::new(),
        diagnostics: Vec::new(),
        next_index: 1,
        hidden_depth: 0,
        base_href: None,
        title: String::new(),
        visible_text: String::new(),
    };
    state.add_root(file_id, &tree);
    if tree.root_node().has_error() {
        state.add_diagnostic("HTML parser recovered from malformed syntax");
        state.mark_partial();
    }
    let root_id = state.file_id.clone();
    state.walk_children(tree.root_node(), Some(&root_id));
    state.finalize_links();
    state.finish_metadata();
    Ok(state.extraction)
}

fn parse(source: &[u8]) -> Result<Tree, HtmlError> {
    let mut parser = Parser::new();
    let language = LANGUAGE.into();
    parser
        .set_language(&language)
        .map_err(|error| HtmlError::Grammar(error.to_string()))?;
    parser.parse(source, None).ok_or(HtmlError::ParseCancelled)
}

struct State<'source, 'path> {
    path: &'path Path,
    source: &'source [u8],
    source_file: String,
    file_id: String,
    extraction: crate::Extraction,
    seen_nodes: HashSet<String>,
    anchor_targets: HashMap<String, Vec<String>>,
    pending_links: Vec<PendingLink>,
    unresolved_links: Vec<Value>,
    external_links: Vec<Value>,
    diagnostics: Vec<String>,
    next_index: usize,
    hidden_depth: usize,
    base_href: Option<String>,
    title: String,
    visible_text: String,
}

#[derive(Clone)]
struct PendingLink {
    raw: String,
    owner_id: String,
    site: LinkSite,
    kind: &'static str,
    rel: Option<String>,
}

#[derive(Clone, Copy)]
struct LinkSite {
    start_byte: usize,
    end_byte: usize,
    line: usize,
}

impl State<'_, '_> {
    fn add_root(&mut self, id: String, tree: &Tree) {
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
            Value::String("html".to_owned()),
        );
        attributes.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".to_owned(), Value::String("L1".to_owned()));
        attributes.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        stamp_source_range(&mut attributes, self.source, 0, self.source.len());
        if tree.root_node().has_error() {
            attributes.insert(
                crate::EXTRACTION_QUALITY_EXTENSION.to_owned(),
                Value::String(crate::EXTRACTION_QUALITY_PARTIAL.to_owned()),
            );
        }
        self.extraction.nodes.push(NodeRecord {
            id: id.clone(),
            attributes,
        });
        self.file_id = id;
    }

    fn walk_children(&mut self, node: Node<'_>, parent: Option<&str>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            self.walk_node(child, parent);
        }
    }

    fn walk_node(&mut self, node: Node<'_>, parent: Option<&str>) {
        if node.kind() == "comment" || node.kind() == "doctype" {
            return;
        }
        if node.kind() == "ERROR" || node.is_error() || node.is_missing() {
            self.add_diagnostic(format!(
                "HTML malformed syntax near byte {}",
                node.start_byte()
            ));
            return;
        }
        if node.kind() == "text" || node.kind() == "entity" {
            if self.hidden_depth == 0 {
                self.append_visible_text(&decode_html_entities(node_text(self.source, node)));
            }
            return;
        }
        let (tag, start_tag) = match node.kind() {
            "element" => element_tag_name(node, self.source),
            "script_element" => (Some("script".to_owned()), None),
            "style_element" => (Some("style".to_owned()), None),
            _ => (None, None),
        };
        let Some(tag) = tag else {
            self.walk_children(node, parent);
            return;
        };
        let tag = tag.to_ascii_lowercase();
        if is_hidden_tag(&tag) {
            self.hidden_depth = self.hidden_depth.saturating_add(1);
            self.hidden_depth = self.hidden_depth.saturating_sub(1);
            return;
        }
        if self.next_index > MAX_NODES {
            self.add_diagnostic("HTML node limit exceeded");
            return;
        }
        let attributes = collect_attributes(start_tag, self.source);
        if tag == "base"
            && let Some(href) = attributes.get("href").and_then(Value::as_str)
        {
            self.base_href = Some(href.to_owned());
        }
        let kind = html_kind(&tag);
        let label = visible_text_for_node(node, self.source);
        let id = crate::make_id(&[
            &self.source_file,
            "html",
            kind,
            &self.next_index.to_string(),
            &node.start_byte().to_string(),
        ]);
        let mut extra = Map::new();
        extra.insert("tag_name".to_owned(), Value::String(tag.clone()));
        extra.insert(
            "html_attributes".to_owned(),
            Value::Object(attributes.clone()),
        );
        extra.insert(
            "visible_text".to_owned(),
            Value::String(bounded_string(&label)),
        );
        if let Some(level) = heading_level(&tag) {
            extra.insert("heading_level".to_owned(), json!(level));
            extra.insert("anchor_slug".to_owned(), Value::String(slugify(&label)));
        }
        if let Some(id_attr) = attributes.get("id").and_then(Value::as_str) {
            extra.insert("explicit_id".to_owned(), Value::String(id_attr.to_owned()));
        }
        let id = self.add_node(id, &bounded_label(&label), kind, node, parent, extra);
        if let Some(anchor) = attributes
            .get("id")
            .or_else(|| attributes.get("name"))
            .and_then(Value::as_str)
        {
            self.anchor_targets
                .entry(anchor.to_ascii_lowercase())
                .or_default()
                .push(id.clone());
        }
        if tag == "title" && self.title.is_empty() {
            self.title = bounded_string(&label);
        }
        if tag == "a" {
            self.add_pending_link(&attributes, node, "anchor", parent);
        } else if tag == "link" {
            self.add_pending_link(&attributes, node, "link", parent);
        }
        self.collect_metadata(&tag, &attributes);
        if tag == "br" {
            self.append_visible_text("\n");
        }
        self.walk_children(node, Some(&id));
        if matches!(
            tag.as_str(),
            "p" | "div"
                | "section"
                | "article"
                | "main"
                | "nav"
                | "li"
                | "blockquote"
                | "pre"
                | "table"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
        ) {
            self.append_visible_text("\n");
        }
    }

    fn add_node(
        &mut self,
        id: String,
        label: &str,
        kind: &str,
        node: Node<'_>,
        parent: Option<&str>,
        mut extra: Map<String, Value>,
    ) -> String {
        if !self.seen_nodes.insert(id.clone()) {
            return id;
        }
        extra.insert("label".to_owned(), Value::String(bounded_label(label)));
        extra.insert("file_type".to_owned(), Value::String("document".to_owned()));
        extra.insert("document_kind".to_owned(), Value::String(kind.to_owned()));
        extra.insert(
            "source_file".to_owned(),
            Value::String(self.source_file.clone()),
        );
        extra.insert(
            "source_location".to_owned(),
            Value::String(format!("L{}", self.line_at(node.start_byte()))),
        );
        extra.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        stamp_source_range(&mut extra, self.source, node.start_byte(), node.end_byte());
        self.extraction.nodes.push(NodeRecord {
            id: id.clone(),
            attributes: extra,
        });
        self.next_index = self.next_index.saturating_add(1);
        if let Some(parent) = parent {
            self.add_relation(parent, &id, "contains", node.start_byte(), node.end_byte());
        }
        id
    }

    fn add_pending_link(
        &mut self,
        attributes: &Map<String, Value>,
        node: Node<'_>,
        kind: &'static str,
        parent: Option<&str>,
    ) {
        let Some(raw) = attributes
            .get("href")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        else {
            return;
        };
        if self.pending_links.len() >= MAX_LINKS {
            self.add_diagnostic("HTML link limit exceeded");
            return;
        }
        let owner_id = self.containing_owner(parent);
        self.pending_links.push(PendingLink {
            raw: raw.to_owned(),
            owner_id,
            site: self.link_site(node.start_byte(), node.end_byte()),
            kind,
            rel: attributes
                .get("rel")
                .and_then(Value::as_str)
                .map(str::to_owned),
        });
    }

    fn containing_owner(&self, parent: Option<&str>) -> String {
        let mut candidate = parent.map(str::to_owned);
        let mut visited = HashSet::new();
        while let Some(id) = candidate {
            if !visited.insert(id.clone()) {
                break;
            }
            if id == self.file_id {
                return id;
            }
            if self
                .extraction
                .nodes
                .iter()
                .find(|node| node.id == id)
                .and_then(|node| node.attributes.get("document_kind"))
                .and_then(Value::as_str)
                .is_some_and(is_html_block_kind)
            {
                return id;
            }
            candidate = self
                .extraction
                .edges
                .iter()
                .rev()
                .find(|edge| {
                    edge.target == id
                        && edge.attributes.get("relation").and_then(Value::as_str)
                            == Some("contains")
                })
                .map(|edge| edge.source.clone());
        }
        self.file_id.clone()
    }

    fn collect_metadata(&mut self, tag: &str, attributes: &Map<String, Value>) {
        if tag == "meta" {
            let key = attributes
                .get("name")
                .or_else(|| attributes.get("property"))
                .or_else(|| attributes.get("charset"))
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase);
            if let Some(key) = key {
                let content = attributes
                    .get("content")
                    .or_else(|| attributes.get("charset"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.extraction
                    .extensions
                    .entry("html_meta".to_owned())
                    .or_insert_with(|| Value::Object(Map::new()));
                if let Some(Value::Object(meta)) = self.extraction.extensions.get_mut("html_meta")
                    && meta.len() < MAX_ATTRIBUTES
                {
                    meta.insert(key, Value::String(bounded_string(content)));
                }
            }
        }
        if tag == "link"
            && attributes
                .get("rel")
                .and_then(Value::as_str)
                .is_some_and(|rel| {
                    rel.split_ascii_whitespace()
                        .any(|item| item.eq_ignore_ascii_case("canonical"))
                })
            && let Some(href) = attributes.get("href").and_then(Value::as_str)
        {
            self.extraction.extensions.insert(
                "html_canonical".to_owned(),
                Value::String(bounded_string(href)),
            );
        }
    }

    fn finish_metadata(&mut self) {
        if !self.title.is_empty() {
            self.extraction
                .extensions
                .insert("html_title".to_owned(), Value::String(self.title.clone()));
        }
        if let Some(base_href) = &self.base_href {
            self.extraction.extensions.insert(
                "html_base_href".to_owned(),
                Value::String(bounded_string(base_href)),
            );
        }
        self.extraction.extensions.insert(
            "html_visible_text".to_owned(),
            Value::String(truncate_bytes(
                &collapse_whitespace(&self.visible_text),
                MAX_VISIBLE_TEXT_BYTES,
            )),
        );
        self.extraction.extensions.insert(
            "html_node_count".to_owned(),
            json!(self.extraction.nodes.len().saturating_sub(1)),
        );
        self.extraction.extensions.insert(
            "html_link_count".to_owned(),
            json!(
                self.extraction
                    .edges
                    .iter()
                    .filter(|edge| edge.attributes.get("link_kind").is_some())
                    .count()
                    .saturating_add(self.external_links.len())
                    .saturating_add(self.unresolved_links.len())
            ),
        );
        if !self.diagnostics.is_empty() {
            self.extraction.extensions.insert(
                "html_diagnostics".to_owned(),
                Value::Array(
                    self.diagnostics
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if !self.external_links.is_empty() {
            self.extraction.extensions.insert(
                "html_external_links".to_owned(),
                Value::Array(std::mem::take(&mut self.external_links)),
            );
        }
        if !self.unresolved_links.is_empty() {
            self.extraction.extensions.insert(
                "html_unresolved_links".to_owned(),
                Value::Array(std::mem::take(&mut self.unresolved_links)),
            );
        }
        let root_metadata = [
            "html_title",
            "html_base_href",
            "html_visible_text",
            "html_meta",
            "html_canonical",
        ];
        let values = root_metadata
            .iter()
            .filter_map(|key| {
                self.extraction
                    .extensions
                    .get(*key)
                    .cloned()
                    .map(|value| ((*key).to_owned(), value))
            })
            .collect::<Vec<_>>();
        if let Some(root) = self
            .extraction
            .nodes
            .iter_mut()
            .find(|node| node.id == self.file_id)
        {
            for (key, value) in values {
                root.attributes.insert(key, value);
            }
        }
    }

    fn finalize_links(&mut self) {
        let pending = std::mem::take(&mut self.pending_links);
        for link in pending {
            let raw = link.raw.trim().trim_matches('<').trim_matches('>');
            if raw.is_empty() {
                continue;
            }
            let (path_part, fragment) =
                raw.split_once('#').map_or((raw, None), |(path, fragment)| {
                    (path, Some(fragment.trim()))
                });
            if is_external_url(raw) {
                self.external_links
                    .push(self.link_value(&link, raw, "external"));
                continue;
            }
            if has_unsupported_scheme(raw) {
                self.add_unresolved(&link, "unsupported_url_scheme", raw);
                continue;
            }
            let target_path = self.resolve_target(path_part);
            let same_file = target_path.as_deref().is_some_and(|target| {
                match (Url::parse(&self.source_file), Url::parse(target)) {
                    (Ok(source), Ok(target)) => source == target,
                    _ => lexical_normalize(self.path) == lexical_normalize(Path::new(target)),
                }
            });
            if same_file {
                if let Some(fragment) = fragment.filter(|fragment| !fragment.is_empty()) {
                    let key = fragment.to_ascii_lowercase();
                    match self.anchor_targets.get(&key) {
                        Some(candidates) if candidates.len() == 1 => {
                            self.add_link_edge(&link, candidates[0].clone(), Some(fragment));
                        }
                        Some(_) => self.add_unresolved(&link, "ambiguous_fragment", fragment),
                        None => self.add_unresolved(&link, "missing_fragment", fragment),
                    }
                }
                continue;
            }
            let Some(target) = target_path else {
                self.add_unresolved(&link, "invalid_relative_url", raw);
                continue;
            };
            if is_external_url(&target) {
                self.external_links
                    .push(self.link_value(&link, &target, "external"));
                continue;
            }
            if !is_supported_local_link(Path::new(&target)) {
                self.add_unresolved(&link, "unsupported_local_suffix", &target);
                continue;
            }
            self.add_link_edge(&link, crate::make_id(&[&target]), fragment);
        }
    }

    fn resolve_target(&self, path_part: &str) -> Option<String> {
        if path_part.is_empty() {
            return Some(self.path.to_string_lossy().replace('\\', "/"));
        }
        if let Ok(source_url) = Url::parse(&self.source_file)
            && matches!(source_url.scheme(), "http" | "https")
        {
            let base = self
                .base_href
                .as_deref()
                .and_then(|href| source_url.join(href).ok())
                .unwrap_or(source_url);
            return base.join(path_part).ok().map(|url| url.to_string());
        }
        let candidate = PathBuf::from(path_part);
        let joined = if candidate.is_absolute() {
            candidate
        } else {
            let parent = self.path.parent().unwrap_or_else(|| Path::new(""));
            if let Some(base_href) = self
                .base_href
                .as_deref()
                .filter(|href| !has_unsupported_scheme(href))
            {
                let base = PathBuf::from(base_href);
                if base.is_absolute() {
                    base.join(candidate)
                } else {
                    parent.join(base).join(candidate)
                }
            } else {
                parent.join(candidate)
            }
        };
        Some(
            lexical_normalize(&joined)
                .to_string_lossy()
                .replace('\\', "/"),
        )
    }

    fn add_link_edge(&mut self, link: &PendingLink, target: String, fragment: Option<&str>) {
        let mut attributes = self.link_attributes(link, "references");
        if let Some(fragment) = fragment.filter(|value| !value.is_empty()) {
            attributes.insert("fragment".to_owned(), Value::String(fragment.to_owned()));
        }
        self.extraction.edges.push(EdgeRecord {
            source: link.owner_id.clone(),
            target,
            attributes,
        });
    }

    fn link_value(&self, link: &PendingLink, target: &str, kind: &str) -> Value {
        let mut value = self.link_attributes(link, kind);
        value.insert("source".to_owned(), Value::String(link.owner_id.clone()));
        value.insert("target".to_owned(), Value::String(target.to_owned()));
        Value::Object(value)
    }

    fn add_unresolved(&mut self, link: &PendingLink, reason: &str, target: &str) {
        if self.unresolved_links.len() < MAX_DIAGNOSTICS {
            let mut value = self.link_attributes(link, "unresolved");
            value.insert("source".to_owned(), Value::String(link.owner_id.clone()));
            value.insert("target".to_owned(), Value::String(target.to_owned()));
            value.insert("reason".to_owned(), Value::String(reason.to_owned()));
            self.unresolved_links.push(Value::Object(value));
        }
    }

    fn link_attributes(&self, link: &PendingLink, relation: &str) -> Map<String, Value> {
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
            Value::String(format!("L{}", link.site.line)),
        );
        attributes.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        stamp_source_range(
            &mut attributes,
            self.source,
            link.site.start_byte,
            link.site.end_byte,
        );
        attributes.insert("start_line".to_owned(), json!(link.site.line));
        attributes.insert(
            "end_line".to_owned(),
            json!(self.line_at(link.site.end_byte)),
        );
        attributes.insert("weight".to_owned(), json!(1.0));
        attributes.insert("link_kind".to_owned(), Value::String(link.kind.to_owned()));
        if let Some(rel) = link.rel.as_deref().filter(|rel| !rel.is_empty()) {
            attributes.insert("rel".to_owned(), Value::String(bounded_string(rel)));
        }
        attributes
    }

    fn add_relation(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        start: usize,
        end: usize,
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
            Value::String(format!("L{}", self.line_at(start))),
        );
        attributes.insert("_origin".to_owned(), Value::String("artifact".to_owned()));
        stamp_source_range(&mut attributes, self.source, start, end);
        attributes.insert("start_line".to_owned(), json!(self.line_at(start)));
        attributes.insert("end_line".to_owned(), json!(self.line_at(end)));
        attributes.insert("weight".to_owned(), json!(1.0));
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn append_visible_text(&mut self, text: &str) {
        if self.visible_text.len() >= MAX_VISIBLE_TEXT_BYTES {
            return;
        }
        let remaining = MAX_VISIBLE_TEXT_BYTES.saturating_sub(self.visible_text.len());
        self.visible_text.push_str(&truncate_bytes(text, remaining));
    }

    fn add_diagnostic(&mut self, text: impl Into<String>) {
        if self.diagnostics.len() < MAX_DIAGNOSTICS {
            self.diagnostics.push(text.into());
        }
    }

    fn mark_partial(&mut self) {
        self.extraction.extensions.insert(
            crate::EXTRACTION_QUALITY_EXTENSION.to_owned(),
            Value::String(crate::EXTRACTION_QUALITY_PARTIAL.to_owned()),
        );
        self.extraction.extensions.insert(
            crate::EXTRACTION_QUALITY_REASON_EXTENSION.to_owned(),
            Value::String("html_parser_recovery".to_owned()),
        );
    }

    fn link_site(&self, start: usize, end: usize) -> LinkSite {
        let start = start.min(self.source.len());
        let end = end.clamp(start, self.source.len());
        LinkSite {
            start_byte: start,
            end_byte: end,
            line: self.line_at(start),
        }
    }

    fn line_at(&self, offset: usize) -> usize {
        self.source[..offset.min(self.source.len())]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            .saturating_add(1)
    }
}

struct Renderer<'source> {
    source: &'source [u8],
    source_file: &'source str,
    diagnostics: Vec<String>,
    metadata: Map<String, Value>,
    title: String,
    base_href: Option<String>,
    rendered_bytes: usize,
}

impl Renderer<'_> {
    fn render_document(&mut self, root: Node<'_>) -> String {
        let mut output = String::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor).filter(Node::is_named) {
            self.render_node(child, &mut output, 0);
        }
        collapse_markdown(&output)
    }

    fn render_node(&mut self, node: Node<'_>, output: &mut String, depth: usize) {
        if node.kind() == "comment" || node.kind() == "doctype" {
            return;
        }
        if node.kind() == "text" || node.kind() == "entity" {
            self.push_rendered(output, &decode_html_entities(node_text(self.source, node)));
            return;
        }
        let (tag, start_tag) = match node.kind() {
            "element" => element_tag_name(node, self.source),
            "script_element" => (Some("script".to_owned()), None),
            "style_element" => (Some("style".to_owned()), None),
            _ => (None, None),
        };
        let Some(tag) = tag else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(Node::is_named) {
                self.render_node(child, output, depth);
            }
            return;
        };
        let tag = tag.to_ascii_lowercase();
        if is_hidden_tag(&tag) {
            return;
        }
        let attrs = collect_attributes(start_tag, self.source);
        if tag == "base" && self.base_href.is_none() {
            self.base_href = attrs.get("href").and_then(Value::as_str).map(str::to_owned);
        }
        if tag == "title" && self.title.is_empty() {
            self.title = bounded_string(&visible_text_for_node(node, self.source));
        }
        self.collect_renderer_metadata(&tag, &attrs);
        if let Some(level) = heading_level(&tag) {
            self.push_rendered(output, &format!("{} ", "#".repeat(level)));
            self.render_children(node, output, depth.saturating_add(1));
            self.push_rendered(output, "\n\n");
            return;
        }
        match tag.as_str() {
            "a" => {
                let start = output.len();
                self.render_children(node, output, depth.saturating_add(1));
                let label = output.get(start..).unwrap_or_default().trim().to_owned();
                if let Some(href) = attrs.get("href").and_then(Value::as_str) {
                    let href = self.resolve_href(href);
                    output.truncate(start);
                    self.push_rendered(output, &format!("[{label}]({href})"));
                }
                self.push_rendered(output, " ");
            }
            "br" => self.push_rendered(output, "\n"),
            "li" => {
                self.push_rendered(
                    output,
                    &format!("{}- ", "  ".repeat(depth.saturating_sub(1))),
                );
                self.render_children(node, output, depth.saturating_add(1));
                self.push_rendered(output, "\n");
            }
            "ul" | "ol" => {
                self.render_children(node, output, depth.saturating_add(1));
                self.push_rendered(output, "\n");
            }
            "blockquote" => {
                let start = output.len();
                self.render_children(node, output, depth.saturating_add(1));
                let text = output.get(start..).unwrap_or_default().trim().to_owned();
                output.truncate(start);
                for line in text.lines() {
                    self.push_rendered(output, &format!("> {line}\n"));
                }
                self.push_rendered(output, "\n");
            }
            "pre" => {
                let raw = raw_text_descendant(node, self.source);
                self.push_rendered(output, "```\n");
                self.push_rendered(output, &raw);
                self.push_rendered(output, "\n```\n\n");
            }
            "table" => {
                self.render_children(node, output, depth.saturating_add(1));
                self.push_rendered(output, "\n");
            }
            "tr" => {
                self.push_rendered(output, "| ");
                self.render_children(node, output, depth.saturating_add(1));
                self.push_rendered(output, "\n");
            }
            "td" | "th" => {
                self.render_children(node, output, depth.saturating_add(1));
                self.push_rendered(output, " | ");
            }
            "p" | "div" | "section" | "article" | "main" | "nav" => {
                self.render_children(node, output, depth.saturating_add(1));
                self.push_rendered(output, "\n\n");
            }
            "title" => {}
            _ => self.render_children(node, output, depth.saturating_add(1)),
        }
    }

    fn render_children(&mut self, node: Node<'_>, output: &mut String, depth: usize) {
        let mut cursor = node.walk();
        let mut previous_end = node.start_byte();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            if previous_end < child.start_byte()
                && self.source[previous_end..child.start_byte()]
                    .iter()
                    .all(u8::is_ascii_whitespace)
            {
                self.push_rendered(output, " ");
            }
            self.render_node(child, output, depth);
            previous_end = child.end_byte();
        }
    }

    fn collect_renderer_metadata(&mut self, tag: &str, attrs: &Map<String, Value>) {
        if tag == "meta"
            && let Some(key) = attrs
                .get("name")
                .or_else(|| attrs.get("property"))
                .or_else(|| attrs.get("charset"))
                .and_then(Value::as_str)
        {
            let value = attrs
                .get("content")
                .or_else(|| attrs.get("charset"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            self.metadata.insert(
                key.to_ascii_lowercase(),
                Value::String(bounded_string(value)),
            );
        }
        if tag == "link"
            && attrs.get("rel").and_then(Value::as_str).is_some_and(|rel| {
                rel.split_ascii_whitespace()
                    .any(|item| item.eq_ignore_ascii_case("canonical"))
            })
            && let Some(href) = attrs.get("href").and_then(Value::as_str)
        {
            self.metadata
                .insert("canonical".to_owned(), Value::String(bounded_string(href)));
        }
    }

    fn resolve_href(&self, href: &str) -> String {
        if href.starts_with('#') || (href.starts_with('/') && !self.source_file.starts_with("http"))
        {
            return href.to_owned();
        }
        let Ok(source_url) = Url::parse(self.source_file) else {
            return href.to_owned();
        };
        if !matches!(source_url.scheme(), "http" | "https") {
            return href.to_owned();
        }
        let base = self
            .base_href
            .as_deref()
            .and_then(|base| source_url.join(base).ok())
            .unwrap_or(source_url);
        base.join(href)
            .map(|url| url.to_string())
            .unwrap_or_else(|_| href.to_owned())
    }

    fn push_rendered(&mut self, output: &mut String, text: &str) {
        if self.rendered_bytes >= MAX_RENDERED_BYTES {
            return;
        }
        let remaining = MAX_RENDERED_BYTES.saturating_sub(self.rendered_bytes);
        let text = truncate_bytes(text, remaining);
        self.rendered_bytes = self.rendered_bytes.saturating_add(text.len());
        output.push_str(&text);
    }

    fn add_diagnostic(&mut self, text: &str) {
        if self.diagnostics.len() < MAX_DIAGNOSTICS {
            self.diagnostics.push(text.to_owned());
        }
    }
}

fn element_tag_name<'a>(node: Node<'a>, source: &[u8]) -> (Option<String>, Option<Node<'a>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(Node::is_named) {
        if matches!(child.kind(), "start_tag" | "self_closing_tag") {
            let mut tag_cursor = child.walk();
            for tag_child in child.children(&mut tag_cursor).filter(Node::is_named) {
                if tag_child.kind() == "tag_name" {
                    return (
                        Some(node_text(source, tag_child).to_ascii_lowercase()),
                        Some(child),
                    );
                }
            }
            return (None, Some(child));
        }
    }
    (None, None)
}

fn collect_attributes(start_tag: Option<Node<'_>>, source: &[u8]) -> Map<String, Value> {
    let mut values = Vec::new();
    let Some(start_tag) = start_tag else {
        return Map::new();
    };
    let mut cursor = start_tag.walk();
    for child in start_tag.children(&mut cursor).filter(Node::is_named) {
        if child.kind() != "attribute" {
            continue;
        }
        let mut name = None;
        let mut value = None;
        let mut inner = child.walk();
        for part in child.children(&mut inner).filter(Node::is_named) {
            match part.kind() {
                "attribute_name" => name = Some(node_text(source, part).to_ascii_lowercase()),
                "attribute_value" | "quoted_attribute_value" => {
                    value = Some(
                        decode_html_entities(node_text(source, part))
                            .trim_matches(['"', '\''])
                            .to_owned(),
                    )
                }
                _ => {}
            }
        }
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            values.push((name, bounded_string(value.as_deref().unwrap_or(""))));
        }
    }
    values.sort_by(|left, right| left.0.cmp(&right.0));
    values.truncate(MAX_ATTRIBUTES);
    let mut output = Map::new();
    for (name, value) in values {
        output.insert(name, Value::String(value));
    }
    output
}

fn visible_text_for_node(node: Node<'_>, source: &[u8]) -> String {
    let mut output = String::new();
    append_visible_node_text(node, source, &mut output);
    collapse_whitespace(&output)
}

fn append_visible_node_text(node: Node<'_>, source: &[u8], output: &mut String) {
    if node.kind() == "comment"
        || node.kind() == "doctype"
        || node.kind() == "script_element"
        || node.kind() == "style_element"
    {
        return;
    }
    if node.kind() == "element"
        && element_tag_name(node, source)
            .0
            .is_some_and(|tag| is_hidden_tag(&tag.to_ascii_lowercase()))
    {
        return;
    }
    if node.kind() == "text" || node.kind() == "entity" {
        output.push_str(&decode_html_entities(node_text(source, node)));
        return;
    }
    let mut cursor = node.walk();
    let mut previous_end = node.start_byte();
    for child in node.children(&mut cursor).filter(Node::is_named) {
        if previous_end < child.start_byte()
            && source[previous_end..child.start_byte()]
                .iter()
                .all(u8::is_ascii_whitespace)
        {
            output.push(' ');
        }
        append_visible_node_text(child, source, output);
        previous_end = child.end_byte();
    }
}

fn raw_text_descendant(node: Node<'_>, source: &[u8]) -> String {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(Node::is_named) {
        if child.kind() == "raw_text" {
            return node_text(source, child);
        }
        let nested = raw_text_descendant(child, source);
        if !nested.is_empty() {
            return nested;
        }
    }
    visible_text_for_node(node, source)
}

fn html_kind(tag: &str) -> &'static str {
    match tag {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "p" => "paragraph",
        "ul" | "ol" => "list",
        "li" => "list_item",
        "blockquote" => "blockquote",
        "pre" => "preformatted",
        "code" => "code",
        "table" => "table",
        "tr" => "table_row",
        "td" | "th" => "table_cell",
        "main" | "article" | "section" | "nav" => "landmark",
        "a" => "link",
        "title" => "title",
        "meta" => "metadata",
        "link" => "resource_link",
        "base" => "base_url",
        _ => "element",
    }
}

fn is_html_block_kind(kind: &str) -> bool {
    matches!(
        kind,
        "heading"
            | "paragraph"
            | "landmark"
            | "list"
            | "list_item"
            | "blockquote"
            | "preformatted"
            | "table"
            | "table_row"
            | "table_cell"
    )
}

fn heading_level(tag: &str) -> Option<usize> {
    match tag {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

fn is_hidden_tag(tag: &str) -> bool {
    matches!(tag, "script" | "style" | "template" | "noscript")
}

fn is_external_url(raw: &str) -> bool {
    raw.starts_with("//")
        || Url::parse(raw)
            .ok()
            .is_some_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

fn has_unsupported_scheme(raw: &str) -> bool {
    raw.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            })
    })
}

fn is_supported_local_link(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        extension.as_str(),
        "html"
            | "htm"
            | "md"
            | "markdown"
            | "mdx"
            | "qmd"
            | "skill"
            | "txt"
            | "rst"
            | "yaml"
            | "yml"
            | "json"
            | "docx"
            | "xlsx"
            | "pptx"
            | "rtf"
            | "pdf"
            | "py"
            | "rs"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "go"
            | "java"
            | "rb"
            | "php"
            | "c"
            | "cpp"
            | "h"
            | "cs"
            | "swift"
            | "kt"
            | "scala"
            | "sql"
    )
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(output.components().next_back(), Some(Component::RootDir)) {
                    output.pop();
                }
            }
            component => output.push(component.as_os_str()),
        }
    }
    output
}

fn node_text(source: &[u8], node: Node<'_>) -> String {
    let start = node.start_byte().min(source.len());
    let end = node.end_byte().clamp(start, source.len());
    String::from_utf8_lossy(&source[start..end]).into_owned()
}

fn bounded_string(value: &str) -> String {
    truncate_bytes(value, MAX_ATTRIBUTE_BYTES)
}

fn bounded_label(value: &str) -> String {
    let value = collapse_whitespace(value);
    if value.is_empty() {
        "HTML element".to_owned()
    } else {
        truncate_bytes(&value, 512)
    }
}

fn truncate_bytes(value: &str, max: usize) -> String {
    if value.len() <= max {
        return value.to_owned();
    }
    let mut end = max;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn collapse_whitespace(value: &str) -> String {
    let mut output = String::new();
    let mut pending_space = false;
    for character in value.chars() {
        if character.is_whitespace() {
            pending_space = !output.is_empty();
        } else {
            if pending_space && !output.ends_with(['\n', ' ']) {
                output.push(' ');
            }
            output.push(character);
            pending_space = false;
        }
    }
    output.trim().to_owned()
}

fn collapse_markdown(value: &str) -> String {
    let mut lines = Vec::new();
    let mut blank = false;
    for raw in value.lines() {
        let line = raw.trim_end();
        if line.is_empty() {
            if !blank {
                lines.push(String::new());
            }
            blank = true;
        } else {
            lines.push(line.to_owned());
            blank = false;
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

fn slugify(value: &str) -> String {
    crate::normalize_id(value)
}

fn decode_html_entities(value: String) -> String {
    let mut output = String::with_capacity(value.len());
    let mut rest = value.as_str();
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        let after = &rest[index..];
        let Some(end) = after.find(';') else {
            output.push_str(after);
            break;
        };
        let entity = &after[1..end];
        let decoded = decode_entity(entity).unwrap_or_else(|| after[..end + 1].to_owned());
        output.push_str(&decoded);
        rest = &after[end + 1..];
    }
    if !rest.is_empty() && !rest.contains('&') {
        output.push_str(rest);
    }
    output
}

fn decode_entity(entity: &str) -> Option<String> {
    if let Some(hex) = entity
        .strip_prefix("#x")
        .or_else(|| entity.strip_prefix("#X"))
    {
        return u32::from_str_radix(hex, 16)
            .ok()
            .and_then(char::from_u32)
            .map(|value| value.to_string());
    }
    if let Some(decimal) = entity.strip_prefix('#') {
        return decimal
            .parse::<u32>()
            .ok()
            .and_then(char::from_u32)
            .map(|value| value.to_string());
    }
    let value = match entity {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        "hellip" => '…',
        "ndash" => '–',
        "mdash" => '—',
        "copy" => '©',
        "reg" => '®',
        "trade" => '™',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "middot" => '·',
        _ => return None,
    };
    Some(value.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::{normalize_html, parse};
    use serde_json::Value;

    #[test]
    fn parser_is_pinned_and_renderer_skips_non_visible_elements() {
        let source = br#"<html><head><title>A &amp; B</title><style>hide</style></head><body><main><h1 id="top">Hello</h1><p>World <a href="/next" rel="next">next</a></p><script>bad()</script><template>template secret</template><noscript>noscript secret</noscript></main></body></html>"#;
        let normalized =
            normalize_html("https://example.com/start.html", source).expect("normalize");
        assert_eq!(normalized.title, "A & B", "{normalized:?}");
        assert!(normalized.markdown.contains("# Hello"));
        assert!(
            normalized
                .markdown
                .contains("[next](https://example.com/next)"),
            "{normalized:?}"
        );
        assert!(!normalized.markdown.contains("hide"));
        assert!(!normalized.markdown.contains("bad()"));
        assert!(!normalized.markdown.contains("template secret"));
        assert!(!normalized.markdown.contains("noscript secret"));
        assert!(
            !normalized
                .metadata
                .get("visible_text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("template secret")
        );
    }

    #[test]
    fn malformed_html_is_recoverable() {
        let tree = parse(b"<main><p>broken").expect("tree");
        assert!(tree.root_node().has_error());
    }
}
