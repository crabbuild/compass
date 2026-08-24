//! Bounded, parser-backed syntax views shared by TypeScript/JavaScript
//! framework packs.
//!
//! The view deliberately exposes copied source values and exact ranges rather
//! than Tree-sitter handles to callers that need to retain facts.  Framework
//! packs may inspect the borrowed tree during extraction, but they must not
//! persist parser nodes or recover semantics by scanning source text.

use tree_sitter::Node;

/// Return whether an export statement contains the parser token for the
/// `default` export keyword.  Anonymous children are intentional here:
/// Tree-sitter represents JavaScript/TypeScript keywords as unnamed tokens,
/// so `named_children` cannot distinguish `export default` from an arbitrary
/// source-text occurrence.
#[must_use]
pub(crate) fn has_default_export_keyword(node: Node<'_>) -> bool {
    if node.kind() != "export_statement" {
        return false;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == "default")
}

pub(crate) const SYNTAX_VIEW_VERSION: &str = "compass.frontend-syntax/2";
pub(crate) const MAX_SYNTAX_DEPTH: usize = 256;
pub(crate) const MAX_SYNTAX_NODES: usize = 100_000;
pub(crate) const MAX_STATIC_DEPTH: usize = 32;
pub(crate) const MAX_STATIC_ITEMS: usize = 2_048;
pub(crate) const MAX_STATIC_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SyntaxRange {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum StaticValue {
    String(String),
    Boolean(bool),
    Number(String),
    Null,
    Regex(String),
    Array(Vec<StaticValue>),
    Object(Vec<(String, StaticValue)>),
    Incomplete,
}

impl StaticValue {
    #[must_use]
    pub(crate) fn as_string(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn object(&self) -> Option<&[(String, StaticValue)]> {
        match self {
            Self::Object(values) => Some(values),
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn array(&self) -> Option<&[StaticValue]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct TypeScriptSyntax<'tree, 'source> {
    root: Node<'tree>,
    source: &'source [u8],
}

impl<'tree, 'source> TypeScriptSyntax<'tree, 'source> {
    #[must_use]
    pub(crate) const fn new(root: Node<'tree>, source: &'source [u8]) -> Self {
        Self { root, source }
    }

    #[must_use]
    pub(crate) const fn root(self) -> Node<'tree> {
        self.root
    }

    #[must_use]
    pub(crate) fn text(self, node: Node<'tree>) -> Option<&'source str> {
        node.utf8_text(self.source).ok()
    }

    #[must_use]
    pub(crate) fn range(self, node: Node<'tree>) -> Option<SyntaxRange> {
        let start = node.start_position();
        let end = node.end_position();
        Some(SyntaxRange {
            start_byte: node.start_byte(),
            end_byte: node.end_byte(),
            start_line: u32::try_from(start.row.saturating_add(1)).ok()?,
            start_column: u32::try_from(start.column).ok()?,
            end_line: u32::try_from(end.row.saturating_add(1)).ok()?,
            end_column: u32::try_from(end.column).ok()?,
        })
    }

    #[must_use]
    pub(crate) fn is_incomplete(self, node: Node<'tree>) -> bool {
        if node.has_error() || node.is_missing() {
            return true;
        }
        if !self.root.has_error() {
            return false;
        }
        let Some(range) = self.range(node) else {
            return true;
        };
        // Tree-sitter may attach a recovery node to a broad ancestor while
        // leaving a neighboring declaration's `has_error` bit clear. Reject
        // only syntax whose byte range actually overlaps an ERROR/missing
        // recovery region; unrelated declarations in the same file remain
        // usable evidence.
        self.descendants(self.root).into_iter().any(|candidate| {
            if candidate.kind() != "ERROR" && !candidate.is_missing() {
                return false;
            }
            candidate.start_byte() < range.end_byte && range.start_byte < candidate.end_byte()
        })
    }

    #[must_use]
    pub(crate) fn contains_kind(self, node: Node<'tree>, kind: &str) -> bool {
        self.descendants(node)
            .into_iter()
            .any(|candidate| candidate.kind() == kind)
    }

    #[must_use]
    pub(crate) fn descendants(self, node: Node<'tree>) -> Vec<Node<'tree>> {
        let mut output = Vec::new();
        let mut stack = vec![(node, 0usize)];
        while let Some((current, depth)) = stack.pop() {
            if output.len() >= MAX_SYNTAX_NODES || depth > MAX_SYNTAX_DEPTH {
                break;
            }
            output.push(current);
            let mut children = current
                .named_children(&mut current.walk())
                .collect::<Vec<_>>();
            children.reverse();
            for child in children {
                stack.push((child, depth.saturating_add(1)));
            }
        }
        output
    }

    #[must_use]
    pub(crate) fn node_count_and_depth(self) -> (usize, usize) {
        let mut count = 0usize;
        let mut max_depth = 0usize;
        let mut stack = vec![(self.root, 0usize)];
        while let Some((current, depth)) = stack.pop() {
            count = count.saturating_add(1);
            max_depth = max_depth.max(depth);
            if count >= MAX_SYNTAX_NODES || depth >= MAX_SYNTAX_DEPTH {
                continue;
            }
            let mut cursor = current.walk();
            for child in current.named_children(&mut cursor) {
                stack.push((child, depth.saturating_add(1)));
            }
        }
        (count, max_depth)
    }

    #[must_use]
    pub(crate) fn top_level_directive(self, expected: &str) -> bool {
        let mut cursor = self.root.walk();
        for statement in self.root.named_children(&mut cursor) {
            if statement.kind() != "expression_statement" {
                break;
            }
            let Some(expression) = statement.named_child(0) else {
                continue;
            };
            let Some(value) = self.literal_string(expression) else {
                break;
            };
            if value == expected {
                return true;
            }
        }
        false
    }

    /// Return whether an export statement is parser-backed evidence for a
    /// default export.  This includes both `export default ...` and
    /// `export { value as default }` forms without scanning source text.
    #[must_use]
    pub(crate) fn is_default_export_statement(self, node: Node<'tree>) -> bool {
        has_default_export_keyword(node)
            || self
                .descendants(node)
                .into_iter()
                .filter(|candidate| candidate.kind() == "export_specifier")
                .any(|specifier| {
                    let name = specifier
                        .child_by_field_name("name")
                        .and_then(|child| self.text(child));
                    let alias = specifier
                        .child_by_field_name("alias")
                        .and_then(|child| self.text(child));
                    name == Some("default") || alias == Some("default")
                })
    }

    /// Return whether an export statement binds or re-exports a named symbol.
    /// The match is restricted to declaration and export-specifier nodes so
    /// comments and string literals cannot activate a framework convention.
    #[must_use]
    pub(crate) fn export_statement_exports_name(
        self,
        statement: Node<'tree>,
        expected: &str,
    ) -> bool {
        if statement.kind() != "export_statement" {
            return false;
        }
        self.descendants(statement).into_iter().any(|candidate| {
            let name = match candidate.kind() {
                "variable_declarator"
                | "function_declaration"
                | "class_declaration"
                | "abstract_class_declaration" => candidate.child_by_field_name("name"),
                "export_specifier" => candidate
                    .child_by_field_name("name")
                    .or_else(|| candidate.child_by_field_name("alias")),
                _ => None,
            };
            name.and_then(|child| self.text(child)) == Some(expected)
        })
    }

    #[must_use]
    pub(crate) fn literal_string(self, node: Node<'tree>) -> Option<String> {
        let text = self.text(node)?;
        match node.kind() {
            "string" | "string_fragment" => unquote_static(text),
            "template_string" => {
                if text.contains("${") {
                    None
                } else {
                    unquote_static(text)
                }
            }
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn property_name(self, node: Node<'tree>) -> Option<String> {
        let key = node
            .child_by_field_name("key")
            .or_else(|| node.child_by_field_name("name"))
            .or_else(|| node.named_child(0))?;
        if key.kind() == "computed_property_name" {
            return None;
        }
        self.literal_string(key).or_else(|| {
            matches!(
                key.kind(),
                "identifier" | "property_identifier" | "shorthand_property_identifier_pattern"
            )
            .then(|| self.text(key).map(str::to_owned))
            .flatten()
        })
    }

    #[must_use]
    pub(crate) fn call_callee(self, node: Node<'tree>) -> Option<String> {
        if node.kind() != "call_expression" {
            return None;
        }
        let function = node.child_by_field_name("function")?;
        if self.contains_kind(function, "subscript_expression")
            || self.contains_kind(function, "computed_property_name")
        {
            return None;
        }
        matches!(
            function.kind(),
            "identifier" | "member_expression" | "nested_identifier" | "this"
        )
        .then(|| self.text(function).map(str::to_owned))
        .flatten()
    }

    /// Return local bindings imported from one static module.  This is used
    /// for framework factory activation so a shadowed identifier such as a
    /// local `defineConfig` cannot masquerade as the framework API.
    #[must_use]
    pub(crate) fn imported_local_names(self, module: &str, imported: &str) -> Vec<String> {
        let mut names = Vec::new();
        for statement in self
            .descendants(self.root)
            .into_iter()
            .filter(|node| node.kind() == "import_statement")
        {
            let source_module = self
                .descendants(statement)
                .into_iter()
                .find_map(|node| self.literal_string(node));
            if source_module.as_deref() != Some(module) {
                continue;
            }
            for node in self.descendants(statement) {
                let local = node
                    .child_by_field_name("alias")
                    .or_else(|| node.child_by_field_name("name"));
                let Some(local) = local else {
                    continue;
                };
                let imported_name = node
                    .child_by_field_name("name")
                    .and_then(|name| self.text(name))
                    .unwrap_or_default();
                let local_name = self.text(local).unwrap_or_default();
                let matches = match imported {
                    "*" => node.kind() == "namespace_import",
                    "default" => node.kind() == "import_clause" || node.kind() == "default_import",
                    _ => imported_name == imported,
                };
                if matches && !local_name.is_empty() {
                    names.push(local_name.to_owned());
                }
            }
        }
        names.sort();
        names.dedup();
        names
    }

    /// Find the statically recoverable object returned to a configuration
    /// factory.  Direct object arguments and arrow/function bodies are
    /// supported; arbitrary nested call arguments remain incomplete.
    #[must_use]
    pub(crate) fn config_object_from_call(self, call: Node<'tree>) -> Option<Node<'tree>> {
        let arguments = call.child_by_field_name("arguments")?;
        let mut cursor = arguments.walk();
        for argument in arguments.named_children(&mut cursor) {
            if let Some(object) = self.config_object_in(argument, 0) {
                return Some(object);
            }
        }
        None
    }

    #[must_use]
    pub(crate) fn exported_default_config_object(self) -> Option<Node<'tree>> {
        for statement in self
            .descendants(self.root)
            .into_iter()
            .filter(|node| node.kind() == "export_statement")
        {
            let mut cursor = statement.walk();
            for child in statement.named_children(&mut cursor) {
                if child.kind() == "object" && !self.is_incomplete(child) {
                    return Some(child);
                }
                if child.kind() == "call_expression"
                    && self.call_callee(child).is_some()
                    && let Some(object) = self.config_object_from_call(child)
                {
                    return Some(object);
                }
            }
        }
        None
    }

    fn config_object_in(self, node: Node<'tree>, depth: usize) -> Option<Node<'tree>> {
        if depth > 8 || self.is_incomplete(node) {
            return None;
        }
        if node.kind() == "object" {
            return Some(node);
        }
        if matches!(
            node.kind(),
            "arrow_function"
                | "function"
                | "function_expression"
                | "parenthesized_expression"
                | "as_expression"
                | "satisfies_expression"
                | "return_statement"
        ) {
            let child = node
                .child_by_field_name("body")
                .or_else(|| node.child_by_field_name("expression"))
                .or_else(|| node.named_child(0));
            return child.and_then(|child| self.config_object_in(child, depth + 1));
        }
        None
    }

    #[must_use]
    pub(crate) fn static_value(self, node: Node<'tree>) -> StaticValue {
        static_value(self, node, 0, 0)
    }
}

fn static_value<'tree, 'source>(
    syntax: TypeScriptSyntax<'tree, 'source>,
    node: Node<'tree>,
    depth: usize,
    items: usize,
) -> StaticValue {
    if depth > MAX_STATIC_DEPTH || items > MAX_STATIC_ITEMS {
        return StaticValue::Incomplete;
    }
    let Some(text) = syntax.text(node) else {
        return StaticValue::Incomplete;
    };
    if text.len() > MAX_STATIC_BYTES {
        return StaticValue::Incomplete;
    }
    if syntax.is_incomplete(node) {
        return StaticValue::Incomplete;
    }
    match node.kind() {
        "string" | "string_fragment" | "template_string" => syntax
            .literal_string(node)
            .map_or(StaticValue::Incomplete, StaticValue::String),
        "true" => StaticValue::Boolean(true),
        "false" => StaticValue::Boolean(false),
        "null" => StaticValue::Null,
        "number" | "regex_pattern" => {
            if node.kind() == "regex_pattern" {
                StaticValue::Regex(text.to_owned())
            } else {
                StaticValue::Number(text.to_owned())
            }
        }
        "regex" => StaticValue::Regex(text.to_owned()),
        "parenthesized_expression" | "as_expression" | "satisfies_expression" => node
            .named_child(0)
            .map_or(StaticValue::Incomplete, |child| {
                static_value(syntax, child, depth.saturating_add(1), items)
            }),
        "array" => {
            let mut cursor = node.walk();
            let children = node.named_children(&mut cursor).collect::<Vec<_>>();
            if children.len().saturating_add(items) > MAX_STATIC_ITEMS {
                return StaticValue::Incomplete;
            }
            StaticValue::Array(
                children
                    .into_iter()
                    .map(|child| static_value(syntax, child, depth.saturating_add(1), items + 1))
                    .collect(),
            )
        }
        "object" => {
            let mut cursor = node.walk();
            let children = node.named_children(&mut cursor).collect::<Vec<_>>();
            if children.len().saturating_add(items) > MAX_STATIC_ITEMS {
                return StaticValue::Incomplete;
            }
            let mut values = Vec::new();
            for child in children {
                let Some(key) = syntax.property_name(child) else {
                    return StaticValue::Incomplete;
                };
                let Some(value_node) = child
                    .child_by_field_name("value")
                    .or_else(|| child.named_child(1))
                else {
                    return StaticValue::Incomplete;
                };
                let value = static_value(
                    syntax,
                    value_node,
                    depth.saturating_add(1),
                    items.saturating_add(1),
                );
                if value == StaticValue::Incomplete {
                    return StaticValue::Incomplete;
                }
                values.push((key, value));
            }
            StaticValue::Object(values)
        }
        _ => StaticValue::Incomplete,
    }
}

fn unquote_static(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    if bytes.len() < 2 || bytes[0] != *bytes.last()? {
        return None;
    }
    if !matches!(bytes[0], b'\'' | b'"' | b'`') {
        return None;
    }
    let value = &text[1..text.len().saturating_sub(1)];
    if value.contains('\\') || value.contains("${") || value.len() > MAX_STATIC_BYTES {
        return None;
    }
    Some(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse(
        language_name: &str,
        source: &[u8],
    ) -> Result<tree_sitter::Tree, Box<dyn std::error::Error>> {
        let language = tree_sitter_language_pack::get_language(language_name)?;
        let mut parser = Parser::new();
        parser.set_language(&language)?;
        parser
            .parse(source, None)
            .ok_or_else(|| "parser returned no tree".into())
    }

    #[test]
    fn exact_directive_and_static_object_values() -> Result<(), Box<dyn std::error::Error>> {
        let source = br#""use client";
const config = { root: "src", enabled: true };
"#;
        let tree = parse("typescript", source)?;
        let syntax = TypeScriptSyntax::new(tree.root_node(), source);
        assert!(syntax.top_level_directive("use client"));
        let object = syntax
            .descendants(tree.root_node())
            .into_iter()
            .find(|node| node.kind() == "object")
            .ok_or("object not found")?;
        let static_object = syntax.static_value(object);
        let values = static_object
            .object()
            .ok_or("object was not statically recoverable")?;
        assert_eq!(
            values
                .iter()
                .find(|(key, _)| key == "root")
                .and_then(|(_, value)| value.as_string()),
            Some("src")
        );
        assert_eq!(
            values
                .iter()
                .find(|(key, _)| key == "enabled")
                .map(|(_, value)| value),
            Some(&StaticValue::Boolean(true))
        );
        Ok(())
    }

    #[test]
    fn static_view_rejects_dynamic_values_and_exposes_callee()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = b"const value = makeConfig(process.env.MODE);";
        let tree = parse("typescript", source)?;
        let syntax = TypeScriptSyntax::new(tree.root_node(), source);
        let call = syntax
            .descendants(tree.root_node())
            .into_iter()
            .find(|node| node.kind() == "call_expression")
            .ok_or("call not found")?;
        assert_eq!(syntax.call_callee(call).as_deref(), Some("makeConfig"));
        assert_eq!(syntax.static_value(call), StaticValue::Incomplete);
        Ok(())
    }

    #[test]
    fn recovered_trees_never_become_complete_values() -> Result<(), Box<dyn std::error::Error>> {
        let source = b"const App = () => <Button />;\nconst broken = (";
        let tree = parse("tsx", source)?;
        let syntax = TypeScriptSyntax::new(tree.root_node(), source);
        assert!(tree.root_node().has_error());
        assert!(syntax.is_incomplete(tree.root_node()));
        assert!(syntax.contains_kind(tree.root_node(), "jsx_self_closing_element"));
        Ok(())
    }

    #[test]
    fn recovery_overlap_is_local_to_the_broken_expression() -> Result<(), Box<dyn std::error::Error>>
    {
        let source = b"const good = { root: \"src\" };\nconst broken = { root: };\n";
        let tree = parse("typescript", source)?;
        let syntax = TypeScriptSyntax::new(tree.root_node(), source);
        assert!(tree.root_node().has_error());
        let objects = syntax
            .descendants(tree.root_node())
            .into_iter()
            .filter(|node| node.kind() == "object")
            .collect::<Vec<_>>();
        assert_eq!(objects.len(), 2);
        assert!(!syntax.is_incomplete(objects[0]));
        assert!(syntax.is_incomplete(objects[1]));
        let (nodes, depth) = syntax.node_count_and_depth();
        assert!(nodes > 0);
        assert!(depth > 0);
        Ok(())
    }
}
