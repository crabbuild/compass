//! Direct universal semantic evidence for PHP.
//!
//! PHP class, interface, trait, enum, function, and method lookup is
//! case-insensitive. The producer therefore retains source spelling in
//! declaration names while using case-folded qualified identities for those
//! symbol families. Properties, variables, and constants keep source case.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use tree_sitter::Node;

use super::build::{EvidenceBuilder, range_for_file, range_for_node};
use super::model::{
    BindingKind, CandidateRelation, HierarchyConstraint, ReceiverDispatchStrategy,
    ResolutionConstraint, SemanticEvidenceBatch, SemanticRole, SymbolNamespace,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits};
use crate::{UniversalEvidenceRegistry, make_id};

const PRODUCER: &str = "compass.languages.php.universal";
const MAX_TRAVERSAL_DEPTH: usize = 512;

const TYPE_KINDS: &[(&str, &str)] = &[
    ("class_declaration", "class"),
    ("enum_declaration", "enum"),
    ("interface_declaration", "interface"),
    ("trait_declaration", "trait"),
];

#[derive(Clone, Debug)]
struct Decl {
    id: String,
    qualified: String,
    kind: String,
    scope_id: String,
    start: usize,
    end: usize,
    ast_id: usize,
    owner: Option<usize>,
    namespace: String,
}

#[derive(Clone, Debug)]
struct NamespaceSpan {
    name: String,
    start: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PhpSymbolSpace {
    Type,
    Function,
    Constant,
}

#[derive(Clone, Debug)]
struct Import {
    namespace: String,
    spelling: String,
    target: String,
    space: PhpSymbolSpace,
    binding_id: String,
}

struct State<'source> {
    source: &'source [u8],
    source_file: &'source str,
    builder: EvidenceBuilder,
    file: Decl,
    namespace_spans: Vec<NamespaceSpan>,
    declarations: Vec<Decl>,
    declarations_by_ast: HashMap<usize, usize>,
    imports: Vec<Import>,
    direct_bases: BTreeMap<String, Vec<String>>,
    value_types: HashMap<(String, String), String>,
}

pub(super) fn emit_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    let pipeline = UniversalEvidenceRegistry::pipeline("php").ok_or_else(|| {
        EvidenceError::new(
            EvidenceErrorCode::InvalidPipeline,
            "PHP universal evidence pipeline is not registered",
        )
    })?;
    let mut builder =
        EvidenceBuilder::new(pipeline, PRODUCER, source_file, EvidenceLimits::default());
    let file_range = range_for_file(source_file, source);
    // Tree-sitter represents an empty source file with a zero-width root. The
    // universal inventory contract admits that exact range for file/module
    // inventory facts, while structural scopes remain non-empty.
    if source.is_empty() {
        let range = file_range;
        let file_id = builder.declare(
            "file",
            &make_id(&[source_file]),
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(source_file),
            source_file,
            Some("<global>"),
            None,
            range.clone(),
        )?;
        builder.open_scope("module", Some(&file_id), None, range)?;
        return builder.finish();
    }
    let file_graph_id = make_id(&[source_file]);
    let file_id = builder.declare(
        "file",
        &file_graph_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(source_file),
        source_file,
        Some("<global>"),
        None,
        file_range.clone(),
    )?;
    let file_scope = builder.open_scope("file", Some(&file_id), None, file_range)?;
    let file = Decl {
        id: file_id,
        qualified: source_file.to_owned(),
        kind: "file".to_owned(),
        scope_id: file_scope,
        start: root.start_byte(),
        end: source.len(),
        ast_id: root.id(),
        owner: None,
        namespace: String::new(),
    };
    let namespace_spans = namespace_spans(root, source);
    let mut state = State {
        source,
        source_file,
        builder,
        file,
        namespace_spans,
        declarations: Vec::new(),
        declarations_by_ast: HashMap::new(),
        imports: Vec::new(),
        direct_bases: BTreeMap::new(),
        value_types: HashMap::new(),
    };
    state.capture_errors(root, 0)?;
    state.collect_declarations(root, None, 0)?;
    state.collect_imports(root, 0)?;
    state.collect_semantics(root, 0)?;
    if root.has_error() {
        state.builder.diagnose(
            "partial_parser_recovery",
            None,
            Some(range_for_node(source_file, root)),
            "parser recovered from malformed PHP source; emitted evidence remains source-bounded",
        )?;
    }
    state.builder.finish()
}

impl<'source> State<'source> {
    fn collect_declarations(
        &mut self,
        node: Node<'_>,
        owner: Option<usize>,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        let namespace = self.namespace_at(node.start_byte());
        let owner = if owner.is_none() && node.kind() != "namespace_definition" {
            self.namespace_owner_at(&namespace, node.start_byte())
        } else {
            owner
        };
        let mut next_owner = owner;

        if node.kind() == "namespace_definition"
            && let Some(name_node) = node.child_by_field_name("name")
        {
            let display = self.text(name_node).trim_matches('\\').to_owned();
            if !display.is_empty() {
                let qualified = fold_php_name(&display);
                let graph = self.graph_id("namespace", &qualified, node);
                let id = self.builder.declare_with_namespace(
                    "namespace",
                    &graph,
                    &display,
                    &qualified,
                    Some(&qualified),
                    Some(&self.file.scope_id),
                    Some(SymbolNamespace::Namespace),
                    self.range(name_node),
                )?;
                let scope_id = self.builder.open_scope(
                    "namespace",
                    Some(&id),
                    Some(&self.file.scope_id),
                    self.range(node),
                )?;
                let index = self.push_decl(Decl {
                    id: id.clone(),
                    qualified,
                    kind: "namespace".to_owned(),
                    scope_id,
                    start: node.start_byte(),
                    end: node.end_byte(),
                    ast_id: node.id(),
                    owner: None,
                    namespace: display,
                });
                self.own(&self.file.id.clone(), &id)?;
                next_owner = Some(index);
            }
        } else if let Some((_, semantic_kind)) = TYPE_KINDS
            .iter()
            .find(|(syntax_kind, _)| *syntax_kind == node.kind())
        {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = self.text(name_node).to_owned();
                let qualified = canonical_qualified(&namespace, &name, PhpSymbolSpace::Type);
                let parent_scope = self.owner_scope(owner);
                let graph = self.graph_id(semantic_kind, &qualified, node);
                let id = self.builder.declare_type(
                    semantic_kind,
                    &graph,
                    &name,
                    &qualified,
                    Some(&module_name(&namespace)),
                    Some(&parent_scope),
                    Some(SymbolNamespace::Type),
                    None,
                    !node.has_error(),
                    self.range(name_node),
                )?;
                let scope_id = self.builder.open_scope(
                    semantic_kind,
                    Some(&id),
                    Some(&parent_scope),
                    self.range(node),
                )?;
                let index = self.push_decl(Decl {
                    id: id.clone(),
                    qualified,
                    kind: (*semantic_kind).to_owned(),
                    scope_id,
                    start: node.start_byte(),
                    end: node.end_byte(),
                    ast_id: node.id(),
                    owner,
                    namespace: namespace.clone(),
                });
                self.own(&self.owner_id(owner), &id)?;
                next_owner = Some(index);
            }
        } else if matches!(
            node.kind(),
            "function_definition" | "method_declaration" | "anonymous_function" | "arrow_function"
        ) {
            let is_closure = matches!(node.kind(), "anonymous_function" | "arrow_function");
            let name_node = node.child_by_field_name("name");
            let name = name_node
                .map(|name| self.text(name).to_owned())
                .unwrap_or_else(|| format!("closure@{}", node.start_byte()));
            let type_owner = owner.and_then(|index| self.enclosing_type_index(index));
            let callable_owner = owner.filter(|index| self.is_callable(*index));
            let qualified = if is_closure {
                let base = callable_owner
                    .map(|index| self.declarations[index].qualified.clone())
                    .or_else(|| type_owner.map(|index| self.declarations[index].qualified.clone()))
                    .unwrap_or_else(|| module_name(&namespace));
                format!("{base}::{name}")
            } else if let Some(type_owner) = type_owner {
                format!(
                    "{}::{}",
                    self.declarations[type_owner].qualified,
                    name.to_ascii_lowercase()
                )
            } else {
                canonical_qualified(&namespace, &name, PhpSymbolSpace::Function)
            };
            let parent_scope = self.owner_scope(owner);
            let parameters = node.child_by_field_name("parameters");
            let parameter_types = parameters
                .map(|parameters| self.parameter_types(parameters, &namespace))
                .unwrap_or_default();
            let signature = format!("({})", parameter_types.join(","));
            let kind = if is_closure {
                "closure"
            } else if type_owner.is_some() {
                "method"
            } else {
                "function"
            };
            let graph = self.graph_id(kind, &qualified, node);
            let id = self.builder.declare_callable(
                kind,
                &graph,
                &name,
                &qualified,
                Some(&module_name(&namespace)),
                Some(&parent_scope),
                Some(SymbolNamespace::Value),
                Some(&signature),
                parameter_types,
                has_descendant(node, "variadic_parameter"),
                name_node.map_or_else(|| self.range(node), |name| self.range(name)),
            )?;
            let scope_id =
                self.builder
                    .open_scope(kind, Some(&id), Some(&parent_scope), self.range(node))?;
            let index = self.push_decl(Decl {
                id: id.clone(),
                qualified,
                kind: kind.to_owned(),
                scope_id,
                start: node.start_byte(),
                end: node.end_byte(),
                ast_id: node.id(),
                owner,
                namespace: namespace.clone(),
            });
            self.own(&self.owner_id(owner), &id)?;
            next_owner = Some(index);
        } else if node.kind() == "property_element" {
            self.declare_property(node, owner, &namespace)?;
        } else if node.kind() == "const_element" {
            self.declare_constant(node, owner, &namespace)?;
        } else if node.kind() == "enum_case" {
            self.declare_enum_case(node, owner, &namespace)?;
        } else if matches!(
            node.kind(),
            "simple_parameter" | "property_promotion_parameter"
        ) {
            self.declare_parameter(node, owner, &namespace)?;
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_declarations(child, next_owner, depth + 1)?;
        }
        Ok(())
    }

    fn collect_imports(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        if node.kind() == "namespace_use_declaration" {
            let namespace = self.namespace_at(node.start_byte());
            let declaration_space = use_space(node, self.source);
            let prefix = direct_child(node, "namespace_name")
                .map(|prefix| self.text(prefix).trim_matches('\\').to_owned())
                .unwrap_or_default();
            let mut clauses = Vec::new();
            descendants_of_kind(node, "namespace_use_clause", &mut clauses, 0);
            for clause in clauses {
                let space = use_space(clause, self.source)
                    .unwrap_or(declaration_space.unwrap_or(PhpSymbolSpace::Type));
                let Some(target_node) = direct_named_child_any(clause, &["name", "qualified_name"])
                else {
                    continue;
                };
                let raw_target = self.text(target_node).trim_matches('\\');
                let joined = if prefix.is_empty() {
                    raw_target.to_owned()
                } else {
                    format!("{prefix}\\{raw_target}")
                };
                let target = canonical_symbol(&joined, space);
                let alias_node = clause.child_by_field_name("alias");
                let alias = alias_node
                    .map(|alias| self.text(alias))
                    .unwrap_or_else(|| raw_target.rsplit('\\').next().unwrap_or(raw_target));
                let spelling = canonical_symbol(alias, space);
                let scope_id = self.namespace_scope(&namespace);
                let binding_id = self.builder.bind_with_identity(
                    if alias_node.is_some() {
                        BindingKind::ImportAlias
                    } else {
                        BindingKind::Import
                    },
                    &spelling,
                    &target,
                    None,
                    Some(&scope_id),
                    Some(symbol_namespace(space)),
                    space == PhpSymbolSpace::Type,
                    self.range(clause),
                )?;
                let occurrence = self.builder.occur_with_context(
                    SemanticRole::Import,
                    &self.file.id,
                    &spelling,
                    Some(&target),
                    Some(&scope_id),
                    Some(import_context(space)),
                    self.range(target_node),
                )?;
                self.builder.relate(
                    CandidateRelation::Imports,
                    &self.file.id,
                    Some(&occurrence),
                    Some(&binding_id),
                    &target,
                    ResolutionConstraint {
                        exact_language: Some("php".to_owned()),
                        module_or_package: namespace_of(&target),
                        scope_id: Some(scope_id),
                        qualified_name: Some(target.clone()),
                        allowed_target_kinds: target_kinds(space),
                        allow_external: true,
                        ..ResolutionConstraint::default()
                    },
                )?;
                self.imports.push(Import {
                    namespace: fold_php_name(&namespace),
                    spelling,
                    target,
                    space,
                    binding_id,
                });
            }
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_imports(child, depth + 1)?;
        }
        Ok(())
    }

    fn collect_semantics(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        if let Some(index) = self.declarations_by_ast.get(&node.id()).copied() {
            self.declaration_semantics(index, node)?;
        }
        match node.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                self.base_semantics(node)?;
            }
            "use_declaration" => self.trait_semantics(node)?,
            "function_call_expression" => self.function_call(node)?,
            "scoped_call_expression" => self.static_call(node)?,
            "member_call_expression" | "nullsafe_member_call_expression" => {
                self.member_call(node)?;
            }
            "object_creation_expression" => self.construction(node)?,
            "assignment_expression" => self.assignment_type(node),
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.collect_semantics(child, depth + 1)?;
        }
        Ok(())
    }

    fn declaration_semantics(&mut self, index: usize, node: Node<'_>) -> Result<(), EvidenceError> {
        let declaration = self.declarations[index].clone();
        if let Some(attributes) = node.child_by_field_name("attributes") {
            let mut values = Vec::new();
            descendants_of_kind(attributes, "attribute", &mut values, 0);
            for attribute in values {
                let Some(name) = direct_named_child_any(attribute, &["name", "qualified_name"])
                else {
                    continue;
                };
                let qualified = self.resolve_name(
                    self.text(name),
                    &declaration.namespace,
                    PhpSymbolSpace::Type,
                );
                let occurrence = self.builder.occur_with_context(
                    SemanticRole::Decorator,
                    &declaration.id,
                    &canonical_symbol(self.text(name), PhpSymbolSpace::Type),
                    Some(&qualified),
                    Some(&declaration.scope_id),
                    Some("php-attribute"),
                    self.range(name),
                )?;
                self.builder.relate(
                    CandidateRelation::Decorates,
                    &declaration.id,
                    Some(&occurrence),
                    None,
                    &qualified,
                    ResolutionConstraint {
                        exact_language: Some("php".to_owned()),
                        qualified_name: Some(qualified.clone()),
                        allowed_target_kinds: vec!["class".to_owned()],
                        allow_external: true,
                        ..ResolutionConstraint::default()
                    },
                )?;
            }
        }
        if matches!(declaration.kind.as_str(), "function" | "method" | "closure") {
            if let Some(parameters) = node.child_by_field_name("parameters") {
                let mut parameter_nodes = Vec::new();
                descendants_any(
                    parameters,
                    &["simple_parameter", "property_promotion_parameter"],
                    &mut parameter_nodes,
                    0,
                );
                for parameter in parameter_nodes {
                    let Some(name_node) = parameter.child_by_field_name("name") else {
                        continue;
                    };
                    let name = variable_name(self.text(name_node));
                    if let Some(type_node) = parameter.child_by_field_name("type")
                        && let Some(raw_type) = first_type_name(type_node, self.source)
                    {
                        let qualified = self.resolve_name(
                            &raw_type,
                            &declaration.namespace,
                            PhpSymbolSpace::Type,
                        );
                        self.value_types.insert(
                            (declaration.scope_id.clone(), name.to_owned()),
                            qualified.clone(),
                        );
                        self.type_occurrence(
                            &declaration,
                            type_node,
                            &raw_type,
                            &qualified,
                            CandidateRelation::References,
                            "parameter-type",
                        )?;
                    }
                }
            }
            if let Some(return_type) = node.child_by_field_name("return_type") {
                for (raw_type, type_node) in type_names(return_type, self.source) {
                    let qualified =
                        self.resolve_name(&raw_type, &declaration.namespace, PhpSymbolSpace::Type);
                    self.type_occurrence(
                        &declaration,
                        type_node,
                        &raw_type,
                        &qualified,
                        CandidateRelation::Returns,
                        "return-type",
                    )?;
                }
            }
        }
        Ok(())
    }

    fn base_semantics(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(owner_index) = self.declarations_by_ast.get(&node.id()).copied() else {
            return Ok(());
        };
        let owner = self.declarations[owner_index].clone();
        let mut bases = Vec::new();
        let mut cursor = node.walk();
        for clause in node
            .children(&mut cursor)
            .filter(|child| matches!(child.kind(), "base_clause" | "class_interface_clause"))
        {
            let relation = if clause.kind() == "class_interface_clause" {
                CandidateRelation::Implements
            } else {
                CandidateRelation::Extends
            };
            let mut names = Vec::new();
            descendants_any(clause, &["name", "qualified_name"], &mut names, 0);
            for name in names {
                let raw = self.text(name);
                let qualified = self.resolve_name(raw, &owner.namespace, PhpSymbolSpace::Type);
                let occurrence = self.builder.occur_with_context(
                    SemanticRole::BaseType,
                    &owner.id,
                    &canonical_symbol(raw, PhpSymbolSpace::Type),
                    Some(&qualified),
                    Some(&owner.scope_id),
                    Some(if relation == CandidateRelation::Implements {
                        "interface"
                    } else {
                        "base"
                    }),
                    self.range(name),
                )?;
                self.builder.relate(
                    relation,
                    &owner.id,
                    Some(&occurrence),
                    None,
                    &qualified,
                    ResolutionConstraint {
                        exact_language: Some("php".to_owned()),
                        qualified_name: Some(qualified.clone()),
                        allowed_target_kinds: if relation == CandidateRelation::Implements {
                            vec!["interface".to_owned()]
                        } else {
                            vec!["class".to_owned(), "interface".to_owned()]
                        },
                        hierarchy: Some(HierarchyConstraint::DirectBase {
                            base_set_complete: !node.has_error(),
                        }),
                        allow_external: true,
                        ..ResolutionConstraint::default()
                    },
                )?;
                bases.push(qualified);
            }
        }
        self.direct_bases.insert(owner.qualified, bases);
        Ok(())
    }

    fn trait_semantics(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(owner_index) = self.enclosing_decl(node.start_byte(), &["class", "trait"]) else {
            return Ok(());
        };
        let owner = self.declarations[owner_index].clone();
        let mut names = Vec::new();
        let mut cursor = node.walk();
        for child in node
            .children(&mut cursor)
            .filter(|child| matches!(child.kind(), "name" | "qualified_name"))
        {
            names.push(child);
        }
        for name in names {
            let raw = self.text(name);
            let qualified = self.resolve_name(raw, &owner.namespace, PhpSymbolSpace::Type);
            let occurrence = self.builder.occur_with_context(
                SemanticRole::TraitBound,
                &owner.id,
                &canonical_symbol(raw, PhpSymbolSpace::Type),
                Some(&qualified),
                Some(&owner.scope_id),
                Some("trait-use"),
                self.range(name),
            )?;
            self.builder.relate(
                CandidateRelation::UsesTrait,
                &owner.id,
                Some(&occurrence),
                None,
                &qualified,
                ResolutionConstraint {
                    exact_language: Some("php".to_owned()),
                    qualified_name: Some(qualified.clone()),
                    allowed_target_kinds: vec!["trait".to_owned()],
                    hierarchy: Some(HierarchyConstraint::DirectBase {
                        base_set_complete: !node.has_error(),
                    }),
                    allow_external: true,
                    ..ResolutionConstraint::default()
                },
            )?;
            self.direct_bases
                .entry(owner.qualified.clone())
                .or_default()
                .push(qualified);
        }
        Ok(())
    }

    fn function_call(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(function) = node.child_by_field_name("function") else {
            return Ok(());
        };
        if !matches!(function.kind(), "name" | "qualified_name") {
            return Ok(());
        }
        let caller = self.calling_context(node.start_byte());
        let raw = self.text(function).to_owned();
        let (qualified, binding_id) =
            self.resolve_with_binding(&raw, &caller.namespace, PhpSymbolSpace::Function);
        self.call_candidate(
            &caller,
            node,
            function,
            &raw,
            &qualified,
            binding_id.as_deref(),
            None,
        )
    }

    fn static_call(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let (Some(scope), Some(name)) = (
            node.child_by_field_name("scope"),
            node.child_by_field_name("name"),
        ) else {
            return Ok(());
        };
        if name.kind() != "name" {
            return Ok(());
        }
        let caller = self.calling_context(node.start_byte());
        let receiver = self.static_receiver(self.text(scope), &caller);
        let Some(receiver) = receiver else {
            return Ok(());
        };
        let name_spelling = self.text(name).to_owned();
        self.call_candidate(
            &caller,
            node,
            name,
            &name_spelling,
            &format!("{receiver}::{}", name_spelling.to_ascii_lowercase()),
            None,
            Some(&receiver),
        )
    }

    fn member_call(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let (Some(object), Some(name)) = (
            node.child_by_field_name("object"),
            node.child_by_field_name("name"),
        ) else {
            return Ok(());
        };
        if name.kind() != "name" {
            return Ok(());
        }
        let caller = self.calling_context(node.start_byte());
        let receiver = if object.kind() == "variable_name" {
            let variable = variable_name(self.text(object));
            if variable == "this" {
                caller
                    .owner
                    .and_then(|owner| self.enclosing_type_index(owner))
                    .map(|owner| self.declarations[owner].qualified.clone())
            } else {
                self.value_types
                    .get(&(caller.scope_id.clone(), variable.to_owned()))
                    .cloned()
            }
        } else if object.kind() == "object_creation_expression" {
            object_type_name(object, self.source)
                .map(|raw| self.resolve_name(&raw, &caller.namespace, PhpSymbolSpace::Type))
        } else {
            None
        };
        let Some(receiver) = receiver else {
            return Ok(());
        };
        let name_spelling = self.text(name).to_owned();
        self.call_candidate(
            &caller,
            node,
            name,
            &name_spelling,
            &format!("{receiver}::{}", name_spelling.to_ascii_lowercase()),
            None,
            Some(&receiver),
        )
    }

    fn construction(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(raw) = object_type_name(node, self.source) else {
            return Ok(());
        };
        let caller = self.calling_context(node.start_byte());
        let (qualified, binding_id) =
            self.resolve_with_binding(&raw, &caller.namespace, PhpSymbolSpace::Type);
        let type_node = direct_named_child_any(node, &["name", "qualified_name"]).unwrap_or(node);
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Construction,
            &caller.id,
            &canonical_symbol(&raw, PhpSymbolSpace::Type),
            Some(&qualified),
            Some(&caller.scope_id),
            Some("constructor"),
            self.range(type_node),
        )?;
        self.builder.relate(
            CandidateRelation::Constructs,
            &caller.id,
            Some(&occurrence),
            binding_id.as_deref(),
            &qualified,
            ResolutionConstraint {
                exact_language: Some("php".to_owned()),
                qualified_name: Some(qualified.clone()),
                argument_count: argument_count(node),
                allowed_target_kinds: vec!["class".to_owned()],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn assignment_type(&mut self, node: Node<'_>) {
        let (Some(left), Some(right)) = (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "variable_name" {
            return;
        }
        let Some(construction) = first_descendant(right, "object_creation_expression") else {
            return;
        };
        let Some(raw) = object_type_name(construction, self.source) else {
            return;
        };
        let declaration = self.calling_context(node.start_byte());
        self.value_types.insert(
            (
                declaration.scope_id.clone(),
                variable_name(self.text(left)).to_owned(),
            ),
            self.resolve_name(&raw, &declaration.namespace, PhpSymbolSpace::Type),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn call_candidate(
        &mut self,
        caller: &Decl,
        call: Node<'_>,
        name_node: Node<'_>,
        spelling: &str,
        qualified: &str,
        binding_id: Option<&str>,
        receiver: Option<&str>,
    ) -> Result<(), EvidenceError> {
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Call,
            &caller.id,
            &spelling.to_ascii_lowercase(),
            receiver.or(Some(qualified)),
            Some(&caller.scope_id),
            Some(if receiver.is_some() {
                "member"
            } else {
                "function"
            }),
            self.range(name_node),
        )?;
        self.builder.relate(
            CandidateRelation::Calls,
            &caller.id,
            Some(&occurrence),
            binding_id,
            &spelling.to_ascii_lowercase(),
            ResolutionConstraint {
                exact_language: Some("php".to_owned()),
                qualified_name: receiver.is_none().then(|| qualified.to_owned()),
                argument_count: argument_count(call),
                allowed_target_kinds: vec![
                    "closure".to_owned(),
                    "function".to_owned(),
                    "method".to_owned(),
                ],
                hierarchy: receiver.map(|receiver| HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name: receiver.to_owned(),
                    strategy: ReceiverDispatchStrategy::C3FromReceiver,
                }),
                allow_external: receiver.is_none(),
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn type_occurrence(
        &mut self,
        owner: &Decl,
        node: Node<'_>,
        raw: &str,
        qualified: &str,
        relation: CandidateRelation,
        context: &str,
    ) -> Result<(), EvidenceError> {
        if is_builtin_type(raw) {
            return Ok(());
        }
        let occurrence = self.builder.occur_with_context(
            SemanticRole::TypeReference,
            &owner.id,
            &canonical_symbol(raw, PhpSymbolSpace::Type),
            Some(qualified),
            Some(&owner.scope_id),
            Some(context),
            self.range(node),
        )?;
        self.builder.relate(
            relation,
            &owner.id,
            Some(&occurrence),
            None,
            qualified,
            ResolutionConstraint {
                exact_language: Some("php".to_owned()),
                qualified_name: Some(qualified.to_owned()),
                allowed_target_kinds: vec![
                    "class".to_owned(),
                    "enum".to_owned(),
                    "interface".to_owned(),
                    "trait".to_owned(),
                ],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn declare_property(
        &mut self,
        node: Node<'_>,
        owner: Option<usize>,
        namespace: &str,
    ) -> Result<(), EvidenceError> {
        let Some(type_owner) = owner.and_then(|owner| self.enclosing_type_index(owner)) else {
            return Ok(());
        };
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let name = variable_name(self.text(name_node)).to_owned();
        let owner_decl = self.declarations[type_owner].clone();
        let qualified = format!("{}::${name}", owner_decl.qualified);
        let graph = self.graph_id("property", &qualified, node);
        let id = self.builder.declare_with_namespace(
            "property",
            &graph,
            &name,
            &qualified,
            Some(&module_name(namespace)),
            Some(&owner_decl.scope_id),
            Some(SymbolNamespace::Value),
            self.range(name_node),
        )?;
        self.own(&owner_decl.id, &id)?;
        Ok(())
    }

    fn declare_constant(
        &mut self,
        node: Node<'_>,
        owner: Option<usize>,
        namespace: &str,
    ) -> Result<(), EvidenceError> {
        let Some(name_node) = direct_child(node, "name") else {
            return Ok(());
        };
        let name = self.text(name_node).to_owned();
        let owner_id = self.owner_id(owner);
        let owner_scope = self.owner_scope(owner);
        let qualified = owner
            .and_then(|owner| self.enclosing_type_index(owner))
            .map_or_else(
                || canonical_qualified(namespace, &name, PhpSymbolSpace::Constant),
                |owner| format!("{}::{name}", self.declarations[owner].qualified),
            );
        let graph = self.graph_id("constant", &qualified, node);
        let id = self.builder.declare_with_namespace(
            "constant",
            &graph,
            &name,
            &qualified,
            Some(&module_name(namespace)),
            Some(&owner_scope),
            Some(SymbolNamespace::Value),
            self.range(name_node),
        )?;
        self.own(&owner_id, &id)?;
        Ok(())
    }

    fn declare_enum_case(
        &mut self,
        node: Node<'_>,
        owner: Option<usize>,
        namespace: &str,
    ) -> Result<(), EvidenceError> {
        let Some(enum_owner) = owner.and_then(|owner| self.enclosing_type_index(owner)) else {
            return Ok(());
        };
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let name = self.text(name_node).to_owned();
        let owner_decl = self.declarations[enum_owner].clone();
        let qualified = format!("{}::{name}", owner_decl.qualified);
        let graph = self.graph_id("enum_member", &qualified, node);
        let id = self.builder.declare_with_namespace(
            "enum_member",
            &graph,
            &name,
            &qualified,
            Some(&module_name(namespace)),
            Some(&owner_decl.scope_id),
            Some(SymbolNamespace::Value),
            self.range(name_node),
        )?;
        self.own(&owner_decl.id, &id)?;
        Ok(())
    }

    fn declare_parameter(
        &mut self,
        node: Node<'_>,
        owner: Option<usize>,
        namespace: &str,
    ) -> Result<(), EvidenceError> {
        let Some(callable) = owner.filter(|owner| self.is_callable(*owner)) else {
            return Ok(());
        };
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let name = variable_name(self.text(name_node)).to_owned();
        let callable_decl = self.declarations[callable].clone();
        let qualified = format!("{}::${name}@{}", callable_decl.qualified, node.start_byte());
        let graph = self.graph_id("parameter", &qualified, node);
        let id = self.builder.declare_with_namespace(
            "parameter",
            &graph,
            &name,
            &qualified,
            Some(&module_name(namespace)),
            Some(&callable_decl.scope_id),
            Some(SymbolNamespace::Value),
            self.range(name_node),
        )?;
        self.own(&callable_decl.id, &id)?;
        if node.kind() == "property_promotion_parameter"
            && let Some(type_owner) = callable_decl
                .owner
                .and_then(|owner| self.enclosing_type_index(owner))
        {
            let owner_decl = self.declarations[type_owner].clone();
            let property_qualified = format!("{}::${name}", owner_decl.qualified);
            let property_graph = self.graph_id("promoted_property", &property_qualified, node);
            let property_id = self.builder.declare_with_namespace(
                "property",
                &property_graph,
                &name,
                &property_qualified,
                Some(&module_name(namespace)),
                Some(&owner_decl.scope_id),
                Some(SymbolNamespace::Value),
                self.range(name_node),
            )?;
            self.own(&owner_decl.id, &property_id)?;
        }
        Ok(())
    }

    fn own(&mut self, owner_id: &str, target_id: &str) -> Result<(), EvidenceError> {
        self.builder.relate(
            CandidateRelation::Owns,
            owner_id,
            None,
            None,
            target_id,
            ResolutionConstraint {
                exact_target_declaration_id: Some(target_id.to_owned()),
                exact_language: Some("php".to_owned()),
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn resolve_name(&self, raw: &str, namespace: &str, space: PhpSymbolSpace) -> String {
        self.resolve_with_binding(raw, namespace, space).0
    }

    fn resolve_with_binding(
        &self,
        raw: &str,
        namespace: &str,
        space: PhpSymbolSpace,
    ) -> (String, Option<String>) {
        let trimmed = raw.trim();
        if trimmed.starts_with('\\') {
            return (canonical_symbol(trimmed, space), None);
        }
        let normalized_namespace = fold_php_name(namespace);
        let first = trimmed.split('\\').next().unwrap_or(trimmed);
        let folded_first = canonical_symbol(first, space);
        let matches = self
            .imports
            .iter()
            .filter(|import| {
                import.namespace == normalized_namespace
                    && import.space == space
                    && import.spelling == folded_first
            })
            .collect::<Vec<_>>();
        if let [import] = matches.as_slice() {
            let suffix = trimmed.strip_prefix(first).unwrap_or_default();
            let qualified = if suffix.is_empty() {
                import.target.clone()
            } else {
                format!(
                    "{}\\{}",
                    import.target,
                    canonical_symbol(suffix.trim_start_matches('\\'), space)
                )
            };
            return (qualified, Some(import.binding_id.clone()));
        }
        if trimmed
            .get(..10)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("namespace\\"))
        {
            return (canonical_qualified(namespace, &trimmed[10..], space), None);
        }
        (canonical_qualified(namespace, trimmed, space), None)
    }

    fn static_receiver(&self, raw: &str, caller: &Decl) -> Option<String> {
        match raw.to_ascii_lowercase().as_str() {
            "self" | "static" => caller
                .owner
                .and_then(|owner| self.enclosing_type_index(owner))
                .map(|owner| self.declarations[owner].qualified.clone()),
            "parent" => caller
                .owner
                .and_then(|owner| self.enclosing_type_index(owner))
                .and_then(|owner| {
                    let bases = self.direct_bases.get(&self.declarations[owner].qualified)?;
                    (bases.len() == 1).then(|| bases[0].clone())
                }),
            _ => Some(self.resolve_name(raw, &caller.namespace, PhpSymbolSpace::Type)),
        }
    }

    fn parameter_types(&self, parameters: Node<'_>, namespace: &str) -> Vec<String> {
        let mut nodes = Vec::new();
        descendants_any(
            parameters,
            &["simple_parameter", "property_promotion_parameter"],
            &mut nodes,
            0,
        );
        nodes
            .into_iter()
            .map(|parameter| {
                let Some(type_node) = parameter.child_by_field_name("type") else {
                    return "?".to_owned();
                };
                if let Some(name) = first_type_name(type_node, self.source) {
                    return self.resolve_name(&name, namespace, PhpSymbolSpace::Type);
                }
                let builtin = self
                    .text(type_node)
                    .chars()
                    .filter(|character| !character.is_ascii_whitespace())
                    .collect::<String>()
                    .to_ascii_lowercase();
                if builtin.is_empty() {
                    "?".to_owned()
                } else {
                    builtin
                }
            })
            .collect()
    }

    fn push_decl(&mut self, declaration: Decl) -> usize {
        let index = self.declarations.len();
        self.declarations_by_ast.insert(declaration.ast_id, index);
        self.declarations.push(declaration);
        index
    }

    fn graph_id(&self, kind: &str, qualified: &str, node: Node<'_>) -> String {
        make_id(&[
            "php",
            self.source_file,
            kind,
            qualified,
            &node.start_byte().to_string(),
        ])
    }

    fn owner_id(&self, owner: Option<usize>) -> String {
        owner
            .map(|owner| self.declarations[owner].id.clone())
            .unwrap_or_else(|| self.file.id.clone())
    }

    fn owner_scope(&self, owner: Option<usize>) -> String {
        owner
            .map(|owner| self.declarations[owner].scope_id.clone())
            .unwrap_or_else(|| self.file.scope_id.clone())
    }

    fn namespace_scope(&self, namespace: &str) -> String {
        let folded = fold_php_name(namespace);
        self.declarations
            .iter()
            .find(|declaration| declaration.kind == "namespace" && declaration.qualified == folded)
            .map(|declaration| declaration.scope_id.clone())
            .unwrap_or_else(|| self.file.scope_id.clone())
    }

    fn namespace_owner_at(&self, namespace: &str, offset: usize) -> Option<usize> {
        let folded = fold_php_name(namespace);
        if folded.is_empty() {
            return None;
        }
        self.declarations
            .iter()
            .enumerate()
            .filter(|(_, declaration)| {
                declaration.kind == "namespace"
                    && declaration.qualified == folded
                    && declaration.start <= offset
            })
            .max_by_key(|(_, declaration)| declaration.start)
            .map(|(index, _)| index)
    }

    fn namespace_at(&self, offset: usize) -> String {
        self.namespace_spans
            .iter()
            .filter(|span| span.start <= offset && offset < span.end)
            .min_by_key(|span| span.end.saturating_sub(span.start))
            .map(|span| span.name.clone())
            .unwrap_or_default()
    }

    fn enclosing_decl(&self, offset: usize, kinds: &[&str]) -> Option<usize> {
        self.declarations
            .iter()
            .enumerate()
            .filter(|(_, declaration)| {
                declaration.start <= offset
                    && offset < declaration.end
                    && kinds.contains(&declaration.kind.as_str())
            })
            .min_by_key(|(_, declaration)| declaration.end.saturating_sub(declaration.start))
            .map(|(index, _)| index)
    }

    fn enclosing_callable(&self, offset: usize) -> Option<usize> {
        self.enclosing_decl(offset, &["function", "method", "closure"])
    }

    fn calling_context(&self, offset: usize) -> Decl {
        if let Some(caller) = self.enclosing_callable(offset) {
            return self.declarations[caller].clone();
        }
        let mut file = self.file.clone();
        file.namespace = self.namespace_at(offset);
        file.scope_id = self.namespace_scope(&file.namespace);
        file
    }

    fn enclosing_type_index(&self, index: usize) -> Option<usize> {
        let mut cursor = Some(index);
        let mut visited = BTreeSet::new();
        while let Some(index) = cursor.filter(|index| visited.insert(*index)) {
            if matches!(
                self.declarations[index].kind.as_str(),
                "class" | "interface" | "trait" | "enum"
            ) {
                return Some(index);
            }
            cursor = self.declarations[index].owner;
        }
        None
    }

    fn is_callable(&self, index: usize) -> bool {
        matches!(
            self.declarations[index].kind.as_str(),
            "function" | "method" | "closure"
        )
    }

    fn text(&self, node: Node<'_>) -> &str {
        node.utf8_text(self.source).unwrap_or_default()
    }

    fn range(&self, node: Node<'_>) -> crate::EvidenceRange {
        range_for_node(self.source_file, node)
    }

    fn capture_errors(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        if node.is_error() || node.is_missing() {
            self.builder.diagnose(
                "parser_recovery_node",
                None,
                Some(self.range(node)),
                "PHP parser recovery node was retained as bounded partial evidence",
            )?;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(|child| child.is_named()) {
            self.capture_errors(child, depth + 1)?;
        }
        Ok(())
    }

    fn depth_diagnostic<T>(&mut self, node: Node<'_>) -> Result<T, EvidenceError> {
        self.builder.diagnose(
            "php_traversal_limit",
            None,
            Some(self.range(node)),
            "PHP AST traversal exceeded the bounded depth limit",
        )?;
        Err(EvidenceError::new(
            EvidenceErrorCode::ResourceLimit,
            "PHP AST traversal exceeded the bounded depth limit",
        ))
    }
}

fn namespace_spans(root: Node<'_>, source: &[u8]) -> Vec<NamespaceSpan> {
    let mut definitions = Vec::new();
    descendants_of_kind(root, "namespace_definition", &mut definitions, 0);
    definitions.sort_by_key(Node::start_byte);
    definitions
        .iter()
        .enumerate()
        .filter_map(|(index, definition)| {
            let name = definition
                .child_by_field_name("name")?
                .utf8_text(source)
                .ok()?
                .trim_matches('\\')
                .to_owned();
            let (start, end) = definition.child_by_field_name("body").map_or_else(
                || {
                    (
                        definition.end_byte(),
                        definitions
                            .get(index + 1)
                            .map_or(root.end_byte(), |next| next.start_byte()),
                    )
                },
                |body| (body.start_byte(), body.end_byte()),
            );
            Some(NamespaceSpan { name, start, end })
        })
        .collect()
}

fn canonical_qualified(namespace: &str, raw: &str, space: PhpSymbolSpace) -> String {
    let raw = raw.trim().trim_start_matches('\\');
    if namespace.is_empty() {
        canonical_symbol(raw, space)
    } else {
        canonical_symbol(&format!("{}\\{raw}", namespace.trim_matches('\\')), space)
    }
}

fn canonical_symbol(raw: &str, space: PhpSymbolSpace) -> String {
    let raw = raw.trim().trim_start_matches('\\');
    match space {
        PhpSymbolSpace::Type | PhpSymbolSpace::Function => fold_php_name(raw),
        PhpSymbolSpace::Constant => raw.to_owned(),
    }
}

fn fold_php_name(value: &str) -> String {
    value.trim().trim_matches('\\').to_ascii_lowercase()
}

fn module_name(namespace: &str) -> String {
    let namespace = fold_php_name(namespace);
    if namespace.is_empty() {
        "<global>".to_owned()
    } else {
        namespace
    }
}

fn namespace_of(qualified: &str) -> Option<String> {
    qualified
        .rsplit_once('\\')
        .map(|(namespace, _)| namespace.to_owned())
}

fn symbol_namespace(space: PhpSymbolSpace) -> SymbolNamespace {
    match space {
        PhpSymbolSpace::Type => SymbolNamespace::Type,
        PhpSymbolSpace::Function | PhpSymbolSpace::Constant => SymbolNamespace::Value,
    }
}

fn import_context(space: PhpSymbolSpace) -> &'static str {
    match space {
        PhpSymbolSpace::Type => "type-import",
        PhpSymbolSpace::Function => "function-import",
        PhpSymbolSpace::Constant => "constant-import",
    }
}

fn target_kinds(space: PhpSymbolSpace) -> Vec<String> {
    match space {
        PhpSymbolSpace::Type => vec![
            "class".to_owned(),
            "enum".to_owned(),
            "interface".to_owned(),
            "trait".to_owned(),
        ],
        PhpSymbolSpace::Function => vec!["function".to_owned()],
        PhpSymbolSpace::Constant => vec!["constant".to_owned()],
    }
}

fn use_space(node: Node<'_>, source: &[u8]) -> Option<PhpSymbolSpace> {
    let type_node = node.child_by_field_name("type")?;
    match type_node
        .utf8_text(source)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "function" => Some(PhpSymbolSpace::Function),
        "const" => Some(PhpSymbolSpace::Constant),
        _ => None,
    }
}

fn type_names<'tree>(node: Node<'tree>, source: &[u8]) -> Vec<(String, Node<'tree>)> {
    let mut names = Vec::new();
    collect_type_names(node, source, &mut names, 0);
    names
}

fn collect_type_names<'tree>(
    node: Node<'tree>,
    source: &[u8],
    output: &mut Vec<(String, Node<'tree>)>,
    depth: usize,
) {
    if depth > MAX_TRAVERSAL_DEPTH {
        return;
    }
    if matches!(node.kind(), "name" | "qualified_name") {
        let name = node.utf8_text(source).unwrap_or_default();
        if !name.is_empty() && !is_builtin_type(name) {
            output.push((name.to_owned(), node));
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        collect_type_names(child, source, output, depth + 1);
    }
}

fn first_type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    type_names(node, source)
        .into_iter()
        .next()
        .map(|(name, _)| name)
}

fn is_builtin_type(value: &str) -> bool {
    matches!(
        value
            .trim()
            .trim_start_matches('\\')
            .to_ascii_lowercase()
            .as_str(),
        "array"
            | "bool"
            | "callable"
            | "false"
            | "float"
            | "int"
            | "iterable"
            | "mixed"
            | "never"
            | "null"
            | "object"
            | "resource"
            | "self"
            | "static"
            | "string"
            | "true"
            | "void"
    )
}

fn object_type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    direct_named_child_any(node, &["name", "qualified_name"])
        .and_then(|name| name.utf8_text(source).ok())
        .map(str::to_owned)
}

fn argument_count(node: Node<'_>) -> Option<u32> {
    let arguments = node
        .child_by_field_name("arguments")
        .or_else(|| direct_child(node, "arguments"))?;
    let mut cursor = arguments.walk();
    u32::try_from(
        arguments
            .children(&mut cursor)
            .filter(|child| child.kind() == "argument")
            .count(),
    )
    .ok()
}

fn variable_name(value: &str) -> &str {
    value.trim().trim_start_matches('$')
}

fn direct_child<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn direct_named_child_any<'tree>(node: Node<'tree>, kinds: &[&str]) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.is_named() && kinds.contains(&child.kind()))
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

fn has_descendant(node: Node<'_>, kind: &str) -> bool {
    first_descendant(node, kind).is_some()
}

fn descendants_of_kind<'tree>(
    node: Node<'tree>,
    kind: &str,
    output: &mut Vec<Node<'tree>>,
    depth: usize,
) {
    descendants_any(node, &[kind], output, depth);
}

fn descendants_any<'tree>(
    node: Node<'tree>,
    kinds: &[&str],
    output: &mut Vec<Node<'tree>>,
    depth: usize,
) {
    if depth > MAX_TRAVERSAL_DEPTH {
        return;
    }
    if kinds.contains(&node.kind()) {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(|child| child.is_named()) {
        descendants_any(child, kinds, output, depth + 1);
    }
}
