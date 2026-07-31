use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::{RawEdgeRecord as EdgeRecord, RawNodeRecord as NodeRecord};
use serde_json::{Map, Value};
use tree_sitter::Node;

use crate::{
    DeclarationKind, Extraction, OccurrenceFact, OccurrenceRole, RawCall, Registry,
    RelationshipCandidate, UniversalEvidence, make_id,
};
use compass_ir::SourceAnchor;

const TRAIT_METHOD_BLOCKLIST: &[&str] = &[
    "new",
    "default",
    "parse",
    "from_str",
    "now",
    "clone",
    "into",
    "from",
    "to_string",
    "to_owned",
    "len",
    "is_empty",
    "iter",
    "next",
    "build",
    "start",
    "run",
    "init",
    "app",
    "get",
    "set",
    "push",
    "pop",
    "insert",
    "remove",
    "contains",
    "collect",
    "map",
    "filter",
    "unwrap",
    "expect",
    "ok",
    "err",
    "some",
    "none",
    "send",
    "recv",
    "lock",
    "read",
    "write",
];

pub(crate) fn extract(path: &Path, source: &[u8], root: Node<'_>) -> Extraction {
    RustState::new(path, source).run(root)
}

struct RustState<'source, 'tree> {
    source: &'source [u8],
    source_file: String,
    stem: String,
    file_id: String,
    extraction: Extraction,
    seen: HashSet<String>,
    function_bodies: Vec<(String, Node<'tree>)>,
    scoped_callables: HashMap<(String, String), Vec<String>>,
    universal_evidence: UniversalEvidence,
}

impl<'source, 'tree> RustState<'source, 'tree> {
    fn new(path: &Path, source: &'source [u8]) -> Self {
        let source_file = path.to_string_lossy().into_owned();
        let stem = crate::file_stem(path);
        let file_id = make_id(&[&source_file]);
        let mut state = Self {
            source,
            source_file,
            stem,
            file_id,
            extraction: Extraction::default(),
            seen: HashSet::new(),
            function_bodies: Vec::new(),
            scoped_callables: HashMap::new(),
            universal_evidence: UniversalEvidence::new(&Registry::adapter("rust")),
        };
        let label = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        state.add_node(&state.file_id.clone(), label, 1);
        state
    }

    fn run(mut self, root: Node<'tree>) -> Extraction {
        self.walk(root, None);
        self.walk_calls();
        let valid = &self.seen;
        self.extraction.edges.retain(|edge| {
            valid.contains(&edge.source)
                && (valid.contains(&edge.target)
                    || matches!(
                        edge.attributes.get("relation").and_then(Value::as_str),
                        Some("imports" | "imports_from")
                    ))
        });
        self.universal_evidence.occurrences.sort();
        self.universal_evidence.occurrences.dedup();
        self.universal_evidence.relationship_candidates.sort();
        self.universal_evidence.relationship_candidates.dedup();
        self.extraction
            .universal_evidence
            .push(self.universal_evidence);
        self.extraction
    }

    fn walk(&mut self, node: Node<'tree>, parent_impl: Option<(&str, &str, bool)>) {
        match node.kind() {
            "function_item" => {
                self.add_function(node, parent_impl);
                return;
            }
            "struct_item" | "enum_item" | "trait_item" => {
                self.add_item(node);
                return;
            }
            "impl_item" => {
                self.add_impl(node);
                return;
            }
            "use_declaration" => {
                self.add_use(node);
                return;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, None);
        }
    }

    fn add_function(&mut self, node: Node<'tree>, parent_impl: Option<(&str, &str, bool)>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let at = line(node);
        let id = if let Some((parent, semantic_owner, inherent)) = parent_impl {
            let id = make_id(&[parent, semantic_owner, &name]);
            self.add_node(&id, &format!(".{name}()"), at);
            if let Some(method) = self
                .extraction
                .nodes
                .iter_mut()
                .find(|method| method.id == id)
            {
                method.attributes.insert(
                    "lexical_owner".to_owned(),
                    Value::String(semantic_owner.to_owned()),
                );
                method.attributes.insert(
                    "qualified_name".to_owned(),
                    Value::String(format!("{semantic_owner}::{name}")),
                );
            }
            self.add_edge(parent, &id, "method", at, None);
            self.stamp_node_range(&id, node);
            if inherent {
                self.scoped_callables
                    .entry((semantic_owner.to_owned(), name.clone()))
                    .or_default()
                    .push(id.clone());
            }
            id
        } else {
            let id = make_id(&[&self.stem, &name]);
            self.add_node(&id, &format!("{name}()"), at);
            self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
            self.stamp_node_range(&id, node);
            id
        };
        self.add_function_references(node, &id, at);
        if let Some(body) = node.child_by_field_name("body") {
            self.function_bodies.push((id, body));
        }
    }

    fn add_item(&mut self, node: Node<'tree>) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node);
        let at = line(node);
        let id = make_id(&[&self.stem, &name]);
        self.add_node(&id, &name, at);
        self.stamp_node_range(&id, node);
        self.add_edge(&self.file_id.clone(), &id, "contains", at, None);
        match node.kind() {
            "trait_item" => self.add_trait_bounds(node, &id, at),
            "struct_item" => self.add_struct_fields(node, &id),
            "enum_item" => self.add_enum_fields(node, &id),
            _ => {}
        }
    }

    fn add_trait_bounds(&mut self, node: Node<'tree>, id: &str, at: usize) {
        let mut cursor = node.walk();
        for bounds in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "trait_bounds")
        {
            let mut bound_cursor = bounds.walk();
            for child in bounds
                .children(&mut bound_cursor)
                .filter(|child| child.is_named())
            {
                let mut refs = Vec::new();
                collect_type_refs(Some(child), self.source, false, &mut refs);
                for (index, (name, _)) in refs.into_iter().enumerate() {
                    let target = self.ensure_named_node(&name);
                    if target == id {
                        continue;
                    }
                    if index == 0 {
                        self.add_edge(id, &target, "inherits", at, None);
                    } else {
                        self.add_edge(id, &target, "references", at, Some("generic_arg"));
                    }
                }
            }
        }
    }

    fn add_struct_fields(&mut self, node: Node<'tree>, id: &str) {
        let mut cursor = node.walk();
        for list in node.children(&mut cursor) {
            if list.kind() == "field_declaration_list" {
                let mut list_cursor = list.walk();
                for field in list
                    .children(&mut list_cursor)
                    .filter(|child| child.kind() == "field_declaration")
                {
                    let type_node = field.child_by_field_name("type").or_else(|| {
                        let mut field_cursor = field.walk();
                        field
                            .children(&mut field_cursor)
                            .find(|child| is_type_node(child.kind()))
                    });
                    self.add_field_type(type_node, id, line(field));
                }
            } else if list.kind() == "ordered_field_declaration_list" {
                let at = line(list);
                let mut list_cursor = list.walk();
                for type_node in list
                    .children(&mut list_cursor)
                    .filter(|child| is_type_node(child.kind()))
                {
                    self.add_field_type(Some(type_node), id, at);
                }
            }
        }
    }

    fn add_enum_fields(&mut self, node: Node<'tree>, id: &str) {
        let mut cursor = node.walk();
        for list in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "enum_variant_list")
        {
            let mut list_cursor = list.walk();
            for variant in list
                .children(&mut list_cursor)
                .filter(|child| child.kind() == "enum_variant")
            {
                let mut variant_cursor = variant.walk();
                for fields in variant.children(&mut variant_cursor) {
                    if fields.kind() == "ordered_field_declaration_list" {
                        let mut fields_cursor = fields.walk();
                        for type_node in fields
                            .children(&mut fields_cursor)
                            .filter(|child| is_type_node(child.kind()))
                        {
                            self.add_field_type(Some(type_node), id, line(variant));
                        }
                    } else if fields.kind() == "field_declaration_list" {
                        let mut fields_cursor = fields.walk();
                        for field in fields
                            .children(&mut fields_cursor)
                            .filter(|child| child.kind() == "field_declaration")
                        {
                            self.add_field_type(field.child_by_field_name("type"), id, line(field));
                        }
                    }
                }
            }
        }
    }

    fn add_field_type(&mut self, type_node: Option<Node<'tree>>, id: &str, at: usize) {
        let mut refs = Vec::new();
        collect_type_refs(type_node, self.source, false, &mut refs);
        for (name, generic) in refs {
            let target = self.ensure_named_node(&name);
            if target != id {
                self.add_edge(
                    id,
                    &target,
                    "references",
                    at,
                    Some(if generic { "generic_arg" } else { "field" }),
                );
            }
        }
    }

    fn add_impl(&mut self, node: Node<'tree>) {
        let Some(type_node) = node.child_by_field_name("type") else {
            return;
        };
        let name = self.text(type_node).trim().to_owned();
        let id = make_id(&[&self.stem, &name]);
        let semantic_owner = node
            .child_by_field_name("trait")
            .map(|trait_node| self.text(trait_node).trim().to_owned())
            .filter(|trait_name| !trait_name.is_empty())
            .map_or_else(
                || name.clone(),
                |trait_name| format!("{name} as {trait_name}"),
            );
        let at = line(node);
        self.add_node(&id, &name, at);
        if let Some(trait_node) = node.child_by_field_name("trait") {
            let mut refs = Vec::new();
            collect_type_refs(Some(trait_node), self.source, false, &mut refs);
            for (index, (name, _)) in refs.into_iter().enumerate() {
                let target = self.ensure_named_node(&name);
                if target == id {
                    continue;
                }
                self.add_edge(
                    &id,
                    &target,
                    if index == 0 {
                        "implements"
                    } else {
                        "references"
                    },
                    at,
                    (index != 0).then_some("generic_arg"),
                );
            }
        }
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                self.walk(
                    child,
                    Some((&id, &semantic_owner, trait_node_is_absent(node))),
                );
            }
        }
    }

    fn add_use(&mut self, node: Node<'tree>) {
        let Some(argument) = node.child_by_field_name("argument") else {
            return;
        };
        let raw = self.text(argument);
        let clean = raw
            .split('{')
            .next()
            .unwrap_or_default()
            .trim_end_matches(':')
            .trim_end_matches('*')
            .trim_end_matches(':');
        let name = clean.rsplit("::").next().unwrap_or_default().trim();
        if !name.is_empty() {
            let target = make_id(&[name]);
            self.add_edge(
                &self.file_id.clone(),
                &target,
                "imports_from",
                line(node),
                Some("import"),
            );
        }
    }

    fn add_function_references(&mut self, node: Node<'tree>, id: &str, at: usize) {
        if let Some(parameters) = node.child_by_field_name("parameters") {
            let mut cursor = parameters.walk();
            for parameter in parameters
                .children(&mut cursor)
                .filter(|child| child.kind() == "parameter")
            {
                self.add_type_references(
                    parameter.child_by_field_name("type"),
                    id,
                    at,
                    "parameter_type",
                );
            }
        }
        self.add_type_references(
            node.child_by_field_name("return_type"),
            id,
            at,
            "return_type",
        );
    }

    fn add_type_references(
        &mut self,
        node: Option<Node<'tree>>,
        id: &str,
        at: usize,
        context: &str,
    ) {
        let mut refs = Vec::new();
        collect_type_refs(node, self.source, false, &mut refs);
        for (name, generic) in refs {
            let target = self.ensure_named_node(&name);
            if target != id {
                self.add_edge(
                    id,
                    &target,
                    "references",
                    at,
                    Some(if generic { "generic_arg" } else { context }),
                );
            }
        }
    }

    fn walk_calls(&mut self) {
        let mut labels = HashMap::new();
        for node in &self.extraction.nodes {
            let label = node
                .attributes
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or_default();
            labels.insert(
                label
                    .trim_matches(|character| character == '(' || character == ')')
                    .trim_start_matches('.')
                    .to_owned(),
                node.id.clone(),
            );
        }
        let mut pairs = HashSet::new();
        for (caller, body) in self.function_bodies.clone() {
            self.walk_calls_in(body, &caller, &labels, &mut pairs);
        }
    }

    fn walk_calls_in(
        &mut self,
        node: Node<'tree>,
        caller: &str,
        labels: &HashMap<String, String>,
        pairs: &mut HashSet<(String, String, usize, usize)>,
    ) {
        if node.kind() == "function_item" {
            return;
        }
        if node.kind() == "call_expression"
            && let Some(function) = node.child_by_field_name("function")
        {
            let (callee, qualifier, member, scoped) = match function.kind() {
                "identifier" => (Some(self.text(function)), None, false, false),
                "field_expression" => {
                    let qualifier = function
                        .child_by_field_name("value")
                        .map(|value| self.text(value));
                    (
                        function
                            .child_by_field_name("field")
                            .map(|field| self.text(field)),
                        qualifier,
                        true,
                        false,
                    )
                }
                "scoped_identifier" => {
                    let qualifier = function
                        .child_by_field_name("path")
                        .map(|path| self.text(path));
                    (
                        function
                            .child_by_field_name("name")
                            .map(|name| self.text(name)),
                        qualifier,
                        false,
                        true,
                    )
                }
                _ => (None, None, false, false),
            };
            if let Some(callee) = callee {
                let exact_scoped_target = qualifier
                    .as_ref()
                    .filter(|_| scoped)
                    .and_then(|qualifier| {
                        self.scoped_callables
                            .get(&(qualifier.clone(), callee.clone()))
                    })
                    .filter(|targets| targets.len() == 1)
                    .and_then(|targets| targets.first())
                    .cloned();
                self.record_call_candidate(
                    caller,
                    &callee,
                    qualifier.as_deref(),
                    node,
                    scoped && exact_scoped_target.is_none(),
                );
                let target = exact_scoped_target.or_else(|| {
                    (!scoped)
                        .then(|| labels.get(&callee))
                        .flatten()
                        .filter(|target| *target != caller)
                        .cloned()
                });
                if let Some(target) = target {
                    let pair = (
                        caller.to_owned(),
                        target.clone(),
                        node.start_byte(),
                        node.end_byte(),
                    );
                    if pairs.insert(pair) {
                        self.add_call_edge(caller, &target, node);
                    }
                } else if scoped
                    && labels.contains_key(&callee)
                    && let Some(qualifier) = qualifier.as_deref()
                {
                    let target = self.ensure_qualified_external_callable(qualifier, &callee);
                    let pair = (
                        caller.to_owned(),
                        target.clone(),
                        node.start_byte(),
                        node.end_byte(),
                    );
                    if pairs.insert(pair) {
                        self.add_call_edge(caller, &target, node);
                    }
                } else if !scoped
                    && !TRAIT_METHOD_BLOCKLIST.contains(&callee.to_lowercase().as_str())
                {
                    self.extraction.raw_calls_mut().push(RawCall {
                        caller_nid: caller.to_owned(),
                        callee,
                        is_member_call: Some(member),
                        source_file: self.source_file.clone(),
                        source_location: format!("L{}", line(node)),
                        receiver: None,
                        receiver_type: None,
                        lang: None,
                        extensions: crate::facts::node_range(node),
                    });
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_calls_in(child, caller, labels, pairs);
        }
    }

    fn record_call_candidate(
        &mut self,
        owner: &str,
        spelling: &str,
        qualifier: Option<&str>,
        node: Node<'_>,
        external_identity: bool,
    ) {
        let anchor = SourceAnchor {
            source_file: self.source_file.clone(),
            start_byte: u64::try_from(node.start_byte()).unwrap_or(u64::MAX),
            end_byte: u64::try_from(node.end_byte()).unwrap_or(u64::MAX),
        };
        let qualifier = qualifier.map(str::to_owned);
        self.universal_evidence.occurrences.push(OccurrenceFact {
            owner: owner.to_owned(),
            role: OccurrenceRole::Call,
            spelling: spelling.to_owned(),
            qualifier: qualifier.clone(),
            anchor: anchor.clone(),
        });
        self.universal_evidence
            .relationship_candidates
            .push(RelationshipCandidate {
                owner: owner.to_owned(),
                role: OccurrenceRole::Call,
                spelling: spelling.to_owned(),
                qualifier,
                anchor,
                target_kinds: vec![DeclarationKind::Function, DeclarationKind::Method],
                external_identity,
            });
    }

    fn ensure_named_node(&mut self, name: &str) -> String {
        let local = make_id(&[&self.stem, name]);
        if self.seen.contains(&local) {
            return local;
        }
        let id = make_id(&[name]);
        if self.seen.insert(id.clone()) {
            let mut attributes = Map::new();
            attributes.insert("label".into(), Value::String(name.to_owned()));
            attributes.insert("file_type".into(), Value::String("code".into()));
            attributes.insert("source_file".into(), Value::String(String::new()));
            attributes.insert("source_location".into(), Value::String(String::new()));
            attributes.insert(
                "origin_file".into(),
                Value::String(self.source_file.clone()),
            );
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
        }
        id
    }

    fn ensure_qualified_external_callable(&mut self, qualifier: &str, callee: &str) -> String {
        let id = make_id(&["external_call", qualifier, callee]);
        if self.seen.insert(id.clone()) {
            let qualified_name = format!("{}.{}", qualifier.replace("::", "."), callee);
            let mut attributes = Map::new();
            attributes.insert("label".into(), Value::String(format!(".{callee}()")));
            attributes.insert("file_type".into(), Value::String("code".into()));
            attributes.insert("source_file".into(), Value::String(String::new()));
            attributes.insert("source_location".into(), Value::String(String::new()));
            attributes.insert("qualified_name".into(), Value::String(qualified_name));
            attributes.insert("external".into(), Value::Bool(true));
            attributes.insert("placeholder".into(), Value::Bool(true));
            attributes.insert("resolution".into(), Value::String("unresolved".to_owned()));
            self.extraction.nodes.push(NodeRecord {
                id: id.clone(),
                attributes,
            });
        }
        id
    }

    fn add_node(&mut self, id: &str, label: &str, at: usize) {
        if !self.seen.insert(id.to_owned()) {
            return;
        }
        let mut attributes = Map::new();
        attributes.insert("label".into(), Value::String(label.to_owned()));
        attributes.insert("file_type".into(), Value::String("code".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        self.extraction.nodes.push(NodeRecord {
            id: id.to_owned(),
            attributes,
        });
    }

    fn stamp_node_range(&mut self, id: &str, node: Node<'_>) {
        if let Some(record) = self
            .extraction
            .nodes
            .iter_mut()
            .find(|record| record.id == id)
        {
            crate::facts::stamp_node_range(&mut record.attributes, node);
        }
    }

    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        relation: &str,
        at: usize,
        context: Option<&str>,
    ) {
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String(relation.to_owned()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert("source_location".into(), Value::String(format!("L{at}")));
        attributes.insert("weight".into(), Value::from(1.0));
        if let Some(context) = context {
            attributes.insert("context".into(), Value::String(context.to_owned()));
        }
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn add_call_edge(&mut self, source: &str, target: &str, node: Node<'tree>) {
        let mut attributes = Map::new();
        attributes.insert("relation".into(), Value::String("calls".into()));
        attributes.insert("context".into(), Value::String("call".into()));
        attributes.insert("confidence".into(), Value::String("EXTRACTED".into()));
        attributes.insert(
            "source_file".into(),
            Value::String(self.source_file.clone()),
        );
        attributes.insert(
            "source_location".into(),
            Value::String(format!("L{}", line(node))),
        );
        attributes.insert("weight".into(), Value::from(1.0));
        crate::facts::stamp_node_range(&mut attributes, node);
        self.extraction.edges.push(EdgeRecord {
            source: source.to_owned(),
            target: target.to_owned(),
            attributes,
        });
    }

    fn text(&self, node: Node<'_>) -> String {
        node.utf8_text(self.source).unwrap_or_default().to_owned()
    }
}

fn collect_type_refs(
    node: Option<Node<'_>>,
    source: &[u8],
    generic: bool,
    output: &mut Vec<(String, bool)>,
) {
    let Some(node) = node else { return };
    match node.kind() {
        "primitive_type" => return,
        "type_identifier" => {
            let name = node.utf8_text(source).unwrap_or_default();
            if !name.is_empty() {
                output.push((name.to_owned(), generic));
            }
            return;
        }
        "scoped_type_identifier" => {
            let name = node
                .utf8_text(source)
                .unwrap_or_default()
                .rsplit("::")
                .next()
                .unwrap_or_default();
            if !name.is_empty() {
                output.push((name.to_owned(), generic));
            }
            return;
        }
        "generic_type" => {
            let type_node = node.child_by_field_name("type").or_else(|| {
                let mut cursor = node.walk();
                node.children(&mut cursor).find(|child| {
                    matches!(child.kind(), "type_identifier" | "scoped_type_identifier")
                })
            });
            if let Some(type_node) = type_node {
                let name = type_node
                    .utf8_text(source)
                    .unwrap_or_default()
                    .rsplit("::")
                    .next()
                    .unwrap_or_default();
                if !name.is_empty() {
                    output.push((name.to_owned(), generic));
                }
            }
            let mut cursor = node.walk();
            for arguments in node
                .children(&mut cursor)
                .filter(|child| child.kind() == "type_arguments")
            {
                let mut arguments_cursor = arguments.walk();
                for argument in arguments
                    .children(&mut arguments_cursor)
                    .filter(|child| child.is_named())
                {
                    collect_type_refs(Some(argument), source, true, output);
                }
            }
            return;
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_type_refs(Some(child), source, generic, output);
    }
}

fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "type_identifier"
            | "generic_type"
            | "scoped_type_identifier"
            | "reference_type"
            | "primitive_type"
            | "tuple_type"
            | "array_type"
    )
}

fn line(node: Node<'_>) -> usize {
    node.start_position().row + 1
}

fn trait_node_is_absent(node: Node<'_>) -> bool {
    node.child_by_field_name("trait").is_none()
}
