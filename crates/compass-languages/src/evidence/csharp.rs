//! Direct universal evidence for C# and .NET source.
//!
//! The adapter is deliberately AST-first and project-neutral. It emits exact
//! declarations, scopes, bindings, occurrences, and constrained relationship
//! candidates; cross-file target choice remains owned by `compass-resolve`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use tree_sitter::Node;

use super::build::{EvidenceBuilder, range_for_byte_span, range_for_node};
use super::model::{
    BindingKind, CandidateRelation, HierarchyConstraint, ReceiverDispatchStrategy,
    ResolutionConstraint, SemanticEvidenceBatch, SemanticRole, SymbolNamespace,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits};
use crate::{AdapterRegistry, file_stem, make_id};

const PRODUCER: &str = "compass.languages.csharp.universal";
const MAX_TRAVERSAL_DEPTH: usize = 512;

const TYPE_KINDS: &[&str] = &[
    "class_declaration",
    "delegate_declaration",
    "enum_declaration",
    "interface_declaration",
    "record_declaration",
    "struct_declaration",
];

const CALLABLE_KINDS: &[&str] = &[
    "constructor_declaration",
    "conversion_operator_declaration",
    "destructor_declaration",
    "local_function_statement",
    "method_declaration",
    "operator_declaration",
];

#[derive(Clone, Debug)]
struct Decl {
    id: String,
    qualified: String,
    kind: String,
    scope_id: String,
    start: usize,
    end: usize,
    enclosing_type: Option<String>,
}

#[derive(Clone, Debug)]
struct Import {
    spelling: String,
    target: String,
    alias: bool,
    scope_id: String,
}

struct State<'source> {
    source: &'source [u8],
    source_file: &'source str,
    builder: EvidenceBuilder,
    file: Decl,
    declarations: Vec<Decl>,
    types: BTreeMap<String, Vec<usize>>,
    imports: Vec<Import>,
    value_types: HashMap<(String, String), String>,
    generic_constraints: HashMap<(String, String), String>,
    direct_class_bases: BTreeMap<String, Vec<String>>,
}

pub(super) fn extract_candidate_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    let profile = AdapterRegistry::universal_profile("csharp").ok_or_else(|| {
        EvidenceError::new(
            EvidenceErrorCode::InvalidAdapter,
            "C# universal adapter is not registered",
        )
    })?;
    let mut builder =
        EvidenceBuilder::new(profile, PRODUCER, source_file, EvidenceLimits::default());
    let file_graph_id = make_id(&[source_file]);
    let file_module = file_stem(Path::new(source_file));
    let file_id = builder.declare(
        "file",
        &file_graph_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(source_file),
        source_file,
        Some(&file_module),
        None,
        range_for_node(source_file, root),
    )?;
    let file_scope = builder.open_scope(
        "file",
        Some(&file_id),
        None,
        range_for_node(source_file, root),
    )?;
    let file = Decl {
        id: file_id,
        qualified: source_file.to_owned(),
        kind: "file".to_owned(),
        scope_id: file_scope,
        start: root.start_byte(),
        end: root.end_byte(),
        enclosing_type: None,
    };
    let mut state = State {
        source,
        source_file,
        builder,
        file,
        declarations: Vec::new(),
        types: BTreeMap::new(),
        imports: Vec::new(),
        value_types: HashMap::new(),
        generic_constraints: HashMap::new(),
        direct_class_bases: BTreeMap::new(),
    };
    state.capture_errors(root, 0)?;
    let file_namespace = file_scoped_namespace(root, source).unwrap_or_default();
    state.collect_declarations(root, &file_namespace, None, None, 0)?;
    state.collect_imports(root, 0)?;
    state.collect_semantics(root, 0)?;
    if root.has_error() {
        state.builder.diagnose(
            "partial_parser_recovery",
            None,
            Some(range_for_node(source_file, root)),
            "parser recovered from malformed C# source; emitted evidence remains source-bounded",
        )?;
    }
    state.builder.finish()
}

impl<'source> State<'source> {
    fn collect_declarations(
        &mut self,
        node: Node<'_>,
        namespace: &str,
        owner: Option<usize>,
        parent_scope: Option<&str>,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        let scope = parent_scope.unwrap_or(&self.file.scope_id).to_owned();
        if matches!(
            node.kind(),
            "namespace_declaration" | "file_scoped_namespace_declaration"
        ) {
            let Some(name) = node.child_by_field_name("name") else {
                return Ok(());
            };
            let segment = self.text(name).trim();
            if segment.is_empty() {
                return Ok(());
            }
            let nested =
                if node.kind() == "file_scoped_namespace_declaration" && namespace == segment {
                    namespace.to_owned()
                } else {
                    join_qualified(namespace, segment, ".")
                };
            let namespace_graph = make_id(&["csharp", "namespace", &nested]);
            let namespace_id = self.builder.declare_with_namespace(
                "namespace",
                &namespace_graph,
                segment,
                &nested,
                Some(&nested),
                Some(&scope),
                Some(SymbolNamespace::Namespace),
                self.evidence_range(name),
            )?;
            let namespace_scope = self.builder.open_scope(
                "namespace",
                Some(&namespace_id),
                Some(&scope),
                self.evidence_range(node),
            )?;
            let namespace_index = self.declarations.len();
            self.declarations.push(Decl {
                id: namespace_id.clone(),
                qualified: nested.clone(),
                kind: "namespace".to_owned(),
                scope_id: namespace_scope.clone(),
                start: node.start_byte(),
                end: node.end_byte(),
                enclosing_type: None,
            });
            let owner_id = owner
                .map(|owner| self.declarations[owner].id.clone())
                .unwrap_or_else(|| self.file.id.clone());
            self.own(&owner_id, &namespace_id)?;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                if child.id() != name.id() {
                    self.collect_declarations(
                        child,
                        &nested,
                        Some(namespace_index),
                        Some(&namespace_scope),
                        depth + 1,
                    )?;
                }
            }
            return Ok(());
        }
        if TYPE_KINDS.contains(&node.kind()) {
            let Some(name_node) = node.child_by_field_name("name") else {
                return Ok(());
            };
            let name = self.text(name_node).trim().to_owned();
            if name.is_empty() {
                return Ok(());
            }
            let owner_qualified = owner.map(|index| self.declarations[index].qualified.as_str());
            let qualified = owner_qualified.map_or_else(
                || join_qualified(namespace, &name, "."),
                |owner| join_qualified(owner, &name, "."),
            );
            let kind = csharp_type_kind(node.kind());
            let graph_id = make_id(&["csharp", kind, &qualified]);
            let direct_bases_complete = matches!(kind, "class" | "interface" | "record" | "struct")
                && first_named_child(node, &["base_list"]).is_none_or(|base| !base.has_error());
            let declaration_id = self.builder.declare_type(
                kind,
                &graph_id,
                &name,
                &qualified,
                (!namespace.is_empty()).then_some(namespace),
                Some(&scope),
                Some(SymbolNamespace::ValueAndType),
                type_parameter_signature(node, self.source).as_deref(),
                direct_bases_complete,
                self.evidence_range(name_node),
            )?;
            let type_scope = self.builder.open_scope(
                kind,
                Some(&declaration_id),
                Some(&scope),
                self.evidence_range(node),
            )?;
            let index = self.declarations.len();
            self.declarations.push(Decl {
                id: declaration_id.clone(),
                qualified: qualified.clone(),
                kind: kind.to_owned(),
                scope_id: type_scope.clone(),
                start: node.start_byte(),
                end: node.end_byte(),
                enclosing_type: Some(qualified.clone()),
            });
            self.types.entry(name).or_default().push(index);
            let owner_id = owner
                .map(|owner| self.declarations[owner].id.clone())
                .unwrap_or_else(|| self.file.id.clone());
            self.own(&owner_id, &declaration_id)?;
            self.collect_generic_constraints(node, &type_scope);
            self.collect_base_types(node, index)?;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(|child| child.is_named()) {
                if child.id() != name_node.id() && child.kind() != "base_list" {
                    self.collect_declarations(
                        child,
                        namespace,
                        Some(index),
                        Some(&type_scope),
                        depth + 1,
                    )?;
                }
            }
            return Ok(());
        }
        if CALLABLE_KINDS.contains(&node.kind()) {
            self.add_callable(node, namespace, owner, &scope)?;
            return Ok(());
        }
        if matches!(
            node.kind(),
            "property_declaration" | "indexer_declaration" | "event_declaration"
        ) {
            self.add_named_member(node, namespace, owner, &scope)?;
            return Ok(());
        }
        if matches!(node.kind(), "field_declaration" | "event_field_declaration") {
            self.add_fields(node, namespace, owner, &scope)?;
            return Ok(());
        }
        if node.kind() == "enum_member_declaration" {
            self.add_enum_member(node, owner, &scope)?;
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_declarations(child, namespace, owner, Some(&scope), depth + 1)?;
        }
        Ok(())
    }

    fn add_callable(
        &mut self,
        node: Node<'_>,
        namespace: &str,
        owner: Option<usize>,
        parent_scope: &str,
    ) -> Result<(), EvidenceError> {
        let name_node = node.child_by_field_name("name").or_else(|| {
            (node.kind() == "constructor_declaration")
                .then(|| first_named_child(node, &["identifier"]))
                .flatten()
        });
        let Some(name_node) = name_node else {
            return Ok(());
        };
        let name = self.text(name_node).trim().to_owned();
        if name.is_empty() {
            return Ok(());
        }
        let owner_qualified = owner.map(|index| self.declarations[index].qualified.clone());
        let qualified = owner_qualified.as_deref().map_or_else(
            || join_qualified(namespace, &name, "::"),
            |owner| join_qualified(owner, &name, "::"),
        );
        let parameter_types = parameter_types(node, self.source);
        let signature = callable_signature(node, &name, &parameter_types, self.source);
        let kind = callable_kind(node.kind());
        let graph_id = make_id(&["csharp", kind, &qualified]);
        let declaration_id = self.builder.declare_callable(
            kind,
            &graph_id,
            &name,
            &qualified,
            (!namespace.is_empty()).then_some(namespace),
            Some(parent_scope),
            Some(SymbolNamespace::Value),
            Some(&signature),
            parameter_types.clone(),
            has_params_parameter(node, self.source),
            self.evidence_range(name_node),
        )?;
        let callable_scope = self.builder.open_scope(
            kind,
            Some(&declaration_id),
            Some(parent_scope),
            self.evidence_range(node),
        )?;
        let index = self.declarations.len();
        self.declarations.push(Decl {
            id: declaration_id.clone(),
            qualified,
            kind: kind.to_owned(),
            scope_id: callable_scope.clone(),
            start: node.start_byte(),
            end: node.end_byte(),
            enclosing_type: owner_qualified,
        });
        let owner_id = owner
            .map(|owner| self.declarations[owner].id.clone())
            .unwrap_or_else(|| self.file.id.clone());
        self.own(&owner_id, &declaration_id)?;
        self.collect_generic_constraints(node, &callable_scope);
        self.collect_override(node, index)?;
        self.collect_parameter_types(node, index, &callable_scope)?;
        self.collect_return_type(node, index, &callable_scope)?;
        self.collect_local_value_types(node, &callable_scope);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            if child.kind() == "local_function_statement" {
                self.collect_declarations(child, namespace, Some(index), Some(&callable_scope), 1)?;
            }
        }
        Ok(())
    }

    fn add_named_member(
        &mut self,
        node: Node<'_>,
        namespace: &str,
        owner: Option<usize>,
        scope: &str,
    ) -> Result<(), EvidenceError> {
        let Some(owner) = owner else { return Ok(()) };
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let name = self.text(name_node).trim();
        if name.is_empty() {
            return Ok(());
        }
        let qualified = join_qualified(&self.declarations[owner].qualified, name, "::");
        let kind = match node.kind() {
            "event_declaration" => "event",
            "indexer_declaration" => "indexer",
            _ => "property",
        };
        let graph_id = make_id(&["csharp", kind, &qualified]);
        let type_node = node.child_by_field_name("type");
        let signature = type_node.map(|node| canonical_type(self.text(node)));
        let id = self.builder.declare_with_signature(
            kind,
            &graph_id,
            name,
            &qualified,
            (!namespace.is_empty()).then_some(namespace),
            Some(scope),
            Some(SymbolNamespace::Value),
            signature.as_deref(),
            self.evidence_range(name_node),
        )?;
        self.own(&self.declarations[owner].id.clone(), &id)?;
        if let Some(value_type) = signature {
            self.value_types
                .insert((scope.to_owned(), name.to_owned()), value_type.clone());
            if let Some(type_node) = type_node {
                self.type_relation(
                    &id,
                    scope,
                    type_node,
                    &value_type,
                    "property_type",
                    CandidateRelation::TypeOf,
                )?;
            }
        }
        Ok(())
    }

    fn add_fields(
        &mut self,
        node: Node<'_>,
        namespace: &str,
        owner: Option<usize>,
        scope: &str,
    ) -> Result<(), EvidenceError> {
        let Some(owner) = owner else { return Ok(()) };
        let Some(variable) = first_descendant(node, "variable_declaration") else {
            return Ok(());
        };
        let value_type = variable
            .child_by_field_name("type")
            .map(|node| canonical_type(self.text(node)));
        let mut stack = vec![variable];
        while let Some(current) = stack.pop() {
            if current.kind() == "variable_declarator" {
                let Some(name_node) = current
                    .child_by_field_name("name")
                    .or_else(|| first_named_child(current, &["identifier"]))
                else {
                    continue;
                };
                let name = self.text(name_node).trim();
                if name.is_empty() {
                    continue;
                }
                let qualified = join_qualified(&self.declarations[owner].qualified, name, "::");
                let kind = if node.kind() == "event_field_declaration" {
                    "event"
                } else if self
                    .text(node)
                    .split_whitespace()
                    .any(|token| token == "const")
                {
                    "constant"
                } else {
                    "field"
                };
                let graph_id = make_id(&["csharp", kind, &qualified]);
                let id = self.builder.declare_with_signature(
                    kind,
                    &graph_id,
                    name,
                    &qualified,
                    (!namespace.is_empty()).then_some(namespace),
                    Some(scope),
                    Some(SymbolNamespace::Value),
                    value_type.as_deref(),
                    self.evidence_range(name_node),
                )?;
                self.own(&self.declarations[owner].id.clone(), &id)?;
                if let Some(value_type) = value_type.as_ref() {
                    self.value_types
                        .insert((scope.to_owned(), name.to_owned()), value_type.clone());
                    self.type_relation(
                        &id,
                        scope,
                        current,
                        value_type,
                        "field_type",
                        CandidateRelation::TypeOf,
                    )?;
                }
                continue;
            }
            let mut cursor = current.walk();
            stack.extend(
                current
                    .children(&mut cursor)
                    .filter(|child| child.is_named()),
            );
        }
        Ok(())
    }

    fn add_enum_member(
        &mut self,
        node: Node<'_>,
        owner: Option<usize>,
        scope: &str,
    ) -> Result<(), EvidenceError> {
        let Some(owner) = owner else { return Ok(()) };
        let Some(name_node) = node
            .child_by_field_name("name")
            .or_else(|| first_named_child(node, &["identifier"]))
        else {
            return Ok(());
        };
        let name = self.text(name_node).trim();
        if name.is_empty() {
            return Ok(());
        }
        let qualified = join_qualified(&self.declarations[owner].qualified, name, "::");
        let graph_id = make_id(&["csharp", "enum_member", &qualified]);
        let id = self.builder.declare_with_namespace(
            "enum_member",
            &graph_id,
            name,
            &qualified,
            None,
            Some(scope),
            Some(SymbolNamespace::Value),
            self.evidence_range(name_node),
        )?;
        self.own(&self.declarations[owner].id.clone(), &id)
    }

    fn collect_imports(&mut self, root: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(root);
        }
        if root.kind() == "using_directive" {
            let raw = self.text(root).trim().trim_end_matches(';').trim();
            let raw = raw.strip_prefix("global ").unwrap_or(raw).trim();
            let Some(body) = raw.strip_prefix("using") else {
                return Ok(());
            };
            let body = body.trim();
            let scope = self.enclosing_scope(root.start_byte());
            let (kind, spelling, target, alias) = if let Some(target) = body.strip_prefix("static ")
            {
                (
                    BindingKind::Import,
                    terminal(target.trim()),
                    target.trim(),
                    false,
                )
            } else if let Some((alias, target)) = body.split_once('=') {
                (BindingKind::ImportAlias, alias.trim(), target.trim(), true)
            } else {
                (BindingKind::Import, terminal(body), body, false)
            };
            if !target.is_empty() {
                let binding = self.builder.bind_with_identity(
                    kind,
                    spelling,
                    target,
                    None,
                    Some(&scope),
                    Some(if alias {
                        SymbolNamespace::ValueAndType
                    } else {
                        SymbolNamespace::Namespace
                    }),
                    false,
                    self.evidence_range(root),
                )?;
                let occurrence = self.builder.occur(
                    SemanticRole::Import,
                    &self.file.id,
                    target,
                    None,
                    Some(&scope),
                    self.evidence_range(root),
                )?;
                self.builder.relate(
                    CandidateRelation::Imports,
                    &self.file.id,
                    Some(&occurrence),
                    Some(&binding),
                    target,
                    ResolutionConstraint {
                        exact_language: Some("csharp".to_owned()),
                        qualified_name: Some(target.to_owned()),
                        allow_external: true,
                        ..ResolutionConstraint::default()
                    },
                )?;
                self.imports.push(Import {
                    spelling: spelling.to_owned(),
                    target: target.to_owned(),
                    alias,
                    scope_id: scope,
                });
            }
            return Ok(());
        }
        let mut cursor = root.walk();
        for child in root.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_imports(child, depth + 1)?;
        }
        Ok(())
    }

    fn collect_semantics(&mut self, root: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(root);
        }
        match root.kind() {
            "attribute" => self.add_attribute(root)?,
            "invocation_expression" => self.add_invocation(root)?,
            "object_creation_expression" | "implicit_object_creation_expression" => {
                self.add_construction(root)?
            }
            _ => {}
        }
        let mut cursor = root.walk();
        for child in root.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_semantics(child, depth + 1)?;
        }
        Ok(())
    }

    fn add_attribute(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(owner) = self.enclosing_declaration(node.start_byte()) else {
            return Ok(());
        };
        let Some(name_node) = node.child_by_field_name("name").or_else(|| {
            first_named_child(
                node,
                &["identifier", "qualified_name", "alias_qualified_name"],
            )
        }) else {
            return Ok(());
        };
        let spelling = self.text(name_node).trim();
        let scope = self.declarations[owner].scope_id.clone();
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Annotation,
            &self.declarations[owner].id,
            spelling,
            spelling.rsplit_once('.').map(|(qualifier, _)| qualifier),
            Some(&scope),
            Some("attribute"),
            self.evidence_range(node),
        )?;
        let qualified = self.resolve_type_name(spelling, &scope, true);
        self.builder.relate(
            CandidateRelation::Annotates,
            &self.declarations[owner].id,
            Some(&occurrence),
            None,
            terminal(spelling),
            ResolutionConstraint {
                exact_language: Some("csharp".to_owned()),
                scope_id: Some(scope),
                qualified_name: qualified,
                allowed_target_kinds: vec!["class".to_owned(), "record".to_owned()],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn add_invocation(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(owner) = self.enclosing_callable(node.start_byte()) else {
            return Ok(());
        };
        let Some(function) = node.child_by_field_name("function") else {
            return Ok(());
        };
        let (spelling, qualifier) = if function.kind() == "member_access_expression" {
            let Some(name) = function.child_by_field_name("name") else {
                return Ok(());
            };
            let qualifier = function
                .child_by_field_name("expression")
                .map(|node| self.text(node).trim().to_owned());
            (self.text(name).trim().to_owned(), qualifier)
        } else {
            (terminal(self.text(function).trim()).to_owned(), None)
        };
        if spelling.is_empty() {
            return Ok(());
        }
        let scope = self.declarations[owner].scope_id.clone();
        let occurrence = self.builder.occur(
            SemanticRole::Call,
            &self.declarations[owner].id,
            &spelling,
            qualifier.as_deref(),
            Some(&scope),
            self.evidence_range(node),
        )?;
        let receiver_type = qualifier
            .as_deref()
            .and_then(|receiver| self.receiver_type(receiver, owner));
        let hierarchy = receiver_type.clone().map(|receiver_qualified_name| {
            HierarchyConstraint::ReceiverDispatch {
                receiver_qualified_name,
                strategy: ReceiverDispatchStrategy::C3FromReceiver,
            }
        });
        let qualified = if hierarchy.is_none() {
            receiver_type
                .as_deref()
                .map(|receiver| join_qualified(receiver, &spelling, "::"))
        } else {
            None
        };
        self.builder.relate(
            CandidateRelation::Calls,
            &self.declarations[owner].id,
            Some(&occurrence),
            None,
            &spelling,
            ResolutionConstraint {
                exact_language: Some("csharp".to_owned()),
                scope_id: Some(scope),
                qualified_name: qualified,
                argument_count: Some(argument_count(node)),
                allowed_target_kinds: vec![
                    "constructor".to_owned(),
                    "local_function".to_owned(),
                    "method".to_owned(),
                    "operator".to_owned(),
                ],
                hierarchy,
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn add_construction(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(owner) = self.enclosing_callable(node.start_byte()) else {
            return Ok(());
        };
        let Some(type_node) = node.child_by_field_name("type") else {
            return Ok(());
        };
        let spelling = canonical_type(self.text(type_node));
        if spelling.is_empty() {
            return Ok(());
        }
        let scope = self.declarations[owner].scope_id.clone();
        let occurrence = self.builder.occur(
            SemanticRole::Construction,
            &self.declarations[owner].id,
            terminal(&spelling),
            spelling.rsplit_once('.').map(|(qualifier, _)| qualifier),
            Some(&scope),
            self.evidence_range(node),
        )?;
        self.builder.relate(
            CandidateRelation::Constructs,
            &self.declarations[owner].id,
            Some(&occurrence),
            None,
            terminal(&spelling),
            ResolutionConstraint {
                exact_language: Some("csharp".to_owned()),
                scope_id: Some(scope.clone()),
                qualified_name: self.resolve_type_name(&spelling, &scope, false),
                argument_count: Some(argument_count(node)),
                allowed_target_kinds: vec![
                    "class".to_owned(),
                    "record".to_owned(),
                    "struct".to_owned(),
                ],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn collect_base_types(&mut self, node: Node<'_>, owner: usize) -> Result<(), EvidenceError> {
        let Some(base_list) = first_named_child(node, &["base_list"]) else {
            return Ok(());
        };
        let mut base_nodes = Vec::new();
        collect_direct_type_children(base_list, &mut base_nodes);
        let complete = !base_list.has_error();
        for base in base_nodes {
            let spelling = canonical_type(self.text(base));
            if spelling.is_empty() || is_predefined(&spelling) {
                continue;
            }
            let scope = self.declarations[owner].scope_id.clone();
            let occurrence = self.builder.occur_with_context(
                SemanticRole::BaseType,
                &self.declarations[owner].id,
                terminal(&spelling),
                spelling.rsplit_once('.').map(|(qualifier, _)| qualifier),
                Some(&scope),
                Some("base_type"),
                self.evidence_range(base),
            )?;
            let qualified = self.resolve_type_name(&spelling, &scope, false);
            let owner_kind = self.declarations[owner].kind.as_str();
            let relation = if owner_kind == "struct" {
                CandidateRelation::Implements
            } else {
                // C# base-list syntax does not identify whether an unresolved
                // first entry is a class or interface. Resolve one typed base
                // candidate project-wide, then let projection publish
                // `implements` when its exact source target is an interface.
                CandidateRelation::Extends
            };
            let allowed_target_kinds = match owner_kind {
                "interface" | "struct" => vec!["interface".to_owned()],
                _ => vec![
                    "class".to_owned(),
                    "interface".to_owned(),
                    "record".to_owned(),
                ],
            };
            self.builder.relate(
                relation,
                &self.declarations[owner].id,
                Some(&occurrence),
                None,
                terminal(&spelling),
                ResolutionConstraint {
                    exact_language: Some("csharp".to_owned()),
                    scope_id: Some(scope.clone()),
                    qualified_name: qualified,
                    allowed_target_kinds,
                    hierarchy: Some(HierarchyConstraint::DirectBase {
                        base_set_complete: complete,
                    }),
                    allow_external: true,
                    ..ResolutionConstraint::default()
                },
            )?;
            if relation == CandidateRelation::Extends
                && self
                    .unique_local_type(&spelling)
                    .is_none_or(|index| self.declarations[index].kind != "interface")
                && let Some(qualified) = self.resolve_type_name(&spelling, &scope, false)
            {
                self.direct_class_bases
                    .entry(self.declarations[owner].qualified.clone())
                    .or_default()
                    .push(qualified);
            }
        }
        Ok(())
    }

    fn collect_override(&mut self, node: Node<'_>, owner: usize) -> Result<(), EvidenceError> {
        if !self
            .text(node)
            .split(|character: char| !character.is_alphanumeric() && character != '_')
            .any(|token| token == "override")
        {
            return Ok(());
        }
        let Some(enclosing_type) = self.declarations[owner].enclosing_type.clone() else {
            return Ok(());
        };
        let bases = self
            .direct_class_bases
            .get(&enclosing_type)
            .cloned()
            .unwrap_or_default();
        let method_name = terminal(&self.declarations[owner].qualified).to_owned();
        for base in bases {
            let occurrence = self.builder.occur_with_context(
                SemanticRole::Override,
                &self.declarations[owner].id,
                &method_name,
                Some(&base),
                Some(&self.declarations[owner].scope_id),
                Some("override_modifier"),
                self.evidence_range(node),
            )?;
            self.builder.relate(
                CandidateRelation::Overrides,
                &self.declarations[owner].id,
                Some(&occurrence),
                None,
                &method_name,
                ResolutionConstraint {
                    exact_language: Some("csharp".to_owned()),
                    scope_id: Some(self.declarations[owner].scope_id.clone()),
                    qualified_name: Some(join_qualified(&base, &method_name, "::")),
                    argument_count: Some(
                        u32::try_from(parameter_types(node, self.source).len()).unwrap_or(u32::MAX),
                    ),
                    allowed_target_kinds: vec!["method".to_owned()],
                    allow_external: false,
                    ..ResolutionConstraint::default()
                },
            )?;
        }
        Ok(())
    }

    fn collect_parameter_types(
        &mut self,
        node: Node<'_>,
        owner: usize,
        scope: &str,
    ) -> Result<(), EvidenceError> {
        let Some(parameters) = node
            .child_by_field_name("parameters")
            .or_else(|| first_named_child(node, &["parameter_list", "bracketed_parameter_list"]))
        else {
            return Ok(());
        };
        let mut parameter_nodes = Vec::new();
        collect_descendants(parameters, "parameter", &mut parameter_nodes);
        parameter_nodes.sort_unstable_by_key(Node::start_byte);
        for current in parameter_nodes {
            if let (Some(name), Some(value_type)) = (
                current.child_by_field_name("name"),
                current.child_by_field_name("type"),
            ) {
                let spelling = canonical_type(self.text(value_type));
                let parameter_name = self.text(name).trim();
                if parameter_name.is_empty() {
                    continue;
                }
                let qualified =
                    join_qualified(&self.declarations[owner].qualified, parameter_name, "::");
                let graph_id = make_id(&["csharp", "parameter", &qualified]);
                let parameter_id = self.builder.declare_with_signature(
                    "parameter",
                    &graph_id,
                    parameter_name,
                    &qualified,
                    None,
                    Some(scope),
                    Some(SymbolNamespace::Value),
                    Some(&spelling),
                    self.evidence_range(name),
                )?;
                self.own(&self.declarations[owner].id.clone(), &parameter_id)?;
                self.value_types.insert(
                    (scope.to_owned(), parameter_name.to_owned()),
                    spelling.clone(),
                );
                self.type_relation(
                    &parameter_id,
                    scope,
                    value_type,
                    &spelling,
                    "parameter_type",
                    CandidateRelation::TypeOf,
                )?;
            }
        }
        Ok(())
    }

    fn collect_return_type(
        &mut self,
        node: Node<'_>,
        owner: usize,
        _scope: &str,
    ) -> Result<(), EvidenceError> {
        let result = node
            .child_by_field_name("returns")
            .or_else(|| node.child_by_field_name("type"));
        if let Some(result) = result {
            let spelling = canonical_type(self.text(result));
            if !spelling.is_empty() && spelling != "void" {
                let declaration_id = self.declarations[owner].id.clone();
                let scope = self.declarations[owner].scope_id.clone();
                self.type_relation(
                    &declaration_id,
                    &scope,
                    result,
                    &spelling,
                    "return_type",
                    CandidateRelation::Returns,
                )?;
            }
        }
        Ok(())
    }

    fn type_relation(
        &mut self,
        source_declaration_id: &str,
        scope: &str,
        node: Node<'_>,
        spelling: &str,
        context: &str,
        relation: CandidateRelation,
    ) -> Result<(), EvidenceError> {
        if is_predefined(spelling) {
            return Ok(());
        }
        let occurrence = self.builder.occur_with_context(
            SemanticRole::TypeReference,
            source_declaration_id,
            terminal(spelling),
            spelling.rsplit_once('.').map(|(qualifier, _)| qualifier),
            Some(scope),
            Some(context),
            self.evidence_range(node),
        )?;
        self.builder.relate(
            relation,
            source_declaration_id,
            Some(&occurrence),
            None,
            terminal(spelling),
            ResolutionConstraint {
                exact_language: Some("csharp".to_owned()),
                scope_id: Some(scope.to_owned()),
                qualified_name: self.resolve_type_name(spelling, scope, false),
                allowed_target_kinds: vec![
                    "class".to_owned(),
                    "delegate".to_owned(),
                    "enum".to_owned(),
                    "interface".to_owned(),
                    "record".to_owned(),
                    "struct".to_owned(),
                ],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn collect_local_value_types(&mut self, node: Node<'_>, scope: &str) {
        let mut stack = vec![node];
        while let Some(current) = stack.pop() {
            if current.kind() == "local_declaration_statement"
                && let Some(variable) = first_descendant(current, "variable_declaration")
            {
                let declared = variable
                    .child_by_field_name("type")
                    .map(|node| canonical_type(self.text(node)));
                let mut cursor = variable.walk();
                for declarator in variable
                    .children(&mut cursor)
                    .filter(|child| child.kind() == "variable_declarator")
                {
                    let Some(name) = declarator
                        .child_by_field_name("name")
                        .or_else(|| first_named_child(declarator, &["identifier"]))
                    else {
                        continue;
                    };
                    let inferred = declared.clone().filter(|value| value != "var").or_else(|| {
                        first_descendant(declarator, "object_creation_expression")
                            .and_then(|creation| creation.child_by_field_name("type"))
                            .map(|node| canonical_type(self.text(node)))
                    });
                    if let Some(inferred) = inferred {
                        self.value_types.insert(
                            (scope.to_owned(), self.text(name).trim().to_owned()),
                            inferred,
                        );
                    }
                }
            }
            let mut cursor = current.walk();
            stack.extend(
                current
                    .children(&mut cursor)
                    .filter(|child| child.is_named()),
            );
        }
    }

    fn collect_generic_constraints(&mut self, node: Node<'_>, scope: &str) {
        let mut cursor = node.walk();
        for clause in node
            .children(&mut cursor)
            .filter(|child| child.kind() == "type_parameter_constraints_clause")
        {
            let raw = self.text(clause).trim();
            let Some(raw) = raw.strip_prefix("where") else {
                continue;
            };
            let Some((parameter, constraints)) = raw.split_once(':') else {
                continue;
            };
            let parameter = parameter.trim();
            let constraint = constraints
                .split(',')
                .map(str::trim)
                .map(nominal_type)
                .find(|constraint| {
                    !constraint.is_empty()
                        && !matches!(
                            constraint.as_str(),
                            "class" | "class?" | "default" | "notnull" | "struct" | "unmanaged"
                        )
                        && !constraint.starts_with("new(")
                });
            if !parameter.is_empty()
                && let Some(constraint) = constraint
            {
                self.generic_constraints
                    .insert((scope.to_owned(), parameter.to_owned()), constraint);
            }
        }
    }

    fn own(&mut self, owner: &str, child: &str) -> Result<(), EvidenceError> {
        self.builder.relate(
            CandidateRelation::Owns,
            owner,
            None,
            None,
            child,
            ResolutionConstraint {
                exact_target_declaration_id: Some(child.to_owned()),
                exact_language: Some("csharp".to_owned()),
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn resolve_type_name(&self, spelling: &str, scope: &str, attribute: bool) -> Option<String> {
        let normalized = canonical_type(spelling);
        if normalized.contains('.') {
            return Some(normalized);
        }
        for import in self.imports.iter().rev() {
            if import.scope_id == scope && import.alias && import.spelling == normalized {
                return Some(import.target.clone());
            }
        }
        if let Some(index) = self.unique_local_type(&normalized) {
            return Some(self.declarations[index].qualified.clone());
        }
        if attribute {
            let attribute_name = if normalized.ends_with("Attribute") {
                normalized
            } else {
                format!("{normalized}Attribute")
            };
            let namespaces = self
                .imports
                .iter()
                .filter(|import| !import.alias)
                .map(|import| import.target.as_str())
                .collect::<BTreeSet<_>>();
            if namespaces.len() == 1 {
                return namespaces
                    .first()
                    .map(|namespace| format!("{namespace}.{attribute_name}"));
            }
        }
        None
    }

    fn unique_local_type(&self, spelling: &str) -> Option<usize> {
        let values = self.types.get(terminal(spelling))?;
        (values.len() == 1).then_some(values[0])
    }

    fn receiver_type(&self, receiver: &str, owner: usize) -> Option<String> {
        if receiver == "this" {
            return self.declarations[owner].enclosing_type.clone();
        }
        if receiver == "base" {
            return self.declarations[owner].enclosing_type.clone();
        }
        let callable_scope = self.declarations[owner].scope_id.as_str();
        let type_scope = self.declarations[owner]
            .enclosing_type
            .as_deref()
            .and_then(|qualified| {
                self.declarations
                    .iter()
                    .find(|declaration| declaration.qualified == qualified)
                    .map(|declaration| declaration.scope_id.as_str())
            });
        [Some(callable_scope), type_scope]
            .into_iter()
            .flatten()
            .find_map(|scope| {
                self.value_types
                    .get(&(scope.to_owned(), receiver.to_owned()))
                    .map(|value_type| {
                        self.constrained_receiver_type(value_type, callable_scope, type_scope)
                    })
            })
            .or_else(|| {
                self.unique_local_type(receiver)
                    .map(|index| self.declarations[index].qualified.clone())
            })
    }

    fn constrained_receiver_type(
        &self,
        value_type: &str,
        callable_scope: &str,
        type_scope: Option<&str>,
    ) -> String {
        let nominal = nominal_type(value_type);
        let constrained = [Some(callable_scope), type_scope]
            .into_iter()
            .flatten()
            .find_map(|scope| {
                self.generic_constraints
                    .get(&(scope.to_owned(), nominal.clone()))
            })
            .cloned()
            .unwrap_or(nominal);
        self.resolve_type_name(&constrained, callable_scope, false)
            .unwrap_or(constrained)
    }

    fn enclosing_declaration(&self, byte: usize) -> Option<usize> {
        self.declarations
            .iter()
            .enumerate()
            .filter(|(_, declaration)| declaration.start <= byte && byte < declaration.end)
            .min_by_key(|(_, declaration)| declaration.end.saturating_sub(declaration.start))
            .map(|(index, _)| index)
    }

    fn enclosing_callable(&self, byte: usize) -> Option<usize> {
        self.declarations
            .iter()
            .enumerate()
            .filter(|(_, declaration)| {
                is_callable_kind(&declaration.kind)
                    && declaration.start <= byte
                    && byte < declaration.end
            })
            .min_by_key(|(_, declaration)| declaration.end.saturating_sub(declaration.start))
            .map(|(index, _)| index)
    }

    fn enclosing_scope(&self, byte: usize) -> String {
        self.enclosing_declaration(byte)
            .map(|index| self.declarations[index].scope_id.clone())
            .unwrap_or_else(|| self.file.scope_id.clone())
    }

    fn capture_errors(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        if node.is_error() || node.is_missing() {
            self.builder.diagnose(
                "parser_error",
                None,
                recovery_diagnostic_range(self.source_file, self.source, node),
                "tree-sitter reported an error or missing C# syntax node",
            )?;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.capture_errors(child, depth + 1)?;
        }
        Ok(())
    }

    fn depth_diagnostic<T>(&mut self, node: Node<'_>) -> Result<T, EvidenceError> {
        self.builder.diagnose(
            "traversal_depth_limit",
            None,
            Some(self.evidence_range(node)),
            "C# syntax traversal exceeded the bounded depth limit",
        )?;
        Err(EvidenceError::new(
            super::validate::EvidenceErrorCode::ResourceLimit,
            "C# syntax traversal exceeded the bounded depth limit",
        ))
    }

    fn text(&self, node: Node<'_>) -> &'source str {
        node.utf8_text(self.source).unwrap_or_default()
    }

    fn evidence_range(&self, node: Node<'_>) -> crate::EvidenceRange {
        recovery_diagnostic_range(self.source_file, self.source, node)
            .unwrap_or_else(|| range_for_node(self.source_file, node))
    }
}

/// Tree-sitter represents an inserted recovery token as a zero-width missing
/// node. Evidence facts must remain non-empty, but a diagnostic can truthfully
/// point at the nearest bounded source byte instead of invalidating every
/// otherwise valid fact in the file. Empty files have no source byte to anchor
/// and therefore retain a range-less diagnostic.
fn recovery_diagnostic_range(
    source_file: &str,
    source: &[u8],
    node: Node<'_>,
) -> Option<crate::EvidenceRange> {
    if node.start_byte() < node.end_byte() {
        return Some(range_for_node(source_file, node));
    }
    if source.is_empty() {
        return None;
    }
    let start = node.start_byte().min(source.len());
    let (start, end) = if start < source.len() {
        (start, start.saturating_add(1))
    } else {
        (source.len().saturating_sub(1), source.len())
    };
    Some(range_for_byte_span(source_file, source, start, end))
}

fn csharp_type_kind(kind: &str) -> &'static str {
    match kind {
        "class_declaration" => "class",
        "delegate_declaration" => "delegate",
        "enum_declaration" => "enum",
        "interface_declaration" => "interface",
        "record_declaration" => "record",
        "struct_declaration" => "struct",
        _ => "type",
    }
}

fn callable_kind(kind: &str) -> &'static str {
    match kind {
        "constructor_declaration" => "constructor",
        "destructor_declaration" => "destructor",
        "local_function_statement" => "local_function",
        "operator_declaration" | "conversion_operator_declaration" => "operator",
        _ => "method",
    }
}

fn is_callable_kind(kind: &str) -> bool {
    matches!(
        kind,
        "constructor" | "destructor" | "local_function" | "method" | "operator"
    )
}

fn join_qualified(owner: &str, name: &str, separator: &str) -> String {
    if owner.is_empty() {
        name.to_owned()
    } else {
        format!("{owner}{separator}{name}")
    }
}

fn terminal(value: &str) -> &str {
    value
        .rsplit(['.', ':'])
        .find(|part| !part.is_empty())
        .unwrap_or(value)
}

fn canonical_type(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn nominal_type(value: &str) -> String {
    let canonical = canonical_type(value);
    let canonical = canonical.strip_prefix("global::").unwrap_or(&canonical);
    let mut nominal = String::with_capacity(canonical.len());
    let mut generic_depth = 0_u32;
    for character in canonical.chars() {
        match character {
            '<' => generic_depth = generic_depth.saturating_add(1),
            '>' => generic_depth = generic_depth.saturating_sub(1),
            _ if generic_depth == 0 => nominal.push(character),
            _ => {}
        }
    }
    while nominal.ends_with('?') || nominal.ends_with("[]") {
        if nominal.ends_with("[]") {
            nominal.truncate(nominal.len().saturating_sub(2));
        } else {
            nominal.pop();
        }
    }
    nominal
}

fn is_predefined(value: &str) -> bool {
    matches!(
        value.trim_end_matches('?'),
        "bool"
            | "byte"
            | "char"
            | "decimal"
            | "double"
            | "dynamic"
            | "float"
            | "int"
            | "long"
            | "nint"
            | "nuint"
            | "object"
            | "sbyte"
            | "short"
            | "string"
            | "uint"
            | "ulong"
            | "ushort"
            | "void"
    )
}

fn first_named_child<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| kinds.contains(&child.kind()))
}

fn first_descendant<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named())
        .find_map(|child| first_descendant(child, kind))
}

fn collect_descendants<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    if node.kind() == kind {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_descendants(child, kind, output);
    }
}

fn file_scoped_namespace(root: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = root.walk();
    root.children(&mut cursor)
        .find(|child| child.kind() == "file_scoped_namespace_declaration")
        .and_then(|declaration| declaration.child_by_field_name("name"))
        .and_then(|name| name.utf8_text(source).ok())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn collect_direct_type_children<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        if matches!(
            child.kind(),
            "identifier"
                | "generic_name"
                | "qualified_name"
                | "alias_qualified_name"
                | "nullable_type"
        ) {
            output.push(child);
        }
    }
}

fn parameter_types(node: Node<'_>, source: &[u8]) -> Vec<String> {
    let Some(parameters) = node
        .child_by_field_name("parameters")
        .or_else(|| first_named_child(node, &["parameter_list", "bracketed_parameter_list"]))
    else {
        return Vec::new();
    };
    let mut nodes = Vec::new();
    collect_descendants(parameters, "parameter", &mut nodes);
    nodes.sort_unstable_by_key(Node::start_byte);
    nodes
        .into_iter()
        .map(|parameter| {
            parameter.child_by_field_name("type").map_or_else(
                || "?".to_owned(),
                |value_type| canonical_type(value_type.utf8_text(source).unwrap_or_default()),
            )
        })
        .collect()
}

fn has_params_parameter(node: Node<'_>, source: &[u8]) -> bool {
    node.child_by_field_name("parameters")
        .or_else(|| first_named_child(node, &["parameter_list", "bracketed_parameter_list"]))
        .is_some_and(|parameters| {
            parameters
                .utf8_text(source)
                .unwrap_or_default()
                .contains("params ")
        })
}

fn callable_signature(node: Node<'_>, name: &str, parameters: &[String], source: &[u8]) -> String {
    let type_parameters = type_parameter_signature(node, source).unwrap_or_default();
    format!("{name}{type_parameters}({})", parameters.join(","))
}

fn type_parameter_signature(node: Node<'_>, source: &[u8]) -> Option<String> {
    first_named_child(node, &["type_parameter_list"])
        .and_then(|parameters| parameters.utf8_text(source).ok())
        .map(canonical_type)
        .filter(|value| !value.is_empty())
}

fn argument_count(node: Node<'_>) -> u32 {
    let arguments = node
        .child_by_field_name("arguments")
        .or_else(|| first_named_child(node, &["argument_list"]));
    let Some(arguments) = arguments else { return 0 };
    let mut cursor = arguments.walk();
    u32::try_from(
        arguments
            .children(&mut cursor)
            .filter(|child| child.is_named())
            .count(),
    )
    .unwrap_or(u32::MAX)
}
