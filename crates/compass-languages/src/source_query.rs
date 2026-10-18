//! Bounded lexical evidence from native callable scopes. No name resolution or graph edges.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use tree_sitter::Node;

mod parameters;
pub use parameters::{ParameterBindingOmission, ParameterBindingStatus, ParameterBindings};

/// Reserved per-file extraction metadata. This is query evidence, not semantic facts.
pub const EXTRACTION_EXTENSION: &str = "_compass_source_query_evidence";
pub const EXTRACTION_SCHEMA: &str = "compass.source-query.extraction/1";
pub const PROJECTION_PROFILE: &str = "source-query/6-parameter-bindings";

/// Byte columns and zero-based rows, matching Tree-sitter's source positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}
impl From<tree_sitter::Point> for Point {
    fn from(point: tree_sitter::Point) -> Self {
        Self {
            row: point.row,
            column: point.column,
        }
    }
}

/// Portable cached producer output. Consumers must bind it to the exact source
/// digest and a unique published callable definition range before indexing it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceQueryExtraction {
    pub schema: String,
    pub profile: String,
    pub source_file: String,
    pub source_digest: String,
    pub byte_size: u64,
    pub language: String,
    pub syntax_recovery: bool,
    pub omission: Option<String>,
    pub projection: Projection,
}

pub(crate) fn attach(
    extraction: &mut crate::Extraction,
    source_file: &str,
    language: &str,
    source: &[u8],
    root: Node<'_>,
) {
    if !matches!(language, "python" | "rust" | "go" | "typescript" | "tsx") {
        return;
    }
    let (projection, omission) = match project(root, source, language, Limits::default()) {
        Ok(projection) => (projection, None),
        Err(reason) => (
            Projection {
                truncated: true,
                ..Projection::default()
            },
            Some(reason.to_owned()),
        ),
    };
    let evidence = SourceQueryExtraction {
        schema: EXTRACTION_SCHEMA.to_owned(),
        profile: PROJECTION_PROFILE.to_owned(),
        source_file: source_file.to_owned(),
        source_digest: format!("sha256:{:x}", Sha256::digest(source)),
        byte_size: u64::try_from(source.len()).unwrap_or(u64::MAX),
        language: language.to_owned(),
        syntax_recovery: root.has_error(),
        omission,
        projection,
    };
    match serde_json::to_value(evidence) {
        Ok(value) => {
            extraction
                .extensions
                .insert(EXTRACTION_EXTENSION.to_owned(), value);
        }
        Err(error) => {
            extraction.error = Some(format!(
                "source-query evidence serialization failed: {error}"
            ));
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    pub file_bytes: usize,
    pub visits: usize,
    pub depth: usize,
    pub scopes: usize,
    pub names_per_scope: usize,
    pub sites_per_name: usize,
    pub comments_per_scope: usize,
    pub comment_bytes: usize,
    pub retained_bytes: usize,
    pub code_visits: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 4 * 1024 * 1024,
            visits: 200_000,
            depth: 512,
            scopes: 512,
            names_per_scope: 256,
            // Recall needs each spelling once, not repeated occurrences that
            // spend the publication site budget without adding searchable terms.
            // Code windows are collected independently before this site limit.
            sites_per_name: 1,
            comments_per_scope: 8,
            comment_bytes: 4096,
            retained_bytes: 1024 * 1024,
            code_visits: 200_000,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Site {
    pub start: usize,
    pub end: usize,
    pub start_point: Point,
    pub end_point: Point,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    pub name: Option<String>,
    pub syntax: String,
    pub start: usize,
    pub end: usize,
    /// Explicit native AST ancestry; class/type barriers leave no parent.
    pub parent_scope: Option<usize>,
    /// Actual callable syntax, distinct from a variable definition envelope.
    pub callable_start: usize,
    pub callable_end: usize,
    pub start_point: Point,
    pub end_point: Point,
    pub names: BTreeMap<String, Vec<Site>>,
    pub comments: Vec<Site>,
    pub parameter_bindings: ParameterBindings,
    #[serde(default)]
    pub code_windows: Vec<Site>,
    #[serde(default)]
    pub omitted_code_windows: u64,
    pub omitted_sites: usize,
    pub truncated: bool,
    #[serde(skip)]
    name_span: Option<(usize, usize)>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Projection {
    pub scopes: Vec<Scope>,
    pub visits: usize,
    pub retained_bytes: usize,
    pub truncated: bool,
    #[serde(default)]
    pub retained_code_bytes: usize,
    #[serde(default)]
    pub retained_parameter_bytes: usize,
}
fn callable(language: &str, kind: &str) -> bool {
    match language {
        "python" => matches!(kind, "function_definition" | "lambda"),
        "rust" => matches!(
            kind,
            "function_item" | "function_signature_item" | "closure_expression"
        ),
        "go" => matches!(
            kind,
            "function_declaration" | "method_declaration" | "func_literal"
        ),
        "typescript" | "tsx" => matches!(
            kind,
            "function_declaration"
                | "function_signature"
                | "method_definition"
                | "arrow_function"
                | "function_expression"
                | "generator_function_declaration"
                | "generator_function"
        ),
        _ => false,
    }
}

fn definition_node<'tree>(language: &str, node: Node<'tree>) -> Node<'tree> {
    if !matches!(language, "typescript" | "tsx") {
        return node;
    }
    let Some(parent) = node.parent().filter(|parent| {
        parent.kind() == "variable_declarator"
            && !parent.has_error()
            && !parent.is_missing()
            && parent.end_byte() == node.end_byte()
            && parent
                .child_by_field_name("value")
                .is_some_and(|value| value.id() == node.id())
            && parent.child_by_field_name("name").is_some_and(|name| {
                name.kind() == "identifier" && name.start_byte() == parent.start_byte()
            })
    }) else {
        return node;
    };
    parent
}

fn barrier(kind: &str) -> bool {
    matches!(
        kind,
        "class_definition"
            | "class_declaration"
            | "interface_declaration"
            | "trait_item"
            | "impl_item"
            | "enum_item"
            | "struct_item"
            | "type_declaration"
            | "type_alias_declaration"
            | "type_item"
    )
}
fn identifier(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "property_identifier"
            | "private_property_identifier"
            | "type_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern"
    )
}
fn comment(kind: &str) -> bool {
    matches!(kind, "comment" | "line_comment" | "block_comment")
}
fn site(node: Node<'_>, source: &[u8], maximum: usize) -> Option<Site> {
    let raw = std::str::from_utf8(source.get(node.byte_range())?).ok()?;
    let mut end = raw.len().min(maximum);
    while !raw.is_char_boundary(end) {
        end = end.checked_sub(1)?;
    }
    if end == 0 {
        return None;
    }
    let text = raw.get(..end)?.to_owned();
    let mut point = Point::from(node.start_position());
    for byte in text.bytes() {
        if byte == b'\n' {
            point.row += 1;
            point.column = 0;
        } else {
            point.column += 1;
        }
    }
    Some(Site {
        start: node.start_byte(),
        end: node.start_byte() + end,
        start_point: Point::from(node.start_position()),
        end_point: point,
        text,
    })
}
/// Largest small enclosing syntax node whose entire subtree belongs to this
/// callable. A nested callable/type, parser recovery, or scan limit rejects the
/// larger window. Exact bytes come from extraction, never a query-time file.
fn code_window(
    node: Node<'_>,
    source: &[u8],
    language: &str,
    visits_left: &mut usize,
) -> Option<Site> {
    let mut selected = None;
    let mut parent = node.parent();
    for _ in 0..8 {
        let Some(candidate) = parent else { break };
        if callable(language, candidate.kind())
            || barrier(candidate.kind())
            || candidate.end_byte().saturating_sub(candidate.start_byte()) > 512
        {
            break;
        }
        let mut cursor = candidate.walk();
        let mut safe = true;
        let mut local_visits = 0;
        loop {
            if *visits_left == 0 || local_visits >= 128 {
                safe = false;
                break;
            }
            *visits_left -= 1;
            local_visits += 1;
            let child = cursor.node();
            if child.has_error()
                || child.is_missing()
                || callable(language, child.kind())
                || barrier(child.kind())
            {
                safe = false;
                break;
            }
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    break;
                }
            }
            if cursor.node() == candidate {
                break;
            }
        }
        if !safe {
            break;
        }
        selected = site(candidate, source, 512);
        parent = candidate.parent();
    }
    selected
}

/// Input and traversal limits are independent of the graph's dependency model.
/// Each nested callable has its own owner; class/type scopes block inheritance.
/// The caller supplies the already parsed native tree and original source bytes.
fn project(
    root: Node<'_>,
    source: &[u8],
    language: &str,
    limits: Limits,
) -> Result<Projection, &'static str> {
    if source.len() > limits.file_bytes {
        return Err("source-query file byte limit exceeded");
    }
    if std::str::from_utf8(source).is_err() {
        return Err("source-query evidence requires decoded UTF-8 source");
    }
    if !matches!(language, "python" | "rust" | "go" | "typescript" | "tsx") {
        return Err("unsupported source-query projection language");
    }
    if root.end_byte() > source.len() {
        return Err("source-query AST range exceeds original source");
    }
    let mut result = Projection::default();
    let mut code_visits_left = limits.code_visits;
    let mut parameter_roots = Vec::new();
    let mut cursor = root.walk();
    let mut owner = None::<usize>;
    let mut ancestors = Vec::<Option<usize>>::new();
    loop {
        if result.visits >= limits.visits {
            result.truncated = true;
            break;
        }
        result.visits += 1;
        let node = cursor.node();
        let mut child_owner = owner;
        if barrier(node.kind()) {
            child_owner = None;
        }
        if node.is_named() && callable(language, node.kind()) {
            child_owner = None;
            if node.has_error() || node.is_missing() || result.scopes.len() >= limits.scopes {
                result.truncated = true;
            } else {
                let name_node = node.child_by_field_name("name");
                let raw_name = name_node
                    .and_then(|name| std::str::from_utf8(source.get(name.byte_range())?).ok());
                let name_too_large = raw_name.is_some_and(|name| name.len() > 256);
                let name = raw_name.filter(|name| name.len() <= 256).map(str::to_owned);
                let definition = definition_node(language, node);
                let at = result.scopes.len();
                result.scopes.push(Scope {
                    name,
                    syntax: node.kind().to_owned(),
                    start: definition.start_byte(),
                    end: definition.end_byte(),
                    parent_scope: owner,
                    callable_start: node.start_byte(),
                    callable_end: node.end_byte(),
                    start_point: Point::from(node.start_position()),
                    end_point: Point::from(node.end_position()),
                    names: BTreeMap::new(),
                    comments: Vec::new(),
                    parameter_bindings: ParameterBindings::default(),
                    code_windows: Vec::new(),
                    omitted_code_windows: 0,
                    omitted_sites: 0,
                    truncated: name_too_large,
                    name_span: name_node.map(|name| (name.start_byte(), name.end_byte())),
                });
                parameter_roots.push(node);
                child_owner = Some(at);
            }
        }
        if let Some(at) = child_owner {
            let scope = &mut result.scopes[at];
            if identifier(node.kind())
                && scope.name_span != Some((node.start_byte(), node.end_byte()))
            {
                // Code-window quotas are independent of identifier/comment
                // quotas. Profiles 1–4 keep windows inspection-only; profile 5
                // also indexes their retained novel vocabulary.
                let covered = scope.code_windows.iter().any(|window| {
                    window.start <= node.start_byte() && node.end_byte() <= window.end
                });
                if !covered {
                    if scope.code_windows.len() >= 16
                        || result.retained_code_bytes >= 256 * 1024
                        || code_visits_left == 0
                    {
                        scope.omitted_code_windows = scope.omitted_code_windows.saturating_add(1);
                    } else {
                        let window = code_window(node, source, language, &mut code_visits_left);
                        if let Some(window) = window
                            && window.start >= scope.start
                            && window.end <= scope.end
                            && !scope
                                .code_windows
                                .iter()
                                .any(|prior| prior.start == window.start && prior.end == window.end)
                        {
                            if result.retained_code_bytes.saturating_add(window.text.len())
                                <= 256 * 1024
                            {
                                result.retained_code_bytes += window.text.len();
                                scope.code_windows.push(window);
                            } else {
                                scope.omitted_code_windows =
                                    scope.omitted_code_windows.saturating_add(1);
                            }
                        } else if code_visits_left == 0 {
                            scope.omitted_code_windows =
                                scope.omitted_code_windows.saturating_add(1);
                        }
                    }
                }
                if let Some(value) = site(node, source, 256) {
                    // Never keep a truncated identifier as a different spelling.
                    if value.end != node.end_byte()
                        || (!scope.names.contains_key(&value.text)
                            && scope.names.len() >= limits.names_per_scope)
                    {
                        scope.truncated = true;
                        scope.omitted_sites += 1;
                    } else if scope
                        .names
                        .get(&value.text)
                        .is_some_and(|sites| sites.len() >= limits.sites_per_name)
                    {
                        scope.omitted_sites += 1;
                    } else if result.retained_bytes.saturating_add(value.text.len())
                        > limits.retained_bytes
                    {
                        scope.truncated = true;
                        scope.omitted_sites += 1;
                        result.truncated = true;
                    } else {
                        result.retained_bytes += value.text.len();
                        scope
                            .names
                            .entry(value.text.clone())
                            .or_default()
                            .push(value);
                    }
                }
            } else if comment(node.kind()) {
                if scope.comments.len() >= limits.comments_per_scope {
                    scope.truncated = true;
                    scope.omitted_sites += 1;
                } else if let Some(value) = site(node, source, limits.comment_bytes) {
                    if result.retained_bytes.saturating_add(value.text.len())
                        > limits.retained_bytes
                    {
                        scope.truncated = true;
                        scope.omitted_sites += 1;
                        result.truncated = true;
                    } else {
                        scope.truncated |= value.end != node.end_byte();
                        result.retained_bytes += value.text.len();
                        scope.comments.push(value);
                    }
                }
            }
        }
        if ancestors.len() < limits.depth && cursor.goto_first_child() {
            ancestors.push(owner);
            owner = child_owner;
            continue;
        }
        if ancestors.len() >= limits.depth && node.child_count() > 0 {
            result.truncated = true;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                if result.truncated {
                    for scope in &mut result.scopes {
                        scope.truncated = true;
                    }
                }
                return Ok(finish_parameters(
                    result,
                    &parameter_roots,
                    source,
                    language,
                    limits,
                ));
            }
            owner = ancestors.pop().flatten();
        }
    }
    for scope in &mut result.scopes {
        scope.truncated = true;
    }
    Ok(finish_parameters(
        result,
        &parameter_roots,
        source,
        language,
        limits,
    ))
}

fn finish_parameters(
    mut projection: Projection,
    roots: &[Node<'_>],
    source: &[u8],
    language: &str,
    limits: Limits,
) -> Projection {
    let mut visits = limits.visits.saturating_sub(projection.visits);
    let initial_visits = visits;
    let mut text = limits
        .retained_bytes
        .saturating_sub(projection.retained_bytes)
        .min(parameters::MAX_PARAMETER_FILE_BYTES);
    let initial_text = text;
    for (scope, root) in projection.scopes.iter_mut().zip(roots) {
        scope.parameter_bindings =
            parameters::collect(*root, source, language, &mut visits, &mut text);
    }
    projection.visits += initial_visits - visits;
    projection.retained_parameter_bytes = initial_text - text;
    projection
}

#[cfg(test)]
mod tests {
    use super::{Limits, project};
    fn parse(
        source: &str,
        language: &str,
    ) -> Result<tree_sitter::Tree, Box<dyn std::error::Error>> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_language_pack::get_language(language)?)?;
        let tree = parser.parse(source, None).ok_or("no syntax tree")?;
        if tree.root_node().has_error() {
            return Err(format!("malformed test fixture: {}", tree.root_node().to_sexp()).into());
        }
        Ok(tree)
    }
    #[test]
    fn parameter_spelling_support_is_explicit_and_keeps_source_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, source, expected) in [
            (
                "python",
                "def probe(café, e\u{301}clair):\n    pass\n",
                vec!["café", "e\u{301}clair"],
            ),
            ("rust", "fn probe(r#type: Value) {}", vec!["r#type"]),
            (
                "go",
                "package p\nfunc probe(κλειδί Value) {}",
                vec!["κλειδί"],
            ),
            (
                "typescript",
                "function probe($block: Value) {}",
                vec!["$block"],
            ),
            (
                "tsx",
                "const probe = ($permit: Value) => <Panel/>;",
                vec!["$permit"],
            ),
            ("python", "def probe(_):\n    return _\n", vec!["_"]),
            (
                "typescript",
                "function probe(_: Value) { return _; }",
                vec!["_"],
            ),
            (
                "tsx",
                "const probe = (_: Value) => <Panel item={_}/>;",
                vec!["_"],
            ),
            ("go", "package p\nfunc probe(_ Value) {}", vec![]),
            ("rust", "fn probe(_: Value) {}", vec![]),
        ] {
            let tree = parse(source, language)?;
            let p = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let bindings = &p
                .scopes
                .first()
                .ok_or("missing callable")?
                .parameter_bindings;
            assert_eq!(
                bindings.status,
                super::ParameterBindingStatus::Complete,
                "{language}"
            );
            assert_eq!(
                bindings
                    .sites
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{language}"
            );
            for site in &bindings.sites {
                assert_eq!(
                    &source.as_bytes()[site.start..site.end],
                    site.text.as_bytes()
                );
            }
        }
        let source = "function probe(\\u0062lock: Value) {}";
        let tree = parse(source, "typescript")?;
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "typescript",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        let bindings = &p
            .scopes
            .first()
            .ok_or("missing callable")?
            .parameter_bindings;
        assert!(bindings.sites.is_empty());
        assert_eq!(bindings.status, super::ParameterBindingStatus::Partial);
        assert_eq!(bindings.omitted_sites, 1);
        assert!(
            bindings
                .reasons
                .contains(&super::ParameterBindingOmission::UnsupportedSpelling)
        );
        Ok(())
    }

    #[test]
    fn parameter_binding_sites_are_typed_and_separate_from_identifier_dedup()
    -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (
                "python",
                "def probe(x=use(block), block: Permit=0, *rest, **kwargs):\n    return block\n",
                vec!["x", "block", "rest", "kwargs"],
            ),
            (
                "rust",
                "fn probe(mut block: Block, permit: &Permit) {}",
                vec!["block", "permit"],
            ),
            (
                "go",
                "package p\nfunc (r *Receiver) probe(block, permit Block, permits ...Permit) (result Result) { return }",
                vec!["block", "permit", "permits"],
            ),
            (
                "typescript",
                "function probe(block: Block = fallback, permit?: Permit, ...rest: Item[]) {}",
                vec!["block", "permit", "rest"],
            ),
            (
                "tsx",
                "const probe = (block: Block, permit = fallback) => block;",
                vec!["block", "permit"],
            ),
        ];
        let mut rows = Vec::new();
        for (language, source, expected) in cases {
            let tree = parse(source, language)?;
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            rows.push((
                language,
                source,
                expected,
                serde_json::to_value(projection)?,
            ));
        }
        for (language, source, expected, value) in rows {
            let bindings = value["scopes"][0]
                .get("parameterBindings")
                .ok_or("typed parameter bindings absent")?;
            assert_eq!(bindings["status"], "complete", "{language}: {bindings}");
            let sites = bindings["sites"]
                .as_array()
                .ok_or("parameter sites missing")?;
            assert_eq!(
                sites
                    .iter()
                    .map(|s| s["text"].as_str().unwrap_or_default())
                    .collect::<Vec<_>>(),
                expected,
                "{language}"
            );
            for site in sites {
                let start = site["start"].as_u64().ok_or("missing start")? as usize;
                let end = site["end"].as_u64().ok_or("missing end")? as usize;
                assert_eq!(
                    &source[start..end],
                    site["text"].as_str().ok_or("missing text")?
                );
            }
            if language == "python" {
                let generic = &value["scopes"][0]["names"]["block"][0];
                let binding = sites
                    .iter()
                    .find(|s| s["text"] == "block")
                    .ok_or("missing block binding")?;
                assert!(
                    generic["start"].as_u64() < binding["start"].as_u64(),
                    "default reference must precede real binding"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn parameter_coverage_preserves_lexical_priority_and_reports_unknown_suffixes()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{ParameterBindingOmission as R, ParameterBindingStatus as S};
        let source = "def probe(block):\n    return block\n";
        let tree = parse(source, "python")?;
        let full = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        let no_spare = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits {
                retained_bytes: full.retained_bytes,
                ..Limits::default()
            },
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(full.scopes[0].names, no_spare.scopes[0].names);
        assert!(!no_spare.truncated && !no_spare.scopes[0].truncated);
        assert_eq!(no_spare.scopes[0].parameter_bindings.status, S::Partial);
        assert!(
            no_spare.scopes[0]
                .parameter_bindings
                .reasons
                .contains(&R::TextLimit)
        );
        let no_visits = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits {
                visits: full.visits - 3,
                ..Limits::default()
            },
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(
            no_visits.scopes[0].parameter_bindings.status,
            S::Unsupported
        );
        assert!(
            no_visits.scopes[0]
                .parameter_bindings
                .reasons
                .contains(&R::TraversalLimit)
        );
        assert!(no_visits.scopes[0].parameter_bindings.sites.is_empty());
        let source = format!(
            "def probe({}):\n    pass\n",
            (0..66)
                .map(|i| format!("arg{i}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let tree = parse(&source, "python")?;
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(p.scopes[0].parameter_bindings.sites.len(), 64);
        assert_eq!(p.scopes[0].parameter_bindings.omitted_sites, 2);
        assert_eq!(p.scopes[0].parameter_bindings.status, S::Partial);
        assert!(
            p.scopes[0]
                .parameter_bindings
                .reasons
                .contains(&R::SiteLimit)
        );
        let source = format!("def probe({}):\n    pass\n", "é".repeat(129));
        let tree = parse(&source, "python")?;
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        assert!(p.scopes[0].parameter_bindings.sites.is_empty());
        assert_eq!(p.scopes[0].parameter_bindings.omitted_sites, 1);
        assert!(
            p.scopes[0]
                .parameter_bindings
                .reasons
                .contains(&R::TextLimit)
        );
        Ok(())
    }

    #[test]
    fn parameter_roles_exclude_types_defaults_returns_and_nested_bindings()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::ParameterBindingStatus as S;
        for (language, source, expected, partial) in [
            (
                "python",
                "def probe(value: Block = factory(permit)) -> Return:\n    local = value\n    return local\n",
                vec!["value"],
                false,
            ),
            (
                "python",
                "probe = lambda block=default: block",
                vec!["block"],
                false,
            ),
            (
                "rust",
                "fn probe(callback: fn(block: Block) -> Permit) {}",
                vec!["callback"],
                false,
            ),
            (
                "rust",
                "fn probe((block, permit): (Block, Permit)) {}",
                vec![],
                true,
            ),
            (
                "rust",
                "impl T { fn probe(&self, block: Block) {} }",
                vec!["block"],
                true,
            ),
            (
                "rust",
                "fn outer() { let probe = |block: Block, permit| block; }",
                vec![],
                false,
            ),
            (
                "go",
                "package p\nfunc probe(Block, Permit) (result Result) { return }",
                vec![],
                false,
            ),
            (
                "typescript",
                "function probe(callback: (block: Block) => Permit) {}",
                vec!["callback"],
                false,
            ),
            (
                "typescript",
                "const probe = block => block;",
                vec!["block"],
                false,
            ),
            (
                "tsx",
                "const probe = (block: Block = fallback) => <Panel item={block}/>;",
                vec!["block"],
                false,
            ),
            (
                "typescript",
                "function probe({block: permit, shorthand}: Params) {}",
                vec![],
                true,
            ),
        ] {
            let tree = parse(source, language)?;
            let p = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let owner = p.scopes.first().ok_or("missing role scope")?;
            assert_eq!(
                owner
                    .parameter_bindings
                    .sites
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{language}: {source}"
            );
            assert_eq!(
                owner.parameter_bindings.status == S::Partial,
                partial,
                "{language}: {source}"
            );
            if source.contains("let probe = |") {
                assert_eq!(
                    p.scopes[1]
                        .parameter_bindings
                        .sites
                        .iter()
                        .map(|s| s.text.as_str())
                        .collect::<Vec<_>>(),
                    ["block", "permit"]
                );
            }
        }
        Ok(())
    }

    #[test]
    fn default_occurrence_budget_preserves_terms_and_code_without_duplicate_sites()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, source) in [
            (
                "python",
                "def opaque(payload):\n    value = payload\n    # retained context\n    return value + payload + value + payload\n",
            ),
            (
                "rust",
                "fn opaque(payload: i32) -> i32 { let value = payload; // retained context\nvalue + payload + value + payload }",
            ),
            (
                "go",
                "package p\nfunc opaque(payload int) int { value := payload; // retained context\nreturn value + payload + value + payload }",
            ),
            (
                "typescript",
                "function opaque(payload: number) { const value = payload; // retained context\nreturn value + payload + value + payload; }",
            ),
            (
                "tsx",
                "function opaque(payload: number) { const value = payload; // retained context\nreturn value + payload + value + payload; }",
            ),
        ] {
            let tree = parse(source, language)?;
            let compact = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let repeated = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits {
                    sites_per_name: 4,
                    ..Limits::default()
                },
            )
            .map_err(std::io::Error::other)?;
            let owner = compact
                .scopes
                .iter()
                .find(|s| s.name.as_deref() == Some("opaque"))
                .ok_or("missing compact owner")?;
            let prior = repeated
                .scopes
                .iter()
                .find(|s| s.name.as_deref() == Some("opaque"))
                .ok_or("missing repeated owner")?;
            assert!(
                owner.names.values().all(|sites| sites.len() == 1),
                "{language}"
            );
            assert_eq!(
                owner.names.keys().collect::<Vec<_>>(),
                prior.names.keys().collect::<Vec<_>>(),
                "{language}"
            );
            for (name, sites) in &owner.names {
                assert_eq!(
                    sites.first(),
                    prior.names[name].first(),
                    "{language}: {name}"
                );
            }
            assert!(
                compact.retained_bytes < repeated.retained_bytes,
                "{language}"
            );
            assert!(owner.omitted_sites > prior.omitted_sites, "{language}");
            assert!(!owner.truncated && !compact.truncated, "{language}");
            assert_eq!(owner.comments, prior.comments, "{language}");
            assert_eq!(owner.code_windows, prior.code_windows, "{language}");
            assert_eq!(
                owner.omitted_code_windows, prior.omitted_code_windows,
                "{language}"
            );
        }
        Ok(())
    }
    #[test]
    fn native_scopes_keep_parameters_and_comments_without_borrowing_nested_bodies()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, source, outer) in [
            (
                "python",
                "# outside_marker\ndef opaque(payload):\n    annotations = payload\n    # retained marker\n    def inner():\n        return secret\n    return annotations\n",
                "opaque",
            ),
            (
                "rust",
                "// outside_marker\nfn opaque(payload: &str) { let annotations = payload; // retained marker\nfn inner() { let secret = 0; } }",
                "opaque",
            ),
            (
                "go",
                "package p\n// outside_marker\nfunc Opaque(payload string) { annotations := payload; // retained marker\ninner := func() { secret() }; _ = annotations; _ = inner }",
                "Opaque",
            ),
            (
                "typescript",
                "// outside_marker\nexport function opaque(payload: unknown) { const annotations = payload; // retained marker\nfunction inner() { const secret = 0; } return annotations; }",
                "opaque",
            ),
        ] {
            let tree = parse(source, language)?;
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            assert!(!projection.truncated, "{language}");
            let scope = projection
                .scopes
                .iter()
                .find(|scope| scope.name.as_deref() == Some(outer))
                .ok_or("missing outer scope")?;
            assert!(scope.names.contains_key("payload"), "{language}");
            assert!(scope.names.contains_key("annotations"), "{language}");
            assert!(!scope.names.contains_key("secret"), "{language}");
            assert!(
                !scope.names.contains_key(outer),
                "do not duplicate declaration name"
            );
            assert!(
                scope
                    .comments
                    .iter()
                    .any(|site| site.text.contains("retained marker")),
                "{language}"
            );
            assert!(
                !scope
                    .comments
                    .iter()
                    .any(|site| site.text.contains("outside_marker")),
                "{language}"
            );
            assert!(
                projection
                    .scopes
                    .iter()
                    .any(|scope| scope.names.contains_key("secret")),
                "nested owner missing: {language}"
            );
            for scope in &projection.scopes {
                for site in scope.names.values().flatten().chain(&scope.comments) {
                    assert_eq!(source.get(site.start..site.end), Some(site.text.as_str()));
                    assert!(scope.start <= site.start && site.end <= scope.end);
                }
            }
        }
        Ok(())
    }
    #[test]
    fn covered_sites_do_not_count_as_omitted_when_scan_budget_ends()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = "def opaque(items):\n    return items + items\n";
        let tree = parse(source, "python")?;
        // This fixture has three physical occurrences. Checking only retained
        // representative sites would mistake term coverage for code coverage.
        let occurrences = source
            .match_indices("items")
            .map(|(start, text)| (start, start + text.len()))
            .collect::<Vec<_>>();
        assert_eq!(occurrences.len(), 3);
        let mut complete_cases = 0;
        let mut representative_only_cases = 0;
        for code_visits in 0..=256 {
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                "python",
                Limits {
                    code_visits,
                    ..Limits::default()
                },
            )
            .map_err(std::io::Error::other)?;
            let owner = projection.scopes.first().ok_or("owner")?;
            let covered = occurrences.iter().all(|&(start, end)| {
                owner
                    .code_windows
                    .iter()
                    .any(|window| window.start <= start && end <= window.end)
            });
            if covered {
                complete_cases += 1;
                assert_eq!(
                    owner.omitted_code_windows, 0,
                    "all sites covered with scan budget {code_visits}"
                );
            } else if owner.names.values().flatten().all(|site| {
                owner
                    .code_windows
                    .iter()
                    .any(|window| window.start <= site.start && site.end <= window.end)
            }) {
                representative_only_cases += 1;
                assert!(owner.omitted_code_windows > 0);
            }
        }
        assert!(complete_cases > 0);
        assert!(representative_only_cases > 0);
        Ok(())
    }

    #[test]
    fn code_window_quota_does_not_drop_lexical_identifiers()
    -> Result<(), Box<dyn std::error::Error>> {
        let body = (0..80)
            .map(|at| format!("    normalize_payload_{at}(items)\n"))
            .collect::<String>();
        let source = format!("def opaque(items):\n{body}");
        let tree = parse(&source, "python")?;
        let projection = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        let owner = projection.scopes.first().ok_or("owner")?;
        assert_eq!(owner.code_windows.len(), 16);
        assert!(owner.omitted_code_windows > 0);
        assert!(owner.names.contains_key("normalize_payload_79"));
        assert!(!owner.truncated);
        assert!(!projection.truncated);
        Ok(())
    }

    #[test]
    fn code_windows_are_exact_and_cannot_borrow_nested_or_sibling_implementations()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, source, outer) in [
            (
                "python",
                "def opaque(items):\n    result = normalize_payload(items)\n    def nested():\n        secret_nested()\n    return result\ndef sibling():\n    secret_sibling()\n",
                "opaque",
            ),
            (
                "rust",
                "fn opaque(items: usize) { normalize_payload(items); fn nested() { secret_nested(); } } fn sibling() { secret_sibling(); }",
                "opaque",
            ),
            (
                "go",
                "package p\nfunc Opaque(items int) { normalize_payload(items); nested := func() { secret_nested() }; _ = nested }\nfunc sibling() { secret_sibling() }",
                "Opaque",
            ),
            (
                "typescript",
                "function opaque(items: unknown) { normalize_payload(items); class Nested { run() { secret_nested(); } } } function sibling() { secret_sibling(); }",
                "opaque",
            ),
        ] {
            let tree = parse(source, language)?;
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let scope = projection
                .scopes
                .iter()
                .find(|scope| scope.name.as_deref() == Some(outer))
                .ok_or("missing owner")?;
            assert!(
                scope
                    .code_windows
                    .iter()
                    .any(|window| window.text.contains("normalize_payload(items)")),
                "{language}"
            );
            for window in &scope.code_windows {
                assert_eq!(
                    source.get(window.start..window.end),
                    Some(window.text.as_str())
                );
                assert!(scope.start <= window.start && window.end <= scope.end);
                assert!(
                    !window.text.contains("secret_nested"),
                    "{language}: {}",
                    window.text
                );
                assert!(!window.text.contains("secret_sibling"));
                assert!(window.text.len() <= 512);
            }
            assert!(projection.retained_code_bytes <= 256 * 1024);
        }
        Ok(())
    }

    #[test]
    fn classes_block_outer_ownership_and_strings_are_not_lexical_identifiers()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = "function opaque(payload: unknown) { const annotations = 'secret_literal'; class Nested { x = classOnly; meth() { methodOnly(); } } return payload; }";
        let tree = parse(source, "typescript")?;
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "typescript",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        let outer = p
            .scopes
            .iter()
            .find(|scope| scope.name.as_deref() == Some("opaque"))
            .ok_or("missing scope")?;
        for term in ["secret_literal", "classOnly", "methodOnly", "Nested"] {
            assert!(!outer.names.contains_key(term), "{term}");
        }
        assert!(
            p.scopes
                .iter()
                .any(|scope| scope.name.as_deref() == Some("meth")
                    && scope.names.contains_key("methodOnly"))
        );
        Ok(())
    }
    #[test]
    fn unicode_ranges_and_quotas_remain_explicit() -> Result<(), Box<dyn std::error::Error>> {
        let source = "function opaque(payload: unknown) { const café = payload; // 雪 retained\nreturn café; }";
        let tree = parse(source, "typescript")?;
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "typescript",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        assert!(p.scopes[0].names.contains_key("café"));
        for site in p.scopes[0]
            .names
            .values()
            .flatten()
            .chain(&p.scopes[0].comments)
        {
            assert_eq!(source.get(site.start..site.end), Some(site.text.as_str()));
        }
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "typescript",
            Limits {
                visits: 1,
                ..Limits::default()
            },
        )
        .map_err(std::io::Error::other)?;
        assert!(p.truncated && p.visits == 1);
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "typescript",
            Limits {
                names_per_scope: 1,
                ..Limits::default()
            },
        )
        .map_err(std::io::Error::other)?;
        assert!(p.scopes[0].truncated && p.scopes[0].names.len() == 1);
        assert!(
            project(
                tree.root_node(),
                source.as_bytes(),
                "typescript",
                Limits {
                    file_bytes: 1,
                    ..Limits::default()
                }
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn independent_projection_limits_disclose_omissions() -> Result<(), Box<dyn std::error::Error>>
    {
        let source = "function opaque(payload: unknown) { // retained marker\nreturn payload + payload; } function other() {}";
        let tree = parse(source, "typescript")?;
        for limits in [
            Limits {
                depth: 1,
                ..Limits::default()
            },
            Limits {
                scopes: 1,
                ..Limits::default()
            },
            Limits {
                retained_bytes: 1,
                ..Limits::default()
            },
        ] {
            let projection = project(tree.root_node(), source.as_bytes(), "typescript", limits)
                .map_err(std::io::Error::other)?;
            assert!(projection.truncated);
        }
        let projection = project(
            tree.root_node(),
            source.as_bytes(),
            "typescript",
            Limits {
                comments_per_scope: 0,
                ..Limits::default()
            },
        )
        .map_err(std::io::Error::other)?;
        assert!(projection.scopes[0].truncated && projection.scopes[0].omitted_sites > 0);
        assert!(projection.scopes[0].comments.is_empty());
        let projection = project(
            tree.root_node(),
            source.as_bytes(),
            "typescript",
            Limits {
                sites_per_name: 1,
                ..Limits::default()
            },
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(projection.scopes[0].names["payload"].len(), 1);
        assert!(projection.scopes[0].omitted_sites >= 2);
        // Only repeated source occurrences were omitted; term coverage survives.
        assert!(!projection.scopes[0].truncated);
        Ok(())
    }

    #[test]
    fn native_parent_facts_and_callable_ranges_are_explicit_in_five_languages()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, source) in [
            (
                "python",
                "def opaque(payload):\n    callback = lambda item: (lambda other: payload_error(other))(item)\n    return callback\ndef sibling():\n    return 0\n",
            ),
            (
                "rust",
                "fn opaque(payload: usize) { let callback = |item: usize| { let inner = |other: usize| payload_error(other); inner(item) }; } fn sibling() {}",
            ),
            (
                "go",
                "package p\nfunc opaque(payload int) { callback := func(item int) { inner := func(other int) { payload_error(other) }; _ = inner }; _ = callback }\nfunc sibling() {}",
            ),
            (
                "typescript",
                "function opaque(payload: number) { const callback = (item: number) => { const inner = (other: number) => payload_error(other); return inner(item); }; } function sibling() {}",
            ),
            (
                "tsx",
                "function opaque(payload: number) { const callback = (item: number) => { const inner = (other: number) => payload_error(other); return inner(item); }; } function sibling() {}",
            ),
        ] {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&tree_sitter_language_pack::get_language(language)?)?;
            let tree = parser.parse(source, None).ok_or("no tree")?;
            assert!(!tree.root_node().has_error(), "{language}");
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            assert_eq!(projection.scopes.len(), 4, "{language}");
            if language == "python" {
                let bounded = project(
                    tree.root_node(),
                    source.as_bytes(),
                    language,
                    Limits {
                        scopes: 4,
                        ..Limits::default()
                    },
                )
                .map_err(std::io::Error::other)?;
                assert!(!bounded.truncated);
                assert!(
                    bounded
                        .scopes
                        .iter()
                        .any(|scope| scope.name.as_deref() == Some("sibling"))
                );
                assert!(
                    bounded
                        .scopes
                        .iter()
                        .all(|scope| source.get(scope.callable_start..scope.callable_end)
                            != Some("lambda"))
                );
            }

            assert_eq!(projection.scopes[0].parent_scope, None);
            assert_eq!(projection.scopes[1].parent_scope, Some(0));
            assert_eq!(projection.scopes[2].parent_scope, Some(1));
            assert_eq!(projection.scopes[3].parent_scope, None);
            for scope in &projection.scopes {
                assert!(scope.start <= scope.callable_start && scope.callable_end <= scope.end);
                let prefix = &source.as_bytes()[..scope.callable_start];
                assert_eq!(
                    scope.start_point.row,
                    prefix.iter().filter(|byte| **byte == b'\n').count()
                );
                let column = prefix
                    .iter()
                    .rposition(|byte| *byte == b'\n')
                    .map_or(prefix.len(), |at| prefix.len() - at - 1);
                assert_eq!(scope.start_point.column, column);
                for window in &scope.code_windows {
                    assert_eq!(
                        source.get(window.start..window.end),
                        Some(window.text.as_str())
                    );
                }
            }
            if matches!(language, "typescript" | "tsx") {
                assert!(projection.scopes[1].start < projection.scopes[1].callable_start);
            }
            assert!(!projection.scopes[0].names.contains_key("payload_error"));
        }
        Ok(())
    }
    #[test]
    fn type_barriers_do_not_supply_an_outer_callable_parent()
    -> Result<(), Box<dyn std::error::Error>> {
        let source =
            "function outer() { class Nested { run() { return (() => secret_payload())(); } } }";
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_language_pack::get_language("typescript")?)?;
        let tree = parser.parse(source, None).ok_or("no tree")?;
        let projection = project(
            tree.root_node(),
            source.as_bytes(),
            "typescript",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(projection.scopes.len(), 3);
        assert_eq!(projection.scopes[0].name.as_deref(), Some("outer"));
        assert_eq!(projection.scopes[1].name.as_deref(), Some("run"));
        assert_eq!(projection.scopes[1].parent_scope, None);
        assert_eq!(projection.scopes[2].parent_scope, Some(1));
        Ok(())
    }
}
