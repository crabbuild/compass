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
pub const PROJECTION_PROFILE: &str = "source-query/7-literal-seeded-code";

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
// Explicit pinned syntax kinds, never interpreted/decoded runtime strings.
fn string_literal(language: &str, kind: &str) -> bool {
    match language {
        "python" => kind == "string",
        "rust" => matches!(kind, "string_literal" | "raw_string_literal"),
        "go" => matches!(kind, "interpreted_string_literal" | "raw_string_literal"),
        "typescript" | "tsx" => matches!(kind, "string" | "template_string"),
        _ => false,
    }
}

struct CodeWindowSelection {
    window: Option<Site>,
    omitted_literal: bool,
}

/// Largest small enclosing syntax node whose entire subtree belongs to this
/// callable. A nested callable/type, parser recovery, or scan limit rejects the
/// larger window. Exact bytes come from extraction, never a query-time file.
fn code_window(
    node: Node<'_>,
    source: &[u8],
    language: &str,
    visits_left: &mut usize,
) -> CodeWindowSelection {
    let literal_seed = string_literal(language, node.kind());
    let mut selected = None;
    // Direct-expression arrows/lambdas have no intervening safe parent.
    // Consider a literal's own whole subtree, then the existing8 ancestors.
    let mut parent = if literal_seed {
        Some(node)
    } else {
        node.parent()
    };
    for _ in 0..(8 + usize::from(literal_seed)) {
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
    CodeWindowSelection {
        omitted_literal: literal_seed && selected.is_none(),
        window: selected,
    }
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
            let identifier_site = identifier(node.kind())
                && scope.name_span != Some((node.start_byte(), node.end_byte()));
            if identifier_site || string_literal(language, node.kind()) {
                // AST preorder is the declared shared code-window priority.
                // New literal seeds can consume capacity before later identifiers.
                // Code-window quotas are independent of identifier/comment
                // quotas. Profiles 3–4 keep windows inspection-only; profiles
                // 5–7 also index their retained novel vocabulary.
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
                        let selection = code_window(node, source, language, &mut code_visits_left);
                        if let Some(window) = selection.window
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
                        } else if selection.omitted_literal || code_visits_left == 0 {
                            scope.omitted_code_windows =
                                scope.omitted_code_windows.saturating_add(1);
                        }
                    }
                }
            }
            if identifier_site {
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
    #[test]
    fn literal_only_bodies_supply_exact_direct_owner_code_windows()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, source) in [
            ("python", "def opaque():\n    return \"session expired\"\n"),
            ("rust", "fn opaque() { \"session expired\"; }"),
            (
                "go",
                "package p\nfunc Opaque() string { return \"session expired\" }",
            ),
            (
                "typescript",
                "function opaque() { return \"session expired\"; }",
            ),
            ("tsx", "function opaque() { return \"session expired\"; }"),
            ("python", "opaque = lambda: \"session expired\""),
            ("typescript", "const opaque = () => \"session expired\";"),
            ("tsx", "const opaque = () => \"session expired\";"),
        ] {
            let tree = parse(source, language)?;
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let owner = projection
                .scopes
                .first()
                .ok_or("missing literal-only callable")?;
            assert!(
                owner
                    .code_windows
                    .iter()
                    .any(|window| window.text.contains("session expired")),
                "literal-only body lacks code evidence: {language}: {source}"
            );
            for window in &owner.code_windows {
                assert_eq!(
                    source.get(window.start..window.end),
                    Some(window.text.as_str())
                );
                assert!(owner.callable_start <= window.start && window.end <= owner.callable_end);
            }
            assert_eq!(owner.omitted_code_windows, 0, "{language}");
        }
        Ok(())
    }

    #[test]
    fn literal_whole_byte_and_scan_limits_remain_honest() -> Result<(), Box<dyn std::error::Error>>
    {
        for size in [512, 513] {
            let literal = format!("\"{}\"", "a".repeat(size - 2));
            let source = format!("def opaque():\n    return {literal}\n");
            let tree = parse(&source, "python")?;
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                "python",
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let owner = projection.scopes.first().ok_or("missing literal owner")?;
            assert_eq!(owner.code_windows.len(), usize::from(size == 512));
            assert_eq!(owner.omitted_code_windows, u64::from(size == 513));
            if size == 512 {
                assert_eq!(owner.code_windows[0].text, literal);
            }
            assert!(!projection.truncated && !owner.truncated);
            assert!(owner.names.is_empty() && owner.comments.is_empty());
        }
        let source = "def opaque():\n    return 'session expired'\n";
        let tree = parse(source, "python")?;
        let mut covered = 0;
        let mut omitted = 0;
        for budget in 0..=64 {
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                "python",
                Limits {
                    code_visits: budget,
                    ..Limits::default()
                },
            )
            .map_err(std::io::Error::other)?;
            let owner = projection.scopes.first().ok_or("missing scan owner")?;
            if owner.code_windows.is_empty() {
                omitted += 1;
                assert_eq!(owner.omitted_code_windows, 1);
            } else {
                covered += 1;
                assert_eq!(owner.omitted_code_windows, 0);
            }
            assert!(!owner.truncated && !projection.truncated);
        }
        assert!(covered > 0 && omitted > 0);
        Ok(())
    }

    #[test]
    fn covered_literals_do_not_add_omissions_and_original_channels_stay_independent()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = "def opaque():\n    return decode('session expired')\n";
        let tree = parse(source, "python")?;
        let mut covered_cases = 0;
        for budget in 0..=256 {
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                "python",
                Limits {
                    code_visits: budget,
                    ..Limits::default()
                },
            )
            .map_err(std::io::Error::other)?;
            let owner = projection.scopes.first().ok_or("missing covered owner")?;
            if owner
                .code_windows
                .iter()
                .any(|w| w.text.contains("decode('session expired')"))
            {
                covered_cases += 1;
                assert_eq!(
                    owner.omitted_code_windows, 0,
                    "covered seeds at budget {budget}"
                );
            }
        }
        assert!(covered_cases > 0);
        for (language, source) in [
            (
                "python",
                "def opaque(block):\n    # lexical comment\n    marker = block\n    return 'session expired'\n",
            ),
            (
                "rust",
                "fn opaque(block: usize) { let marker = block; /* lexical comment */ \"session expired\"; }",
            ),
            (
                "go",
                "package p\nfunc Opaque(block int) string { marker := block; /* lexical comment */ _ = marker; return \"session expired\" }",
            ),
            (
                "typescript",
                "function opaque(block: number) { const marker = block; /* lexical comment */ return 'session expired'; }",
            ),
            (
                "tsx",
                "function opaque(block: number) { const marker = block; /* lexical comment */ return 'session expired'; }",
            ),
        ] {
            let tree = parse(source, language)?;
            let zero = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits {
                    code_visits: 0,
                    ..Limits::default()
                },
            )
            .map_err(std::io::Error::other)?;
            let full = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            assert_eq!(zero.visits, full.visits);
            assert_eq!(zero.retained_bytes, full.retained_bytes);
            assert_eq!(zero.retained_parameter_bytes, full.retained_parameter_bytes);
            assert_eq!(zero.scopes.len(), full.scopes.len());
            for (left, right) in zero.scopes.iter().zip(&full.scopes) {
                assert_eq!(left.names, right.names, "{language}");
                assert_eq!(left.comments, right.comments, "{language}");
                assert_eq!(
                    left.parameter_bindings, right.parameter_bindings,
                    "{language}"
                );
                assert_eq!(left.truncated, right.truncated);
            }
        }
        Ok(())
    }

    #[test]
    fn literal_capacity_uses_declared_ast_preorder_without_losing_names()
    -> Result<(), Box<dyn std::error::Error>> {
        for identifier_first in [false, true] {
            let literals = (0..20)
                .map(|i| format!("    'marker_{i}_{}'\n", "q".repeat(180)))
                .collect::<String>();
            let source = if identifier_first {
                format!("def opaque():\n    decode()\n{literals}")
            } else {
                format!("def opaque():\n{literals}    decode()\n")
            };
            let tree = parse(&source, "python")?;
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                "python",
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let owner = projection.scopes.first().ok_or("missing capacity owner")?;
            assert_eq!(owner.code_windows.len(), 16);
            assert!(owner.omitted_code_windows > 0);
            assert!(owner.names.contains_key("decode"));
            assert_eq!(
                owner
                    .code_windows
                    .iter()
                    .any(|w| w.text.contains("decode()")),
                identifier_first
            );
            assert!(!owner.truncated && !projection.truncated);
        }
        Ok(())
    }

    #[test]
    fn literal_windows_never_borrow_nested_callable_or_type_text()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, source, root_name) in [
            (
                "python",
                "def opaque():\n    def inner():\n        return 'nested_secret'\n    class Hidden:\n        def method(self):\n            return 'type_secret'\n    return 'outer_marker'\n\ndef peer():\n    return 'peer_secret'\n",
                "opaque",
            ),
            (
                "typescript",
                "function opaque() { function inner() { return 'nested_secret'; } class Hidden { method() { return 'type_secret'; } } return 'outer_marker'; } function peer() { return 'peer_secret'; }",
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
            let owner = projection
                .scopes
                .iter()
                .find(|o| o.name.as_deref() == Some(root_name))
                .ok_or("missing outer literal owner")?;
            assert!(
                owner
                    .code_windows
                    .iter()
                    .any(|w| w.text.contains("outer_marker"))
            );
            assert!(
                owner
                    .code_windows
                    .iter()
                    .all(|w| !w.text.contains("nested_secret")
                        && !w.text.contains("type_secret")
                        && !w.text.contains("peer_secret"))
            );
        }
        for (language, source) in [
            (
                "python",
                "def opaque():\n    return f\"topic {(lambda: 'nested_secret')()}\"\n",
            ),
            (
                "typescript",
                "function opaque() { return `topic ${(() => 'nested_secret')()}`; }",
            ),
            (
                "tsx",
                "function opaque() { return `topic ${(() => 'nested_secret')()}`; }",
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
            let owner = projection
                .scopes
                .iter()
                .find(|o| o.name.as_deref() == Some("opaque"))
                .ok_or("missing interpolation owner")?;
            assert!(
                owner
                    .code_windows
                    .iter()
                    .all(|w| !w.text.contains("nested_secret"))
            );
            assert!(
                owner.omitted_code_windows > 0,
                "unsafe literal must disclose omission: {language}"
            );
        }
        Ok(())
    }

    #[test]
    fn literal_markers_remain_with_their_unique_direct_callable_owner()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, source) in [
            (
                "python",
                "def opaque():\n    def inner():\n        return 'nested_secret'\n    class Hidden:\n        def method(self):\n            return 'type_secret'\n    return 'outer_marker'\n\ndef peer():\n    return 'peer_secret'\n",
            ),
            (
                "rust",
                "fn opaque() -> &'static str { fn inner() -> &'static str { \"nested_secret\" } \"outer_marker\" } struct Hidden; impl Hidden { fn method() -> &'static str { \"type_secret\" } } fn peer() -> &'static str { \"peer_secret\" }",
            ),
            (
                "go",
                "package p\nfunc Opaque() string { inner := func() string { return \"nested_secret\" }; _ = inner; return \"outer_marker\" }\ntype Hidden struct{}\nfunc (h Hidden) Method() string { return \"type_secret\" }\nfunc Peer() string { return \"peer_secret\" }",
            ),
            (
                "typescript",
                "function opaque() { function inner() { return 'nested_secret'; } class Hidden { method() { return 'type_secret'; } } return 'outer_marker'; } function peer() { return 'peer_secret'; }",
            ),
            (
                "tsx",
                "function opaque() { function inner() { return 'nested_secret'; } class Hidden { method() { return 'type_secret'; } } return 'outer_marker'; } function peer() { return 'peer_secret'; }",
            ),
        ] {
            let tree = parse(source, language)?;
            assert!(!tree.root_node().has_error(), "invalid {language} control");
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let mut owner_ranges = std::collections::BTreeSet::new();
            for marker in [
                "nested_secret",
                "type_secret",
                "outer_marker",
                "peer_secret",
            ] {
                let start = source.find(marker).ok_or("missing fixture marker")?;
                let owners: Vec<_> = projection
                    .scopes
                    .iter()
                    .filter(|owner| {
                        owner
                            .code_windows
                            .iter()
                            .any(|window| window.text.contains(marker))
                    })
                    .collect();
                assert_eq!(owners.len(), 1, "{language}: {marker}");
                let owner = owners[0];
                assert!(
                    owner.callable_start <= start && start + marker.len() <= owner.callable_end
                );
                assert!(
                    owner_ranges.insert((owner.callable_start, owner.callable_end)),
                    "markers borrowed by one owner: {language}"
                );
                for window in owner
                    .code_windows
                    .iter()
                    .filter(|window| window.text.contains(marker))
                {
                    assert_eq!(
                        source.get(window.start..window.end),
                        Some(window.text.as_str())
                    );
                    assert!(
                        window.start >= owner.callable_start && window.end <= owner.callable_end
                    );
                    for other in [
                        "nested_secret",
                        "type_secret",
                        "outer_marker",
                        "peer_secret",
                    ] {
                        if other != marker {
                            assert!(
                                !window.text.contains(other),
                                "{language}: {marker} borrowed {other}"
                            );
                        }
                    }
                }
            }
        }
        for (language, source, syntax) in [
            (
                "python",
                "def opaque():\n    return f\"topic {(lambda: 'nested_secret')()}\"\n",
                "lambda",
            ),
            (
                "typescript",
                "function opaque() { return `topic ${(() => 'nested_secret')()}`; }",
                "arrow_function",
            ),
            (
                "tsx",
                "function opaque() { return `topic ${(() => 'nested_secret')()}`; }",
                "arrow_function",
            ),
        ] {
            let tree = parse(source, language)?;
            assert!(
                !tree.root_node().has_error(),
                "invalid {language} interpolation control"
            );
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let owners: Vec<_> = projection
                .scopes
                .iter()
                .filter(|owner| {
                    owner
                        .code_windows
                        .iter()
                        .any(|window| window.text.contains("nested_secret"))
                })
                .collect();
            assert_eq!(owners.len(), 1, "missing unique inner literal: {language}");
            assert_eq!(owners[0].syntax, syntax);
            assert!(
                owners[0]
                    .code_windows
                    .iter()
                    .all(|window| !window.text.contains("topic"))
            );
        }
        Ok(())
    }

    #[test]
    fn raw_literal_bytes_and_unicode_source_positions_are_preserved()
    -> Result<(), Box<dyn std::error::Error>> {
        for (language, prefix, literal, suffix) in [
            (
                "python",
                "def café():\n    return ",
                r####"r"session \u0065xpired界""####,
                "\n",
            ),
            (
                "python",
                "def café():\n    return ",
                "'''session\nexpired界'''",
                "\n",
            ),
            (
                "python",
                "def café():\n    return ",
                r####"b"session\x20expired""####,
                "\n",
            ),
            (
                "rust",
                "fn café() { ",
                r####"r##"session \u{20}expired界"##"####,
                "; }",
            ),
            (
                "rust",
                "fn café() { ",
                r####""session\nexpired界""####,
                "; }",
            ),
            (
                "rust",
                "fn café() { ",
                r####"b"session\x20expired""####,
                "; }",
            ),
            (
                "rust",
                "fn café() { ",
                r####"br##"session \x20expired"##"####,
                "; }",
            ),
            (
                "go",
                "package p\nfunc Café() string { return ",
                "`session\nexpired界`",
                " }",
            ),
            (
                "go",
                "package p\nfunc Café() string { return ",
                r####""session\nexpired界""####,
                " }",
            ),
            (
                "typescript",
                "function café() { return ",
                "`session\nexpired界`",
                "; }",
            ),
            (
                "typescript",
                "function café() { return ",
                r####"'session\nexpired界'"####,
                "; }",
            ),
            (
                "tsx",
                "function café() { return ",
                r####""session\u0065xpired界""####,
                "; }",
            ),
        ] {
            let source = format!("{prefix}{literal}{suffix}");
            let tree = parse(&source, language)?;
            let projection = project(
                tree.root_node(),
                source.as_bytes(),
                language,
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let owner = projection.scopes.first().ok_or("missing literal owner")?;
            if language == "rust" && literal.starts_with('b') {
                assert!(
                    owner.names.is_empty(),
                    "Rust byte control must have no identifier seed"
                );
            }
            let window = owner
                .code_windows
                .iter()
                .find(|w| w.start <= prefix.len() && prefix.len() + literal.len() <= w.end)
                .ok_or("whole raw literal missing")?;
            assert_eq!(
                source.get(window.start..window.end),
                Some(window.text.as_str())
            );
            assert!(
                window.text.contains(literal),
                "raw source was decoded: {language}"
            );
            for (byte, point) in [
                (window.start, window.start_point),
                (window.end, window.end_point),
            ] {
                let preceding = source.get(..byte).ok_or("non-UTF8 site boundary")?;
                assert_eq!(point.row, preceding.bytes().filter(|b| *b == b'\n').count());
                assert_eq!(
                    point.column,
                    preceding.rfind('\n').map_or(byte, |at| byte - at - 1)
                );
            }
            assert_eq!(owner.omitted_code_windows, 0, "{language}");
        }
        for (language, kind) in [
            ("rust", "char_literal"),
            ("go", "rune_literal"),
            ("tsx", "jsx_text"),
            ("python", "integer"),
            ("javascript", "string"),
        ] {
            assert!(!super::string_literal(language, kind));
        }
        Ok(())
    }

    #[test]
    fn nested_python_decorator_stays_with_its_lexical_enclosing_owner()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = "def outer():\n    @decorate('decorator_secret')\n    def inner():\n        return 'inner_marker'\n    return 0\n";
        let tree = parse(source, "python")?;
        assert!(!tree.root_node().has_error());
        let projection = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        for (marker, definition) in [
            ("decorator_secret", "def outer"),
            ("inner_marker", "def inner"),
        ] {
            let owners = projection
                .scopes
                .iter()
                .filter(|o| o.code_windows.iter().any(|w| w.text.contains(marker)))
                .collect::<Vec<_>>();
            assert_eq!(owners.len(), 1, "{marker}");
            assert_eq!(
                owners[0].callable_start,
                source.find(definition).ok_or("missing definition")?
            );
            for window in owners[0]
                .code_windows
                .iter()
                .filter(|w| w.text.contains(marker))
            {
                assert_eq!(
                    source.get(window.start..window.end),
                    Some(window.text.as_str())
                );
                assert!(
                    window.start >= owners[0].callable_start
                        && window.end <= owners[0].callable_end
                );
            }
        }
        Ok(())
    }

    #[test]
    fn literal_subtree_and_ancestor_boundaries_are_exact() -> Result<(), Box<dyn std::error::Error>>
    {
        for (tail, expected_nodes) in [("", 128), ("b", 129)] {
            let literal = format!("f\"a{}{tail}\"", "{0}".repeat(31));
            let source = format!("def opaque():\n    return {literal}\n");
            let tree = parse(&source, "python")?;
            let start = source.find(&literal).ok_or("missing fixture literal")?;
            let node = tree
                .root_node()
                .named_descendant_for_byte_range(start, start + literal.len())
                .filter(|node| node.kind() == "string" && node.start_byte() == start)
                .ok_or("missing whole string AST")?;
            let mut stack = vec![node];
            let mut nodes = 0;
            while let Some(next) = stack.pop() {
                nodes += 1;
                assert!(nodes <= 256);
                let mut cursor = next.walk();
                stack.extend(next.children(&mut cursor));
            }
            assert_eq!(nodes, expected_nodes, "pinned grammar boundary");
            let mut budget = 4096;
            let selected = super::code_window(node, source.as_bytes(), "python", &mut budget);
            assert_eq!(selected.omitted_literal, expected_nodes > 128);
            assert_eq!(
                selected.window.map(|w| w.text),
                (expected_nodes == 128).then_some(literal)
            );
            assert!(4096 - budget <= 9 * 128);
        }
        for depth in [8, 9, 10] {
            let source = format!(
                "def opaque():\n    return {}'topic'{}\n",
                "(".repeat(depth),
                ")".repeat(depth)
            );
            let tree = parse(&source, "python")?;
            let start = source.find("'topic'").ok_or("missing nested literal")?;
            let node = tree
                .root_node()
                .named_descendant_for_byte_range(start, start + 7)
                .filter(|node| node.kind() == "string")
                .ok_or("missing nested string AST")?;
            let mut budget = 4096;
            let selected = super::code_window(node, source.as_bytes(), "python", &mut budget);
            let window = selected.window.ok_or("safe self/ancestor window missing")?;
            assert_eq!(
                window.text,
                format!("{}'topic'{}", "(".repeat(8), ")".repeat(8))
            );
            assert_eq!(window.start, start - 8);
            assert_eq!(window.end, start + 7 + 8);
            assert!(!selected.omitted_literal);
        }
        Ok(())
    }

    #[test]
    fn literal_file_bytes_and_precovered_last_slot_disclose_only_real_omissions()
    -> Result<(), Box<dyn std::error::Error>> {
        let literals = (0..15)
            .map(|i| format!("    'marker{i}_{}'\n", "q".repeat(180)))
            .collect::<String>();
        let source = format!("def opaque():\n{literals}    decode('tail')\n");
        let tree = parse(&source, "python")?;
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(p.scopes[0].code_windows.len(), 16);
        assert_eq!(p.scopes[0].omitted_code_windows, 0);
        assert!(
            p.scopes[0]
                .code_windows
                .iter()
                .any(|w| w.text.contains("decode('tail')"))
        );
        for last_size in [511, 512] {
            let mut source = String::new();
            for owner in 0..64 {
                source.push_str(&format!("def opaque{owner}():\n"));
                for slot in 0..8 {
                    let size = if owner == 63 && slot == 7 {
                        last_size
                    } else {
                        512
                    };
                    source.push_str(&format!("    '{}'\n", "q".repeat(size - 2)));
                }
            }
            source.push_str("def overflow():\n    ''\n");
            let tree = parse(&source, "python")?;
            let p = project(
                tree.root_node(),
                source.as_bytes(),
                "python",
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            assert_eq!(p.retained_code_bytes, 511 * 512 + last_size);
            assert_eq!(
                p.scopes.iter().map(|o| o.code_windows.len()).sum::<usize>(),
                512
            );
            assert_eq!(
                p.scopes.iter().map(|o| o.omitted_code_windows).sum::<u64>(),
                1
            );
            assert!(!p.truncated && p.scopes.iter().all(|o| !o.truncated));
            assert!(
                p.scopes
                    .last()
                    .ok_or("missing overflow scope")?
                    .code_windows
                    .is_empty()
            );
        }
        Ok(())
    }

    #[test]
    fn literal_global_scan_exhaustion_keeps_original_channels_complete()
    -> Result<(), Box<dyn std::error::Error>> {
        let literal = format!("f\"a{}\"", "{0}".repeat(31));
        let mut source = String::new();
        for owner in 0..64 {
            source.push_str(&format!("def opaque{owner}():\n"));
            for _ in 0..16 {
                source.push_str(&format!("    return {literal}\n"));
            }
        }
        let tree = parse(&source, "python")?;
        // A literal has128 nodes and its return parent129: two128-node
        // attempts consume256 visits, exhausting200000 before all1024 seeds.
        assert_eq!(Limits::default().code_visits, 200_000);
        let p = project(
            tree.root_node(),
            source.as_bytes(),
            "python",
            Limits::default(),
        )
        .map_err(std::io::Error::other)?;
        assert_eq!(
            p.scopes.iter().map(|o| o.code_windows.len()).sum::<usize>(),
            781
        );
        assert_eq!(p.retained_code_bytes, 781 * literal.len());
        assert_eq!(
            p.scopes.iter().map(|o| o.omitted_code_windows).sum::<u64>(),
            1024 - 781
        );
        assert!(!p.truncated && p.visits < 200_000);
        assert!(p.scopes.iter().all(|o| !o.truncated
            && o.names.is_empty()
            && o.parameter_bindings.status == super::ParameterBindingStatus::Complete));
        Ok(())
    }

    #[test]
    fn whole_multibyte_literals_and_recovered_syntax_keep_honest_boundaries()
    -> Result<(), Box<dyn std::error::Error>> {
        for suffix in ["", "x"] {
            let literal = format!("'{}{suffix}'", "界".repeat(170));
            let source = format!("def opaque():\n    return {literal}\n");
            assert_eq!(literal.len(), 512 + suffix.len());
            let tree = parse(&source, "python")?;
            let p = project(
                tree.root_node(),
                source.as_bytes(),
                "python",
                Limits::default(),
            )
            .map_err(std::io::Error::other)?;
            let owner = p.scopes.first().ok_or("missing multibyte owner")?;
            assert_eq!(owner.code_windows.len(), usize::from(suffix.is_empty()));
            assert_eq!(owner.omitted_code_windows, u64::from(!suffix.is_empty()));
            if let Some(window) = owner.code_windows.first() {
                assert_eq!(window.text, literal);
            }
            assert!(!owner.truncated && !p.truncated);
        }
        for (language, source) in [
            ("python", "def opaque():\n    return 'unsafe_marker\n"),
            ("rust", "fn opaque() { \"unsafe_marker }"),
            (
                "go",
                "package p\nfunc Opaque() string { return \"unsafe_marker }",
            ),
            ("typescript", "function opaque() { return 'unsafe_marker; }"),
            ("tsx", "const opaque = () => 'unsafe_marker;"),
        ] {
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&tree_sitter_language_pack::get_language(language)?)?;
            let tree = parser.parse(source, None).ok_or("missing recovery tree")?;
            assert!(
                tree.root_node().has_error(),
                "{language}: fixture must be malformed"
            );
            let mut extraction = crate::Extraction::default();
            super::attach(
                &mut extraction,
                "sample",
                language,
                source.as_bytes(),
                tree.root_node(),
            );
            let value: super::SourceQueryExtraction = serde_json::from_value(
                extraction
                    .extensions
                    .remove(super::EXTRACTION_EXTENSION)
                    .ok_or("recovery evidence missing")?,
            )?;
            assert!(value.syntax_recovery);
            assert_eq!(value.byte_size, source.len() as u64);
            assert!(value.projection.visits <= 200_000 && value.projection.scopes.len() <= 512);
            assert!(
                value
                    .projection
                    .scopes
                    .iter()
                    .flat_map(|o| &o.code_windows)
                    .all(|w| !w.text.contains("unsafe_marker")
                        && source.get(w.start..w.end) == Some(w.text.as_str())),
                "unsafe recovered code: {language}"
            );
        }
        Ok(())
    }
}
