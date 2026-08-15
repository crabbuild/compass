//! Direct universal evidence for Kotlin source.
//!
//! Kotlin syntax is intentionally resolved only inside the Kotlin language
//! partition. Java/Kotlin interoperability requires an exact compiler or SCIP
//! endpoint and is never inferred from a JVM-family terminal name.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use tree_sitter::Node;

use super::build::{EvidenceBuilder, range_for_file, range_for_node};
use super::model::{
    BindingKind, CandidateRelation, HierarchyConstraint, ReceiverDispatchStrategy,
    ResolutionConstraint, SemanticEvidenceBatch, SemanticRole, SymbolNamespace,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits};
use crate::{AdapterRegistry, file_stem, make_id};

const PRODUCER: &str = "compass.languages.kotlin.universal";
const MAX_TRAVERSAL_DEPTH: usize = 128;
const MAX_SCOPE_DEPTH: usize = 64;

#[derive(Clone, Debug)]
struct Decl {
    id: String,
    qualified: String,
    kind: String,
    scope_id: String,
    enclosing_type: Option<String>,
}

#[derive(Clone, Debug)]
struct Import {
    spelling: String,
    target: String,
    binding_id: String,
}

struct State<'source> {
    source: &'source [u8],
    source_file: &'source str,
    package: String,
    builder: EvidenceBuilder,
    file: Decl,
    declarations: Vec<Decl>,
    by_node: HashMap<usize, usize>,
    by_terminal: BTreeMap<String, Vec<usize>>,
    imports: Vec<Import>,
    value_types: HashMap<(String, String), String>,
    scope_parents: HashMap<String, String>,
    parser_errors: Vec<(usize, usize)>,
}

pub(super) fn extract_candidate_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    let profile = AdapterRegistry::universal_profile("kotlin").ok_or_else(|| {
        EvidenceError::new(
            EvidenceErrorCode::InvalidAdapter,
            "Kotlin universal adapter is not registered",
        )
    })?;
    let mut builder =
        EvidenceBuilder::new(profile, PRODUCER, source_file, EvidenceLimits::default());
    let file_range = range_for_file(source_file, source);
    let file_graph_id = make_id(&[source_file]);
    let file_id = builder.declare(
        "file",
        &file_graph_id,
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(source_file),
        source_file,
        Some(&file_stem(Path::new(source_file))),
        None,
        file_range.clone(),
    )?;
    let file_scope = builder.open_scope("module", Some(&file_id), None, file_range)?;
    if root.end_byte() == root.start_byte() {
        return builder.finish();
    }
    let file = Decl {
        id: file_id,
        qualified: source_file.to_owned(),
        kind: "file".to_owned(),
        scope_id: file_scope,
        enclosing_type: None,
    };
    let package = package_name(root, source).unwrap_or_else(|| "<default>".to_owned());
    let mut state = State {
        source,
        source_file,
        package,
        builder,
        file,
        declarations: Vec::new(),
        by_node: HashMap::new(),
        by_terminal: BTreeMap::new(),
        imports: Vec::new(),
        value_types: HashMap::new(),
        scope_parents: HashMap::new(),
        parser_errors: Vec::new(),
    };
    state.capture_parser_errors(root, 0)?;
    let package_owner = state.add_package(root)?;
    state.collect_imports(root, &package_owner, 0)?;
    state.collect_declarations(root, Some(&package_owner), None, 0)?;
    state.collect_value_types(root, Some(&package_owner), 0)?;
    state.collect_semantics(root, Some(&package_owner), None, 0)?;
    if root.has_error() {
        state.builder.diagnose(
            "partial_parser_recovery",
            None,
            Some(range_for_node(source_file, root)),
            "parser recovered from malformed Kotlin source; emitted evidence remains source-bounded",
        )?;
    }
    state.builder.finish()
}

impl<'source> State<'source> {
    fn add_package(&mut self, root: Node<'_>) -> Result<Decl, EvidenceError> {
        let package_node = direct_named_child(root, "package_header");
        let range = package_node
            .and_then(|node| direct_named_child(node, "identifier"))
            .map_or_else(
                || range_for_node(self.source_file, root),
                |node| range_for_node(self.source_file, node),
            );
        let name = self.package.clone();
        let graph_id = make_id(&["kotlin", "package", &name]);
        let id = self.builder.declare_with_namespace(
            "package",
            &graph_id,
            &name,
            &name,
            Some(&name),
            Some(&self.file.scope_id),
            Some(SymbolNamespace::Namespace),
            range,
        )?;
        let scope_id = self.builder.open_scope(
            "package",
            Some(&id),
            Some(&self.file.scope_id),
            range_for_node(self.source_file, root),
        )?;
        self.scope_parents
            .insert(scope_id.clone(), self.file.scope_id.clone());
        self.own(&self.file.id.clone(), &id)?;
        Ok(Decl {
            id,
            qualified: name,
            kind: "package".to_owned(),
            scope_id,
            enclosing_type: None,
        })
    }

    fn collect_imports(
        &mut self,
        node: Node<'_>,
        owner: &Decl,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        if node.kind() == "import_header" {
            if !self.trusted(node) {
                return Ok(());
            }
            let Some(target_node) = direct_named_child(node, "identifier") else {
                return Ok(());
            };
            let target = self.text(target_node).trim().to_owned();
            if target.is_empty() || target.ends_with(".*") {
                return Ok(());
            }
            let alias_node =
                direct_named_child(node, "import_alias").and_then(|alias| first_identifier(alias));
            let spelling = alias_node
                .map(|alias| self.text(alias).trim().to_owned())
                .unwrap_or_else(|| terminal(&target).to_owned());
            if spelling.is_empty() {
                return Ok(());
            }
            let binding_id = self.builder.bind_with_identity(
                if alias_node.is_some() {
                    BindingKind::ImportAlias
                } else {
                    BindingKind::Import
                },
                &spelling,
                &target,
                None,
                Some(&owner.scope_id),
                Some(SymbolNamespace::ValueAndType),
                false,
                alias_node.map_or_else(
                    || range_for_node(self.source_file, target_node),
                    |alias| range_for_node(self.source_file, alias),
                ),
            )?;
            let occurrence_id = self.builder.occur(
                SemanticRole::Import,
                &owner.id,
                &spelling,
                qualified_parent(&target),
                Some(&owner.scope_id),
                range_for_node(self.source_file, target_node),
            )?;
            self.builder.relate(
                CandidateRelation::Imports,
                &owner.id,
                Some(&occurrence_id),
                Some(&binding_id),
                &spelling,
                ResolutionConstraint {
                    exact_target_declaration_id: None,
                    exact_language: Some("kotlin".to_owned()),
                    module_or_package: qualified_parent(&target).map(str::to_owned),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: Some(target.clone()),
                    argument_count: None,
                    argument_types: Vec::new(),
                    allowed_target_kinds: kotlin_import_target_kinds(),
                    hierarchy: None,
                    allow_external: true,
                },
            )?;
            self.imports.push(Import {
                spelling,
                target,
                binding_id,
            });
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            self.collect_imports(child, owner, depth + 1)?;
        }
        Ok(())
    }

    fn collect_declarations(
        &mut self,
        node: Node<'_>,
        owner: Option<&Decl>,
        enclosing_type: Option<&str>,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        if matches!(node.kind(), "import_header" | "package_header") {
            return Ok(());
        }
        if is_type_node(node.kind()) {
            if !self.trusted(node) {
                return Ok(());
            }
            let kind = kotlin_type_kind(node, self.source);
            let name_node = direct_named_child(node, "type_identifier");
            let name = name_node
                .map(|name| self.text(name).trim().to_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Companion".to_owned());
            let qualified = enclosing_type.map_or_else(
                || join_qualified(&self.package, &name, "."),
                |parent| join_qualified(parent, &name, "."),
            );
            let signature = type_parameter_signature(node, self.source);
            let graph_id = make_id(&["kotlin", kind, &qualified]);
            let parent_scope = owner.map_or(&self.file.scope_id, |owner| &owner.scope_id);
            let id = self.builder.declare_type(
                kind,
                &graph_id,
                &name,
                &qualified,
                Some(&self.package),
                Some(parent_scope),
                Some(SymbolNamespace::ValueAndType),
                signature.as_deref(),
                direct_bases_complete(node),
                name_node.map_or_else(
                    || range_for_node(self.source_file, node),
                    |name| range_for_node(self.source_file, name),
                ),
            )?;
            let scope_id = self.builder.open_scope(
                kind,
                Some(&id),
                Some(parent_scope),
                range_for_node(self.source_file, node),
            )?;
            self.scope_parents
                .insert(scope_id.clone(), parent_scope.clone());
            let decl = Decl {
                id: id.clone(),
                qualified: qualified.clone(),
                kind: kind.to_owned(),
                scope_id,
                enclosing_type: Some(qualified.clone()),
            };
            let index = self.declarations.len();
            self.declarations.push(decl.clone());
            self.by_node.insert(node.id(), index);
            self.by_terminal.entry(name).or_default().push(index);
            let owner_id = owner.map_or_else(|| self.file.id.clone(), |owner| owner.id.clone());
            self.own(&owner_id, &id)?;
            self.add_primary_constructor(node, &decl)?;
            self.add_promoted_properties(node, &decl)?;
            let mut cursor = node.walk();
            for child in node.children(&mut cursor).filter(Node::is_named) {
                if child.kind() != "primary_constructor" {
                    self.collect_declarations(child, Some(&decl), Some(&qualified), depth + 1)?;
                }
            }
            return Ok(());
        }
        if node.kind() == "function_declaration" {
            self.add_function(node, owner, enclosing_type)?;
            return Ok(());
        }
        if node.kind() == "secondary_constructor" {
            self.add_secondary_constructor(node, owner)?;
            return Ok(());
        }
        if node.kind() == "property_declaration" {
            self.add_property(node, owner, enclosing_type)?;
            return Ok(());
        }
        if node.kind() == "type_alias" {
            self.add_type_alias(node, owner)?;
            return Ok(());
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            self.collect_declarations(child, owner, enclosing_type, depth + 1)?;
        }
        Ok(())
    }

    fn add_primary_constructor(
        &mut self,
        node: Node<'_>,
        owner: &Decl,
    ) -> Result<(), EvidenceError> {
        let Some(constructor) = direct_named_child(node, "primary_constructor") else {
            return Ok(());
        };
        let parameters = kotlin_parameters(constructor, self.source);
        let signature = kotlin_callable_signature("<init>", None, &parameters);
        let graph_id = make_id(&["kotlin", "constructor", &owner.qualified, &signature]);
        let id = self.builder.declare_callable(
            "constructor",
            &graph_id,
            "<init>",
            &format!("{}::<init>", owner.qualified),
            Some(&self.package),
            Some(&owner.scope_id),
            Some(SymbolNamespace::Value),
            Some(&signature),
            parameters
                .iter()
                .map(|parameter| parameter.kind.clone())
                .collect(),
            parameters.iter().any(|parameter| parameter.variadic),
            range_for_node(self.source_file, constructor),
        )?;
        self.own(&owner.id, &id)
    }

    fn add_promoted_properties(
        &mut self,
        node: Node<'_>,
        owner: &Decl,
    ) -> Result<(), EvidenceError> {
        let Some(constructor) = direct_named_child(node, "primary_constructor") else {
            return Ok(());
        };
        let mut cursor = constructor.walk();
        for parameter in constructor
            .children(&mut cursor)
            .filter(|child| child.kind() == "class_parameter")
        {
            if direct_named_child(parameter, "binding_pattern_kind").is_none() {
                continue;
            }
            let Some(name_node) = direct_named_child(parameter, "simple_identifier") else {
                continue;
            };
            let name = self.text(name_node).trim().to_owned();
            if name.is_empty() {
                continue;
            }
            let qualified = format!("{}::{name}", owner.qualified);
            let graph_id = make_id(&["kotlin", "property", &qualified]);
            let type_text =
                parameter_type_node(parameter).map(|kind| self.text(kind).trim().to_owned());
            let id = self.builder.declare_with_signature(
                "property",
                &graph_id,
                &name,
                &qualified,
                Some(&self.package),
                Some(&owner.scope_id),
                Some(SymbolNamespace::Value),
                type_text.as_deref(),
                range_for_node(self.source_file, name_node),
            )?;
            self.own(&owner.id, &id)?;
        }
        Ok(())
    }

    fn add_function(
        &mut self,
        node: Node<'_>,
        owner: Option<&Decl>,
        enclosing_type: Option<&str>,
    ) -> Result<(), EvidenceError> {
        if !self.trusted(node) {
            return Ok(());
        }
        let Some(name_node) = function_name_node(node) else {
            return Ok(());
        };
        let name = self.text(name_node).trim().to_owned();
        if name.is_empty() {
            return Ok(());
        }
        let receiver = node
            .child_by_field_name("receiver")
            .or_else(|| direct_named_child(node, "receiver_type"))
            .map(|receiver| normalize_type(self.text(receiver)));
        let parameters = kotlin_parameters(node, self.source);
        let signature = kotlin_callable_signature(&name, receiver.as_deref(), &parameters);
        let qualified = enclosing_type.map_or_else(
            || format!("{}::{name}", self.package),
            |owner| format!("{owner}::{name}"),
        );
        let graph_id = make_id(&["kotlin", "function", &qualified, &signature]);
        let parent_scope = owner.map_or(&self.file.scope_id, |owner| &owner.scope_id);
        let id = self.builder.declare_callable(
            if enclosing_type.is_some() {
                "method"
            } else {
                "function"
            },
            &graph_id,
            &name,
            &qualified,
            Some(&self.package),
            Some(parent_scope),
            Some(SymbolNamespace::Value),
            Some(&signature),
            parameters
                .iter()
                .map(|parameter| parameter.kind.clone())
                .collect(),
            parameters.iter().any(|parameter| parameter.variadic),
            range_for_node(self.source_file, name_node),
        )?;
        let scope_id = self.builder.open_scope(
            "function",
            Some(&id),
            Some(parent_scope),
            range_for_node(self.source_file, node),
        )?;
        self.scope_parents
            .insert(scope_id.clone(), parent_scope.clone());
        let decl = Decl {
            id: id.clone(),
            qualified,
            kind: if enclosing_type.is_some() {
                "method".to_owned()
            } else {
                "function".to_owned()
            },
            scope_id,
            enclosing_type: enclosing_type.map(str::to_owned),
        };
        let index = self.declarations.len();
        self.declarations.push(decl.clone());
        self.by_node.insert(node.id(), index);
        self.by_terminal.entry(name).or_default().push(index);
        let owner_id = owner.map_or_else(|| self.file.id.clone(), |owner| owner.id.clone());
        self.own(&owner_id, &id)?;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            if child.kind() == "function_body" {
                self.collect_declarations(child, Some(&decl), enclosing_type, 1)?;
            }
        }
        Ok(())
    }

    fn add_secondary_constructor(
        &mut self,
        node: Node<'_>,
        owner: Option<&Decl>,
    ) -> Result<(), EvidenceError> {
        let Some(owner) = owner.filter(|owner| owner.enclosing_type.is_some()) else {
            return Ok(());
        };
        let parameters = kotlin_parameters(node, self.source);
        let signature = kotlin_callable_signature("<init>", None, &parameters);
        let qualified = format!("{}::<init>", owner.qualified);
        let graph_id = make_id(&["kotlin", "constructor", &qualified, &signature]);
        let id = self.builder.declare_callable(
            "constructor",
            &graph_id,
            "<init>",
            &qualified,
            Some(&self.package),
            Some(&owner.scope_id),
            Some(SymbolNamespace::Value),
            Some(&signature),
            parameters
                .iter()
                .map(|parameter| parameter.kind.clone())
                .collect(),
            parameters.iter().any(|parameter| parameter.variadic),
            range_for_node(self.source_file, node),
        )?;
        let scope_id = self.builder.open_scope(
            "constructor",
            Some(&id),
            Some(&owner.scope_id),
            range_for_node(self.source_file, node),
        )?;
        self.scope_parents
            .insert(scope_id.clone(), owner.scope_id.clone());
        let decl = Decl {
            id: id.clone(),
            qualified,
            kind: "constructor".to_owned(),
            scope_id,
            enclosing_type: owner.enclosing_type.clone(),
        };
        let index = self.declarations.len();
        self.declarations.push(decl.clone());
        self.by_node.insert(node.id(), index);
        self.own(&owner.id, &id)?;
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            self.collect_declarations(child, Some(&decl), owner.enclosing_type.as_deref(), 1)?;
        }
        Ok(())
    }

    fn add_property(
        &mut self,
        node: Node<'_>,
        owner: Option<&Decl>,
        enclosing_type: Option<&str>,
    ) -> Result<(), EvidenceError> {
        let Some(variable) = direct_named_child(node, "variable_declaration") else {
            return Ok(());
        };
        let Some(name_node) = direct_named_child(variable, "simple_identifier") else {
            return Ok(());
        };
        let name = self.text(name_node).trim().to_owned();
        if name.is_empty() {
            return Ok(());
        }
        let qualified = enclosing_type.map_or_else(
            || format!("{}::{name}", self.package),
            |owner| format!("{owner}::{name}"),
        );
        let kind = if modifier_contains(node, self.source, "const") {
            "constant"
        } else {
            "property"
        };
        let graph_id = make_id(&["kotlin", kind, &qualified]);
        let parent_scope = owner.map_or(&self.file.scope_id, |owner| &owner.scope_id);
        let type_text = parameter_type_node(variable).map(|kind| self.text(kind).trim().to_owned());
        let id = self.builder.declare_with_signature(
            kind,
            &graph_id,
            &name,
            &qualified,
            Some(&self.package),
            Some(parent_scope),
            Some(SymbolNamespace::Value),
            type_text.as_deref(),
            range_for_node(self.source_file, name_node),
        )?;
        let decl = Decl {
            id: id.clone(),
            qualified,
            kind: kind.to_owned(),
            scope_id: parent_scope.clone(),
            enclosing_type: enclosing_type.map(str::to_owned),
        };
        let index = self.declarations.len();
        self.declarations.push(decl);
        self.by_node.insert(node.id(), index);
        self.by_terminal.entry(name).or_default().push(index);
        let owner_id = owner.map_or_else(|| self.file.id.clone(), |owner| owner.id.clone());
        self.own(&owner_id, &id)
    }

    fn add_type_alias(
        &mut self,
        node: Node<'_>,
        owner: Option<&Decl>,
    ) -> Result<(), EvidenceError> {
        let Some(name_node) = direct_named_child(node, "type_identifier") else {
            return Ok(());
        };
        let name = self.text(name_node).trim().to_owned();
        if name.is_empty() {
            return Ok(());
        }
        let qualified = join_qualified(&self.package, &name, ".");
        let target = u32::try_from(node.named_child_count().saturating_sub(1))
            .ok()
            .and_then(|index| node.named_child(index))
            .map(|target| normalize_type(self.text(target)))
            .unwrap_or_default();
        let graph_id = make_id(&["kotlin", "type_alias", &qualified]);
        let parent_scope = owner.map_or(&self.file.scope_id, |owner| &owner.scope_id);
        let id = self.builder.declare_with_signature(
            "type_alias",
            &graph_id,
            &name,
            &qualified,
            Some(&self.package),
            Some(parent_scope),
            Some(SymbolNamespace::Type),
            (!target.is_empty()).then_some(target.as_str()),
            range_for_node(self.source_file, name_node),
        )?;
        self.by_terminal
            .entry(name.clone())
            .or_default()
            .push(self.declarations.len());
        self.declarations.push(Decl {
            id: id.clone(),
            qualified,
            kind: "type_alias".to_owned(),
            scope_id: parent_scope.clone(),
            enclosing_type: None,
        });
        self.by_node.insert(node.id(), self.declarations.len() - 1);
        let owner_id = owner.map_or_else(|| self.file.id.clone(), |owner| owner.id.clone());
        self.own(&owner_id, &id)
    }

    fn collect_value_types(
        &mut self,
        node: Node<'_>,
        active: Option<&Decl>,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        let owned = self
            .by_node
            .get(&node.id())
            .map(|index| self.declarations[*index].clone());
        let current = owned.as_ref().or(active);
        if matches!(node.kind(), "parameter" | "class_parameter")
            && let (Some(owner), Some(name), Some(kind)) = (
                current,
                direct_named_child(node, "simple_identifier"),
                parameter_type_node(node),
            )
        {
            let name = self.text(name).trim().to_owned();
            let kind = self.resolve_type(&normalize_type(self.text(kind)));
            if !name.is_empty()
                && let Some(kind) = kind
            {
                self.value_types
                    .insert((owner.scope_id.clone(), name), kind);
            }
        }
        if node.kind() == "property_declaration"
            && let (Some(owner), Some(variable)) =
                (current, direct_named_child(node, "variable_declaration"))
            && let Some(name_node) = direct_named_child(variable, "simple_identifier")
        {
            let name = self.text(name_node).trim().to_owned();
            let explicit = parameter_type_node(variable)
                .and_then(|kind| self.resolve_type(&normalize_type(self.text(kind))));
            let inferred = explicit.or_else(|| {
                inferred_initializer_type(node, self.source)
                    .and_then(|kind| self.resolve_type(&kind))
            });
            if !name.is_empty()
                && let Some(kind) = inferred
            {
                self.value_types
                    .insert((owner.scope_id.clone(), name), kind);
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            self.collect_value_types(child, current, depth + 1)?;
        }
        Ok(())
    }

    fn collect_semantics(
        &mut self,
        node: Node<'_>,
        active: Option<&Decl>,
        behavioral: Option<&Decl>,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        let owned = self
            .by_node
            .get(&node.id())
            .map(|index| self.declarations[*index].clone());
        let current = owned.as_ref().or(active);
        let current_behavioral = owned
            .as_ref()
            .filter(|owner| is_behavioral_owner(owner))
            .or(behavioral);
        if let Some(owner) = current {
            if owned.is_some() {
                self.add_annotations(node, owner)?;
                self.add_declaration_type_references(node, owner)?;
                if is_type_node(node.kind()) {
                    self.add_base_types(node, owner)?;
                }
            }
            match (node.kind(), current_behavioral) {
                ("call_expression", Some(callable)) => self.add_call(node, callable)?,
                ("navigation_expression", Some(callable))
                    if node
                        .parent()
                        .is_none_or(|parent| parent.kind() != "call_expression") =>
                {
                    self.add_member_access(node, callable)?;
                }
                _ => {}
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            if child.kind() != "import_header" {
                self.collect_semantics(child, current, current_behavioral, depth + 1)?;
            }
        }
        Ok(())
    }

    fn add_annotations(&mut self, node: Node<'_>, owner: &Decl) -> Result<(), EvidenceError> {
        let Some(modifiers) = direct_named_child(node, "modifiers") else {
            return Ok(());
        };
        let mut annotations = Vec::new();
        collect_nodes(modifiers, "annotation", &mut annotations, 0);
        for annotation in annotations {
            let Some(type_node) =
                first_descendant(annotation, &["user_type", "type_identifier"], 0)
            else {
                continue;
            };
            let raw = normalize_type(self.text(type_node));
            let spelling = terminal(&raw);
            if spelling.is_empty() {
                continue;
            }
            let binding = self.import_for(spelling).cloned();
            let qualified = self.resolve_type(&raw);
            let occurrence_id = self.builder.occur(
                SemanticRole::Annotation,
                &owner.id,
                spelling,
                qualified_parent(&raw),
                Some(&owner.scope_id),
                range_for_node(self.source_file, type_node),
            )?;
            self.builder.relate(
                CandidateRelation::Annotates,
                &owner.id,
                Some(&occurrence_id),
                binding.as_ref().map(|binding| binding.binding_id.as_str()),
                spelling,
                ResolutionConstraint {
                    exact_target_declaration_id: None,
                    exact_language: Some("kotlin".to_owned()),
                    module_or_package: Some(self.package.clone()),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: qualified,
                    argument_count: None,
                    argument_types: Vec::new(),
                    allowed_target_kinds: vec!["annotation_type".to_owned(), "class".to_owned()],
                    hierarchy: None,
                    allow_external: true,
                },
            )?;
        }
        Ok(())
    }

    fn add_declaration_type_references(
        &mut self,
        node: Node<'_>,
        owner: &Decl,
    ) -> Result<(), EvidenceError> {
        let mut roots = Vec::new();
        match node.kind() {
            "function_declaration" => {
                if let Some(receiver) = node
                    .child_by_field_name("receiver")
                    .or_else(|| direct_named_child(node, "receiver_type"))
                {
                    roots.push((receiver, "extension_receiver"));
                }
                if let Some(parameters) = direct_named_child(node, "function_value_parameters") {
                    let mut parameter_nodes = Vec::new();
                    collect_nodes(parameters, "parameter", &mut parameter_nodes, 0);
                    for parameter in parameter_nodes {
                        if let Some(kind) = parameter_type_node(parameter) {
                            roots.push((kind, "parameter_type"));
                        }
                    }
                }
                if let Some(return_type) = function_return_type(node) {
                    roots.push((return_type, "return_type"));
                }
            }
            "property_declaration" => {
                if let Some(variable) = direct_named_child(node, "variable_declaration")
                    && let Some(kind) = parameter_type_node(variable)
                {
                    roots.push((kind, "property_type"));
                }
            }
            "type_alias" => {
                if let Some(target) = u32::try_from(node.named_child_count().saturating_sub(1))
                    .ok()
                    .and_then(|index| node.named_child(index))
                {
                    roots.push((target, "alias_target"));
                }
            }
            _ if is_type_node(node.kind()) => {
                if let Some(parameters) = direct_named_child(node, "type_parameters") {
                    roots.push((parameters, "generic_bound"));
                }
                if let Some(constructor) = direct_named_child(node, "primary_constructor") {
                    roots.push((constructor, "constructor_parameter_type"));
                }
            }
            _ => {}
        }
        for (root, context) in roots {
            let mut types = Vec::new();
            collect_type_nodes(root, &mut types, 0);
            types.sort_by_key(Node::start_byte);
            types.dedup_by_key(|kind| (kind.start_byte(), kind.end_byte()));
            for kind in types {
                self.add_type_reference(owner, kind, context)?;
            }
        }
        Ok(())
    }

    fn add_type_reference(
        &mut self,
        owner: &Decl,
        node: Node<'_>,
        context: &str,
    ) -> Result<(), EvidenceError> {
        let raw = normalize_type(self.text(node));
        let spelling = terminal(&raw);
        if spelling.is_empty() || kotlin_builtin_type(spelling) {
            return Ok(());
        }
        let binding = self.import_for(spelling).cloned();
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::TypeReference,
            &owner.id,
            spelling,
            qualified_parent(&raw),
            Some(&owner.scope_id),
            Some(context),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            CandidateRelation::References,
            &owner.id,
            Some(&occurrence_id),
            binding.as_ref().map(|binding| binding.binding_id.as_str()),
            spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some("kotlin".to_owned()),
                module_or_package: Some(self.package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: self.resolve_type(&raw),
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: kotlin_type_target_kinds(),
                hierarchy: None,
                allow_external: true,
            },
        )?;
        Ok(())
    }

    fn add_base_types(&mut self, node: Node<'_>, owner: &Decl) -> Result<(), EvidenceError> {
        let mut specifications = Vec::new();
        let mut cursor = node.walk();
        specifications.extend(
            node.children(&mut cursor)
                .filter(|child| child.kind() == "delegation_specifier"),
        );
        let complete = direct_bases_complete(node);
        for specification in specifications {
            let Some(type_node) =
                first_descendant(specification, &["user_type", "type_identifier"], 0)
            else {
                continue;
            };
            let raw = normalize_type(self.text(type_node));
            let spelling = terminal(&raw);
            if spelling.is_empty() {
                continue;
            }
            let extends = owner.kind != "interface"
                && first_descendant(specification, &["constructor_invocation"], 0).is_some();
            let relation = if extends || owner.kind == "interface" {
                CandidateRelation::Extends
            } else {
                CandidateRelation::Implements
            };
            let binding = self.import_for(spelling).cloned();
            let occurrence_id = self.builder.occur(
                SemanticRole::BaseType,
                &owner.id,
                spelling,
                qualified_parent(&raw),
                Some(&owner.scope_id),
                range_for_node(self.source_file, type_node),
            )?;
            self.builder.relate(
                relation,
                &owner.id,
                Some(&occurrence_id),
                binding.as_ref().map(|binding| binding.binding_id.as_str()),
                spelling,
                ResolutionConstraint {
                    exact_target_declaration_id: None,
                    exact_language: Some("kotlin".to_owned()),
                    module_or_package: Some(self.package.clone()),
                    scope_id: Some(owner.scope_id.clone()),
                    qualified_name: self.resolve_type(&raw),
                    argument_count: None,
                    argument_types: Vec::new(),
                    allowed_target_kinds: kotlin_type_target_kinds(),
                    hierarchy: Some(HierarchyConstraint::DirectBase {
                        base_set_complete: complete,
                    }),
                    allow_external: true,
                },
            )?;
        }
        Ok(())
    }

    fn add_call(&mut self, node: Node<'_>, owner: &Decl) -> Result<(), EvidenceError> {
        if !self.trusted(node) {
            return Ok(());
        }
        let Some(callee) = node.named_child(0) else {
            return Ok(());
        };
        let (qualifier, name_node) = match callee.kind() {
            "simple_identifier" | "type_identifier" => (None, callee),
            "navigation_expression" => {
                let Some(name) = navigation_member(callee) else {
                    return Ok(());
                };
                let qualifier =
                    navigation_receiver(callee).map(|receiver| self.text(receiver).to_owned());
                (qualifier, name)
            }
            _ => return Ok(()),
        };
        let spelling = self.text(name_node).trim().to_owned();
        if spelling.is_empty() {
            return Ok(());
        }
        let arguments = call_arguments(node, self.source);
        let argument_types = arguments
            .iter()
            .map(|argument| self.expression_type(owner, argument.node, 0))
            .collect::<Vec<_>>();
        let argument_context = kotlin_argument_context(&arguments);
        let construction = spelling.starts_with(char::is_uppercase);
        let receiver_type = qualifier
            .as_deref()
            .and_then(|receiver| self.receiver_type(owner, receiver));
        // An imported extension remains a candidate at a qualified call site,
        // but the Kotlin resolver checks a real member first.
        let binding = self.import_for(&spelling).cloned();
        let qualified_name = if construction {
            self.resolve_type(&spelling)
        } else if let Some(receiver) = receiver_type.as_ref() {
            Some(format!("{receiver}::{spelling}"))
        } else if let Some(binding) = binding.as_ref() {
            Some(imported_callable_name(&binding.target))
        } else if qualifier.is_none() {
            owner
                .enclosing_type
                .as_ref()
                .map(|container| format!("{container}::{spelling}"))
                .or_else(|| Some(format!("{}::{spelling}", self.package)))
        } else {
            None
        };
        let constrained_qualified_name =
            receiver_type.is_none().then_some(qualified_name).flatten();
        let role = if construction {
            SemanticRole::Construction
        } else {
            SemanticRole::Call
        };
        let occurrence_id = self.builder.occur_with_context(
            role,
            &owner.id,
            &spelling,
            qualifier.as_deref(),
            Some(&owner.scope_id),
            Some(&argument_context),
            range_for_node(self.source_file, name_node),
        )?;
        self.builder.relate(
            if construction {
                CandidateRelation::Constructs
            } else {
                CandidateRelation::Calls
            },
            &owner.id,
            Some(&occurrence_id),
            binding.as_ref().map(|binding| binding.binding_id.as_str()),
            &spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some("kotlin".to_owned()),
                module_or_package: Some(self.package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: constrained_qualified_name,
                argument_count: Some(u32::try_from(arguments.len()).unwrap_or(u32::MAX)),
                argument_types,
                allowed_target_kinds: if construction {
                    vec![
                        "class".to_owned(),
                        "enum".to_owned(),
                        "object".to_owned(),
                        "annotation_type".to_owned(),
                    ]
                } else {
                    vec!["function".to_owned(), "method".to_owned()]
                },
                hierarchy: receiver_type.as_ref().map(|receiver_qualified_name| {
                    HierarchyConstraint::ReceiverDispatch {
                        receiver_qualified_name: receiver_qualified_name.clone(),
                        strategy: ReceiverDispatchStrategy::C3FromReceiver,
                    }
                }),
                allow_external: construction || binding.is_some() || receiver_type.is_some(),
            },
        )?;
        Ok(())
    }

    fn add_member_access(&mut self, node: Node<'_>, owner: &Decl) -> Result<(), EvidenceError> {
        let Some(name_node) = navigation_member(node) else {
            return Ok(());
        };
        let Some(receiver_node) = navigation_receiver(node) else {
            return Ok(());
        };
        let spelling = self.text(name_node).trim().to_owned();
        let qualifier = self.text(receiver_node).trim().to_owned();
        let Some(receiver_type) = self.receiver_type(owner, &qualifier) else {
            return Ok(());
        };
        let occurrence_id = self.builder.occur(
            SemanticRole::MemberAccess,
            &owner.id,
            &spelling,
            Some(&qualifier),
            Some(&owner.scope_id),
            range_for_node(self.source_file, name_node),
        )?;
        self.builder.relate(
            CandidateRelation::AccessesMember,
            &owner.id,
            Some(&occurrence_id),
            None,
            &spelling,
            ResolutionConstraint {
                exact_target_declaration_id: None,
                exact_language: Some("kotlin".to_owned()),
                module_or_package: Some(self.package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name: None,
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: vec!["property".to_owned(), "constant".to_owned()],
                hierarchy: Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name: receiver_type,
                    strategy: ReceiverDispatchStrategy::C3FromReceiver,
                }),
                allow_external: true,
            },
        )?;
        Ok(())
    }

    fn expression_type(&self, owner: &Decl, node: Node<'_>, depth: usize) -> Option<String> {
        if depth >= 8 {
            return None;
        }
        match node.kind() {
            "simple_identifier" => self.local_value_type(owner, self.text(node).trim()),
            "string_literal" | "line_string_literal" | "multi_line_string_literal" => {
                Some("kotlin.String".to_owned())
            }
            "character_literal" => Some("kotlin.Char".to_owned()),
            "boolean_literal" => Some("kotlin.Boolean".to_owned()),
            "null_literal" => Some("null".to_owned()),
            "integer_literal" => Some(
                if self.text(node).ends_with(['l', 'L']) {
                    "kotlin.Long"
                } else {
                    "kotlin.Int"
                }
                .to_owned(),
            ),
            "real_literal" => Some(
                if self.text(node).ends_with(['f', 'F']) {
                    "kotlin.Float"
                } else {
                    "kotlin.Double"
                }
                .to_owned(),
            ),
            "call_expression" => node.named_child(0).and_then(|callee| {
                let spelling = terminal(self.text(callee).trim());
                spelling
                    .starts_with(char::is_uppercase)
                    .then(|| self.resolve_type(spelling))
                    .flatten()
            }),
            "parenthesized_expression" => node
                .named_child(0)
                .and_then(|inner| self.expression_type(owner, inner, depth + 1)),
            _ => None,
        }
    }

    fn receiver_type(&self, owner: &Decl, receiver: &str) -> Option<String> {
        let receiver = receiver.trim();
        if receiver == "this" || receiver.starts_with("this@") || receiver == "super" {
            return owner.enclosing_type.clone();
        }
        if let Some(kind) = self.local_value_type(owner, receiver) {
            return Some(kind);
        }
        if receiver.starts_with(char::is_uppercase) {
            return self.resolve_type(receiver);
        }
        None
    }

    fn local_value_type(&self, owner: &Decl, name: &str) -> Option<String> {
        let mut scope = Some(owner.scope_id.as_str());
        for _ in 0..MAX_SCOPE_DEPTH {
            let current = scope?;
            if let Some(kind) = self.value_types.get(&(current.to_owned(), name.to_owned())) {
                return Some(kind.clone());
            }
            scope = self.scope_parents.get(current).map(String::as_str);
        }
        None
    }

    fn resolve_type(&self, raw: &str) -> Option<String> {
        let normalized = normalize_type(raw);
        let base = erase_type_arguments(&normalized);
        if base.is_empty() {
            return None;
        }
        if let Some(builtin) = kotlin_builtin_qualified(&base) {
            return Some(builtin);
        }
        if base.contains('.') && base.starts_with(char::is_lowercase) {
            return Some(base);
        }
        let spelling = terminal(&base);
        let imported = self
            .imports
            .iter()
            .filter(|import| import.spelling == spelling)
            .map(|import| import.target.as_str())
            .collect::<BTreeSet<_>>();
        if let [target] = imported.into_iter().collect::<Vec<_>>().as_slice() {
            return Some((*target).to_owned());
        }
        let local = self
            .by_terminal
            .get(spelling)
            .into_iter()
            .flatten()
            .filter_map(|index| {
                self.declarations
                    .get(*index)
                    .map(|decl| decl.qualified.as_str())
            })
            .collect::<BTreeSet<_>>();
        if let [target] = local.into_iter().collect::<Vec<_>>().as_slice() {
            return Some((*target).to_owned());
        }
        Some(join_qualified(&self.package, &base, "."))
    }

    fn import_for(&self, spelling: &str) -> Option<&Import> {
        let mut imports = self
            .imports
            .iter()
            .filter(|import| import.spelling == spelling);
        let only = imports.next()?;
        imports.next().is_none().then_some(only)
    }

    fn capture_parser_errors(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return self.depth_diagnostic(node);
        }
        if node.is_error() || node.is_missing() {
            let start = node.start_byte().min(self.source.len());
            let mut end = node.end_byte().min(self.source.len());
            if end <= start {
                end = self.source[start..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(self.source.len(), |offset| {
                        start.saturating_add(offset).max(start + 1)
                    });
            }
            self.parser_errors.push((start, end));
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.capture_parser_errors(child, depth + 1)?;
        }
        self.parser_errors.sort_unstable();
        self.parser_errors.dedup();
        Ok(())
    }

    fn trusted(&self, node: Node<'_>) -> bool {
        !self.parser_errors.iter().any(|(start, end)| {
            node.start_byte() < *end && node.end_byte().max(node.start_byte() + 1) > *start
        })
    }

    fn depth_diagnostic(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        self.builder.diagnose(
            "kotlin_traversal_limit",
            None,
            Some(range_for_node(self.source_file, node)),
            "Kotlin syntax traversal exceeded its bounded depth",
        )
    }

    fn own(&mut self, owner_id: &str, member_id: &str) -> Result<(), EvidenceError> {
        self.builder.relate(
            CandidateRelation::Owns,
            owner_id,
            None,
            None,
            member_id,
            ResolutionConstraint {
                exact_target_declaration_id: Some(member_id.to_owned()),
                exact_language: Some("kotlin".to_owned()),
                module_or_package: Some(self.package.clone()),
                scope_id: None,
                qualified_name: None,
                argument_count: None,
                argument_types: Vec::new(),
                allowed_target_kinds: Vec::new(),
                hierarchy: None,
                allow_external: false,
            },
        )?;
        Ok(())
    }

    fn text(&self, node: Node<'_>) -> &str {
        self.source
            .get(node.start_byte()..node.end_byte())
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default()
    }
}

fn is_behavioral_owner(owner: &Decl) -> bool {
    matches!(owner.kind.as_str(), "constructor" | "function" | "method")
}

#[derive(Clone, Debug)]
struct KotlinParameter {
    name: String,
    kind: String,
    defaulted: bool,
    variadic: bool,
}

#[derive(Clone, Copy)]
struct KotlinArgument<'tree> {
    node: Node<'tree>,
    name: Option<&'tree str>,
}

fn package_name(root: Node<'_>, source: &[u8]) -> Option<String> {
    let header = direct_named_child(root, "package_header")?;
    let identifier = direct_named_child(header, "identifier")?;
    node_text(source, identifier)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "class_declaration" | "object_declaration" | "companion_object"
    )
}

fn kotlin_type_kind(node: Node<'_>, source: &[u8]) -> &'static str {
    if node.kind() == "object_declaration" {
        return "object";
    }
    if node.kind() == "companion_object" {
        return "companion_object";
    }
    let declaration = node_text(source, node).unwrap_or_default().trim_start();
    let modifiers = direct_named_child(node, "modifiers")
        .and_then(|modifiers| node_text(source, modifiers))
        .unwrap_or_default();
    if declaration.starts_with("interface ") || has_direct_token(node, "interface") {
        "interface"
    } else if declaration.starts_with("enum class ") || has_direct_token(node, "enum") {
        "enum"
    } else if declaration.starts_with("annotation class ")
        || has_direct_token(node, "annotation")
        || modifiers
            .split_whitespace()
            .any(|part| part == "annotation")
    {
        "annotation_type"
    } else {
        "class"
    }
}

fn has_direct_token(node: Node<'_>, expected: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| child.kind() == expected)
}

fn direct_bases_complete(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.kind() == "delegation_specifier")
        .all(|child| !child.has_error())
}

fn modifier_contains(node: Node<'_>, source: &[u8], expected: &str) -> bool {
    direct_named_child(node, "modifiers")
        .and_then(|modifiers| node_text(source, modifiers))
        .is_some_and(|modifiers| modifiers.split_whitespace().any(|part| part == expected))
}

fn function_name_node(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("name").or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|child| child.is_named())
            .find(|child| child.kind() == "simple_identifier")
    })
}

fn function_return_type(node: Node<'_>) -> Option<Node<'_>> {
    let parameters = direct_named_child(node, "function_value_parameters")?;
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.is_named() && child.start_byte() >= parameters.end_byte())
        .find(|child| is_type_syntax(child.kind()))
}

fn parameter_type_node(node: Node<'_>) -> Option<Node<'_>> {
    node.child_by_field_name("type").or_else(|| {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .filter(|child| child.is_named())
            .find(|child| is_type_syntax(child.kind()))
    })
}

fn is_type_syntax(kind: &str) -> bool {
    matches!(
        kind,
        "user_type"
            | "nullable_type"
            | "function_type"
            | "parenthesized_type"
            | "dynamic_type"
            | "type_identifier"
    )
}

fn kotlin_parameters(node: Node<'_>, source: &[u8]) -> Vec<KotlinParameter> {
    let root = direct_named_child(node, "function_value_parameters")
        .or_else(|| direct_named_child(node, "primary_constructor"))
        .unwrap_or(node);
    let expected = if root.kind() == "primary_constructor" {
        "class_parameter"
    } else {
        "parameter"
    };
    let mut cursor = root.walk();
    let mut parameters = root
        .children(&mut cursor)
        .filter(|child| child.kind() == expected)
        .collect::<Vec<_>>();
    parameters.sort_by_key(Node::start_byte);
    parameters.dedup_by_key(|parameter| parameter.id());
    let segments = node_text(source, root)
        .map(split_top_level_parameters)
        .unwrap_or_default();
    parameters
        .into_iter()
        .enumerate()
        .filter_map(|(index, parameter)| {
            let name = direct_named_child(parameter, "simple_identifier")
                .and_then(|name| node_text(source, name))?
                .trim()
                .to_owned();
            let kind = parameter_type_node(parameter)
                .and_then(|kind| node_text(source, kind))
                .map(normalize_type)
                .unwrap_or_else(|| "_".to_owned());
            let text = segments
                .get(index)
                .map(String::as_str)
                .unwrap_or_else(|| node_text(source, parameter).unwrap_or_default());
            Some(KotlinParameter {
                name,
                kind,
                defaulted: top_level_contains(text, '='),
                variadic: text.split_whitespace().any(|part| part == "vararg"),
            })
        })
        .collect()
}

fn split_top_level_parameters(value: &str) -> Vec<String> {
    let value = value.trim();
    let value = value
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(value);
    let mut output = Vec::new();
    let mut start = 0_usize;
    let mut round = 0_u32;
    let mut square = 0_u32;
    let mut angle = 0_u32;
    let mut quoted = None;
    let mut escaped = false;
    for (offset, character) in value.char_indices() {
        if let Some(quote) = quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == quote {
                quoted = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quoted = Some(character),
            '(' => round = round.saturating_add(1),
            ')' => round = round.saturating_sub(1),
            '[' => square = square.saturating_add(1),
            ']' => square = square.saturating_sub(1),
            '<' => angle = angle.saturating_add(1),
            '>' => angle = angle.saturating_sub(1),
            ',' if round == 0 && square == 0 && angle == 0 => {
                output.push(value[start..offset].trim().to_owned());
                start = offset.saturating_add(character.len_utf8());
            }
            _ => {}
        }
    }
    if start < value.len() {
        output.push(value[start..].trim().to_owned());
    }
    output
}

fn kotlin_callable_signature(
    name: &str,
    receiver: Option<&str>,
    parameters: &[KotlinParameter],
) -> String {
    let receiver = receiver.map_or(String::new(), |receiver| format!("receiver={receiver};"));
    let parameters = parameters
        .iter()
        .map(|parameter| {
            format!(
                "{}:{}{}{}",
                parameter.name,
                parameter.kind,
                if parameter.defaulted { "=" } else { "" },
                if parameter.variadic { "..." } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{name}({receiver}{parameters})")
}

fn type_parameter_signature(node: Node<'_>, source: &[u8]) -> Option<String> {
    direct_named_child(node, "type_parameters")
        .and_then(|parameters| node_text(source, parameters))
        .map(str::trim)
        .filter(|parameters| !parameters.is_empty())
        .map(str::to_owned)
}

fn call_arguments<'tree>(node: Node<'tree>, source: &'tree [u8]) -> Vec<KotlinArgument<'tree>> {
    let Some(arguments) = first_descendant(node, &["value_arguments"], 0) else {
        return Vec::new();
    };
    let mut cursor = arguments.walk();
    arguments
        .children(&mut cursor)
        .filter(|child| child.kind() == "value_argument")
        .filter_map(|argument| {
            let mut children_cursor = argument.walk();
            let children = argument
                .children(&mut children_cursor)
                .filter(|child| child.is_named())
                .collect::<Vec<_>>();
            let text = node_text(source, argument)?;
            let named = top_level_contains(text, '=') && children.len() >= 2;
            let name = named
                .then(|| node_text(source, children[0]).map(str::trim))
                .flatten();
            let node = children.last().copied().unwrap_or(argument);
            Some(KotlinArgument { node, name })
        })
        .collect()
}

fn kotlin_argument_context(arguments: &[KotlinArgument<'_>]) -> String {
    let names = arguments
        .iter()
        .map(|argument| argument.name.unwrap_or("_"))
        .collect::<Vec<_>>()
        .join(",");
    format!("kotlin_args:{names}")
}

fn navigation_receiver(node: Node<'_>) -> Option<Node<'_>> {
    node.named_child(0)
}

fn navigation_member(node: Node<'_>) -> Option<Node<'_>> {
    let suffix = direct_named_child(node, "navigation_suffix")?;
    first_identifier(suffix)
}

fn inferred_initializer_type(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let initializer = node
        .children(&mut cursor)
        .filter(|child| child.is_named())
        .find(|child| child.kind() == "call_expression")?;
    let callee = initializer.named_child(0)?;
    let spelling = terminal(node_text(source, callee)?.trim());
    spelling
        .starts_with(char::is_uppercase)
        .then(|| spelling.to_owned())
}

fn collect_type_nodes<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>, depth: usize) {
    if depth > MAX_TRAVERSAL_DEPTH {
        return;
    }
    if node.kind() == "user_type" {
        output.push(node);
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(Node::is_named) {
        collect_type_nodes(child, output, depth + 1);
    }
}

fn collect_nodes<'tree>(
    node: Node<'tree>,
    expected: &str,
    output: &mut Vec<Node<'tree>>,
    depth: usize,
) {
    if depth > MAX_TRAVERSAL_DEPTH {
        return;
    }
    if node.kind() == expected {
        output.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor).filter(Node::is_named) {
        collect_nodes(child, expected, output, depth + 1);
    }
}

fn first_descendant<'tree>(
    node: Node<'tree>,
    expected: &[&str],
    depth: usize,
) -> Option<Node<'tree>> {
    if depth > MAX_TRAVERSAL_DEPTH {
        return None;
    }
    if expected.contains(&node.kind()) {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(Node::is_named)
        .find_map(|child| first_descendant(child, expected, depth + 1))
}

fn direct_named_child<'tree>(node: Node<'tree>, expected: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.is_named() && child.kind() == expected)
}

fn first_identifier(node: Node<'_>) -> Option<Node<'_>> {
    first_descendant(
        node,
        &["simple_identifier", "type_identifier", "identifier"],
        0,
    )
}

fn node_text<'source>(source: &'source [u8], node: Node<'_>) -> Option<&'source str> {
    source
        .get(node.start_byte()..node.end_byte())
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
}

fn normalize_type(raw: &str) -> String {
    raw.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn erase_type_arguments(raw: &str) -> String {
    let mut depth = 0_u32;
    raw.chars()
        .filter(|character| match character {
            '<' => {
                depth = depth.saturating_add(1);
                false
            }
            '>' => {
                depth = depth.saturating_sub(1);
                false
            }
            '?' if depth == 0 => false,
            _ => depth == 0,
        })
        .collect()
}

fn top_level_contains(value: &str, needle: char) -> bool {
    let mut round = 0_u32;
    let mut square = 0_u32;
    let mut angle = 0_u32;
    let mut quoted = None;
    let mut escaped = false;
    for character in value.chars() {
        if let Some(quote) = quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == quote {
                quoted = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quoted = Some(character),
            '(' => round = round.saturating_add(1),
            ')' => round = round.saturating_sub(1),
            '[' => square = square.saturating_add(1),
            ']' => square = square.saturating_sub(1),
            '<' => angle = angle.saturating_add(1),
            '>' => angle = angle.saturating_sub(1),
            _ if character == needle && round == 0 && square == 0 && angle == 0 => return true,
            _ => {}
        }
    }
    false
}

fn imported_callable_name(target: &str) -> String {
    target.rsplit_once('.').map_or_else(
        || target.to_owned(),
        |(owner, name)| format!("{owner}::{name}"),
    )
}

fn join_qualified(owner: &str, name: &str, separator: &str) -> String {
    if owner.is_empty() || owner == "<default>" {
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

fn qualified_parent(value: &str) -> Option<&str> {
    value.rsplit_once('.').map(|(parent, _)| parent)
}

fn kotlin_builtin_type(name: &str) -> bool {
    kotlin_builtin_qualified(name).is_some()
}

fn kotlin_builtin_qualified(name: &str) -> Option<String> {
    let base = name.strip_prefix("kotlin.").unwrap_or(name);
    matches!(
        base,
        "Any"
            | "Boolean"
            | "Byte"
            | "Char"
            | "Double"
            | "Float"
            | "Int"
            | "Long"
            | "Nothing"
            | "Short"
            | "String"
            | "Unit"
            | "Array"
    )
    .then(|| format!("kotlin.{base}"))
}

fn kotlin_type_target_kinds() -> Vec<String> {
    [
        "annotation_type",
        "class",
        "companion_object",
        "enum",
        "interface",
        "object",
        "type_alias",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn kotlin_import_target_kinds() -> Vec<String> {
    let mut kinds = kotlin_type_target_kinds();
    kinds.extend(
        ["function", "method", "property", "constant"]
            .into_iter()
            .map(str::to_owned),
    );
    kinds
}
