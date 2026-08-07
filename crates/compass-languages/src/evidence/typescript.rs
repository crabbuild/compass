//! Direct, test-only universal evidence for the ECMAScript family.
//!
//! This module deliberately does not participate in `UNIVERSAL_ADAPTERS` yet.
//! It is a source-grounded qualification path for the Phase 2 work in Plan
//! 013. The production extractor remains the compatibility path until this
//! emitter has complete capability coverage and passes the hard-cut gates.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use tree_sitter::Node;

use super::build::{EvidenceBuilder, range_for_node};
use super::model::{
    BindingKind, CandidateRelation, HierarchyConstraint, LanguageCapability, ResolutionConstraint,
    SemanticEvidenceBatch, SemanticRole, SymbolNamespace,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits};
use crate::{AdapterProfile, UNIVERSAL_EVIDENCE_SCHEMA, make_id};

const MAX_TRAVERSAL_DEPTH: usize = 512;
const MAX_INLINE_OBJECT_PROPERTIES: usize = 256;
const MAX_TYPE_SHAPE_DEPTH: u32 = 32;
const MAX_TYPE_SHAPE_BYTES: usize = 16 * 1024;

const TYPESCRIPT_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
    LanguageCapability::Imports,
    LanguageCapability::Reexports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::Decorators,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

const JAVASCRIPT_CAPABILITIES: &[LanguageCapability] = &[
    LanguageCapability::Declarations,
    LanguageCapability::LexicalScopes,
    LanguageCapability::Namespaces,
    LanguageCapability::Imports,
    LanguageCapability::Reexports,
    LanguageCapability::Aliases,
    LanguageCapability::Calls,
    LanguageCapability::Construction,
    LanguageCapability::Decorators,
    LanguageCapability::TypeReferences,
    LanguageCapability::BaseTypes,
    LanguageCapability::HierarchyDispatch,
    LanguageCapability::Members,
    LanguageCapability::Ownership,
    LanguageCapability::Receivers,
    LanguageCapability::ExternalReferences,
];

static TYPESCRIPT_PROFILE: AdapterProfile = AdapterProfile {
    id: "compass.typescript.candidate",
    language: "typescript",
    version: 3,
    evidence_schema: UNIVERSAL_EVIDENCE_SCHEMA,
    profile: crate::UniversalAdapterProfile::UniversalCandidate,
    capabilities: TYPESCRIPT_CAPABILITIES,
};

static JAVASCRIPT_PROFILE: AdapterProfile = AdapterProfile {
    id: "compass.javascript.candidate",
    language: "javascript",
    version: 3,
    evidence_schema: UNIVERSAL_EVIDENCE_SCHEMA,
    profile: crate::UniversalAdapterProfile::UniversalCandidate,
    capabilities: JAVASCRIPT_CAPABILITIES,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Namespace {
    Value,
    Type,
    Module,
    Both,
}

impl Namespace {
    fn accepts(self, requested: Self) -> bool {
        matches!(self, Self::Both)
            || matches!(requested, Self::Both)
            || self == requested
            || (self == Self::Module && matches!(requested, Self::Value | Self::Type))
    }
}

#[derive(Clone, Debug)]
struct DeclarationInfo {
    id: String,
    name: String,
    kind: String,
    qualified_name: String,
    scope_id: String,
    /// Source start of the declaration's canonical evidence range.  This is
    /// retained for the narrow flow-sensitive structural-object rule below;
    /// nominal class/type members continue to use scope/ambiguity proof
    /// instead of source order.
    range_start_byte: usize,
    namespace: Namespace,
    parameter_count: Option<u32>,
    /// Canonical parameter types when every parameter is source-annotated and
    /// fixed-arity. `None` means overload selection must not rely on types.
    parameter_types: Option<Vec<String>>,
    /// Minimum and maximum source-level arity. `None` as the maximum means a
    /// rest parameter permits an unbounded suffix. These ranges are used only
    /// when a class inherits a proven local constructor signature.
    parameter_min_count: Option<u32>,
    parameter_max_count: Option<u32>,
    return_type_name: Option<String>,
    /// A direct source annotation for a value/member receiver. This is kept
    /// intentionally shallow; generic constraints and imported identities are
    /// resolved only when a later member chain needs them.
    declared_type_name: Option<String>,
    /// A source annotation or initializer proves that a variable/parameter
    /// is callable even when its concrete signature is not available in this
    /// file (for example `factory: (x: T) => U` or
    /// `const Ctor: typeof Imported = ...`).
    callable_shape: bool,
    /// Whether a class declares its own constructor. An absent constructor
    /// may inherit a fixed signature from a proven local base class.
    explicit_constructor: bool,
}

#[derive(Clone, Debug)]
struct ImportInfo {
    binding_id: String,
    target: String,
    module: String,
    imported_name: String,
    namespace: Namespace,
    type_only: bool,
    callable_namespace: bool,
}

#[derive(Clone, Debug)]
enum Resolution {
    Local(DeclarationInfo),
    Import(ImportInfo),
}

impl Resolution {
    fn qualified_target(&self) -> Option<String> {
        match self {
            Self::Local(declaration)
                if matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "namespace" | "type_alias"
                ) =>
            {
                Some(declaration.qualified_name.clone())
            }
            Self::Import(import) => Some(import_target_without_namespace(&import.target)),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct ReceiverTarget {
    qualified_name: String,
    import: Option<ImportInfo>,
    scope_id: Option<String>,
    /// Type arguments preserved when a source annotation instantiates a
    /// generic nominal receiver.  The vector follows the declaration's
    /// source parameter order and is intentionally shallow; unresolved or
    /// ambiguous instantiations carry `None`.
    type_arguments: Option<Vec<String>>,
}

#[derive(Clone)]
struct FlowAssignment {
    start_byte: usize,
    receiver: ReceiverTarget,
}

struct CandidateState<'source, 'tree> {
    source_file: &'source str,
    source: &'source [u8],
    language: &'static str,
    root_scope: String,
    file_declaration: String,
    file_qualified_name: String,
    scope_by_node: HashMap<usize, String>,
    scope_parents: HashMap<String, Option<String>>,
    scope_owners: HashMap<String, String>,
    scope_kinds: HashMap<String, String>,
    declarations: HashMap<String, DeclarationInfo>,
    declarations_by_scope: HashMap<String, Vec<String>>,
    declarations_by_qualified: HashMap<String, Vec<String>>,
    generic_parameters_by_declaration: HashMap<String, HashMap<String, Option<String>>>,
    generic_parameter_order_by_declaration: HashMap<String, Vec<String>>,
    /// Source-visible union constituents for local type aliases. This is
    /// retained only for bounded discriminant narrowing (for example
    /// `if (result.success) result.data`) and is never used as a general
    /// structural assignability fallback.
    type_alias_union_targets: HashMap<String, Vec<String>>,
    property_literal_values: HashMap<String, String>,
    index_value_types: HashMap<String, String>,
    import_bindings: HashMap<(String, String), ImportInfo>,
    variable_types: HashMap<String, String>,
    /// Source-order receiver facts for simple local assignments.  A fact is
    /// usable only when it is in the variable's binding scope and precedes
    /// the member use; branch/unknown writes are tracked separately as
    /// barriers so an older fact cannot survive an unproven mutation.
    flow_assignments: HashMap<String, Vec<FlowAssignment>>,
    flow_assignment_barriers: HashMap<String, Vec<(usize, String)>>,
    /// Escape/dynamic barriers survive later local assignments because a
    /// captured binding or dynamic evaluator can observe the rebinding too.
    flow_escape_barriers: HashMap<String, Vec<usize>>,
    /// Property-specific writes on an immutable source-proven receiver alias.
    /// A write to an unrelated property does not erase the receiver identity,
    /// while a write to the queried member remains a fail-closed barrier.
    flow_member_write_barriers: HashMap<(String, String), Vec<usize>>,
    /// `const` bindings whose receiver identity is source-proven and cannot
    /// be rebound. Stable nominal and exact object-literal aliases may be
    /// captured by a closure; mutable/unsupported aliases still take the
    /// conservative escape barrier below.
    immutable_bindings: HashSet<String>,
    structural_object_variables: HashSet<String>,
    stable_structural_object_variables: HashSet<String>,
    return_object_functions: HashSet<String>,
    /// A local variable initialized from a source-visible function whose
    /// return value is an object.  Store the declaration identity rather than
    /// only its qualified spelling so duplicate block-local functions cannot
    /// collapse the receiver scope during member resolution.
    variable_object_sources: HashMap<String, String>,
    /// Inline object annotations (`const value: { key: T } = ...`) use the
    /// type-literal property declaration as the compiler's member target.
    /// Keep the containing qualified prefix so `value.key` can reach that
    /// declaration without conflating it with the runtime object literal.
    variable_inline_type_receivers: HashMap<String, String>,
    /// Direct property type names from a small inline object annotation. This
    /// is intentionally string-only and shallow: it avoids retaining parser
    /// nodes or recursively walking arbitrary type expressions while allowing
    /// one-step destructuring flow to use source evidence.
    inline_object_property_types: HashMap<String, HashMap<String, String>>,
    inline_object_property_declaration_ids: HashMap<(String, String), String>,
    /// Variables initialized from an exact local constructor prototype (for
    /// example `const proto = Legacy.prototype`). Keeping the source-backed
    /// prototype identity allows the common JavaScript alias style without
    /// treating an arbitrary object named `prototype` as a constructor.
    prototype_sources: HashMap<String, String>,
    /// Same-file constructor names observed at a `new` site. This narrow
    /// proof lets `this.field = ...` inside a function constructor publish an
    /// instance member without treating every dynamic function receiver as a
    /// nominal class.
    constructor_name_hints: HashSet<String>,
    /// Property declarations initialized from a local callable identifier.
    /// The map is collected before inference and converted to a set only when
    /// the target declaration is proven callable, keeping aliases such as
    /// `ZodEnum.create = createZodEnum` precise without guessing from name
    /// spelling.
    property_alias_values: HashMap<String, String>,
    callable_property_aliases: HashSet<String>,
    /// Variable aliases to source-proven callable values. These are kept
    /// separate from receiver aliases: passing a callback through an
    /// unknown API is a reference to the callable binding, not an indirect
    /// call to a guessed handler.
    callable_variable_aliases: HashSet<String>,
    /// Variable aliases to a source member/function value. These are retained
    /// only to recover a source-declared return type at a subsequent call
    /// site (for example `const stringType = ZodString.create`).
    variable_alias_values: HashMap<String, String>,
    this_receivers: HashMap<String, String>,
    /// Direct `extends` targets proven while emitting the class heritage.
    /// Values are retained only for classes with one source-visible base; an
    /// unresolved or ambiguous base never becomes a `super` receiver.
    base_targets: HashMap<String, ReceiverTarget>,
    implements_targets: HashMap<String, Vec<ReceiverTarget>>,
    /// Source-proven bindings from a derived class's base type parameters to
    /// the concrete type arguments used in its `extends` clause.
    base_type_bindings: HashMap<String, HashMap<String, String>>,
    declaration_name_nodes: HashSet<usize>,
    import_nodes: HashSet<usize>,
    emitted_facts: HashSet<String>,
    builder: EvidenceBuilder,
    _tree: std::marker::PhantomData<Node<'tree>>,
}

pub(crate) fn extract_candidate_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
    dialect: &str,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    let (language, profile) = match dialect {
        "typescript" | "tsx" => ("typescript", &TYPESCRIPT_PROFILE),
        "javascript" => ("javascript", &JAVASCRIPT_PROFILE),
        _ => {
            return Err(EvidenceError::new(
                EvidenceErrorCode::InvalidAdapter,
                format!("unsupported ECMAScript candidate dialect {dialect:?}"),
            ));
        }
    };
    let module_name = module_name(path, source_file);
    let dialect_name = dialect_for_path(path, dialect);
    let mut builder = EvidenceBuilder::new_with_dialect(
        profile,
        format!("compass.languages.{language}.universal.candidate"),
        source_file,
        EvidenceLimits::default(),
        Some(&dialect_name),
    );
    let file_range = range_for_node(source_file, root);
    let file_graph_id = stable_graph_id(source_file, "module", &module_name, 0);
    let file_declaration = builder.declare_with_namespace(
        "module",
        &file_graph_id,
        &module_name,
        &module_name,
        Some(source_file),
        None,
        Some(SymbolNamespace::Namespace),
        file_range.clone(),
    )?;
    let root_scope = builder.open_scope("module", Some(&file_declaration), None, file_range)?;
    let mut state = CandidateState {
        source_file,
        source,
        language,
        root_scope: root_scope.clone(),
        file_declaration: file_declaration.clone(),
        file_qualified_name: module_name.clone(),
        scope_by_node: HashMap::new(),
        scope_parents: HashMap::from([(root_scope.clone(), None)]),
        scope_owners: HashMap::from([(root_scope.clone(), file_declaration)]),
        scope_kinds: HashMap::from([(root_scope.clone(), "module".to_owned())]),
        declarations: HashMap::new(),
        declarations_by_scope: HashMap::new(),
        declarations_by_qualified: HashMap::new(),
        generic_parameters_by_declaration: HashMap::new(),
        generic_parameter_order_by_declaration: HashMap::new(),
        type_alias_union_targets: HashMap::new(),
        property_literal_values: HashMap::new(),
        index_value_types: HashMap::new(),
        import_bindings: HashMap::new(),
        variable_types: HashMap::new(),
        flow_assignments: HashMap::new(),
        flow_assignment_barriers: HashMap::new(),
        flow_escape_barriers: HashMap::new(),
        flow_member_write_barriers: HashMap::new(),
        immutable_bindings: HashSet::new(),
        structural_object_variables: HashSet::new(),
        stable_structural_object_variables: HashSet::new(),
        return_object_functions: HashSet::new(),
        variable_object_sources: HashMap::new(),
        variable_inline_type_receivers: HashMap::new(),
        inline_object_property_types: HashMap::new(),
        inline_object_property_declaration_ids: HashMap::new(),
        prototype_sources: HashMap::new(),
        constructor_name_hints: HashSet::new(),
        property_alias_values: HashMap::new(),
        callable_property_aliases: HashSet::new(),
        callable_variable_aliases: HashSet::new(),
        variable_alias_values: HashMap::new(),
        this_receivers: HashMap::new(),
        base_targets: HashMap::new(),
        implements_targets: HashMap::new(),
        base_type_bindings: HashMap::new(),
        declaration_name_nodes: HashSet::new(),
        import_nodes: HashSet::new(),
        emitted_facts: HashSet::new(),
        builder,
        _tree: std::marker::PhantomData,
    };
    state.collect_constructor_name_hints(root, 0);
    state.collect_declarations(root, root_scope, module_name, 0)?;
    // Imports are lexically hoisted by ECMAScript/TypeScript.  Materialize
    // their bindings before receiver inference so a later declaration such
    // as `const client = new ImportedClient()` can use the imported class
    // without depending on source order.
    state.precollect_imports(root, 0)?;
    state.resolve_callable_property_aliases();
    state.precollect_base_targets(root, 0);
    state.infer_variable_types(root, 0);
    state.collect_flow_assignment_facts(root, 0);
    state.emit_nodes(root, 0)?;
    if root.has_error() {
        state.builder.diagnose(
            "partial_parser_recovery",
            None,
            Some(range_for_node(source_file, root)),
            "parser recovered from malformed ECMAScript source; emitted evidence remains source-bounded",
        )?;
    }
    state.builder.finish()
}

impl<'source, 'tree> CandidateState<'source, 'tree> {
    fn collect_constructor_name_hints(&mut self, node: Node<'tree>, depth: usize) {
        let mut pending = vec![(node, depth)];
        while let Some((current, current_depth)) = pending.pop() {
            if current_depth > MAX_TRAVERSAL_DEPTH {
                continue;
            }
            if current.kind() == "new_expression"
                && let Some(constructor) = current
                    .child_by_field_name("constructor")
                    .or_else(|| first_named_child(current))
                && let Some(name) = rightmost_identifier(constructor)
            {
                let spelling = node_text(self.source, name);
                if !spelling.is_empty() {
                    self.constructor_name_hints.insert(spelling);
                }
            }
            if current.kind() == "assignment_expression"
                && let Some(left) = current.child_by_field_name("left")
                && let Some(object) = left.child_by_field_name("object")
            {
                let object_text = node_text(self.source, object);
                if let Some(base) = object_text.strip_suffix(".prototype") {
                    let spelling = base
                        .rsplit(['.', ':'])
                        .next()
                        .map(str::trim)
                        .unwrap_or_default();
                    if !spelling.is_empty() {
                        self.constructor_name_hints.insert(spelling.to_owned());
                    }
                }
            }
            if current.kind() == "variable_declarator"
                && let Some(value) = current.child_by_field_name("value")
                && let Some(object) = value.child_by_field_name("object")
                && let Some(property) = member_property_node(value)
                && member_property_name(self.source, property).as_deref() == Some("prototype")
            {
                let object_text = node_text(self.source, object);
                let spelling = object_text
                    .rsplit(['.', ':'])
                    .next()
                    .map(str::trim)
                    .unwrap_or_default();
                if !spelling.is_empty() {
                    self.constructor_name_hints.insert(spelling.to_owned());
                }
            }
            let mut cursor = current.walk();
            let children = current.named_children(&mut cursor).collect::<Vec<_>>();
            for child in children.into_iter().rev() {
                pending.push((child, current_depth.saturating_add(1)));
            }
        }
    }

    fn collect_declarations(
        &mut self,
        node: Node<'tree>,
        scope_id: String,
        qualified_prefix: String,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            self.builder.diagnose(
                "traversal_depth_limit",
                None,
                Some(range_for_node(self.source_file, node)),
                "ECMAScript declaration traversal exceeded the bounded depth",
            )?;
            return Ok(());
        }
        let mut scope_id = self.enter_lexical_scope(node, scope_id)?;
        // Conditional and mapped types introduce their own type-binding
        // environments.  Keeping them distinct is essential for repeated
        // mapped keys (`K`) and conditional `infer` names (`Intersection`,
        // `Last`): the checker binds each occurrence to the nearest source
        // type construct rather than treating same-spelled keys in a whole
        // alias as overloads.
        if matches!(
            node.kind(),
            "conditional_type" | "mapped_type" | "object_type"
        ) && !self.scope_by_node.contains_key(&node.id())
        {
            let owner = self.owner_for_scope(&scope_id);
            let kind = if node.kind() == "conditional_type" {
                "conditional_type"
            } else if node.kind() == "object_type" {
                "object_type"
            } else {
                "mapped_type"
            };
            let child_scope = self.builder.open_scope(
                kind,
                Some(&owner),
                Some(&scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_by_node.insert(node.id(), child_scope.clone());
            self.scope_parents
                .insert(child_scope.clone(), Some(scope_id.clone()));
            self.scope_owners.insert(child_scope.clone(), owner);
            self.scope_kinds
                .insert(child_scope.clone(), kind.to_owned());
            scope_id = child_scope;
        }
        if node.kind() == "return_statement"
            && return_value_node(node)
                .map(unwrap_expression_node)
                .is_some_and(|value| value.kind() == "object")
        {
            let owner = self.owner_for_scope(&scope_id);
            if let Some(declaration) = self.declarations.get(&owner) {
                self.return_object_functions
                    .insert(declaration.qualified_name.clone());
            }
        }
        if matches!(node.kind(), "for_in_statement" | "for_statement")
            && node.child_by_field_name("kind").is_some()
            && let Some(left) = node.child_by_field_name("left")
        {
            let mut names = Vec::new();
            collect_pattern_names(left, self.source, &mut names);
            for (name, name_node) in names {
                if !name.is_empty() {
                    self.add_declaration_at(
                        left,
                        name_node,
                        "variable",
                        Namespace::Value,
                        &self.binding_scope_for(left, &scope_id),
                        &qualified_prefix,
                        &name,
                    )?;
                }
            }
        }
        if node.kind() == "catch_clause"
            && let Some(parameter) = node.child_by_field_name("parameter")
        {
            let mut names = Vec::new();
            collect_pattern_names(parameter, self.source, &mut names);
            for (name, name_node) in names {
                if !name.is_empty() {
                    self.add_declaration_at(
                        parameter,
                        name_node,
                        "variable",
                        Namespace::Value,
                        &scope_id,
                        &qualified_prefix,
                        &name,
                    )?;
                }
            }
        }
        if node.kind() == "index_signature"
            && let Some(name_node) = node.child_by_field_name("name")
        {
            let name = node_text(self.source, name_node);
            if !name.is_empty() {
                self.add_declaration_at(
                    node,
                    name_node,
                    "parameter",
                    Namespace::Value,
                    &scope_id,
                    &qualified_prefix,
                    &name,
                )?;
            }
        }
        if node.kind() == "mapped_type_clause"
            && let Some(name_node) = node.child_by_field_name("name")
        {
            let name = node_text(self.source, name_node);
            if !name.is_empty() {
                self.add_declaration_at(
                    node,
                    name_node,
                    "type_parameter",
                    Namespace::Type,
                    &scope_id,
                    &qualified_prefix,
                    &name,
                )?;
            }
        }
        if node.kind() == "infer_type"
            && let Some(name_node) = first_named_child_kind(node, "type_identifier")
        {
            let name = node_text(self.source, name_node);
            if !name.is_empty() {
                let binding_scope = self.infer_binding_scope(node, &scope_id);
                self.add_declaration_at(
                    node,
                    name_node,
                    "type_parameter",
                    Namespace::Type,
                    &binding_scope,
                    &qualified_prefix,
                    &name,
                )?;
            }
        }
        if matches!(
            node.kind(),
            "arrow_function"
                | "function"
                | "function_expression"
                | "function_type"
                | "generator_function"
        ) && !self.scope_by_node.contains_key(&node.id())
        {
            let owner = self.owner_for_scope(&scope_id);
            let this_receiver = self.prototype_assignment_receiver(node, &scope_id);
            let child_scope = self.builder.open_scope(
                "function",
                Some(&owner),
                Some(&scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_by_node.insert(node.id(), child_scope.clone());
            self.scope_parents
                .insert(child_scope.clone(), Some(scope_id.clone()));
            self.scope_owners.insert(child_scope.clone(), owner);
            self.scope_kinds
                .insert(child_scope.clone(), "function".to_owned());
            if let Some(receiver) = this_receiver {
                self.this_receivers.insert(child_scope.clone(), receiver);
            }
            self.collect_unwrapped_callable_parameters(node, &child_scope, &qualified_prefix)?;
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                self.collect_declarations(
                    child,
                    child_scope.clone(),
                    qualified_prefix.clone(),
                    depth + 1,
                )?;
            }
            return Ok(());
        }
        if node.kind() == "export_statement"
            && node_text(self.source, node)
                .trim_start()
                .starts_with("export default")
            && let Some(exported) = first_named_child(node)
            && exported.child_by_field_name("name").is_none()
            && let Some((kind, namespace, creates_scope, scope_kind)) =
                anonymous_declaration_shape(exported)
        {
            let declaration = self.add_declaration_at(
                exported,
                exported,
                kind,
                namespace,
                &scope_id,
                &qualified_prefix,
                "default",
            )?;
            if creates_scope {
                let child_scope = self.builder.open_scope(
                    scope_kind,
                    Some(&declaration.id),
                    Some(&scope_id),
                    range_for_node(self.source_file, exported),
                )?;
                self.scope_by_node
                    .insert(exported.id(), child_scope.clone());
                self.scope_parents
                    .insert(child_scope.clone(), Some(scope_id));
                self.scope_owners
                    .insert(child_scope.clone(), declaration.id);
                self.scope_kinds
                    .insert(child_scope.clone(), scope_kind.to_owned());
                self.collect_unwrapped_callable_parameters(
                    exported,
                    &child_scope,
                    &declaration.qualified_name,
                )?;
                let mut cursor = exported.walk();
                for child in exported.named_children(&mut cursor) {
                    self.collect_declarations(
                        child,
                        child_scope.clone(),
                        declaration.qualified_name.clone(),
                        depth + 1,
                    )?;
                }
            }
            return Ok(());
        }
        if is_anonymous_signature_scope(node) {
            let owner = self.scope_owners.get(&scope_id).cloned();
            let signature_scope = self.builder.open_scope(
                "signature",
                owner.as_deref(),
                Some(&scope_id),
                range_for_node(self.source_file, node),
            )?;
            self.scope_by_node
                .insert(node.id(), signature_scope.clone());
            self.scope_parents
                .insert(signature_scope.clone(), Some(scope_id.clone()));
            if let Some(owner) = owner {
                self.scope_owners.insert(signature_scope.clone(), owner);
            }
            self.scope_kinds
                .insert(signature_scope.clone(), "signature".to_owned());
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                self.collect_declarations(
                    child,
                    signature_scope.clone(),
                    qualified_prefix.clone(),
                    depth + 1,
                )?;
            }
            return Ok(());
        }
        if is_parameter_node(node)
            && let Some(pattern) = parameter_pattern_node(node)
        {
            let mut names = Vec::new();
            collect_pattern_names(pattern, self.source, &mut names);
            for (name, name_node) in names {
                if !name.is_empty() {
                    self.add_declaration_at(
                        node,
                        name_node,
                        "parameter",
                        Namespace::Value,
                        &scope_id,
                        &qualified_prefix,
                        &name,
                    )?;
                }
            }
        }
        if let Some((kind, name_node, namespace, creates_scope, scope_kind)) =
            declaration_shape(node)
        {
            let name = node_text(self.source, name_node);
            if !name.is_empty() {
                let kind = if kind == "method" && name == "constructor" {
                    "constructor"
                } else {
                    kind
                };
                let declaration = self.add_declaration(
                    node,
                    name_node,
                    kind,
                    namespace,
                    &scope_id,
                    &qualified_prefix,
                )?;
                let next_prefix = declaration.qualified_name.clone();
                if creates_scope {
                    let child_scope = self.builder.open_scope(
                        scope_kind,
                        Some(&declaration.id),
                        Some(&scope_id),
                        range_for_node(self.source_file, node),
                    )?;
                    self.scope_by_node.insert(node.id(), child_scope.clone());
                    self.scope_parents
                        .insert(child_scope.clone(), Some(scope_id));
                    self.scope_owners
                        .insert(child_scope.clone(), declaration.id);
                    self.scope_kinds
                        .insert(child_scope.clone(), scope_kind.to_owned());
                    self.collect_unwrapped_callable_parameters(node, &child_scope, &next_prefix)?;
                    let mut cursor = node.walk();
                    for child in node.named_children(&mut cursor) {
                        self.collect_declarations(
                            child,
                            child_scope.clone(),
                            next_prefix.clone(),
                            depth + 1,
                        )?;
                    }
                } else {
                    let mut cursor = node.walk();
                    for child in node.named_children(&mut cursor) {
                        self.collect_declarations(
                            child,
                            scope_id.clone(),
                            next_prefix.clone(),
                            depth + 1,
                        )?;
                    }
                }
                return Ok(());
            }
        }
        let mut object_literal_value = None;
        if node.kind() == "assignment_expression"
            && let Some(left) = node.child_by_field_name("left")
            && matches!(
                left.kind(),
                "member_expression" | "optional_member_expression" | "subscript_expression"
            )
            && let Some(property) = member_property_node(left)
            && let Some(property_name) = member_property_name(self.source, property)
            && let Some(object) = left.child_by_field_name("object")
            && let Some(receiver) = self.receiver_target(&scope_id, object)
            && receiver.import.is_none()
        {
            let qualified_name = format!("{}.{property_name}", receiver.qualified_name);
            let class_member_alias =
                object.kind() == "identifier" && self.enclosing_type(&scope_id).is_some();
            let prototype_instance_write =
                object.kind() == "this" && receiver.qualified_name.ends_with(".prototype");
            if !class_member_alias
                && !prototype_instance_write
                && !self.declarations_by_qualified.contains_key(&qualified_name)
            {
                let class_receiver = self
                    .resolve_name(&scope_id, &node_text(self.source, object), Namespace::Value)
                    .is_some_and(|resolution| {
                        matches!(
                            resolution,
                            Resolution::Local(declaration)
                                if matches!(
                                    declaration.kind.as_str(),
                                    "class" | "interface" | "enum" | "namespace"
                            )
                        )
                    });
                let prototype_receiver = self.is_function_prototype_receiver(&scope_id, object);
                let prototype_alias_receiver = self
                    .prototype_sources
                    .values()
                    .any(|source| source == &receiver.qualified_name);
                let structural_object_receiver = self
                    .structural_object_variables
                    .contains(&receiver.qualified_name);
                // The checker anchors an assignment-backed member at its
                // property name for class-static and function-prototype
                // assignments, while instance/object writes use the
                // enclosing assignment as their source-backed structural
                // declaration.
                let range_node = if class_receiver
                    || prototype_receiver
                    || prototype_alias_receiver
                    || structural_object_receiver
                {
                    property
                } else {
                    node
                };
                self.add_declaration_at_with_range(
                    node,
                    property,
                    range_node,
                    "property",
                    Namespace::Value,
                    &scope_id,
                    &receiver.qualified_name,
                    &property_name,
                )?;
                if let Some(right) = node
                    .child_by_field_name("right")
                    .map(unwrap_expression_node)
                    && right.kind() == "object"
                {
                    self.structural_object_variables
                        .insert(qualified_name.clone());
                    object_literal_value = Some(right);
                    let mut cursor = right.walk();
                    for child in right.named_children(&mut cursor) {
                        self.collect_declarations(
                            child,
                            scope_id.clone(),
                            qualified_name.clone(),
                            depth + 1,
                        )?;
                    }
                }
            }
        }
        if node.kind() == "assignment_expression"
            && let Some(left) = node.child_by_field_name("left")
            && left.kind() == "identifier"
            && let Some(value) = node
                .child_by_field_name("right")
                .map(unwrap_expression_node)
            && value.kind() == "object"
            && let Some(Resolution::Local(variable)) =
                self.resolve_name(&scope_id, &node_text(self.source, left), Namespace::Value)
            && variable.kind == "variable"
        {
            object_literal_value = Some(value);
            self.structural_object_variables
                .insert(variable.qualified_name.clone());
            let mut cursor = value.walk();
            for child in value.named_children(&mut cursor) {
                self.collect_declarations(
                    child,
                    scope_id.clone(),
                    variable.qualified_name.clone(),
                    depth + 1,
                )?;
            }
        }
        if node.kind() == "variable_declarator"
            && let Some(name_node) = node.child_by_field_name("name")
        {
            let binding_object_literal_value = node
                .child_by_field_name("value")
                .and_then(|value| self.object_literal_value_for_binding(value));
            let object_literal_prefix = binding_object_literal_value.and_then(|_| {
                // A typed identifier can contain a nested object type
                // (`const value: { key: string } = { key: ... }`).
                // Walking that annotation as a binding pattern would
                // collect `key` as an additional variable and lose the
                // receiver prefix, collapsing unrelated object members.
                let names = if name_node.kind() == "identifier" {
                    vec![(node_text(self.source, name_node), name_node)]
                } else {
                    let mut names = Vec::new();
                    collect_pattern_names(name_node, self.source, &mut names);
                    names
                };
                (names.len() == 1).then(|| {
                    if qualified_prefix.is_empty() {
                        names[0].0.clone()
                    } else {
                        format!("{qualified_prefix}.{}", names[0].0)
                    }
                })
            });
            if let Some(value) = node.child_by_field_name("value")
                && is_callable_node(value)
                && name_node.kind() == "identifier"
            {
                let name = node_text(self.source, name_node);
                if !name.is_empty() {
                    let declaration = self.add_declaration(
                        node,
                        name_node,
                        "function",
                        Namespace::Value,
                        &scope_id,
                        &qualified_prefix,
                    )?;
                    let this_receiver = self.prototype_assignment_receiver(value, &scope_id);
                    let child_scope = self.builder.open_scope(
                        "function",
                        Some(&declaration.id),
                        Some(&scope_id),
                        range_for_node(self.source_file, value),
                    )?;
                    self.scope_by_node.insert(value.id(), child_scope.clone());
                    self.scope_parents
                        .insert(child_scope.clone(), Some(scope_id.clone()));
                    self.scope_owners
                        .insert(child_scope.clone(), declaration.id);
                    self.scope_kinds
                        .insert(child_scope.clone(), "function".to_owned());
                    if let Some(receiver) = this_receiver {
                        self.this_receivers.insert(child_scope.clone(), receiver);
                    }
                    self.collect_unwrapped_callable_parameters(
                        value,
                        &child_scope,
                        &declaration.qualified_name,
                    )?;
                    let mut cursor = value.walk();
                    for child in value.named_children(&mut cursor) {
                        self.collect_declarations(
                            child,
                            child_scope.clone(),
                            declaration.qualified_name.clone(),
                            depth + 1,
                        )?;
                    }
                    let mut cursor = node.walk();
                    for child in node.named_children(&mut cursor) {
                        if child.id() != value.id() {
                            self.collect_declarations(
                                child,
                                scope_id.clone(),
                                qualified_prefix.clone(),
                                depth + 1,
                            )?;
                        }
                    }
                    return Ok(());
                }
            }
            let mut names = Vec::new();
            collect_pattern_names(name_node, self.source, &mut names);
            for (name, name_node) in names {
                if name.is_empty() {
                    continue;
                }
                let declaration = self.add_declaration_at(
                    node,
                    name_node,
                    "variable",
                    Namespace::Value,
                    &self.binding_scope_for(node, &scope_id),
                    &qualified_prefix,
                    &name,
                )?;
                if variable_binding_is_immutable(node, self.source) {
                    self.immutable_bindings.insert(declaration.id);
                }
            }
            // Prototype aliases must be available while the declaration pass
            // visits following assignments (`const proto = Ctor.prototype;
            // proto.run = ...`). The helper remains source/namespace bounded;
            // the later inference pass repeats it for declarations discovered
            // after their constructor.
            self.infer_variable_prototype_source(node, &scope_id);
            if let Some(prefix) = object_literal_prefix.as_deref()
                && let Some(value) = binding_object_literal_value
            {
                self.structural_object_variables.insert(prefix.to_owned());
                if !object_literal_has_spread(value) {
                    self.stable_structural_object_variables
                        .insert(prefix.to_owned());
                }
                object_literal_value = Some(value);
                let mut cursor = value.walk();
                for child in value.named_children(&mut cursor) {
                    self.collect_declarations(
                        child,
                        scope_id.clone(),
                        prefix.to_owned(),
                        depth + 1,
                    )?;
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if object_literal_value.is_some_and(|object| node_is_descendant_or_same(object, child))
            {
                continue;
            }
            self.collect_declarations(
                child,
                scope_id.clone(),
                qualified_prefix.clone(),
                depth + 1,
            )?;
        }
        Ok(())
    }

    fn collect_unwrapped_callable_parameters(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
        qualified_prefix: &str,
    ) -> Result<(), EvidenceError> {
        let mut add_parameter = |parameter: Node<'tree>| -> Result<(), EvidenceError> {
            let mut names = Vec::new();
            collect_pattern_names(parameter, self.source, &mut names);
            for (name, name_node) in names {
                if !name.is_empty() {
                    self.add_declaration_at(
                        parameter,
                        name_node,
                        "parameter",
                        Namespace::Value,
                        scope_id,
                        qualified_prefix,
                        &name,
                    )?;
                }
            }
            Ok(())
        };
        if let Some(parameters) = node
            .child_by_field_name("parameters")
            .or_else(|| first_named_child_kind(node, "formal_parameters"))
        {
            let mut cursor = parameters.walk();
            for parameter in parameters.named_children(&mut cursor) {
                if is_parameter_node(parameter) || parameter.kind().contains("comment") {
                    continue;
                }
                add_parameter(parameter)?;
            }
        } else if let Some(parameter) = node.child_by_field_name("parameter") {
            // The TypeScript grammar uses a singular `parameter` field for
            // the unparenthesized arrow form (`value => value`).  It is not
            // nested in `formal_parameters`, so the normal recursive
            // parameter-wrapper path cannot see it as a declaration.
            add_parameter(parameter)?;
        }
        Ok(())
    }

    fn add_declaration(
        &mut self,
        node: Node<'tree>,
        name_node: Node<'tree>,
        kind: &str,
        namespace: Namespace,
        scope_id: &str,
        qualified_prefix: &str,
    ) -> Result<DeclarationInfo, EvidenceError> {
        let name = declaration_name_text(self.source, name_node);
        self.add_declaration_at(
            node,
            name_node,
            kind,
            namespace,
            scope_id,
            qualified_prefix,
            &name,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_declaration_at(
        &mut self,
        node: Node<'tree>,
        name_node: Node<'tree>,
        kind: &str,
        namespace: Namespace,
        scope_id: &str,
        qualified_prefix: &str,
        name: &str,
    ) -> Result<DeclarationInfo, EvidenceError> {
        self.add_declaration_at_with_range(
            node,
            name_node,
            name_node,
            kind,
            namespace,
            scope_id,
            qualified_prefix,
            name,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_declaration_at_with_range(
        &mut self,
        node: Node<'tree>,
        name_node: Node<'tree>,
        range_node: Node<'tree>,
        kind: &str,
        namespace: Namespace,
        scope_id: &str,
        qualified_prefix: &str,
        name: &str,
    ) -> Result<DeclarationInfo, EvidenceError> {
        let qualified_name = if qualified_prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{qualified_prefix}.{name}")
        };
        let graph_node_id = stable_graph_id(self.source_file, kind, name, node.start_byte());
        let signature = typescript_declaration_signature(node, kind, self.source)
            .map(|signature| self.canonicalize_declaration_signature(scope_id, kind, &signature));
        let declaration_id = self.builder.declare_with_signature(
            kind,
            &graph_node_id,
            name,
            &qualified_name,
            Some(self.source_file),
            Some(scope_id),
            Some(symbol_namespace(namespace)),
            signature.as_deref(),
            range_for_node(self.source_file, range_node),
        )?;
        self.declaration_name_nodes.insert(name_node.start_byte());
        let callable_value = callable_value_node(node);
        let parameter_arity = callable_parameter_arity(node, self.source).or_else(|| {
            callable_value.and_then(|value| callable_parameter_arity(value, self.source))
        });
        let generic_parameters = callable_type_parameters(node, self.source)
            .or_else(|| {
                callable_value.and_then(|value| callable_type_parameters(value, self.source))
            })
            .unwrap_or_default();
        let generic_parameter_order = callable_type_parameter_order(node, self.source)
            .or_else(|| {
                callable_value.and_then(|value| callable_type_parameter_order(value, self.source))
            })
            .unwrap_or_default();
        let declaration = DeclarationInfo {
            id: declaration_id.clone(),
            name: name.to_owned(),
            kind: kind.to_owned(),
            qualified_name,
            scope_id: scope_id.to_owned(),
            range_start_byte: range_node.start_byte(),
            namespace,
            parameter_count: parameter_arity
                .and_then(|(minimum, maximum)| (Some(minimum) == maximum).then_some(minimum)),
            parameter_types: callable_parameter_types(node, self.source).or_else(|| {
                callable_value.and_then(|value| callable_parameter_types(value, self.source))
            }),
            parameter_min_count: parameter_arity.map(|(minimum, _)| minimum),
            parameter_max_count: parameter_arity.and_then(|(_, maximum)| maximum),
            return_type_name: callable_return_type_name(node, self.source)
                .or_else(|| {
                    callable_value.and_then(|value| callable_return_type_name(value, self.source))
                })
                .or_else(|| callable_return_constructor_name(node, self.source))
                .or_else(|| {
                    matches!(kind, "method" | "constructor")
                        .then(|| callable_returns_this(node).then_some("this".to_owned()))
                        .flatten()
                }),
            declared_type_name: direct_type_reference_name(node, self.source),
            callable_shape: declaration_is_callable_shape(node, kind, self.source),
            explicit_constructor: kind == "class"
                && class_has_explicit_constructor(node, self.source),
        };
        self.declarations
            .insert(declaration_id, declaration.clone());
        if kind == "property"
            && let Some(value) = node.child_by_field_name("value")
        {
            let value = unwrap_expression_node(value);
            if matches!(value.kind(), "identifier" | "type_identifier") {
                let target = node_text(self.source, value);
                if !target.is_empty() && target.len() <= MAX_TYPE_SHAPE_BYTES {
                    self.property_alias_values
                        .insert(declaration.id.clone(), target);
                }
            }
        }
        if kind == "variable"
            && let Some(value) = node.child_by_field_name("value")
        {
            let value = unwrap_expression_node(value);
            if matches!(
                value.kind(),
                "identifier"
                    | "type_identifier"
                    | "member_expression"
                    | "optional_member_expression"
                    | "subscript_expression"
            ) {
                let target = node_text(self.source, value);
                if !target.is_empty() && target.len() <= MAX_TYPE_SHAPE_BYTES {
                    self.variable_alias_values
                        .insert(declaration.id.clone(), target);
                }
            }
        }
        if kind == "property"
            && let Some(value) = property_literal_value(node, self.source)
        {
            self.property_literal_values
                .insert(declaration.qualified_name.clone(), value);
        }
        if kind == "type_alias"
            && let Some(targets) = type_alias_union_targets(node, self.source)
        {
            self.type_alias_union_targets
                .insert(declaration.qualified_name.clone(), targets);
        }
        if matches!(
            kind,
            "type_alias" | "interface" | "parameter" | "variable" | "property"
        ) && let Some(value_type) = index_value_type(node, self.source)
        {
            self.index_value_types
                .insert(declaration.qualified_name.clone(), value_type);
        }
        if !generic_parameters.is_empty() {
            self.generic_parameters_by_declaration
                .insert(declaration.id.clone(), generic_parameters);
        }
        if !generic_parameter_order.is_empty() {
            self.generic_parameter_order_by_declaration
                .insert(declaration.id.clone(), generic_parameter_order);
        }
        self.declarations_by_scope
            .entry(scope_id.to_owned())
            .or_default()
            .push(declaration.id.clone());
        self.declarations_by_qualified
            .entry(declaration.qualified_name.clone())
            .or_default()
            .push(declaration.id.clone());
        Ok(declaration)
    }

    /// Resolve property aliases after the declaration table is complete so a
    /// static/member field initialized from a local function can be treated
    /// as callable at its exact member site. Imported or ambiguous aliases
    /// remain unresolved rather than inheriting an unknown call signature.
    fn resolve_callable_property_aliases(&mut self) {
        let aliases = self
            .property_alias_values
            .iter()
            .map(|(declaration_id, target)| (declaration_id.clone(), target.clone()))
            .collect::<Vec<_>>();
        let variable_aliases = self
            .variable_alias_values
            .iter()
            .map(|(declaration_id, target)| (declaration_id.clone(), target.clone()))
            .collect::<Vec<_>>();
        // A short fixed point handles a bounded alias chain (`a -> b -> fn`)
        // without turning callback classification into whole-program data
        // flow. The same budget bounds both source syntax and work here.
        for _ in 0..MAX_INLINE_OBJECT_PROPERTIES {
            let mut changed = false;
            for (declaration_id, target) in aliases.iter().chain(variable_aliases.iter()) {
                let Some(declaration) = self.declarations.get(declaration_id) else {
                    continue;
                };
                let callable = self.callable_alias_target(&declaration.scope_id, target);
                let destination = if declaration.kind == "property" {
                    &mut self.callable_property_aliases
                } else {
                    &mut self.callable_variable_aliases
                };
                if callable && destination.insert(declaration_id.clone()) {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn callable_alias_target(&self, scope_id: &str, target: &str) -> bool {
        if let Some((base, property)) = target.rsplit_once('.') {
            let Some(receiver) = self.resolve_name(scope_id, base, Namespace::Value) else {
                return false;
            };
            let qualified_name = match receiver {
                Resolution::Local(declaration) => {
                    format!("{}.{}", declaration.qualified_name, property)
                }
                // An imported member may be callable, but this per-file
                // candidate has no source declaration proof for its shape.
                // The project resolver may still resolve an explicit call;
                // callback references stay conservative here.
                Resolution::Import(_) => return false,
            };
            let Some(ids) = self.declarations_by_qualified.get(&qualified_name) else {
                return false;
            };
            return ids.len() == 1 && ids.iter().all(|id| self.proven_callable_declaration(id));
        }
        match self.resolve_name(scope_id, target, Namespace::Value) {
            Some(Resolution::Local(declaration)) => {
                self.proven_callable_declaration(&declaration.id)
            }
            // Overloaded local functions have no unique `Resolution`, but
            // every visible declaration can still prove that the alias is a
            // callable value. The alias itself remains a unique local
            // binding, so its reference target is not ambiguous.
            None => self.visible_callable_alias_target(scope_id, target),
            Some(Resolution::Import(_)) => false,
        }
    }

    fn proven_callable_declaration(&self, declaration_id: &str) -> bool {
        let Some(declaration) = self.declarations.get(declaration_id) else {
            return false;
        };
        declaration.callable_shape
            || self.callable_property_aliases.contains(declaration_id)
            || self.callable_variable_aliases.contains(declaration_id)
            || matches!(
                declaration.kind.as_str(),
                "class" | "constructor" | "function" | "method"
            )
    }

    fn visible_callable_alias_target(&self, scope_id: &str, name: &str) -> bool {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            let candidates = self
                .declarations_by_scope
                .get(&scope)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|id| self.declarations.get(id))
                .filter(|declaration| {
                    declaration.name == name
                        && declaration.namespace.accepts(Namespace::Value)
                        && self.lexically_visible_unqualified(&scope, declaration)
                })
                .collect::<Vec<_>>();
            if !candidates.is_empty() {
                return candidates
                    .iter()
                    .all(|candidate| self.proven_callable_declaration(&candidate.id));
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        false
    }

    fn precollect_imports(&mut self, node: Node<'tree>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return Ok(());
        }
        let scope_id = self.scope_for_node(node);
        match node.kind() {
            "import_statement" => self.emit_import(node, &scope_id, false)?,
            "export_statement" if first_named_child_kind(node, "string").is_some() => {
                self.emit_import(node, &scope_id, true)?;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.precollect_imports(child, depth + 1)?;
        }
        Ok(())
    }

    fn precollect_base_targets(&mut self, node: Node<'tree>, depth: usize) {
        if depth > MAX_TRAVERSAL_DEPTH {
            return;
        }
        if matches!(
            node.kind(),
            "class_declaration"
                | "abstract_class_declaration"
                | "class"
                | "interface_declaration"
                | "interface"
        ) {
            let scope_id = self.scope_for_node(node);
            if let Some(class_name) = self.enclosing_type(&scope_id) {
                let mut clauses = Vec::new();
                collect_nodes_of_kind(node, "extends_clause", &mut clauses);
                collect_nodes_of_kind(node, "extends_type_clause", &mut clauses);
                if let Some(clause) = clauses.into_iter().next() {
                    let mut names = Vec::new();
                    collect_type_name_nodes(clause, &mut names);
                    if let Some(base) = names.first()
                        && let Some(resolution) = self.resolve_type_name_node(&scope_id, *base)
                    {
                        let clause_text = node_text(self.source, clause);
                        let clause_base_text = clause_text
                            .trim()
                            .strip_prefix("extends")
                            .unwrap_or(clause_text.trim());
                        let base_declaration_id = match &resolution {
                            Resolution::Local(declaration) => Some(declaration.id.clone()),
                            Resolution::Import(_) => None,
                        };
                        let receiver = match resolution {
                            Resolution::Local(declaration)
                                if matches!(
                                    declaration.kind.as_str(),
                                    "class" | "interface" | "enum" | "namespace"
                                ) =>
                            {
                                Some(ReceiverTarget {
                                    qualified_name: declaration.qualified_name.clone(),
                                    import: None,
                                    scope_id: self.member_scope_for_declaration(&declaration),
                                    type_arguments: None,
                                })
                            }
                            Resolution::Import(import) => Some(ReceiverTarget {
                                qualified_name: import_target_without_namespace(&import.target),
                                import: Some(import),
                                scope_id: None,
                                type_arguments: None,
                            }),
                            _ => None,
                        };
                        if let Some(receiver) = receiver {
                            self.base_targets.insert(class_name.clone(), receiver);
                            if let Some(base_declaration_id) = base_declaration_id
                                && let Some((_, arguments)) = generic_type_parts(clause_base_text)
                                && let Some(order) = self
                                    .generic_parameter_order_by_declaration
                                    .get(&base_declaration_id)
                            {
                                let bindings = order
                                    .iter()
                                    .zip(arguments)
                                    .map(|(name, argument)| {
                                        (name.clone(), strip_type_arguments(&argument).to_owned())
                                    })
                                    .collect::<HashMap<_, _>>();
                                if !bindings.is_empty() {
                                    self.base_type_bindings.insert(class_name.clone(), bindings);
                                }
                            }
                        }
                    }
                }
                let mut implements = Vec::new();
                collect_nodes_of_kind(node, "implements_clause", &mut implements);
                for clause in implements.into_iter().take(MAX_INLINE_OBJECT_PROPERTIES) {
                    let mut names = Vec::new();
                    collect_type_name_nodes(clause, &mut names);
                    let Some(base) = names.first() else {
                        continue;
                    };
                    let Some(resolution) = self.resolve_type_name_node(&scope_id, *base) else {
                        continue;
                    };
                    let receiver = match resolution {
                        Resolution::Local(declaration)
                            if matches!(
                                declaration.kind.as_str(),
                                "class" | "interface" | "enum" | "namespace"
                            ) =>
                        {
                            Some(ReceiverTarget {
                                qualified_name: declaration.qualified_name.clone(),
                                import: None,
                                scope_id: self.member_scope_for_declaration(&declaration),
                                type_arguments: None,
                            })
                        }
                        Resolution::Import(import) => Some(ReceiverTarget {
                            qualified_name: import_target_without_namespace(&import.target),
                            import: Some(import),
                            scope_id: None,
                            type_arguments: None,
                        }),
                        _ => None,
                    };
                    if let Some(receiver) = receiver {
                        self.implements_targets
                            .entry(class_name.clone())
                            .or_default()
                            .push(receiver);
                    }
                }
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.precollect_base_targets(child, depth + 1);
        }
    }

    fn infer_variable_types(&mut self, node: Node<'tree>, depth: usize) {
        let mut pending = vec![(node, depth)];
        while let Some((current, current_depth)) = pending.pop() {
            if current_depth > MAX_TRAVERSAL_DEPTH {
                continue;
            }
            if is_parameter_node(current) {
                let scope_id = self.scope_for_node(current);
                self.infer_parameter_nominal_type(current, &scope_id);
                self.infer_inline_object_type_receiver(current, &scope_id);
            }
            if current.kind() == "variable_declarator"
                && let Some(name_node) = current.child_by_field_name("name")
            {
                let scope_id = self.scope_for_node(current);
                self.infer_variable_object_sources(current, &scope_id);
                self.infer_variable_prototype_source(current, &scope_id);
                self.infer_inline_object_type_receiver(current, &scope_id);
                self.infer_destructured_variable_types(current, &scope_id);
                if let Some(qualified_type) = self.nominal_type_for_variable(current, &scope_id) {
                    let mut names = Vec::new();
                    collect_pattern_names(name_node, self.source, &mut names);
                    for (name, _) in names {
                        if let Some(Resolution::Local(declaration)) =
                            self.resolve_name(&scope_id, &name, Namespace::Value)
                            && declaration.kind == "variable"
                        {
                            self.variable_types
                                .insert(declaration.id, qualified_type.clone());
                        }
                    }
                }
            }
            if matches!(
                current.kind(),
                "call_expression" | "optional_call_expression"
            ) {
                let scope_id = self.scope_for_node(current);
                self.infer_contextual_callable_parameters(current, &scope_id);
            }
            let mut children = Vec::new();
            let mut cursor = current.walk();
            children.extend(current.named_children(&mut cursor));
            for child in children.into_iter().rev() {
                pending.push((child, current_depth.saturating_add(1)));
            }
        }
    }

    /// Collect source-order receiver facts for the small flow slice that is
    /// safe without a control-flow graph: a local variable is assigned a
    /// source-proven constructor/call result in its own binding scope, and
    /// the use occurs after that assignment.  Any branch, compound write, or
    /// unsupported write becomes a barrier so an older fact cannot leak past
    /// an unknown mutation.  This keeps JavaScript reassignment useful while
    /// failing closed on aliasing and control-flow shapes the native
    /// candidate does not model.
    fn collect_flow_assignment_facts(&mut self, node: Node<'tree>, depth: usize) {
        let mut pending = vec![(node, depth)];
        while let Some((current, current_depth)) = pending.pop() {
            if current_depth > MAX_TRAVERSAL_DEPTH {
                continue;
            }
            let scope_id = self.scope_for_node(current);
            if matches!(
                current.kind(),
                "call_expression" | "optional_call_expression" | "new_expression"
            ) {
                self.record_flow_call_argument_escapes(current, &scope_id);
            }
            if current.kind() == "with_statement" {
                self.record_flow_scope_barriers(
                    &scope_id,
                    current.start_byte(),
                    "dynamic with scope",
                );
            }
            if is_callable_node(current) {
                self.record_flow_closure_captures(current, &scope_id);
            }
            if current.kind() == "variable_declarator"
                && let Some(name_node) = current.child_by_field_name("name")
                && let Some(value) = current.child_by_field_name("value")
            {
                let value = unwrap_expression_node(value);
                if name_node.kind() == "identifier"
                    && let Some(receiver) = self.flow_receiver_for_value(&scope_id, value)
                {
                    let mut names = Vec::new();
                    collect_pattern_names(name_node, self.source, &mut names);
                    for (name, _) in names {
                        let Some(Resolution::Local(variable)) =
                            self.resolve_name(&scope_id, &name, Namespace::Value)
                        else {
                            continue;
                        };
                        if variable.kind == "variable"
                            && self.flow_scope_is_compatible(&variable.scope_id, &scope_id)
                            && self.flow_assignment_is_straight_line(name_node, &scope_id)
                        {
                            self.record_flow_assignment(
                                &variable.id,
                                value.start_byte(),
                                receiver.clone(),
                            );
                        }
                    }
                }
            }
            if matches!(
                current.kind(),
                "assignment_expression" | "augmented_assignment_expression"
            ) && let Some(left) = current.child_by_field_name("left")
            {
                if left.kind() == "identifier" {
                    let name = node_text(self.source, left);
                    if let Some(Resolution::Local(variable)) =
                        self.resolve_name(&scope_id, &name, Namespace::Value)
                        && variable.kind == "variable"
                    {
                        let right = current.child_by_field_name("right");
                        let operator = right
                            .and_then(|right| {
                                self.source
                                    .get(left.end_byte()..right.start_byte())
                                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                            })
                            .map(str::trim)
                            .unwrap_or_default();
                        let straight_line = self
                            .flow_scope_is_compatible(&variable.scope_id, &scope_id)
                            && self.flow_assignment_is_straight_line(left, &scope_id);
                        let receiver = right
                            .map(unwrap_expression_node)
                            .and_then(|right| self.flow_receiver_for_value(&scope_id, right));
                        if straight_line && operator == "=" {
                            if let Some(receiver) = receiver {
                                self.record_flow_assignment(
                                    &variable.id,
                                    current.start_byte(),
                                    receiver,
                                );
                            } else {
                                self.record_flow_assignment_barrier(
                                    &variable.id,
                                    current.start_byte(),
                                    "unsupported local assignment value",
                                );
                            }
                        } else {
                            self.record_flow_assignment_barrier(
                                &variable.id,
                                current.start_byte(),
                                if straight_line {
                                    "compound local assignment"
                                } else {
                                    "conditional or out-of-scope local assignment"
                                },
                            );
                        }
                    }
                }
                if matches!(
                    left.kind(),
                    "member_expression" | "optional_member_expression" | "subscript_expression"
                ) && let Some(object) = left.child_by_field_name("object")
                {
                    self.record_flow_member_write(
                        &scope_id,
                        object,
                        current.start_byte(),
                        member_property_node(left)
                            .and_then(|property| member_property_name(self.source, property))
                            .as_deref(),
                    );
                }
            }
            if current.kind() == "return_statement"
                && let Some(value) = current
                    .child_by_field_name("argument")
                    .or_else(|| current.child_by_field_name("value"))
            {
                self.record_flow_value_escape(
                    &scope_id,
                    unwrap_expression_node(value),
                    current.start_byte(),
                    "returned local alias",
                );
            }
            let mut cursor = current.walk();
            let children = current.named_children(&mut cursor).collect::<Vec<_>>();
            for child in children.into_iter().rev() {
                pending.push((child, current_depth.saturating_add(1)));
            }
        }
    }

    fn flow_receiver_for_value(
        &mut self,
        scope_id: &str,
        value: Node<'tree>,
    ) -> Option<ReceiverTarget> {
        if matches!(
            value.kind(),
            "as_expression"
                | "satisfies_expression"
                | "parenthesized_expression"
                | "type_assertion"
                | "non_null_expression"
        ) {
            return self.receiver_target(scope_id, value);
        }
        match value.kind() {
            "assignment_expression" => {
                let left = value.child_by_field_name("left");
                let right = value
                    .child_by_field_name("right")
                    .map(unwrap_expression_node);
                let is_plain_assignment = left.zip(right).is_some_and(|(left, right)| {
                    self.source
                        .get(left.end_byte()..right.start_byte())
                        .and_then(|bytes| std::str::from_utf8(bytes).ok())
                        .is_some_and(|operator| operator.trim() == "=")
                });
                if is_plain_assignment
                    && self
                        .object_literal_value_for_binding(value)
                        .is_some_and(|object| !object_literal_has_spread(object))
                    && let Some(receiver) = self.flow_inline_object_receiver(scope_id, value)
                {
                    return Some(receiver);
                }
                right.and_then(|right| self.flow_receiver_for_value(scope_id, right))
            }
            "new_expression" => self.receiver_target(scope_id, value),
            "call_expression" | "optional_call_expression" => {
                self.call_return_receiver(scope_id, value)
            }
            "identifier" | "type_identifier" | "this" | "super" => {
                self.receiver_target(scope_id, value)
            }
            _ => None,
        }
    }

    fn object_literal_value_for_binding(&self, value: Node<'tree>) -> Option<Node<'tree>> {
        let mut value = unwrap_expression_node(value);
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            if value.kind() == "object" {
                return Some(value);
            }
            if value.kind() != "assignment_expression" {
                return None;
            }
            let left = value.child_by_field_name("left")?;
            let right = value
                .child_by_field_name("right")
                .map(unwrap_expression_node)?;
            let operator = self
                .source
                .get(left.end_byte()..right.start_byte())
                .and_then(|bytes| std::str::from_utf8(bytes).ok())?;
            if operator.trim() != "=" {
                return None;
            }
            value = right;
        }
        None
    }

    fn assignment_chain_contains(&self, root: Node<'tree>, target_id: usize) -> bool {
        let mut current = unwrap_expression_node(root);
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            if current.id() == target_id {
                return true;
            }
            if current.kind() != "assignment_expression" {
                return false;
            }
            let left = current.child_by_field_name("left");
            let right = current
                .child_by_field_name("right")
                .map(unwrap_expression_node);
            let Some((left, right)) = left.zip(right) else {
                return false;
            };
            let Some(operator) = self
                .source
                .get(left.end_byte()..right.start_byte())
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
            else {
                return false;
            };
            if operator.trim() != "=" {
                return false;
            }
            current = right;
        }
        false
    }

    fn flow_inline_object_receiver(
        &self,
        scope_id: &str,
        assignment: Node<'tree>,
    ) -> Option<ReceiverTarget> {
        let mut current = assignment.parent();
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            let Some(ancestor) = current else {
                break;
            };
            if ancestor.kind() == "variable_declarator"
                && ancestor
                    .child_by_field_name("value")
                    .map(unwrap_expression_node)
                    .is_some_and(|value| {
                        self.object_literal_value_for_binding(value)
                            .is_some_and(|object| !object_literal_has_spread(object))
                            && self.assignment_chain_contains(value, assignment.id())
                    })
                && let Some(name) = ancestor.child_by_field_name("name")
                && name.kind() == "identifier"
            {
                let name = node_text(self.source, name);
                let Resolution::Local(variable) =
                    self.resolve_name(scope_id, &name, Namespace::Value)?
                else {
                    return None;
                };
                if self
                    .structural_object_variables
                    .contains(&variable.qualified_name)
                {
                    return Some(ReceiverTarget {
                        qualified_name: variable.qualified_name,
                        import: None,
                        scope_id: Some(variable.scope_id),
                        type_arguments: None,
                    });
                }
                return None;
            }
            current = ancestor.parent();
        }
        None
    }

    fn record_flow_call_argument_escapes(&mut self, node: Node<'tree>, scope_id: &str) {
        let Some(arguments) = node
            .child_by_field_name("arguments")
            .or_else(|| first_named_child_kind(node, "arguments"))
        else {
            return;
        };
        let mut cursor = arguments.walk();
        for argument in arguments
            .named_children(&mut cursor)
            .filter(|argument| !argument.kind().contains("comment"))
            .take(MAX_INLINE_OBJECT_PROPERTIES)
        {
            self.record_flow_value_escape(
                scope_id,
                unwrap_expression_node(argument),
                node.start_byte(),
                "passed to callable argument",
            );
        }
        let function = node
            .child_by_field_name("function")
            .or_else(|| first_named_child(node));
        let is_eval = function
            .and_then(|function| rightmost_identifier(function))
            .is_some_and(|identifier| {
                node_text(self.source, identifier) == "eval"
                    && self
                        .resolve_name(scope_id, "eval", Namespace::Value)
                        .is_none()
            });
        if is_eval {
            self.record_flow_scope_barriers(scope_id, node.start_byte(), "dynamic eval");
        }
        let is_proxy = function
            .and_then(|function| rightmost_identifier(function))
            .is_some_and(|identifier| node_text(self.source, identifier) == "Proxy");
        if is_proxy {
            self.record_flow_scope_barriers(scope_id, node.start_byte(), "unsupported Proxy");
        }
    }

    fn record_flow_value_escape(
        &mut self,
        scope_id: &str,
        value: Node<'tree>,
        start_byte: usize,
        reason: &str,
    ) {
        let value = unwrap_expression_node(value);
        if !matches!(value.kind(), "identifier" | "type_identifier") {
            return;
        }
        let name = node_text(self.source, value);
        let Some(Resolution::Local(declaration)) =
            self.resolve_name(scope_id, &name, Namespace::Value)
        else {
            return;
        };
        if matches!(declaration.kind.as_str(), "variable" | "parameter") {
            self.record_flow_assignment_barrier(&declaration.id, start_byte, reason);
        }
    }

    fn record_flow_member_write(
        &mut self,
        scope_id: &str,
        object: Node<'tree>,
        start_byte: usize,
        property_name: Option<&str>,
    ) {
        let value = unwrap_expression_node(object);
        let Some(property_name) = property_name.filter(|name| !name.is_empty()) else {
            self.record_flow_value_escape(scope_id, value, start_byte, "dynamic member write");
            return;
        };
        let Some(name_node) = rightmost_identifier(value) else {
            self.record_flow_value_escape(scope_id, value, start_byte, "dynamic member write");
            return;
        };
        let name = node_text(self.source, name_node);
        let Some(Resolution::Local(declaration)) =
            self.resolve_name(scope_id, &name, Namespace::Value)
        else {
            self.record_flow_value_escape(scope_id, value, start_byte, "dynamic member write");
            return;
        };
        if !matches!(declaration.kind.as_str(), "variable" | "parameter") {
            self.record_flow_value_escape(scope_id, value, start_byte, "dynamic member write");
            return;
        }
        if self.stable_immutable_receiver_alias(&declaration.id) {
            self.record_flow_member_write_barrier(&declaration.id, property_name, start_byte);
        } else {
            self.record_flow_escape_barrier(&declaration.id, start_byte);
        }
    }

    fn record_flow_member_write_barrier(
        &mut self,
        variable_id: &str,
        property_name: &str,
        start_byte: usize,
    ) {
        let barriers = self
            .flow_member_write_barriers
            .entry((variable_id.to_owned(), property_name.to_owned()))
            .or_default();
        if barriers.len() < MAX_INLINE_OBJECT_PROPERTIES {
            barriers.push(start_byte);
        } else if let Some(latest) = barriers.last_mut()
            && *latest < start_byte
        {
            *latest = start_byte;
        }
    }

    fn flow_member_write_barrier_before(
        &self,
        scope_id: &str,
        object: Node<'tree>,
        property_name: &str,
        use_start_byte: usize,
    ) -> bool {
        let Some(name_node) = rightmost_identifier(object) else {
            return false;
        };
        let name = node_text(self.source, name_node);
        let Some(Resolution::Local(declaration)) =
            self.resolve_name(scope_id, &name, Namespace::Value)
        else {
            return false;
        };
        self.flow_member_write_barriers
            .get(&(declaration.id, property_name.to_owned()))
            .is_some_and(|barriers| barriers.iter().any(|start| *start <= use_start_byte))
    }

    fn record_flow_scope_barriers(&mut self, scope_id: &str, start_byte: usize, _reason: &str) {
        let mut visible = self
            .declarations
            .values()
            .filter(|declaration| matches!(declaration.kind.as_str(), "variable" | "parameter"))
            .filter(|declaration| self.scope_is_descendant_or_same(scope_id, &declaration.scope_id))
            .map(|declaration| (declaration.id.clone(), declaration.name.clone()))
            .collect::<Vec<_>>();
        visible.sort_unstable();
        visible.truncate(MAX_INLINE_OBJECT_PROPERTIES);
        for (declaration_id, name) in visible {
            if self
                .resolve_name(scope_id, &name, Namespace::Value)
                .is_some_and(|resolution| {
                    matches!(resolution, Resolution::Local(declaration) if declaration.id == declaration_id)
                })
            {
                self.record_flow_escape_barrier(&declaration_id, start_byte);
            }
        }
    }

    fn record_flow_closure_captures(&mut self, node: Node<'tree>, scope_id: &str) {
        let Some(body) = node
            .child_by_field_name("body")
            .or_else(|| first_named_child_kind(node, "statement_block"))
        else {
            return;
        };
        let mut pending = vec![(body, 0_usize)];
        let mut captured = BTreeSet::new();
        while let Some((current, depth)) = pending.pop() {
            if depth > MAX_TRAVERSAL_DEPTH || captured.len() >= MAX_INLINE_OBJECT_PROPERTIES {
                break;
            }
            if current.kind() == "identifier" {
                let name = node_text(self.source, current);
                if let Some(Resolution::Local(declaration)) =
                    self.resolve_name(scope_id, &name, Namespace::Value)
                    && matches!(declaration.kind.as_str(), "variable" | "parameter")
                    && declaration.scope_id != self.scope_for_node(node)
                    && self.scope_is_descendant_or_same(
                        &self.scope_for_node(node),
                        &declaration.scope_id,
                    )
                    && !self.stable_immutable_receiver_alias(&declaration.id)
                {
                    captured.insert(declaration.id.clone());
                }
            }
            let mut cursor = current.walk();
            for child in current
                .named_children(&mut cursor)
                .take(MAX_INLINE_OBJECT_PROPERTIES)
            {
                pending.push((child, depth.saturating_add(1)));
            }
        }
        for declaration_id in captured {
            self.record_flow_escape_barrier(&declaration_id, node.start_byte());
        }
    }

    /// A closure capture is an escape barrier for mutable or structural
    /// bindings because a later write can change the receiver observed by the
    /// closure. A `const` alias to one source-proven nominal receiver (most
    /// notably `const token = this`) cannot be rebound, so retaining that
    /// identity improves common JavaScript class patterns without selecting a
    /// structural object by spelling alone.
    fn stable_immutable_nominal_alias(&self, declaration_id: &str) -> bool {
        if !self.immutable_bindings.contains(declaration_id) {
            return false;
        }
        let Some(assignments) = self.flow_assignments.get(declaration_id) else {
            return false;
        };
        let Some(assignment) = assignments
            .iter()
            .min_by_key(|assignment| assignment.start_byte)
        else {
            return false;
        };
        if self
            .structural_object_variables
            .contains(&assignment.receiver.qualified_name)
        {
            return false;
        }
        assignment.receiver.import.is_some() || !assignment.receiver.qualified_name.is_empty()
    }

    fn stable_immutable_structural_alias(&self, declaration_id: &str) -> bool {
        if !self.immutable_bindings.contains(declaration_id) {
            return false;
        }
        self.declarations
            .get(declaration_id)
            .is_some_and(|declaration| {
                self.stable_structural_object_variables
                    .contains(&declaration.qualified_name)
            })
    }

    fn stable_immutable_receiver_alias(&self, declaration_id: &str) -> bool {
        self.stable_immutable_nominal_alias(declaration_id)
            || self.stable_immutable_structural_alias(declaration_id)
    }

    /// Preserve a nominal receiver after an unknown call/escape when the
    /// binding itself is immutable and its source assignment is still the
    /// unique proven value. An escape may mutate a property, but it cannot
    /// rebind a `const` alias to a different receiver. Structural aliases do
    /// not use this recovery because their property identity depends on the
    /// exact object value that may have escaped.
    fn stable_nominal_flow_receiver_at(
        &self,
        declaration_id: &str,
        use_start_byte: usize,
    ) -> Option<ReceiverTarget> {
        if !self.immutable_bindings.contains(declaration_id) {
            return None;
        }
        let assignment = self
            .flow_assignments
            .get(declaration_id)
            .into_iter()
            .flat_map(|assignments| assignments.iter())
            .filter(|assignment| assignment.start_byte <= use_start_byte)
            .max_by_key(|assignment| assignment.start_byte)?;
        if assignment.receiver.import.is_some()
            || assignment.receiver.qualified_name.is_empty()
            || self
                .structural_object_variables
                .contains(&assignment.receiver.qualified_name)
        {
            return None;
        }
        Some(assignment.receiver.clone())
    }

    fn flow_assignment_is_straight_line(&self, node: Node<'tree>, scope_id: &str) -> bool {
        let mut current = node.parent();
        while let Some(ancestor) = current {
            if matches!(
                ancestor.kind(),
                "if_statement"
                    | "switch_statement"
                    | "switch_case"
                    | "for_statement"
                    | "for_in_statement"
                    | "for_of_statement"
                    | "while_statement"
                    | "do_statement"
                    | "try_statement"
                    | "catch_clause"
                    | "finally_clause"
                    | "with_statement"
                    | "conditional_expression"
                    | "ternary_expression"
            ) {
                return false;
            }
            if is_callable_node(ancestor) {
                let function_scope = self.scope_for_node(ancestor);
                return self.scope_is_descendant_or_same(scope_id, &function_scope);
            }
            current = ancestor.parent();
        }
        true
    }

    fn flow_scope_is_compatible(&self, variable_scope: &str, assignment_scope: &str) -> bool {
        self.flow_context_scope(variable_scope) == self.flow_context_scope(assignment_scope)
            && self.scope_is_descendant_or_same(assignment_scope, variable_scope)
    }

    fn flow_context_scope(&self, scope_id: &str) -> String {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            if self
                .scope_kinds
                .get(&scope)
                .is_some_and(|kind| matches!(kind.as_str(), "function" | "module"))
            {
                return scope;
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        self.root_scope.clone()
    }

    fn record_flow_assignment_barrier(
        &mut self,
        variable_id: &str,
        start_byte: usize,
        reason: &str,
    ) {
        let barriers = self
            .flow_assignment_barriers
            .entry(variable_id.to_owned())
            .or_default();
        if barriers.len() < MAX_INLINE_OBJECT_PROPERTIES {
            barriers.push((start_byte, reason.to_owned()));
        } else if let Some((latest_start, latest_reason)) = barriers.last_mut()
            && *latest_start < start_byte
        {
            *latest_start = start_byte;
            *latest_reason = reason.to_owned();
        }
    }

    fn record_flow_escape_barrier(&mut self, variable_id: &str, start_byte: usize) {
        let barriers = self
            .flow_escape_barriers
            .entry(variable_id.to_owned())
            .or_default();
        if barriers.len() < MAX_INLINE_OBJECT_PROPERTIES {
            barriers.push(start_byte);
        } else if let Some(latest) = barriers.last_mut()
            && *latest < start_byte
        {
            *latest = start_byte;
        }
    }

    fn record_flow_assignment(
        &mut self,
        variable_id: &str,
        start_byte: usize,
        receiver: ReceiverTarget,
    ) {
        let assignments = self
            .flow_assignments
            .entry(variable_id.to_owned())
            .or_default();
        if assignments.len() < MAX_INLINE_OBJECT_PROPERTIES {
            assignments.push(FlowAssignment {
                start_byte,
                receiver,
            });
        } else {
            self.record_flow_assignment_barrier(variable_id, start_byte, "flow assignment limit");
        }
    }

    fn flow_receiver_at(&self, variable_id: &str, use_start_byte: usize) -> Option<ReceiverTarget> {
        if self
            .flow_escape_barriers
            .get(variable_id)
            .is_some_and(|barriers| barriers.iter().any(|start| *start <= use_start_byte))
        {
            return None;
        }
        let latest_assignment = self
            .flow_assignments
            .get(variable_id)
            .into_iter()
            .flat_map(|assignments| assignments.iter())
            .filter(|assignment| assignment.start_byte <= use_start_byte)
            .max_by_key(|assignment| assignment.start_byte);
        let latest_barrier = self
            .flow_assignment_barriers
            .get(variable_id)
            .into_iter()
            .flat_map(|barriers| barriers.iter())
            .filter(|(start_byte, _)| *start_byte <= use_start_byte)
            .max_by_key(|(start_byte, _)| *start_byte)
            .map(|(start_byte, _)| *start_byte);
        if latest_barrier.is_some_and(|barrier| {
            latest_assignment.is_none_or(|assignment| assignment.start_byte <= barrier)
        }) {
            return None;
        }
        latest_assignment.map(|assignment| assignment.receiver.clone())
    }

    fn flow_assignment_barrier_before(&self, variable_id: &str, use_start_byte: usize) -> bool {
        if self
            .flow_escape_barriers
            .get(variable_id)
            .is_some_and(|barriers| barriers.iter().any(|start| *start <= use_start_byte))
        {
            return true;
        }
        let latest_assignment = self
            .flow_assignments
            .get(variable_id)
            .into_iter()
            .flat_map(|assignments| assignments.iter())
            .filter(|assignment| assignment.start_byte <= use_start_byte)
            .map(|assignment| assignment.start_byte)
            .max();
        self.flow_assignment_barriers
            .get(variable_id)
            .into_iter()
            .flat_map(|barriers| barriers.iter())
            .filter(|(start_byte, _)| *start_byte <= use_start_byte)
            .map(|(start_byte, _)| *start_byte)
            .max()
            .is_some_and(|barrier| latest_assignment.is_none_or(|assignment| assignment <= barrier))
    }

    /// Propagate a source-visible callback parameter type through a call.
    /// TypeScript commonly supplies the parameter type contextually rather
    /// than spelling it on the callback itself (`api.use((value, ctx) => ...)`).
    /// This pass only follows a unique local callable declaration and direct
    /// indexed access into a local object/type alias; unresolved, imported,
    /// union, and structural cases remain unresolved.
    fn infer_contextual_callable_parameters(&mut self, node: Node<'tree>, scope_id: &str) {
        let Some(function) = node
            .child_by_field_name("function")
            .or_else(|| first_named_child_kind(node, "identifier"))
        else {
            return;
        };
        let resolution = if matches!(
            function.kind(),
            "member_expression" | "optional_member_expression" | "subscript_expression"
        ) {
            let Some(property) = member_property_node(function) else {
                return;
            };
            self.resolve_member_target(
                scope_id,
                function.child_by_field_name("object"),
                property,
                None,
                &[],
            )
        } else {
            let Some(target) = rightmost_identifier(function) else {
                return;
            };
            self.resolve_name(scope_id, &node_text(self.source, target), Namespace::Value)
        };
        let Some(Resolution::Local(target)) = resolution else {
            return;
        };
        let Some(parameter_types) = target.parameter_types.as_ref() else {
            return;
        };
        let Some(arguments) = node
            .child_by_field_name("arguments")
            .or_else(|| first_named_child_kind(node, "arguments"))
        else {
            return;
        };
        let mut cursor = arguments.walk();
        for (argument, parameter_type) in arguments
            .named_children(&mut cursor)
            .zip(parameter_types.iter())
        {
            let argument = unwrap_expression_node(argument);
            if !is_callable_node(argument) {
                continue;
            }
            let Some(callback_parameter_types) =
                self.contextual_callable_parameter_types(scope_id, parameter_type)
            else {
                continue;
            };
            let Some(parameters) = argument
                .child_by_field_name("parameters")
                .or_else(|| first_named_child_kind(argument, "formal_parameters"))
            else {
                continue;
            };
            let callback_scope = self.scope_for_node(argument);
            let mut parameter_cursor = parameters.walk();
            for (parameter, type_name) in parameters
                .named_children(&mut parameter_cursor)
                .zip(callback_parameter_types.iter())
            {
                let Some(pattern) = parameter_pattern_node(parameter) else {
                    continue;
                };
                let Some(ReceiverTarget {
                    qualified_name,
                    import: None,
                    ..
                }) = self.resolve_declared_type_receiver(&callback_scope, type_name)
                else {
                    continue;
                };
                let mut names = Vec::new();
                collect_pattern_names(pattern, self.source, &mut names);
                for (name, _) in names {
                    if let Some(Resolution::Local(callback_parameter)) =
                        self.resolve_name(&callback_scope, &name, Namespace::Value)
                        && callback_parameter.kind == "parameter"
                    {
                        self.variable_types
                            .insert(callback_parameter.id, qualified_name.clone());
                    }
                }
            }
        }
    }

    fn contextual_callable_parameter_types(
        &self,
        scope_id: &str,
        type_name: &str,
    ) -> Option<Vec<String>> {
        let (base, property) = indexed_type_parts(type_name)?;
        let base = strip_type_arguments(base);
        let Resolution::Local(alias) = self.resolve_name(scope_id, base, Namespace::Type)? else {
            return None;
        };
        let qualified_name = format!("{}.{}", alias.qualified_name, property);
        let ids = self.declarations_by_qualified.get(&qualified_name)?;
        let declarations = ids
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter_map(|declaration| declaration.parameter_types.clone())
            .collect::<Vec<_>>();
        let [parameters] = declarations.as_slice() else {
            return None;
        };
        Some(parameters.clone())
    }

    /// Record a direct nominal annotation on a parameter (`ctx: ParseContext`)
    /// as a receiver type. This is deliberately limited to one type reference
    /// and direct binding names; structural, conditional, and computed types
    /// remain unresolved instead of triggering an unbounded type walk.
    fn infer_parameter_nominal_type(&mut self, node: Node<'tree>, scope_id: &str) {
        let Some(name_node) = parameter_pattern_node(node) else {
            return;
        };
        let Some(type_name) = direct_type_reference_name(node, self.source) else {
            return;
        };
        let Some(resolution) = self.resolve_name(scope_id, &type_name, Namespace::Both) else {
            return;
        };
        let qualified_type = resolution.qualified_target().or_else(|| {
            self.resolve_declared_type_receiver(scope_id, &type_name)
                .map(|receiver| receiver.qualified_name)
        });
        let Some(qualified_type) = qualified_type else {
            return;
        };
        let mut names = Vec::new();
        collect_pattern_names(name_node, self.source, &mut names);
        for (name, _) in names {
            if let Some(Resolution::Local(parameter)) =
                self.resolve_name(scope_id, &name, Namespace::Value)
                && parameter.kind == "parameter"
            {
                self.variable_types
                    .insert(parameter.id, qualified_type.clone());
            }
        }
    }

    fn infer_variable_object_sources(&mut self, node: Node<'tree>, scope_id: &str) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        if !matches!(
            value.kind(),
            "call_expression" | "optional_call_expression" | "new_expression"
        ) {
            return;
        }
        let Some(function) = value
            .child_by_field_name("function")
            .or_else(|| first_named_child_kind(value, "identifier"))
        else {
            return;
        };
        let name = node_text(self.source, function);
        if name.is_empty() {
            return;
        }
        let Some(Resolution::Local(target)) = self.resolve_name(scope_id, &name, Namespace::Value)
        else {
            return;
        };
        if !self
            .return_object_functions
            .contains(&target.qualified_name)
        {
            return;
        }
        let mut names = Vec::new();
        collect_pattern_names(name_node, self.source, &mut names);
        for (name, _) in names {
            if let Some(Resolution::Local(variable)) =
                self.resolve_name(scope_id, &name, Namespace::Value)
                && variable.kind == "variable"
            {
                self.variable_object_sources
                    .insert(variable.id, target.id.clone());
            }
        }
    }

    fn infer_variable_prototype_source(&mut self, node: Node<'tree>, scope_id: &str) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        if name_node.kind() != "identifier" {
            return;
        }
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        let Some(property) = member_property_node(value) else {
            return;
        };
        if member_property_name(self.source, property).as_deref() != Some("prototype") {
            return;
        }
        let Some(base) = value.child_by_field_name("object") else {
            return;
        };
        let Some(base_name) = rightmost_identifier(base) else {
            return;
        };
        let Some(Resolution::Local(constructor)) = self.resolve_name(
            scope_id,
            &node_text(self.source, base_name),
            Namespace::Value,
        ) else {
            return;
        };
        if !matches!(constructor.kind.as_str(), "function" | "class") {
            return;
        }
        let variable_name = node_text(self.source, name_node);
        let Some(Resolution::Local(variable)) =
            self.resolve_name(scope_id, &variable_name, Namespace::Value)
        else {
            return;
        };
        if variable.kind == "variable" {
            self.prototype_sources.insert(
                variable.id,
                format!("{}.prototype", constructor.qualified_name),
            );
        }
    }

    fn infer_inline_object_type_receiver(&mut self, node: Node<'tree>, scope_id: &str) {
        let Some(type_node) = node
            .child_by_field_name("type")
            .or_else(|| first_named_child_kind(node, "type_annotation"))
        else {
            return;
        };
        let Some(name_node) = node
            .child_by_field_name("name")
            .or_else(|| parameter_pattern_node(node))
        else {
            return;
        };
        let Some(Resolution::Local(variable)) = self.resolve_name(
            scope_id,
            &node_text(self.source, name_node),
            Namespace::Value,
        ) else {
            return;
        };
        if !matches!(variable.kind.as_str(), "variable" | "parameter") {
            return;
        }
        let Some((prefix, _)) = variable.qualified_name.rsplit_once('.') else {
            return;
        };
        let variable_id = variable.id.clone();
        let Some(object_type) = inline_object_type_node(type_node) else {
            return;
        };
        self.variable_inline_type_receivers
            .insert(variable_id.clone(), prefix.to_owned());
        let mut property_types = HashMap::new();
        let mut property_names = Vec::new();
        let mut cursor = object_type.walk();
        for property in object_type
            .named_children(&mut cursor)
            .take(MAX_INLINE_OBJECT_PROPERTIES)
        {
            let Some(name) = property
                .child_by_field_name("name")
                .or_else(|| property.child_by_field_name("key"))
            else {
                continue;
            };
            let Some(property_name) = member_property_name(self.source, name) else {
                continue;
            };
            if !property_names.contains(&property_name) {
                property_names.push(property_name.clone());
            }
            let Some(type_name) = direct_type_reference_name(property, self.source) else {
                continue;
            };
            property_types.insert(property_name, type_name);
        }
        for property_name in property_names {
            let qualified_name = format!("{prefix}.{property_name}");
            let candidates = self
                .declarations_by_qualified
                .get(&qualified_name)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|id| self.declarations.get(id))
                .filter(|declaration| {
                    declaration.range_start_byte >= object_type.start_byte()
                        && declaration.range_start_byte <= object_type.end_byte()
                })
                .collect::<Vec<_>>();
            if let [declaration] = candidates.as_slice() {
                self.inline_object_property_declaration_ids
                    .insert((variable_id.clone(), property_name), declaration.id.clone());
            }
        }
        if !property_types.is_empty() {
            self.inline_object_property_types
                .insert(variable_id, property_types);
        }
        if let Some(index_value) = index_value_type(object_type, self.source) {
            self.index_value_types
                .insert(variable.qualified_name.clone(), index_value);
        }
    }

    /// Propagate one source-grounded inline-object property through the
    /// shallow destructuring form `const { key: local } = receiver`.
    /// Nested patterns, spreads, computed keys, and reassignment flow remain
    /// unresolved rather than being inferred from spelling alone.
    fn infer_destructured_variable_types(&mut self, node: Node<'tree>, scope_id: &str) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        if name_node.kind() != "object_pattern" {
            return;
        }
        let Some(value) = node.child_by_field_name("value") else {
            return;
        };
        if value.kind() != "identifier" {
            return;
        }
        let source_name = node_text(self.source, value);
        let Some(Resolution::Local(source)) =
            self.resolve_name(scope_id, &source_name, Namespace::Value)
        else {
            return;
        };
        let Some(property_types) = self.inline_object_property_types.get(&source.id).cloned()
        else {
            return;
        };
        let mut cursor = name_node.walk();
        for pair in name_node.named_children(&mut cursor) {
            if pair.kind() != "pair_pattern" {
                continue;
            }
            let Some(key) = pair.child_by_field_name("key") else {
                continue;
            };
            let Some(binding) = pair.child_by_field_name("value") else {
                continue;
            };
            if binding.kind() != "identifier" {
                continue;
            }
            let Some(key_name) = member_property_name(self.source, key) else {
                continue;
            };
            let Some(type_name) = property_types.get(&key_name) else {
                continue;
            };
            let Some(Resolution::Local(type_declaration)) =
                self.resolve_name(scope_id, type_name, Namespace::Both)
            else {
                continue;
            };
            let binding_name = node_text(self.source, binding);
            let Some(Resolution::Local(variable)) =
                self.resolve_name(scope_id, &binding_name, Namespace::Value)
            else {
                continue;
            };
            if variable.kind == "variable" {
                self.variable_types
                    .insert(variable.id, type_declaration.qualified_name.clone());
            }
        }
    }

    fn nominal_type_for_variable(&self, node: Node<'tree>, scope_id: &str) -> Option<String> {
        let type_node = node.child_by_field_name("type").or_else(|| {
            node.child_by_field_name("name")
                .and_then(|name| name.child_by_field_name("type"))
        });
        if let Some(type_node) = type_node {
            let mut type_names = Vec::new();
            collect_type_name_nodes(type_node, &mut type_names);
            if let Some(name_node) = rightmost_identifier(type_node) {
                type_names.push(name_node);
            }
            for name_node in type_names {
                if let Some(resolution) = self.resolve_name(
                    scope_id,
                    &node_text(self.source, name_node),
                    Namespace::Both,
                ) && let Some(qualified) = resolution.qualified_target()
                {
                    return Some(qualified);
                }
            }
        }
        let mut value = node.child_by_field_name("value")?;
        loop {
            if matches!(value.kind(), "as_expression" | "satisfies_expression")
                && let Some(inner) = value.child_by_field_name("expression")
            {
                value = inner;
                continue;
            }
            if value.kind() == "parenthesized_expression"
                && let Some(inner) = value
                    .child_by_field_name("expression")
                    .or_else(|| first_named_child(value))
            {
                value = inner;
                continue;
            }
            if matches!(value.kind(), "binary_expression" | "logical_expression")
                && node_text(self.source, value).len() <= MAX_TYPE_SHAPE_BYTES
                && (node_text(self.source, value).contains("||")
                    || node_text(self.source, value).contains("??"))
                && let Some(inner) = value.child_by_field_name("left")
            {
                value = inner;
                continue;
            }
            break;
        }
        if value.kind() == "assignment_expression"
            && let Some(inner) = value.child_by_field_name("right")
        {
            value = inner;
        }
        if matches!(value.kind(), "call_expression" | "optional_call_expression")
            && let Some(receiver) = self.call_return_receiver(scope_id, value)
        {
            return Some(receiver.qualified_name);
        }
        if matches!(value.kind(), "call_expression" | "optional_call_expression")
            && let Some(function) = value
                .child_by_field_name("function")
                .or_else(|| first_named_child_kind(value, "identifier"))
            && let Some(name_node) = rightmost_identifier(function)
            && let Some(Resolution::Local(target)) = self.resolve_name(
                scope_id,
                &node_text(self.source, name_node),
                Namespace::Value,
            )
            && let Some(return_type_name) = target.return_type_name.as_deref()
            && let Some(Resolution::Local(return_type)) =
                self.resolve_name(scope_id, return_type_name, Namespace::Both)
            && let Some(qualified) = Resolution::Local(return_type).qualified_target()
        {
            return Some(qualified);
        }
        if matches!(
            value.kind(),
            "member_expression" | "optional_member_expression" | "subscript_expression"
        ) && let Some(property) = member_property_node(value)
            && let Some(receiver_node) = value.child_by_field_name("object")
            && let Some(receiver) = self.receiver_target(scope_id, receiver_node)
            && let Some(Resolution::Local(declaration)) =
                self.resolve_member_target(scope_id, Some(receiver_node), property, None, &[])
            && let Some(typed_receiver) =
                self.member_value_receiver(scope_id, &receiver, &declaration)
        {
            return Some(typed_receiver.qualified_name);
        }
        if value.kind() == "subscript_expression"
            && let Some(receiver) = self.receiver_target(scope_id, value)
        {
            return Some(receiver.qualified_name);
        }
        value = unwrap_expression_node(value);
        if value.kind() == "subscript_expression"
            && let Some(receiver) = self.receiver_target(scope_id, value)
        {
            return Some(receiver.qualified_name);
        }
        if value.kind() == "this" {
            return self.enclosing_type(scope_id);
        }
        if value.kind() == "object"
            && let Some(name_node) = node.child_by_field_name("name")
            && name_node.kind() == "identifier"
            && let Some(Resolution::Local(declaration)) = self.resolve_name(
                scope_id,
                &node_text(self.source, name_node),
                Namespace::Value,
            )
            && declaration.kind == "variable"
        {
            // Object-literal properties are collected below the variable's
            // qualified identity (for example `config.legacyAgreements`).
            // Treat the variable as a nominal structural receiver so exact
            // same-file property declarations can be targeted without
            // pretending that an arbitrary object has a class type.
            return Some(declaration.qualified_name);
        }
        if value.kind() != "new_expression" {
            return None;
        }
        let constructor = value
            .child_by_field_name("constructor")
            .or_else(|| first_named_child(value))?;
        if node_text(self.source, constructor) == "this" {
            return self.enclosing_type(scope_id);
        }
        let name_node = rightmost_identifier(constructor)?;
        let resolution = self.resolve_name(
            scope_id,
            &node_text(self.source, name_node),
            Namespace::Both,
        )?;
        match &resolution {
            Resolution::Local(declaration) if declaration.kind == "function" => {
                Some(declaration.qualified_name.clone())
            }
            _ => resolution.qualified_target(),
        }
    }

    fn emit_nodes(&mut self, node: Node<'tree>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            return Ok(());
        }
        let scope_id = self.scope_for_node(node);
        match node.kind() {
            "import_statement" => {
                if !self.import_nodes.contains(&node.start_byte()) {
                    self.emit_import(node, &scope_id, false)?;
                }
                return Ok(());
            }
            "export_statement" => {
                if first_named_child_kind(node, "string").is_some() {
                    if !self.import_nodes.contains(&node.start_byte()) {
                        self.emit_export(node, &scope_id)?;
                    }
                    return Ok(());
                }
                self.emit_export(node, &scope_id)?;
                if first_named_child_kind(node, "export_clause").is_some() {
                    // The clause was consumed as a single source construct;
                    // still walk a declaration attached to the export.
                    let mut cursor = node.walk();
                    for child in node.named_children(&mut cursor) {
                        if child.kind() != "export_clause" {
                            self.emit_nodes(child, depth + 1)?;
                        }
                    }
                    return Ok(());
                }
            }
            "call_expression" | "optional_call_expression" => {
                self.emit_call(node, &scope_id, false)?;
                self.emit_dynamic_import(node, &scope_id)?;
            }
            "new_expression" => self.emit_call(node, &scope_id, true)?,
            "member_expression" | "optional_member_expression" | "subscript_expression" => {
                self.emit_member(node, &scope_id)?;
            }
            "jsx_opening_element" | "jsx_self_closing_element" => {
                self.emit_jsx(node, &scope_id)?;
            }
            "decorator" => self.emit_decorator(node, &scope_id)?,
            "class_heritage" if self.language == "javascript" => {
                self.emit_bases(node, &scope_id)?;
            }
            "extends_clause" | "extends_type_clause" | "implements_clause" => {
                self.emit_bases(node, &scope_id)?;
            }
            "assignment_expression" => self.emit_commonjs_export(node, &scope_id)?,
            "variable_declarator" => self.emit_require_declarator(node, &scope_id)?,
            "type_identifier" | "nested_type_identifier" => {
                if is_type_reference_node(node) {
                    self.emit_type_reference(node, &scope_id)?;
                }
            }
            "identifier" => {
                self.emit_callable_reference(node, &scope_id)?;
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if self.import_nodes.contains(&child.start_byte()) {
                continue;
            }
            self.emit_nodes(child, depth + 1)?;
        }
        Ok(())
    }

    fn enter_lexical_scope(
        &mut self,
        node: Node<'tree>,
        parent_scope: String,
    ) -> Result<String, EvidenceError> {
        let Some(kind) = lexical_scope_kind(node) else {
            return Ok(parent_scope);
        };
        if let Some(scope) = self.scope_by_node.get(&node.id()) {
            return Ok(scope.clone());
        }
        let owner = self.owner_for_scope(&parent_scope);
        let scope = self.builder.open_scope(
            kind,
            Some(&owner),
            Some(&parent_scope),
            range_for_node(self.source_file, node),
        )?;
        self.scope_by_node.insert(node.id(), scope.clone());
        self.scope_parents.insert(scope.clone(), Some(parent_scope));
        self.scope_owners.insert(scope.clone(), owner);
        self.scope_kinds.insert(scope.clone(), kind.to_owned());
        Ok(scope)
    }

    fn prototype_assignment_receiver(
        &self,
        function: Node<'tree>,
        scope_id: &str,
    ) -> Option<String> {
        let mut parent = function.parent();
        while let Some(candidate) = parent {
            if candidate.kind() == "parenthesized_expression" {
                parent = candidate.parent();
                continue;
            }
            if candidate.kind() != "assignment_expression"
                || candidate
                    .child_by_field_name("right")
                    .is_none_or(|right| right.id() != function.id())
            {
                return None;
            }
            let left = candidate.child_by_field_name("left")?;
            let prototype = left.child_by_field_name("object")?;
            if let Some(property) = member_property_node(prototype)
                && member_property_name(self.source, property).as_deref() == Some("prototype")
                && let Some(base) = prototype.child_by_field_name("object")
                && let Some(name) = rightmost_identifier(base)
                && let Some(Resolution::Local(declaration)) =
                    self.resolve_name(scope_id, &node_text(self.source, name), Namespace::Value)
            {
                return match declaration.kind.as_str() {
                    "class" | "interface" | "enum" | "namespace" => {
                        Some(declaration.qualified_name)
                    }
                    // A function expression assigned to `Ctor.prototype.name`
                    // receives an instance `this` whose source-backed receiver is
                    // the explicit prototype namespace. This keeps `this.member`
                    // and `new Ctor().member` on the same declaration identity.
                    "function" => Some(format!("{}.prototype", declaration.qualified_name)),
                    _ => None,
                };
            }
            if prototype.kind() == "identifier"
                && let Some(Resolution::Local(variable)) = self.resolve_name(
                    scope_id,
                    &node_text(self.source, prototype),
                    Namespace::Value,
                )
                && let Some(source) = self.prototype_sources.get(&variable.id)
            {
                return Some(source.clone());
            }
            return None;
        }
        None
    }

    fn is_function_prototype_receiver(&self, scope_id: &str, object: Node<'tree>) -> bool {
        let Some(property) = member_property_node(object) else {
            return false;
        };
        if member_property_name(self.source, property).as_deref() != Some("prototype") {
            return false;
        }
        let Some(base) = object.child_by_field_name("object") else {
            return false;
        };
        let Some(name) = rightmost_identifier(base) else {
            return false;
        };
        matches!(
            self.resolve_name(scope_id, &node_text(self.source, name), Namespace::Value),
            Some(Resolution::Local(declaration)) if declaration.kind == "function"
        )
    }

    fn binding_scope_for(&self, node: Node<'tree>, scope_id: &str) -> String {
        if !is_var_binding_node(node) {
            return scope_id.to_owned();
        }
        let mut current = scope_id.to_owned();
        loop {
            if self
                .scope_kinds
                .get(&current)
                .is_some_and(|kind| matches!(kind.as_str(), "function" | "module"))
            {
                return current;
            }
            let Some(parent) = self.scope_parents.get(&current).cloned().flatten() else {
                return current;
            };
            current = parent;
        }
    }

    fn scope_for_node(&self, node: Node<'tree>) -> String {
        let mut current = Some(node);
        while let Some(candidate) = current {
            if let Some(scope) = self.scope_by_node.get(&candidate.id()) {
                return scope.clone();
            }
            current = candidate.parent();
        }
        self.root_scope.clone()
    }

    fn infer_binding_scope(&self, node: Node<'tree>, fallback: &str) -> String {
        let mut current = node.parent();
        while let Some(candidate) = current {
            if candidate.kind() == "conditional_type"
                && let Some(scope) = self.scope_by_node.get(&candidate.id())
            {
                return scope.clone();
            }
            current = candidate.parent();
        }
        fallback.to_owned()
    }

    fn owner_for_scope(&self, scope_id: &str) -> String {
        self.scope_owners
            .get(scope_id)
            .cloned()
            .unwrap_or_else(|| self.file_declaration.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_declaration_resolution(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
        relation: CandidateRelation,
        role: SemanticRole,
        spelling: &str,
        qualifier: Option<&str>,
        context: Option<&str>,
        _namespace: Namespace,
        resolution: Resolution,
        allowed_kinds: &[&str],
    ) -> Result<(), EvidenceError> {
        self.add_declaration_resolution_with_arguments(
            node,
            scope_id,
            relation,
            role,
            spelling,
            qualifier,
            context,
            _namespace,
            None,
            Vec::new(),
            resolution,
            allowed_kinds,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn add_declaration_resolution_with_arguments(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
        relation: CandidateRelation,
        role: SemanticRole,
        spelling: &str,
        qualifier: Option<&str>,
        context: Option<&str>,
        _namespace: Namespace,
        argument_count: Option<u32>,
        argument_types: Vec<Option<String>>,
        resolution: Resolution,
        allowed_kinds: &[&str],
    ) -> Result<(), EvidenceError> {
        let owner = self.owner_for_scope(scope_id);
        let key = fact_key(role, node, context);
        if !self.emitted_facts.insert(key) {
            return Ok(());
        }
        let (binding_id, exact_target, qualified_name, module, target_kind) = match resolution {
            Resolution::Local(declaration) => (
                None,
                Some(declaration.id),
                Some(declaration.qualified_name),
                Some(self.source_file.to_owned()),
                declaration.kind,
            ),
            Resolution::Import(import) => (
                Some(import.binding_id),
                None,
                Some(import.target),
                Some(import.module),
                "external".to_owned(),
            ),
        };
        let allow_external = target_kind == "external"
            && qualified_name
                .as_deref()
                .is_none_or(|qualified| !qualified.contains("#call<"));
        let occurrence_id = self.builder.occur_with_context(
            role,
            &owner,
            spelling,
            qualifier,
            Some(scope_id),
            context,
            range_for_node(self.source_file, node),
        )?;
        let mut constraints = ResolutionConstraint {
            exact_target_declaration_id: exact_target,
            exact_language: Some(self.language.to_owned()),
            module_or_package: module,
            scope_id: Some(scope_id.to_owned()),
            qualified_name,
            argument_count,
            argument_types,
            allowed_target_kinds: allowed_kinds
                .iter()
                .map(|kind| (*kind).to_owned())
                .collect(),
            allow_external,
            ..ResolutionConstraint::default()
        };
        if relation == CandidateRelation::Extends {
            constraints.hierarchy = Some(HierarchyConstraint::DirectBase {
                base_set_complete: true,
            });
        }
        self.builder.relate(
            relation,
            &owner,
            Some(&occurrence_id),
            binding_id.as_deref(),
            spelling,
            constraints,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_external_resolution_candidate(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
        relation: CandidateRelation,
        role: SemanticRole,
        spelling: &str,
        qualifier: Option<&str>,
        context: &str,
        target: &str,
        module: &str,
        argument_count: Option<u32>,
        argument_types: Vec<Option<String>>,
        allowed_kinds: &[&str],
    ) -> Result<(), EvidenceError> {
        let owner = self.owner_for_scope(scope_id);
        if !self
            .emitted_facts
            .insert(fact_key(role, node, Some(context)))
        {
            return Ok(());
        }
        let occurrence_id = self.builder.occur_with_context(
            role,
            &owner,
            spelling,
            qualifier,
            Some(scope_id),
            Some(context),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            relation,
            &owner,
            Some(&occurrence_id),
            None,
            spelling,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(module.to_owned()),
                qualified_name: Some(target.to_owned()),
                argument_count,
                argument_types,
                allowed_target_kinds: allowed_kinds
                    .iter()
                    .map(|kind| (*kind).to_owned())
                    .collect(),
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    /// Preserve an exact source occurrence when the local extractor cannot
    /// prove a target.  This is intentionally a target-less candidate: the
    /// shared resolver may use later project evidence, but it cannot turn the
    /// spelling into an external node or choose a same-named declaration by
    /// terminal-name fallback.  Keeping the occurrence lets qualification
    /// distinguish "observed but unresolved" from "silently dropped".
    #[allow(clippy::too_many_arguments)]
    fn add_unresolved_candidate(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
        relation: CandidateRelation,
        role: SemanticRole,
        spelling: &str,
        qualifier: Option<&str>,
        context: &str,
        argument_count: Option<u32>,
        argument_types: Vec<Option<String>>,
        allowed_kinds: &[&str],
        hierarchy: Option<HierarchyConstraint>,
    ) -> Result<(), EvidenceError> {
        let owner = self.owner_for_scope(scope_id);
        if !self
            .emitted_facts
            .insert(fact_key(role, node, Some(context)))
        {
            return Ok(());
        }
        let occurrence_id = self.builder.occur_with_context(
            role,
            &owner,
            spelling,
            qualifier,
            Some(scope_id),
            Some(context),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            relation,
            &owner,
            Some(&occurrence_id),
            None,
            spelling,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                argument_count,
                argument_types,
                allowed_target_kinds: allowed_kinds
                    .iter()
                    .map(|kind| (*kind).to_owned())
                    .collect(),
                hierarchy,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn is_unshadowed_builtin(&self, scope_id: &str, name: &str) -> bool {
        crate::builtins::is_language_builtin_global(self.language, name)
            && self.is_unshadowed_name(scope_id, name)
    }

    fn is_unshadowed_name(&self, scope_id: &str, name: &str) -> bool {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            if self
                .import_bindings
                .contains_key(&(scope.clone(), name.to_owned()))
                || self
                    .declarations_by_scope
                    .get(&scope)
                    .into_iter()
                    .flat_map(|ids| ids.iter())
                    .filter_map(|id| self.declarations.get(id))
                    .any(|declaration| declaration.name == name)
            {
                return false;
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        true
    }

    fn builtin_global_target(&self, scope_id: &str, name: &str) -> Option<(String, String)> {
        self.is_unshadowed_builtin(scope_id, name)
            .then(|| (format!("global::{name}"), "javascript.global".to_owned()))
    }

    fn builtin_receiver_name(&self, scope_id: &str, object: Node<'tree>) -> Option<String> {
        match object.kind() {
            "identifier" | "type_identifier" | "jsx_identifier" => {
                let name = node_text(self.source, object);
                self.is_unshadowed_builtin(scope_id, &name).then_some(name)
            }
            "new_expression" => {
                let constructor = object
                    .child_by_field_name("constructor")
                    .or_else(|| first_named_child(object))?;
                let name = rightmost_identifier(constructor)?;
                let spelling = node_text(self.source, name);
                self.is_unshadowed_builtin(scope_id, &spelling)
                    .then_some(spelling)
            }
            _ => None,
        }
    }

    fn builtin_member_target(
        &self,
        scope_id: &str,
        object: Node<'tree>,
        property_name: &str,
    ) -> Option<(String, String)> {
        let receiver = self.builtin_receiver_name(scope_id, object)?;
        let known = if object.kind() == "new_expression" {
            known_builtin_instance_member(&receiver, property_name)
        } else {
            known_builtin_static_member(&receiver, property_name)
        };
        if !known {
            return None;
        }
        Some((
            format!("global::{receiver}.{property_name}"),
            "javascript.global".to_owned(),
        ))
    }

    fn builtin_type_target(&self, scope_id: &str, name: &str) -> Option<(String, String)> {
        if !self.is_unshadowed_name(scope_id, name) {
            return None;
        }
        if crate::builtins::is_language_builtin_global(self.language, name) {
            return Some((format!("global::{name}"), "javascript.global".to_owned()));
        }
        is_typescript_utility_type(name)
            .then(|| {
                (
                    format!("typescript.lib::{name}"),
                    "typescript.lib".to_owned(),
                )
            })
            .or_else(|| standard_library_type_target(name))
    }

    fn ambient_qualified_type_target(
        &self,
        scope_id: &str,
        spelling: &str,
    ) -> Option<(String, String)> {
        let namespace = spelling.split('.').next()?;
        if !self.is_unshadowed_name(scope_id, namespace) {
            return None;
        }
        let module = match namespace {
            "React" | "JSX" => "@types/react",
            "NodeJS" => "@types/node",
            _ => return None,
        };
        Some((format!("{module}::{spelling}"), module.to_owned()))
    }

    fn emit_import(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
        reexport: bool,
    ) -> Result<(), EvidenceError> {
        if !reexport
            && let Some(require_clause) = first_named_child_kind(node, "import_require_clause")
        {
            return self.emit_import_equals(node, scope_id, require_clause);
        }
        let Some(module_node) = first_named_child_kind(node, "string") else {
            return Ok(());
        };
        let module = string_literal(self.source, module_node);
        if module.is_empty() {
            return Ok(());
        }
        self.import_nodes.insert(node.start_byte());
        let statement_text = node_text(self.source, node);
        let type_only = statement_text.trim_start().starts_with(if reexport {
            "export type"
        } else {
            "import type"
        });
        let mut bindings = Vec::new();
        if let Some(clause) = first_named_child_kind(
            node,
            if reexport {
                "export_clause"
            } else {
                "import_clause"
            },
        ) {
            collect_import_bindings(self.source, clause, reexport, type_only, &mut bindings);
        }
        let owner = self.owner_for_scope(scope_id);
        let module_occurrence = self.builder.occur_with_context(
            if reexport {
                SemanticRole::Reexport
            } else {
                SemanticRole::Import
            },
            &owner,
            &module,
            None,
            Some(scope_id),
            Some(if type_only {
                "type_only_module"
            } else {
                "module"
            }),
            range_for_node(self.source_file, module_node),
        )?;
        self.builder.relate(
            if reexport {
                CandidateRelation::Reexports
            } else {
                CandidateRelation::Imports
            },
            &owner,
            Some(&module_occurrence),
            None,
            &module,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(module.clone()),
                allowed_target_kinds: vec!["module".to_owned()],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        if bindings.is_empty() {
            return Ok(());
        }
        for binding in bindings {
            let target = format!("{}::{}", module, binding.imported_name);
            let kind = if reexport {
                BindingKind::Reexport
            } else if binding.local_name == binding.imported_name {
                BindingKind::Import
            } else {
                BindingKind::ImportAlias
            };
            let namespace = import_namespace(&binding);
            let binding_id = self.builder.bind_with_identity(
                kind,
                &binding.local_name,
                &target,
                None,
                Some(scope_id),
                Some(symbol_namespace(namespace)),
                binding.type_only,
                range_for_node(self.source_file, binding.anchor),
            )?;
            if !reexport {
                self.import_bindings.insert(
                    (scope_id.to_owned(), binding.local_name.clone()),
                    ImportInfo {
                        binding_id: binding_id.clone(),
                        target: target.clone(),
                        module: module.clone(),
                        imported_name: binding.imported_name.clone(),
                        namespace,
                        type_only: binding.type_only,
                        callable_namespace: false,
                    },
                );
            }
            let owner = self.owner_for_scope(scope_id);
            let occurrence_id = self.builder.occur_with_context(
                if reexport {
                    SemanticRole::Reexport
                } else {
                    SemanticRole::Import
                },
                &owner,
                &binding.local_name,
                None,
                Some(scope_id),
                Some(if binding.type_only {
                    "type_only"
                } else {
                    "binding"
                }),
                range_for_node(self.source_file, binding.anchor),
            )?;
            self.builder.relate(
                if reexport {
                    CandidateRelation::Reexports
                } else {
                    CandidateRelation::Imports
                },
                &owner,
                Some(&occurrence_id),
                Some(&binding_id),
                &binding.local_name,
                ResolutionConstraint {
                    exact_language: Some(self.language.to_owned()),
                    module_or_package: Some(module.clone()),
                    qualified_name: Some(target),
                    allowed_target_kinds: vec![
                        "module".to_owned(),
                        "function".to_owned(),
                        "class".to_owned(),
                        "interface".to_owned(),
                        "type_alias".to_owned(),
                        "variable".to_owned(),
                    ],
                    allow_external: true,
                    ..ResolutionConstraint::default()
                },
            )?;
        }
        Ok(())
    }

    fn emit_import_equals(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
        clause: Node<'tree>,
    ) -> Result<(), EvidenceError> {
        let Some(module_node) = clause
            .child_by_field_name("source")
            .or_else(|| first_named_child_kind(clause, "string"))
        else {
            return Ok(());
        };
        let module = string_literal(self.source, module_node);
        if module.is_empty() {
            return Ok(());
        }
        self.import_nodes.insert(node.start_byte());
        let owner = self.owner_for_scope(scope_id);
        let module_occurrence = self.builder.occur_with_context(
            SemanticRole::Import,
            &owner,
            &module,
            None,
            Some(scope_id),
            Some("import_equals_module"),
            range_for_node(self.source_file, module_node),
        )?;
        self.builder.relate(
            CandidateRelation::Imports,
            &owner,
            Some(&module_occurrence),
            None,
            &module,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(module.clone()),
                allowed_target_kinds: vec!["module".to_owned()],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;

        let Some(local_node) = first_identifier_node(clause) else {
            return Ok(());
        };
        let local_name = node_text(self.source, local_node);
        if local_name.is_empty() {
            return Ok(());
        }
        let target = format!("{module}::*");
        let binding_id = self.builder.bind_with_identity(
            BindingKind::Import,
            &local_name,
            &target,
            None,
            Some(scope_id),
            Some(SymbolNamespace::ValueAndType),
            false,
            range_for_node(self.source_file, local_node),
        )?;
        self.import_bindings.insert(
            (scope_id.to_owned(), local_name.clone()),
            ImportInfo {
                binding_id: binding_id.clone(),
                target: target.clone(),
                module: module.clone(),
                imported_name: "*".to_owned(),
                namespace: Namespace::Both,
                type_only: false,
                callable_namespace: true,
            },
        );
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Import,
            &owner,
            &local_name,
            None,
            Some(scope_id),
            Some("import_equals"),
            range_for_node(self.source_file, local_node),
        )?;
        self.builder.relate(
            CandidateRelation::Imports,
            &owner,
            Some(&occurrence_id),
            Some(&binding_id),
            &local_name,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(module),
                qualified_name: Some(target),
                allowed_target_kinds: vec![
                    "module".to_owned(),
                    "function".to_owned(),
                    "class".to_owned(),
                    "interface".to_owned(),
                    "type_alias".to_owned(),
                    "variable".to_owned(),
                    "external".to_owned(),
                ],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_export(&mut self, node: Node<'tree>, scope_id: &str) -> Result<(), EvidenceError> {
        let Some(module_node) = first_named_child_kind(node, "string") else {
            if let Some(clause) = first_named_child_kind(node, "export_clause") {
                self.emit_local_export_clause(clause, scope_id)?;
            } else {
                self.emit_default_export(node, scope_id)?;
            }
            return Ok(());
        };
        let module = string_literal(self.source, module_node);
        if module.is_empty() {
            return Ok(());
        }
        self.emit_import(node, scope_id, true)
    }

    fn emit_default_export(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
    ) -> Result<(), EvidenceError> {
        let Some(exported) = first_named_child(node) else {
            return Ok(());
        };
        let (spelling, anchor) = if let Some(target_node) = default_export_target_node(node) {
            (node_text(self.source, target_node), target_node)
        } else if anonymous_declaration_shape(exported).is_some() {
            ("default".to_owned(), exported)
        } else {
            return Ok(());
        };
        if spelling.is_empty() {
            return Ok(());
        }
        let Some(resolution) = self.resolve_name(scope_id, &spelling, Namespace::Both) else {
            return Ok(());
        };
        let (target, target_declaration_id, namespace) = match resolution {
            Resolution::Local(declaration) => (
                declaration.qualified_name,
                Some(declaration.id),
                declaration.namespace,
            ),
            Resolution::Import(import) => (import.target, None, import.namespace),
        };
        let owner = self.owner_for_scope(scope_id);
        let binding_id = self.builder.bind_with_identity(
            BindingKind::Reexport,
            "default",
            &target,
            target_declaration_id.as_deref(),
            Some(scope_id),
            Some(symbol_namespace(namespace)),
            false,
            range_for_node(self.source_file, anchor),
        )?;
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Reexport,
            &owner,
            "default",
            Some(&spelling),
            Some(scope_id),
            Some("default"),
            range_for_node(self.source_file, anchor),
        )?;
        self.builder.relate(
            CandidateRelation::Reexports,
            &owner,
            Some(&occurrence_id),
            Some(&binding_id),
            "default",
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                qualified_name: Some(target),
                allowed_target_kinds: vec![
                    "function".to_owned(),
                    "class".to_owned(),
                    "variable".to_owned(),
                    "interface".to_owned(),
                    "type_alias".to_owned(),
                    "enum".to_owned(),
                ],
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_local_export_clause(
        &mut self,
        clause: Node<'tree>,
        scope_id: &str,
    ) -> Result<(), EvidenceError> {
        let mut specifiers = Vec::new();
        collect_nodes_of_kind(clause, "export_specifier", &mut specifiers);
        for specifier in specifiers {
            let identifiers = direct_identifier_children(specifier, self.source);
            let Some(local) = identifiers.first() else {
                continue;
            };
            let exported = identifiers.get(1).copied().unwrap_or(*local);
            let name = node_text(self.source, *local);
            let exported_name = node_text(self.source, exported);
            let type_only = node_text(self.source, specifier)
                .trim_start()
                .starts_with("type ");
            let Some(resolution) = self.resolve_name(scope_id, &name, Namespace::Both) else {
                continue;
            };
            let owner = self.owner_for_scope(scope_id);
            let (target, target_declaration_id, namespace, imported_type_only) =
                match resolution.clone() {
                    Resolution::Local(declaration) => (
                        declaration.qualified_name,
                        Some(declaration.id),
                        declaration.namespace,
                        false,
                    ),
                    Resolution::Import(import) => {
                        (import.target, None, import.namespace, import.type_only)
                    }
                };
            let type_only = type_only || imported_type_only;
            let namespace = if type_only {
                if namespace == Namespace::Module {
                    Namespace::Module
                } else {
                    Namespace::Type
                }
            } else {
                namespace
            };
            let binding_id = self.builder.bind_with_identity(
                BindingKind::Reexport,
                &exported_name,
                &target,
                target_declaration_id.as_deref(),
                Some(scope_id),
                Some(symbol_namespace(namespace)),
                type_only,
                range_for_node(self.source_file, exported),
            )?;
            let occurrence_id = self.builder.occur_with_context(
                SemanticRole::Reexport,
                &owner,
                &exported_name,
                Some(&name),
                Some(scope_id),
                Some("local"),
                range_for_node(self.source_file, exported),
            )?;
            self.builder.relate(
                CandidateRelation::Reexports,
                &owner,
                Some(&occurrence_id),
                Some(&binding_id),
                &exported_name,
                ResolutionConstraint {
                    exact_language: Some(self.language.to_owned()),
                    qualified_name: Some(target),
                    allowed_target_kinds: vec![
                        "function".to_owned(),
                        "class".to_owned(),
                        "variable".to_owned(),
                        "type_alias".to_owned(),
                        "interface".to_owned(),
                    ],
                    ..ResolutionConstraint::default()
                },
            )?;
        }
        Ok(())
    }

    fn emit_call(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
        construction: bool,
    ) -> Result<(), EvidenceError> {
        let Some(function) = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("constructor"))
        else {
            return Ok(());
        };
        let argument_types = self.call_argument_types(node, scope_id);
        let argument_count = call_argument_count(node);
        let member_call = matches!(
            function.kind(),
            "member_expression" | "optional_member_expression" | "subscript_expression"
        );
        if member_call {
            let Some(property) = member_property_node(function) else {
                // A computed/dynamic member call is still a real call
                // occurrence. Preserve its exact callee range, but do not
                // invent a property target from a runtime key.
                let spelling = node_text(self.source, function);
                if spelling.is_empty() {
                    return Ok(());
                }
                let qualifier = function
                    .child_by_field_name("object")
                    .map(|object| node_text(self.source, object));
                return self.add_unresolved_candidate(
                    function,
                    scope_id,
                    if construction {
                        CandidateRelation::Constructs
                    } else {
                        CandidateRelation::Calls
                    },
                    if construction {
                        SemanticRole::Construction
                    } else {
                        SemanticRole::Call
                    },
                    &spelling,
                    qualifier.as_deref(),
                    if construction {
                        "dynamic_new_member"
                    } else {
                        "dynamic_member_call"
                    },
                    argument_count,
                    argument_types,
                    &[
                        "function",
                        "method",
                        "constructor",
                        "class",
                        "variable",
                        "property",
                        "external",
                    ],
                    None,
                );
            };
            let Some(property_name) = member_property_name(self.source, property) else {
                // A computed key such as `receiver[key]()` has a property
                // syntax node, but no source-proven property name. Preserve
                // the whole dynamic callee just as for a missing property
                // field above.
                let spelling = node_text(self.source, function);
                if spelling.is_empty() {
                    return Ok(());
                }
                let qualifier = function
                    .child_by_field_name("object")
                    .map(|object| node_text(self.source, object));
                return self.add_unresolved_candidate(
                    function,
                    scope_id,
                    if construction {
                        CandidateRelation::Constructs
                    } else {
                        CandidateRelation::Calls
                    },
                    if construction {
                        SemanticRole::Construction
                    } else {
                        SemanticRole::Call
                    },
                    &spelling,
                    qualifier.as_deref(),
                    if construction {
                        "dynamic_new_member"
                    } else {
                        "dynamic_member_call"
                    },
                    argument_count,
                    argument_types,
                    &[
                        "function",
                        "method",
                        "constructor",
                        "class",
                        "variable",
                        "property",
                        "external",
                    ],
                    None,
                );
            };
            let object = function.child_by_field_name("object");
            let resolution = self.resolve_member_target(
                scope_id,
                object,
                property,
                call_argument_count(node),
                &argument_types,
            );
            let Some(resolution) = resolution else {
                if let Some(object) = object
                    && let Some((target, module)) =
                        self.builtin_member_target(scope_id, object, &property_name)
                {
                    return self.add_external_resolution_candidate(
                        property,
                        scope_id,
                        if construction {
                            CandidateRelation::Constructs
                        } else {
                            CandidateRelation::Calls
                        },
                        if construction {
                            SemanticRole::Construction
                        } else {
                            SemanticRole::Call
                        },
                        &property_name,
                        Some(&node_text(self.source, function)),
                        if construction {
                            "new_member"
                        } else {
                            "member_call"
                        },
                        &target,
                        &module,
                        argument_count,
                        argument_types,
                        &[
                            "function",
                            "method",
                            "constructor",
                            "class",
                            "variable",
                            "property",
                            "external",
                        ],
                    );
                }
                // A dynamic receiver, proxy, or ambiguous member is not a
                // safe call target. Preserve its source occurrence as
                // unresolved rather than falling back to an unrelated
                // top-level function with the same spelling.
                return self.add_unresolved_candidate(
                    property,
                    scope_id,
                    if construction {
                        CandidateRelation::Constructs
                    } else {
                        CandidateRelation::Calls
                    },
                    if construction {
                        SemanticRole::Construction
                    } else {
                        SemanticRole::Call
                    },
                    &property_name,
                    Some(&node_text(self.source, function)),
                    if construction {
                        "new_member"
                    } else {
                        "member_call"
                    },
                    argument_count,
                    argument_types,
                    &[
                        "function",
                        "method",
                        "constructor",
                        "class",
                        "variable",
                        "property",
                        "external",
                    ],
                    None,
                );
            };
            return self.add_declaration_resolution_with_arguments(
                property,
                scope_id,
                if construction {
                    CandidateRelation::Constructs
                } else {
                    CandidateRelation::Calls
                },
                if construction {
                    SemanticRole::Construction
                } else {
                    SemanticRole::Call
                },
                &property_name,
                Some(&node_text(self.source, function)),
                Some(if construction {
                    "new_member"
                } else {
                    "member_call"
                }),
                Namespace::Value,
                argument_count,
                argument_types,
                resolution,
                &[
                    "function",
                    "method",
                    "constructor",
                    "class",
                    "variable",
                    "parameter",
                    "property",
                    "external",
                ],
            );
        }
        // The callee node itself is the source oracle's target anchor for
        // direct identifiers and dynamic expressions alike. Do not walk to a
        // nested identifier inside `(fn => fn())` or `(factory || Ctor)`,
        // because that would shrink a dynamic call into an unrelated body
        // spelling and lose the exact occurrence range.
        let target = function;
        if target.kind() != "identifier"
            && target.kind() != "property_identifier"
            && target.kind() != "type_identifier"
            && target.kind() != "this"
            && target.kind() != "super"
        {
            // Calls through a parenthesized function, conditional
            // expression, `import()`, or another dynamic callee cannot be
            // assigned a declaration target locally. Keep the call's exact
            // callee occurrence so qualification and later project evidence
            // can distinguish unresolved from silently dropped.
            let spelling = node_text(self.source, function);
            if spelling.is_empty() {
                return Ok(());
            }
            return self.add_unresolved_candidate(
                function,
                scope_id,
                if construction {
                    CandidateRelation::Constructs
                } else {
                    CandidateRelation::Calls
                },
                if construction {
                    SemanticRole::Construction
                } else {
                    SemanticRole::Call
                },
                &spelling,
                None,
                if construction {
                    "dynamic_new"
                } else {
                    "dynamic_call"
                },
                argument_count,
                argument_types,
                &["function", "class", "method", "constructor", "external"],
                None,
            );
        }
        let spelling = node_text(self.source, target);
        if target.kind() == "super" {
            if let Some(resolution) = self.resolve_super_target(scope_id) {
                return self.add_declaration_resolution_with_arguments(
                    target,
                    scope_id,
                    CandidateRelation::Calls,
                    SemanticRole::Call,
                    &spelling,
                    None,
                    Some("super"),
                    Namespace::Value,
                    argument_count,
                    argument_types,
                    resolution,
                    &["class", "constructor", "function", "external"],
                );
            }
            return self.add_unresolved_candidate(
                target,
                scope_id,
                CandidateRelation::Calls,
                SemanticRole::Call,
                &spelling,
                None,
                "super",
                argument_count,
                argument_types,
                &["class", "constructor", "function", "external"],
                None,
            );
        }
        let is_this_target =
            target.kind() == "this" || (target.kind() == "identifier" && spelling == "this");
        if is_this_target {
            if let Some(receiver) = self.this_receiver_target(scope_id)
                && let Some(ids) = self.declarations_by_qualified.get(&receiver.qualified_name)
            {
                let mut classes = ids
                    .iter()
                    .filter_map(|id| self.declarations.get(id))
                    .filter(|declaration| declaration.kind == "class")
                    .filter(|declaration| {
                        receiver.scope_id.as_deref().is_none_or(|scope| {
                            self.scope_owners
                                .get(scope)
                                .is_some_and(|owner| owner == &declaration.id)
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if classes.len() == 1 {
                    return self.add_declaration_resolution_with_arguments(
                        target,
                        scope_id,
                        if construction {
                            CandidateRelation::Constructs
                        } else {
                            CandidateRelation::Calls
                        },
                        if construction {
                            SemanticRole::Construction
                        } else {
                            SemanticRole::Call
                        },
                        "this",
                        None,
                        Some(if construction {
                            "new_this"
                        } else {
                            "this_call"
                        }),
                        Namespace::Value,
                        argument_count,
                        argument_types,
                        Resolution::Local(classes.remove(0)),
                        &["class", "constructor", "function", "external"],
                    );
                }
            }
            return self.add_unresolved_candidate(
                target,
                scope_id,
                if construction {
                    CandidateRelation::Constructs
                } else {
                    CandidateRelation::Calls
                },
                if construction {
                    SemanticRole::Construction
                } else {
                    SemanticRole::Call
                },
                "this",
                None,
                if construction {
                    "new_this"
                } else {
                    "this_call"
                },
                argument_count,
                argument_types,
                &["class", "constructor", "function", "external"],
                None,
            );
        }
        let resolution = self.resolve_name_for_call(
            scope_id,
            &spelling,
            Namespace::Value,
            argument_count,
            &argument_types,
        );
        let Some(resolution) = resolution else {
            if let Some((qualified_target, module)) =
                self.builtin_global_target(scope_id, &spelling)
            {
                return self.add_external_resolution_candidate(
                    target,
                    scope_id,
                    if construction {
                        CandidateRelation::Constructs
                    } else {
                        CandidateRelation::Calls
                    },
                    if construction {
                        SemanticRole::Construction
                    } else {
                        SemanticRole::Call
                    },
                    &spelling,
                    None,
                    if construction { "new" } else { "call" },
                    &qualified_target,
                    &module,
                    argument_count,
                    argument_types,
                    &["function", "class", "external"],
                );
            }
            return self.add_unresolved_candidate(
                target,
                scope_id,
                if construction {
                    CandidateRelation::Constructs
                } else {
                    CandidateRelation::Calls
                },
                if construction {
                    SemanticRole::Construction
                } else {
                    SemanticRole::Call
                },
                &spelling,
                None,
                if construction { "new" } else { "call" },
                argument_count,
                argument_types,
                &["function", "class", "method", "constructor", "external"],
                None,
            );
        };
        self.add_declaration_resolution_with_arguments(
            target,
            scope_id,
            if construction {
                CandidateRelation::Constructs
            } else {
                CandidateRelation::Calls
            },
            if construction {
                SemanticRole::Construction
            } else {
                SemanticRole::Call
            },
            &spelling,
            None,
            Some(if construction { "new" } else { "call" }),
            Namespace::Value,
            argument_count,
            argument_types,
            resolution,
            &["function", "method", "class", "variable"],
        )
    }

    fn call_argument_types(&self, node: Node<'tree>, scope_id: &str) -> Vec<Option<String>> {
        let Some(arguments) = node
            .child_by_field_name("arguments")
            .or_else(|| first_named_child_kind(node, "arguments"))
        else {
            return Vec::new();
        };
        let mut cursor = arguments.walk();
        arguments
            .named_children(&mut cursor)
            .filter(|argument| !argument.kind().contains("comment"))
            .map(|argument| self.call_argument_type(argument, scope_id))
            .collect()
    }

    fn call_argument_type(&self, node: Node<'tree>, scope_id: &str) -> Option<String> {
        match node.kind() {
            "string" | "string_fragment" | "template_string" => Some("string".to_owned()),
            "number" => Some("number".to_owned()),
            "true" | "false" => Some("boolean".to_owned()),
            "null" => Some("null".to_owned()),
            "array" => Some("array".to_owned()),
            "object" => Some("object".to_owned()),
            "arrow_function" | "function" | "function_expression" | "generator_function" => {
                Some("function".to_owned())
            }
            "as_expression" | "satisfies_expression" | "parenthesized_expression" => node
                .child_by_field_name("expression")
                .or_else(|| first_named_child(node))
                .and_then(|inner| self.call_argument_type(inner, scope_id)),
            "new_expression" => {
                let constructor = node
                    .child_by_field_name("constructor")
                    .or_else(|| first_named_child(node))?;
                let name = rightmost_identifier(constructor)?;
                match self.resolve_name(scope_id, &node_text(self.source, name), Namespace::Both)? {
                    Resolution::Local(declaration)
                        if matches!(declaration.kind.as_str(), "class" | "enum") =>
                    {
                        Some(declaration.qualified_name)
                    }
                    Resolution::Import(import) => Some(import.target),
                    _ => None,
                }
            }
            "identifier" | "type_identifier" | "jsx_identifier" => {
                let name = node_text(self.source, node);
                if name == "undefined" {
                    return Some("undefined".to_owned());
                }
                match self.resolve_name(scope_id, &name, Namespace::Value)? {
                    Resolution::Local(declaration) => self
                        .variable_types
                        .get(&declaration.id)
                        .cloned()
                        .or_else(|| {
                            matches!(declaration.kind.as_str(), "class" | "enum")
                                .then_some(declaration.qualified_name)
                        }),
                    Resolution::Import(import) => Some(import.target),
                }
            }
            _ => None,
        }
    }

    fn emit_dynamic_import(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
    ) -> Result<(), EvidenceError> {
        let Some(function) = node.child_by_field_name("function") else {
            return Ok(());
        };
        if node_text(self.source, function) != "import" {
            return Ok(());
        }
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return Ok(());
        };
        let Some(module_node) = first_named_child_kind(arguments, "string") else {
            return Ok(());
        };
        let module = string_literal(self.source, module_node);
        if module.is_empty() {
            return Ok(());
        }
        let owner = self.owner_for_scope(scope_id);
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Import,
            &owner,
            &module,
            None,
            Some(scope_id),
            Some("dynamic_import"),
            range_for_node(self.source_file, module_node),
        )?;
        self.builder.relate(
            CandidateRelation::Imports,
            &owner,
            Some(&occurrence_id),
            None,
            &module,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(module.clone()),
                allowed_target_kinds: vec!["module".to_owned()],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_require_declarator(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
    ) -> Result<(), EvidenceError> {
        let Some(value) = node.child_by_field_name("value") else {
            return Ok(());
        };
        let value = unwrap_expression_node(value);
        let Some(call) = direct_require_call(value, self.source) else {
            return Ok(());
        };
        let Some(arguments) = call.child_by_field_name("arguments") else {
            return Ok(());
        };
        let Some(module_node) = first_named_child_kind(arguments, "string") else {
            return Ok(());
        };
        let module = string_literal(self.source, module_node);
        if module.is_empty() {
            return Ok(());
        }
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        self.emit_require_pattern_bindings(scope_id, name_node, &module, 0)
    }

    fn emit_require_pattern_bindings(
        &mut self,
        scope_id: &str,
        pattern: Node<'tree>,
        module: &str,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        if depth >= MAX_TYPE_SHAPE_DEPTH as usize {
            return Ok(());
        }
        match pattern.kind() {
            // `const api = require("./api")` binds the module namespace.  A
            // namespace target supports both callable CommonJS modules and
            // source-grounded member access without pretending that the local
            // spelling is an exported symbol.
            "identifier" => {
                let local_name = node_text(self.source, pattern);
                if !local_name.is_empty() {
                    self.add_import_binding(
                        scope_id,
                        &local_name,
                        "*",
                        module,
                        pattern,
                        false,
                        "require",
                    )?;
                }
            }
            "object_pattern" => {
                let mut cursor = pattern.walk();
                for child in pattern
                    .named_children(&mut cursor)
                    .take(MAX_INLINE_OBJECT_PROPERTIES)
                {
                    match child.kind() {
                        "shorthand_property_identifier_pattern" => {
                            let name = node_text(self.source, child);
                            if !name.is_empty() {
                                self.add_import_binding(
                                    scope_id, &name, &name, module, child, false, "require",
                                )?;
                            }
                        }
                        "pair_pattern" => {
                            let Some(key) = child.child_by_field_name("key") else {
                                continue;
                            };
                            let Some(value) = child.child_by_field_name("value") else {
                                continue;
                            };
                            let Some(imported_name) =
                                static_require_property_name(self.source, key)
                            else {
                                // Computed or malformed keys do not prove a
                                // stable CommonJS export spelling.
                                continue;
                            };
                            let Some((local_name, anchor)) =
                                direct_require_binding_identifier(value, self.source)
                            else {
                                // Nested patterns and rest/default objects
                                // need object-shape and flow evidence that a
                                // single module binding cannot provide.
                                continue;
                            };
                            self.add_import_binding(
                                scope_id,
                                &local_name,
                                &imported_name,
                                module,
                                anchor,
                                false,
                                "require",
                            )?;
                        }
                        _ => {
                            // Rest, nested, array, and dynamic patterns remain
                            // unresolved rather than being flattened into
                            // unrelated module exports.
                        }
                    }
                }
            }
            _ => {
                // Array, rest, assignment, and malformed declarator patterns
                // are intentionally not projected as named module bindings.
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn add_import_binding(
        &mut self,
        scope_id: &str,
        local_name: &str,
        imported_name: &str,
        module: &str,
        anchor: Node<'tree>,
        type_only: bool,
        context: &str,
    ) -> Result<(), EvidenceError> {
        let target = format!("{module}::{imported_name}");
        let kind = if local_name == imported_name {
            BindingKind::Import
        } else {
            BindingKind::ImportAlias
        };
        let namespace = if type_only {
            Namespace::Type
        } else if imported_name == "*" {
            Namespace::Module
        } else {
            Namespace::Value
        };
        let binding_id = self.builder.bind_with_identity(
            kind,
            local_name,
            &target,
            None,
            Some(scope_id),
            Some(symbol_namespace(namespace)),
            type_only,
            range_for_node(self.source_file, anchor),
        )?;
        self.import_bindings.insert(
            (scope_id.to_owned(), local_name.to_owned()),
            ImportInfo {
                binding_id: binding_id.clone(),
                target: target.clone(),
                module: module.to_owned(),
                imported_name: imported_name.to_owned(),
                namespace,
                type_only,
                callable_namespace: imported_name == "*" && context == "require",
            },
        );
        let owner = self.owner_for_scope(scope_id);
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Import,
            &owner,
            local_name,
            None,
            Some(scope_id),
            Some(context),
            range_for_node(self.source_file, anchor),
        )?;
        self.builder.relate(
            CandidateRelation::Imports,
            &owner,
            Some(&occurrence_id),
            Some(&binding_id),
            local_name,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(module.to_owned()),
                qualified_name: Some(target),
                allowed_target_kinds: vec![
                    "module".to_owned(),
                    "function".to_owned(),
                    "class".to_owned(),
                    "variable".to_owned(),
                    "type_alias".to_owned(),
                    "interface".to_owned(),
                ],
                allow_external: true,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_member(&mut self, node: Node<'tree>, scope_id: &str) -> Result<(), EvidenceError> {
        let Some(property) = member_property_node(node) else {
            return Ok(());
        };
        let Some(object) = node.child_by_field_name("object") else {
            return Ok(());
        };
        let Some(property_name) = member_property_name(self.source, property) else {
            return Ok(());
        };
        if let Some(resolution) =
            self.resolve_member_target(scope_id, Some(object), property, None, &[])
        {
            return self.add_declaration_resolution(
                property,
                scope_id,
                CandidateRelation::AccessesMember,
                SemanticRole::MemberAccess,
                &property_name,
                Some(&node_text(self.source, object)),
                Some("member"),
                Namespace::Value,
                resolution,
                &[
                    "property",
                    "method",
                    "constructor",
                    "function",
                    "class",
                    "variable",
                    "parameter",
                    "external",
                ],
            );
        }
        if let Some((target, module)) = self.builtin_member_target(scope_id, object, &property_name)
        {
            return self.add_external_resolution_candidate(
                property,
                scope_id,
                CandidateRelation::AccessesMember,
                SemanticRole::MemberAccess,
                &property_name,
                Some(&node_text(self.source, object)),
                "builtin_member",
                &target,
                &module,
                None,
                Vec::new(),
                &[
                    "property", "method", "function", "class", "variable", "external",
                ],
            );
        }
        // A local variable without a source-proven nominal type is still a
        // real member occurrence, but its target is not safe to derive from
        // the variable spelling. Keep it unresolved instead of manufacturing
        // an external `object.property` identity.
        self.add_unresolved_candidate(
            property,
            scope_id,
            CandidateRelation::AccessesMember,
            SemanticRole::MemberAccess,
            &property_name,
            Some(&node_text(self.source, object)),
            "member",
            None,
            Vec::new(),
            &["property", "field", "method", "function", "external"],
            None,
        )
    }

    fn emit_jsx(&mut self, node: Node<'tree>, scope_id: &str) -> Result<(), EvidenceError> {
        let Some(name) = node
            .child_by_field_name("name")
            .or_else(|| first_named_child_kind(node, "jsx_identifier"))
        else {
            return Ok(());
        };
        let spelling = node_text(self.source, name);
        // JSX member tags (`<UI.Button />`) carry a different tree-sitter
        // node kind from ordinary JavaScript member expressions. Resolve the
        // member through the same nominal receiver proof used by calls and
        // accesses so a namespace import targets `module::Button`, not the
        // unrelated `UI` binding or a same-spelled local declaration.
        if matches!(
            name.kind(),
            "member_expression" | "jsx_member_expression" | "jsx_namespace_name"
        ) && let Some(property) = name.child_by_field_name("property")
            && let Some(object) = name.child_by_field_name("object")
            && let Some(resolution) =
                self.resolve_member_target(scope_id, Some(object), property, None, &[])
        {
            let property_spelling = node_text(self.source, property);
            let qualifier = node_text(self.source, object);
            return self.add_declaration_resolution(
                property,
                scope_id,
                CandidateRelation::References,
                SemanticRole::CallableReference,
                &property_spelling,
                Some(&qualifier),
                Some("jsx"),
                Namespace::Value,
                resolution,
                &["function", "class", "variable", "property", "external"],
            );
        }
        let first = spelling.split(['.', ':']).next().unwrap_or_default();
        if first.is_empty() || !first.chars().next().is_some_and(char::is_uppercase) {
            return Ok(());
        }
        let Some(resolution) = self.resolve_name(scope_id, first, Namespace::Value) else {
            return Ok(());
        };
        self.add_declaration_resolution(
            name,
            scope_id,
            CandidateRelation::References,
            SemanticRole::CallableReference,
            &spelling,
            None,
            Some("jsx"),
            Namespace::Value,
            resolution,
            &["function", "class", "variable"],
        )
    }

    fn emit_decorator(&mut self, node: Node<'tree>, scope_id: &str) -> Result<(), EvidenceError> {
        let Some(target) = first_named_child_kind(node, "identifier") else {
            return Ok(());
        };
        let spelling = node_text(self.source, target);
        let Some(resolution) = self.resolve_name(scope_id, &spelling, Namespace::Value) else {
            return Ok(());
        };
        self.add_declaration_resolution(
            target,
            scope_id,
            CandidateRelation::Decorates,
            SemanticRole::Decorator,
            &spelling,
            None,
            Some("decorator"),
            Namespace::Value,
            resolution,
            &["function", "class", "variable"],
        )
    }

    fn emit_bases(&mut self, node: Node<'tree>, scope_id: &str) -> Result<(), EvidenceError> {
        let relation = if node.kind() == "implements_clause" {
            CandidateRelation::Implements
        } else {
            CandidateRelation::Extends
        };
        let mut candidates = Vec::new();
        collect_type_name_nodes(node, &mut candidates);
        for candidate in candidates {
            let spelling = node_text(self.source, candidate);
            let context = if relation == CandidateRelation::Implements {
                "implements"
            } else {
                "extends"
            };
            if let Some(resolution) = self.resolve_type_name_node(scope_id, candidate) {
                if relation == CandidateRelation::Extends
                    && let Some(class_name) = self.enclosing_type(scope_id)
                {
                    let receiver = match &resolution {
                        Resolution::Local(declaration)
                            if matches!(
                                declaration.kind.as_str(),
                                "class" | "interface" | "enum" | "namespace"
                            ) =>
                        {
                            Some(ReceiverTarget {
                                qualified_name: declaration.qualified_name.clone(),
                                import: None,
                                scope_id: self.member_scope_for_declaration(declaration),
                                type_arguments: None,
                            })
                        }
                        Resolution::Import(import) => Some(ReceiverTarget {
                            qualified_name: import_target_without_namespace(&import.target),
                            import: Some(import.clone()),
                            scope_id: None,
                            type_arguments: None,
                        }),
                        _ => None,
                    };
                    if let Some(receiver) = receiver {
                        // A class can have only one direct `extends` target.
                        // Keep the proof only while all observed candidates
                        // agree; declaration merging and malformed trees stay
                        // unresolved rather than selecting traversal order.
                        match self.base_targets.get(&class_name) {
                            Some(previous)
                                if previous.qualified_name != receiver.qualified_name =>
                            {
                                self.base_targets.remove(&class_name);
                            }
                            None => {
                                self.base_targets.insert(class_name, receiver);
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(parent) = candidate.parent()
                    && matches!(
                        parent.kind(),
                        "nested_type_identifier" | "member_expression"
                    )
                    && let Some(module) = parent
                        .child_by_field_name("module")
                        .or_else(|| parent.child_by_field_name("object"))
                    && parent
                        .child_by_field_name("name")
                        .or_else(|| parent.child_by_field_name("property"))
                        .is_some_and(|property| property.id() == candidate.id())
                {
                    self.add_declaration_resolution(
                        candidate,
                        scope_id,
                        CandidateRelation::AccessesMember,
                        SemanticRole::MemberAccess,
                        &spelling,
                        Some(&node_text(self.source, module)),
                        Some("type_member"),
                        Namespace::Type,
                        resolution.clone(),
                        &[
                            "property",
                            "method",
                            "class",
                            "interface",
                            "type_alias",
                            "enum",
                            "external",
                        ],
                    )?;
                }
                self.add_declaration_resolution(
                    candidate,
                    scope_id,
                    relation,
                    SemanticRole::BaseType,
                    &spelling,
                    None,
                    Some(context),
                    Namespace::Type,
                    resolution,
                    &["class", "interface", "type_alias"],
                )?;
            } else if let Some((target, module)) = self.builtin_type_target(scope_id, &spelling) {
                self.add_external_resolution_candidate(
                    candidate,
                    scope_id,
                    relation,
                    SemanticRole::BaseType,
                    &spelling,
                    None,
                    context,
                    &target,
                    &module,
                    None,
                    Vec::new(),
                    &["class", "interface", "type_alias", "external"],
                )?;
            } else {
                self.add_unresolved_candidate(
                    candidate,
                    scope_id,
                    relation,
                    SemanticRole::BaseType,
                    &spelling,
                    None,
                    context,
                    None,
                    Vec::new(),
                    &["class", "interface", "type_alias", "external"],
                    (relation == CandidateRelation::Extends).then_some(
                        HierarchyConstraint::DirectBase {
                            base_set_complete: true,
                        },
                    ),
                )?;
            }
        }
        Ok(())
    }

    fn emit_type_reference(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
    ) -> Result<(), EvidenceError> {
        if self.declaration_name_nodes.contains(&node.start_byte()) {
            return Ok(());
        }
        if node
            .parent()
            .is_some_and(|parent| parent.kind() == "nested_type_identifier")
        {
            // The enclosing nested type carries the complete `Namespace.Type`
            // source anchor. Resolving the trailing `Type` as an unrelated
            // unqualified symbol would fabricate an edge when a same-named
            // declaration happens to be visible in the current scope.
            return Ok(());
        }
        if node.parent().is_some_and(|parent| {
            matches!(
                parent.kind(),
                "extends_clause" | "extends_type_clause" | "implements_clause"
            )
        }) {
            return Ok(());
        }
        let spelling = node_text(self.source, node);
        if node.kind() == "nested_type_identifier"
            && let (Some(module), Some(name)) = (
                node.child_by_field_name("module"),
                node.child_by_field_name("name"),
            )
        {
            let module_spelling = node_text(self.source, module);
            let name_spelling = node_text(self.source, name);
            if let Some(resolution) = self.resolve_type_member_target(scope_id, module, name) {
                self.add_declaration_resolution(
                    name,
                    scope_id,
                    CandidateRelation::AccessesMember,
                    SemanticRole::MemberAccess,
                    &name_spelling,
                    Some(&module_spelling),
                    Some("type_member"),
                    Namespace::Type,
                    resolution.clone(),
                    &[
                        "property",
                        "method",
                        "class",
                        "interface",
                        "type_alias",
                        "enum",
                        "external",
                    ],
                )?;
                return self.add_declaration_resolution(
                    node,
                    scope_id,
                    CandidateRelation::References,
                    SemanticRole::TypeReference,
                    &spelling,
                    Some(&module_spelling),
                    Some("type"),
                    Namespace::Type,
                    resolution,
                    &[
                        "class",
                        "interface",
                        "type_alias",
                        "enum",
                        "type_parameter",
                        "external",
                    ],
                );
            }
            // Keep the qualified member occurrence even when the namespace
            // or property cannot yet be resolved. This is type-space
            // evidence, not permission to select a same-named value or
            // external declaration.
            self.add_unresolved_candidate(
                name,
                scope_id,
                CandidateRelation::AccessesMember,
                SemanticRole::MemberAccess,
                &name_spelling,
                Some(&module_spelling),
                "type_member",
                None,
                Vec::new(),
                &[
                    "property",
                    "method",
                    "class",
                    "interface",
                    "type_alias",
                    "enum",
                    "external",
                ],
                None,
            )?;
        }
        if let Some(resolution) = self.resolve_name(scope_id, &spelling, Namespace::Type) {
            return self.add_declaration_resolution(
                node,
                scope_id,
                CandidateRelation::References,
                SemanticRole::TypeReference,
                &spelling,
                None,
                Some("type"),
                Namespace::Type,
                resolution,
                &["class", "interface", "type_alias", "enum", "type_parameter"],
            );
        }
        if let Some((target, module)) = self.ambient_qualified_type_target(scope_id, &spelling) {
            return self.add_external_resolution_candidate(
                node,
                scope_id,
                CandidateRelation::References,
                SemanticRole::TypeReference,
                &spelling,
                None,
                "type",
                &target,
                &module,
                None,
                Vec::new(),
                &[
                    "class",
                    "interface",
                    "type_alias",
                    "enum",
                    "type_parameter",
                    "external",
                ],
            );
        }
        let Some((target, module)) = self.builtin_type_target(scope_id, &spelling) else {
            return self.add_unresolved_candidate(
                node,
                scope_id,
                CandidateRelation::References,
                SemanticRole::TypeReference,
                &spelling,
                None,
                "type",
                None,
                Vec::new(),
                &[
                    "class",
                    "interface",
                    "type_alias",
                    "enum",
                    "type_parameter",
                    "external",
                ],
                None,
            );
        };
        self.add_external_resolution_candidate(
            node,
            scope_id,
            CandidateRelation::References,
            SemanticRole::TypeReference,
            &spelling,
            None,
            "type",
            &target,
            &module,
            None,
            Vec::new(),
            &[
                "class",
                "interface",
                "type_alias",
                "enum",
                "type_parameter",
                "external",
            ],
        )
    }

    fn emit_callable_reference(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
    ) -> Result<(), EvidenceError> {
        if self.declaration_name_nodes.contains(&node.start_byte())
            || node.parent().is_some_and(|parent| {
                matches!(
                    parent.kind(),
                    "import_clause"
                        | "namespace_import"
                        | "named_imports"
                        | "import_specifier"
                        | "export_clause"
                        | "export_specifier"
                        | "member_expression"
                        | "optional_member_expression"
                        | "subscript_expression"
                        | "jsx_opening_element"
                        | "jsx_self_closing_element"
                )
            })
            || is_type_reference_node(node)
        {
            return Ok(());
        }
        let spelling = node_text(self.source, node);
        let Some(resolution) = self.resolve_name(scope_id, &spelling, Namespace::Value) else {
            return Ok(());
        };
        let is_callable = match &resolution {
            Resolution::Local(declaration) => self.proven_callable_declaration(&declaration.id),
            Resolution::Import(import) => {
                import.imported_name == "default"
                    || (import.imported_name == "*" && import.callable_namespace)
            }
        };
        if !is_callable {
            return Ok(());
        }
        self.add_declaration_resolution(
            node,
            scope_id,
            CandidateRelation::References,
            SemanticRole::CallableReference,
            &spelling,
            None,
            Some("value"),
            Namespace::Value,
            resolution,
            &["function", "method", "class", "variable"],
        )
    }

    fn emit_commonjs_export(
        &mut self,
        node: Node<'tree>,
        scope_id: &str,
    ) -> Result<(), EvidenceError> {
        let Some(left) = node.child_by_field_name("left") else {
            return Ok(());
        };
        let Some((export_name, export_anchor)) = commonjs_export_name(left, self.source) else {
            return Ok(());
        };
        let Some(right) = node.child_by_field_name("right") else {
            return Ok(());
        };
        let right = unwrap_expression_node(right);
        if export_name == "default" && right.kind() == "object" {
            // `module.exports = { ... }` is a default module export plus a
            // bounded set of named properties. The declaration pass already
            // records each spread-free property under the file module's
            // source-qualified owner; publish those exact declarations as
            // reexports instead of leaving `require()` consumers external.
            self.emit_commonjs_module_default_export(scope_id, export_anchor)?;
            if object_literal_has_spread(right) {
                return Ok(());
            }
            let mut cursor = right.walk();
            for property in right
                .named_children(&mut cursor)
                .take(MAX_INLINE_OBJECT_PROPERTIES)
            {
                self.emit_commonjs_object_property_export(scope_id, property)?;
            }
            return Ok(());
        }
        let resolution = self.commonjs_export_value_resolution(scope_id, right);
        let Some(resolution) = resolution else {
            return Ok(());
        };
        self.emit_commonjs_reexport_binding(scope_id, &export_name, export_anchor, resolution)
    }

    fn commonjs_export_value_resolution(
        &self,
        scope_id: &str,
        value: Node<'tree>,
    ) -> Option<Resolution> {
        match value.kind() {
            "identifier" | "type_identifier" | "shorthand_property_identifier" => {
                self.resolve_name(scope_id, &node_text(self.source, value), Namespace::Both)
            }
            "member_expression" | "optional_member_expression" | "subscript_expression" => {
                let property = member_property_node(value)?;
                self.resolve_member_target(
                    scope_id,
                    value.child_by_field_name("object"),
                    property,
                    None,
                    &[],
                )
            }
            _ => None,
        }
    }

    fn emit_commonjs_module_default_export(
        &mut self,
        scope_id: &str,
        anchor: Node<'tree>,
    ) -> Result<(), EvidenceError> {
        let target = self.file_qualified_name.clone();
        let target_declaration_id = self.file_declaration.clone();
        self.emit_commonjs_reexport_target(
            scope_id,
            "default",
            anchor,
            &target,
            Some(&target_declaration_id),
            Namespace::Module,
        )
    }

    fn emit_commonjs_object_property_export(
        &mut self,
        scope_id: &str,
        property: Node<'tree>,
    ) -> Result<(), EvidenceError> {
        let Some((name_node, value)) = commonjs_object_property(property) else {
            return Ok(());
        };
        let Some(export_name) = member_property_name(self.source, name_node) else {
            // Computed or malformed object keys do not prove a stable export
            // spelling. Keep the module's default evidence, but do not emit a
            // guessed named binding.
            return Ok(());
        };
        if export_name.is_empty() || export_name.len() > MAX_TYPE_SHAPE_BYTES {
            return Ok(());
        }
        let resolution = value
            .and_then(|value| self.commonjs_export_value_resolution(scope_id, value))
            .or_else(|| self.commonjs_object_property_declaration(property, &export_name));
        let Some(resolution) = resolution else {
            return Ok(());
        };
        self.emit_commonjs_reexport_binding(scope_id, &export_name, name_node, resolution)
    }

    fn commonjs_object_property_declaration(
        &self,
        property: Node<'tree>,
        property_name: &str,
    ) -> Option<Resolution> {
        let qualified_name = format!("{}.{property_name}", self.file_qualified_name);
        let declarations = self
            .declarations_by_qualified
            .get(&qualified_name)?
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter(|declaration| {
                matches!(
                    declaration.kind.as_str(),
                    "property" | "method" | "function" | "class" | "variable"
                ) && declaration.range_start_byte >= property.start_byte()
                    && declaration.range_start_byte <= property.end_byte()
            })
            .cloned()
            .collect::<Vec<_>>();
        (declarations.len() == 1).then(|| Resolution::Local(declarations[0].clone()))
    }

    fn emit_commonjs_reexport_binding(
        &mut self,
        scope_id: &str,
        export_name: &str,
        anchor: Node<'tree>,
        resolution: Resolution,
    ) -> Result<(), EvidenceError> {
        let (target, target_declaration_id, namespace) = match resolution {
            Resolution::Local(declaration) => (
                declaration.qualified_name,
                Some(declaration.id),
                declaration.namespace,
            ),
            Resolution::Import(import) => (import.target, None, import.namespace),
        };
        self.emit_commonjs_reexport_target(
            scope_id,
            export_name,
            anchor,
            &target,
            target_declaration_id.as_deref(),
            namespace,
        )
    }

    fn emit_commonjs_reexport_target(
        &mut self,
        scope_id: &str,
        export_name: &str,
        anchor: Node<'tree>,
        target: &str,
        target_declaration_id: Option<&str>,
        namespace: Namespace,
    ) -> Result<(), EvidenceError> {
        if export_name.is_empty() || target.is_empty() {
            return Ok(());
        }
        let owner = self.owner_for_scope(scope_id);
        let binding_id = self.builder.bind_with_identity(
            BindingKind::Reexport,
            export_name,
            target,
            target_declaration_id,
            Some(scope_id),
            Some(symbol_namespace(namespace)),
            false,
            range_for_node(self.source_file, anchor),
        )?;
        let occurrence_id = self.builder.occur_with_context(
            SemanticRole::Reexport,
            &owner,
            export_name,
            Some("commonjs"),
            Some(scope_id),
            Some("commonjs"),
            range_for_node(self.source_file, anchor),
        )?;
        self.builder.relate(
            CandidateRelation::Reexports,
            &owner,
            Some(&occurrence_id),
            Some(&binding_id),
            export_name,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                allowed_target_kinds: vec![
                    "module".to_owned(),
                    "function".to_owned(),
                    "class".to_owned(),
                    "variable".to_owned(),
                    "property".to_owned(),
                    "method".to_owned(),
                    "enum".to_owned(),
                    "interface".to_owned(),
                    "type_alias".to_owned(),
                ],
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn resolve_name(&self, scope_id: &str, name: &str, namespace: Namespace) -> Option<Resolution> {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            if let Some(import) = self.import_bindings.get(&(scope.clone(), name.to_owned())) {
                let accepts = if import.namespace == Namespace::Module {
                    matches!(namespace, Namespace::Type | Namespace::Both)
                        || (namespace == Namespace::Value && !import.type_only)
                } else {
                    import.namespace.accepts(namespace)
                };
                if accepts {
                    return Some(Resolution::Import(import.clone()));
                }
            }
            let candidates = self
                .declarations_by_scope
                .get(&scope)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|id| self.declarations.get(id))
                .filter(|declaration| {
                    declaration.name == name
                        && declaration.namespace.accepts(namespace)
                        && self.lexically_visible_unqualified(&scope, declaration)
                })
                .cloned()
                .collect::<Vec<_>>();
            if candidates.len() == 1 {
                return candidates.into_iter().next().map(Resolution::Local);
            }
            if candidates.len() > 1 {
                return None;
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        None
    }

    fn resolve_name_for_call(
        &self,
        scope_id: &str,
        name: &str,
        namespace: Namespace,
        argument_count: Option<u32>,
        argument_types: &[Option<String>],
    ) -> Option<Resolution> {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            if let Some(import) = self.import_bindings.get(&(scope.clone(), name.to_owned())) {
                let accepts = if import.namespace == Namespace::Module {
                    matches!(namespace, Namespace::Type | Namespace::Both)
                        || (namespace == Namespace::Value && !import.type_only)
                } else {
                    import.namespace.accepts(namespace)
                };
                if accepts && (import.imported_name != "*" || import.callable_namespace) {
                    return Some(Resolution::Import(import.clone()));
                }
            }
            let candidates = self
                .declarations_by_scope
                .get(&scope)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|id| self.declarations.get(id))
                .filter(|declaration| {
                    declaration.name == name
                        && declaration.namespace.accepts(namespace)
                        && self.lexically_visible_unqualified(&scope, declaration)
                })
                .cloned()
                .collect::<Vec<_>>();
            if let Some(argument_count) = argument_count {
                let matching = candidates
                    .iter()
                    .filter(|declaration| {
                        self.declaration_call_matches(declaration, argument_count, argument_types)
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if matching.len() == 1 {
                    return matching.into_iter().next().map(Resolution::Local);
                }
                // A source-visible signature that does not accept this call
                // shape is a negative result. Optional/default/rest ranges
                // are accepted only when they are the sole visible match;
                // declaration-merged overload ambiguity remains unresolved.
                if !candidates.is_empty() {
                    return None;
                }
            } else if candidates.len() == 1 {
                return candidates.into_iter().next().map(Resolution::Local);
            }
            if !candidates.is_empty() {
                return None;
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        None
    }

    fn lexically_visible_unqualified(&self, scope_id: &str, declaration: &DeclarationInfo) -> bool {
        // Object-literal and assignment properties are structural members, not
        // lexical bindings. Treating `{ gzip: gzip(body) }` as a declaration
        // named `gzip` would shadow the real helper in the surrounding scope
        // and produce a false call target. Member resolution reaches these
        // declarations through their qualified receiver instead.
        if matches!(
            declaration.kind.as_str(),
            "property" | "method" | "constructor"
        ) {
            return false;
        }
        let Some(owner_id) = self.scope_owners.get(scope_id) else {
            return true;
        };
        let Some(owner) = self.declarations.get(owner_id) else {
            return true;
        };
        if owner.kind != "class" {
            return true;
        }
        // Class methods and fields are members, not lexical bindings inside
        // another method body. Without this boundary `parseParameters()` in
        // a static method could resolve to `Class.parseParameters` instead of
        // the same-file top-level helper with the same spelling.
        !matches!(
            declaration.kind.as_str(),
            "method" | "constructor" | "property"
        )
    }

    fn declaration_call_matches(
        &self,
        declaration: &DeclarationInfo,
        argument_count: u32,
        argument_types: &[Option<String>],
    ) -> bool {
        self.declaration_call_matches_seen(
            declaration,
            argument_count,
            argument_types,
            &mut HashSet::new(),
        )
    }

    fn declaration_call_matches_seen(
        &self,
        declaration: &DeclarationInfo,
        argument_count: u32,
        argument_types: &[Option<String>],
        seen: &mut HashSet<String>,
    ) -> bool {
        if !seen.insert(declaration.id.clone()) {
            return false;
        }
        // A direct identifier call or construction is already source
        // evidence for the binding being used as a callable/constructor
        // value.  The TypeScript checker resolves the identifier's symbol
        // even when its inferred signature is supplied by another module or
        // is only available through contextual typing (for example Promise
        // executor parameters and factory results).  Keep duplicate-name
        // ambiguity fail-closed in the caller; once this declaration is the
        // unique visible candidate, do not discard it for lack of a local
        // signature.
        if matches!(declaration.kind.as_str(), "variable" | "parameter") {
            return true;
        }
        if declaration.kind == "class" && !declaration.explicit_constructor {
            if let Some(base) = self.base_targets.get(&declaration.qualified_name)
                && base.import.is_none()
                && let Some(ids) = self.declarations_by_qualified.get(&base.qualified_name)
            {
                let bases = ids
                    .iter()
                    .filter_map(|id| self.declarations.get(id))
                    .filter(|candidate| candidate.kind == "class")
                    .collect::<Vec<_>>();
                if bases.len() == 1 {
                    return self.declaration_call_matches_seen(
                        bases[0],
                        argument_count,
                        argument_types,
                        seen,
                    );
                }
            }
            // A class without an explicit constructor has a zero-argument
            // default constructor in TypeScript. JavaScript also permits
            // extra arguments, matching ordinary function-call behavior.
            return self.language == "javascript" || argument_count == 0;
        }
        if declaration.callable_shape
            && declaration.parameter_count.is_none()
            && declaration.parameter_min_count.is_none()
            && declaration.parameter_max_count.is_none()
        {
            return true;
        }
        if self.callable_property_aliases.contains(&declaration.id) {
            return true;
        }
        let arity_matches = if self.language == "javascript" {
            // JavaScript permits both omitted and extra arguments for
            // ordinary functions. Resolve the unique lexical declaration and
            // leave duplicate-name ambiguity unresolved below.
            true
        } else if let Some(parameter_count) = declaration.parameter_count {
            parameter_count == argument_count
        } else {
            declaration
                .parameter_min_count
                .is_some_and(|minimum| argument_count >= minimum)
                && declaration
                    .parameter_max_count
                    .is_none_or(|maximum| argument_count <= maximum)
        };
        arity_matches && self.arguments_match_parameters(declaration, argument_types)
    }

    fn arguments_match_parameters(
        &self,
        declaration: &DeclarationInfo,
        argument_types: &[Option<String>],
    ) -> bool {
        let Some(parameter_types) = declaration.parameter_types.as_ref() else {
            return true;
        };
        // JavaScript permits omitted and extra arguments, and parameter
        // annotations are not part of the runtime contract. Preserve the
        // unique source declaration selected by lexical/member resolution
        // without rejecting it on a static type-shape comparison.
        if self.language == "javascript" {
            return true;
        }
        if parameter_types.len() != argument_types.len() {
            return false;
        }
        parameter_types
            .iter()
            .zip(argument_types)
            .all(|(parameter, argument)| {
                argument.as_deref().is_none_or(|argument| {
                    self.parameter_type_matches(declaration, parameter, argument)
                })
            })
    }

    fn parameter_type_matches(
        &self,
        declaration: &DeclarationInfo,
        parameter: &str,
        argument: &str,
    ) -> bool {
        if argument == "function"
            && self
                .contextual_callable_parameter_types(&declaration.scope_id, parameter)
                .is_some()
        {
            return true;
        }
        if parameter_type_matches(parameter, argument) {
            return true;
        }
        if self.source_type_assignable(&declaration.scope_id, parameter, argument) {
            return true;
        }
        if let Some(constraint) = self
            .generic_parameters_by_declaration
            .get(&declaration.id)
            .and_then(|parameters| parameters.get(parameter.trim()))
        {
            return constraint
                .as_deref()
                .is_none_or(|constraint| self.generic_constraint_matches(constraint, argument));
        }
        if let Some((utility, base, arguments)) = utility_type_parts(parameter) {
            return self.source_utility_argument_matches(utility, base, &arguments, argument);
        }
        self.source_inline_object_argument_matches(parameter, argument)
    }

    /// Follow source-declared `extends`/`implements` edges for one nominal
    /// argument. This is intentionally narrower than a full TypeScript
    /// assignability engine: it proves only a unique local argument class and
    /// a bounded hierarchy path to the expected imported/local type.
    fn source_type_assignable(&self, scope_id: &str, expected: &str, actual: &str) -> bool {
        let Some(expected_receiver) =
            self.resolve_declared_type_receiver(scope_id, strip_type_arguments(expected))
        else {
            return false;
        };
        let expected_name = expected_receiver.qualified_name;
        let mut pending = vec![strip_type_arguments(actual).to_owned()];
        let mut seen = HashSet::new();
        let mut steps = 0_usize;
        while let Some(current) = pending.pop() {
            if steps >= MAX_INLINE_OBJECT_PROPERTIES
                || current.is_empty()
                || !seen.insert(current.clone())
            {
                break;
            }
            steps = steps.saturating_add(1);
            if current == expected_name
                || type_names_compatible(&current, &expected_name)
                    && (!current.contains("::") || !expected_name.contains("::"))
            {
                return true;
            }
            if let Some(base) = self.base_targets.get(&current)
                && base.import.is_none()
            {
                pending.push(base.qualified_name.clone());
            } else if let Some(base) = self.base_targets.get(&current)
                && base.qualified_name == expected_name
            {
                return true;
            }
            if let Some(bases) = self.implements_targets.get(&current) {
                for base in bases {
                    if base.qualified_name == expected_name {
                        return true;
                    }
                    if base.import.is_none() {
                        pending.push(base.qualified_name.clone());
                    }
                }
            }
        }
        false
    }

    fn generic_constraint_matches(&self, constraint: &str, argument: &str) -> bool {
        if argument == "any" || argument == "unknown" {
            return true;
        }
        if parameter_type_matches(constraint, argument) {
            return true;
        }
        let constraint = constraint.trim();
        if (constraint.starts_with('[') && constraint.contains("..."))
            || constraint.ends_with("[]")
            || constraint.starts_with("readonly ") && constraint.ends_with("]")
        {
            return argument == "array";
        }
        is_object_like_type(constraint) && self.source_type_property_names(argument).is_some()
    }

    fn source_utility_argument_matches(
        &self,
        utility: &str,
        base: &str,
        arguments: &[&str],
        argument: &str,
    ) -> bool {
        let Some(argument_properties) = self.source_type_property_names(argument) else {
            return false;
        };
        let Some(base_properties) = self.source_type_property_names(base) else {
            return false;
        };
        match utility {
            "Pick" => {
                let Some(selected) = arguments.get(1).and_then(|keys| type_literal_names(keys))
                else {
                    return false;
                };
                selected
                    .iter()
                    .all(|property| argument_properties.contains(property))
            }
            "Omit" => {
                let omitted = arguments
                    .get(1)
                    .and_then(|keys| type_literal_names(keys))
                    .unwrap_or_default();
                base_properties
                    .iter()
                    .filter(|property| !omitted.contains(*property))
                    .all(|property| argument_properties.contains(property))
            }
            "Partial" => argument_properties
                .iter()
                .all(|property| base_properties.contains(property)),
            "Required" | "Readonly" => base_properties
                .iter()
                .all(|property| argument_properties.contains(property)),
            _ => false,
        }
    }

    fn source_inline_object_argument_matches(&self, parameter: &str, argument: &str) -> bool {
        let Some(required_properties) = inline_object_required_property_names(parameter) else {
            return false;
        };
        let Some(argument_properties) = self.source_type_property_names(argument) else {
            return false;
        };
        required_properties
            .iter()
            .all(|property| argument_properties.contains(property))
    }

    fn source_type_property_names(&self, type_name: &str) -> Option<HashSet<String>> {
        let type_name = type_name.trim().trim_end_matches("[]").trim();
        if type_name.is_empty() {
            return None;
        }
        let mut candidates = Vec::new();
        if let Some(ids) = self.declarations_by_qualified.get(type_name) {
            candidates.extend(ids.iter().filter_map(|id| self.declarations.get(id)));
        } else {
            let leaf = type_name.rsplit(['.', ':']).next().unwrap_or(type_name);
            candidates.extend(
                self.declarations
                    .values()
                    .filter(|declaration| declaration.name == leaf),
            );
        }
        candidates.sort_by(|left, right| left.id.cmp(&right.id));
        candidates.dedup_by(|left, right| left.id == right.id);
        let mut qualified_names = candidates
            .into_iter()
            .filter(|declaration| {
                matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "namespace" | "type_alias" | "variable"
                )
            })
            .map(|declaration| declaration.qualified_name.clone())
            .collect::<Vec<_>>();
        qualified_names.sort();
        qualified_names.dedup();
        let [qualified_name] = qualified_names.as_slice() else {
            return None;
        };
        let prefix = format!("{qualified_name}.");
        let mut properties = HashSet::new();
        for name in self.declarations_by_qualified.keys() {
            let Some(property) = name.strip_prefix(&prefix) else {
                continue;
            };
            if property.is_empty() || property.contains('.') {
                continue;
            }
            properties.insert(property.to_owned());
            if properties.len() >= MAX_INLINE_OBJECT_PROPERTIES {
                break;
            }
        }
        (!properties.is_empty()).then_some(properties)
    }

    fn resolve_member_target(
        &self,
        scope_id: &str,
        object: Option<Node<'tree>>,
        property: Node<'tree>,
        argument_count: Option<u32>,
        argument_types: &[Option<String>],
    ) -> Option<Resolution> {
        let object = object?;
        let property_name = member_property_name(self.source, property)?;
        let nominal_write_receiver = self.nominal_member_write_receiver(scope_id, object, property);
        let nominal_write = nominal_write_receiver.is_some();
        let receiver = nominal_write_receiver.or_else(|| self.receiver_target(scope_id, object))?;
        if !nominal_write
            && self.flow_member_write_barrier_before(
                scope_id,
                object,
                &property_name,
                property.start_byte(),
            )
        {
            return None;
        }
        let projection = decode_utility_projection(&receiver.qualified_name);
        if let Some((utility, _, keys)) = projection.as_ref()
            && ((utility == "Pick" && !keys.contains(&property_name))
                || (utility == "Omit" && keys.contains(&property_name)))
        {
            return None;
        }
        let mut lookup_receiver = receiver.clone();
        if let Some((_, base, _)) = projection.as_ref() {
            lookup_receiver.qualified_name.clone_from(base);
        }
        // A member value can carry a source-visible nominal type. Resolve
        // that type only when the value is used as a receiver; keeping this
        // bridge out of `receiver_target` avoids recursively re-typing every
        // intermediate member expression in a long chain.
        let receiver = self
            .typed_member_receiver(scope_id, lookup_receiver.clone())
            .unwrap_or(lookup_receiver);
        if let Some(name_node) = rightmost_identifier(object)
            && let Some(Resolution::Local(variable)) = self.resolve_name(
                scope_id,
                &node_text(self.source, name_node),
                Namespace::Value,
            )
            && let Some(declaration_id) = self
                .inline_object_property_declaration_ids
                .get(&(variable.id.clone(), property_name.clone()))
            && let Some(declaration) = self.declarations.get(declaration_id)
        {
            return Some(Resolution::Local(declaration.clone()));
        }
        let namespace_import = receiver
            .import
            .as_ref()
            .is_some_and(|import| import.namespace == Namespace::Module);
        let qualified_name = typescript_member_qualified_name(
            &receiver.qualified_name,
            receiver.type_arguments.as_deref(),
            &property_name,
            namespace_import,
            receiver.import.is_some(),
        );
        if let Some(ids) = self.declarations_by_qualified.get(&qualified_name) {
            let flow_sensitive = self.receiver_is_flow_sensitive(&receiver);
            let all_declarations = ids
                .iter()
                .filter_map(|id| self.declarations.get(id))
                .collect::<Vec<_>>();
            // Qualified names intentionally remain stable across lexical
            // scopes, so duplicate object literals/classes can share one
            // spelling.  Prefer declarations owned by the receiver's scope
            // (including nested scopes), but retain a fail-closed fallback for
            // external/prototype cases where no local scope proof exists.
            let scoped_declarations = receiver.scope_id.as_deref().map(|receiver_scope| {
                all_declarations
                    .iter()
                    .filter(|declaration| {
                        self.scope_is_descendant_or_same(&declaration.scope_id, receiver_scope)
                    })
                    .copied()
                    .collect::<Vec<_>>()
            });
            let declarations = scoped_declarations
                .filter(|declarations| !declarations.is_empty())
                .unwrap_or(all_declarations);
            let declarations = if !flow_sensitive {
                if let Some(receiver_scope) = receiver.scope_id.as_deref() {
                    // Prefer the nearest source declaration for a receiver.  A
                    // class field/property lives in the class scope, whereas a
                    // later `this.field = ...` write lives in a method scope. The
                    // checker binds both uses to the class field, so selecting the
                    // minimum lexical distance avoids a later assignment
                    // declaration stealing the canonical member identity.
                    let nearest = declarations
                        .iter()
                        .filter_map(|declaration| {
                            self.scope_distance(&declaration.scope_id, receiver_scope)
                        })
                        .min();
                    if let Some(distance) = nearest {
                        declarations
                            .into_iter()
                            .filter(|declaration| {
                                self.scope_distance(&declaration.scope_id, receiver_scope)
                                    == Some(distance)
                            })
                            .collect::<Vec<_>>()
                    } else {
                        declarations
                    }
                } else {
                    declarations
                }
            } else {
                declarations
            };
            let declarations = if flow_sensitive {
                // Object-valued JavaScript variables can be reassigned.  For
                // a use after multiple source-visible object writes, the
                // checker binds the member to the latest write that dominates
                // the use.  Apply this only to structural receivers: using
                // source order for nominal classes/interfaces would turn
                // methods or declarations in a different lexical scope into
                // accidental shadowing.
                let preceding = declarations
                    .iter()
                    .copied()
                    .filter(|declaration| {
                        declaration.kind == "property"
                            && declaration.range_start_byte <= property.start_byte()
                    })
                    .collect::<Vec<_>>();
                if let Some(latest) = preceding
                    .iter()
                    .max_by_key(|declaration| declaration.range_start_byte)
                    .copied()
                {
                    vec![latest]
                } else {
                    declarations
                }
            } else {
                declarations
            };
            if let Some(argument_count) = argument_count {
                let matching = declarations
                    .iter()
                    .filter(|declaration| {
                        self.declaration_call_matches(declaration, argument_count, argument_types)
                    })
                    .collect::<Vec<_>>();
                if matching.len() == 1 {
                    return matching
                        .first()
                        .map(|declaration| Resolution::Local((**declaration).clone()));
                }
                let matching_properties = matching
                    .iter()
                    .filter(|declaration| declaration.kind == "property")
                    .collect::<Vec<_>>();
                if matching_properties.len() == 1 {
                    return matching_properties
                        .first()
                        .map(|declaration| Resolution::Local((***declaration).clone()));
                }
                let typed_properties = declarations
                    .iter()
                    .filter(|declaration| {
                        declaration.kind == "property" && declaration.declared_type_name.is_some()
                    })
                    .collect::<Vec<_>>();
                if typed_properties.len() == 1 {
                    return typed_properties
                        .first()
                        .map(|declaration| Resolution::Local((**declaration).clone()));
                }
                if !declarations.is_empty() {
                    return None;
                }
            } else {
                let properties = declarations
                    .iter()
                    .filter(|declaration| declaration.kind == "property")
                    .collect::<Vec<_>>();
                if properties.len() == 1 {
                    return properties
                        .first()
                        .map(|declaration| Resolution::Local((**declaration).clone()));
                }
                if declarations.len() == 1 {
                    return declarations
                        .first()
                        .map(|declaration| Resolution::Local((*declaration).clone()));
                }
            }
            // Overloads/declaration merging share a qualified name but are
            // distinct source declarations. Preserve the ambiguity instead
            // of selecting one by traversal order.
            return None;
        }
        if receiver.import.is_none()
            && let Some(base_receiver) = receiver.qualified_name.strip_suffix(".prototype")
        {
            // Prototype methods commonly read fields initialized by the
            // constructor (`this._pairs`). The constructor assignment is the
            // canonical source declaration, so try that exact base identity
            // only after no prototype member matched. Never fall back through
            // a dynamic or imported receiver.
            let base_name = format!("{base_receiver}.{property_name}");
            if let Some(ids) = self.declarations_by_qualified.get(&base_name) {
                let declarations = ids
                    .iter()
                    .filter_map(|id| self.declarations.get(id))
                    .collect::<Vec<_>>();
                if let Some(argument_count) = argument_count {
                    let matching = declarations
                        .iter()
                        .filter(|declaration| {
                            self.declaration_call_matches(
                                declaration,
                                argument_count,
                                argument_types,
                            )
                        })
                        .collect::<Vec<_>>();
                    if matching.len() == 1 {
                        return matching
                            .first()
                            .map(|declaration| Resolution::Local((**declaration).clone()));
                    }
                } else if declarations.len() == 1 {
                    return declarations
                        .first()
                        .map(|declaration| Resolution::Local((*declaration).clone()));
                }
            }
        }
        if receiver.import.is_none()
            && let Some(inherited) = self.resolve_inherited_member_target(
                &receiver.qualified_name,
                &property_name,
                argument_count,
                argument_types,
            )
        {
            return Some(inherited);
        }
        let import = receiver.import?;
        Some(Resolution::Import(ImportInfo {
            binding_id: import.binding_id,
            target: qualified_name,
            module: import.module,
            imported_name: property_name,
            namespace: Namespace::Value,
            type_only: import.type_only,
            callable_namespace: false,
        }))
    }

    /// A plain assignment to a source-proven nominal receiver is itself a
    /// member occurrence. Keep the declaration target for that write while
    /// retaining property-scoped barriers for later reads. Structural object
    /// writes stay conservative because a write can replace the only source
    /// value that gave the property its identity.
    fn nominal_member_write_receiver(
        &self,
        scope_id: &str,
        object: Node<'tree>,
        property: Node<'tree>,
    ) -> Option<ReceiverTarget> {
        let member = property.parent()?;
        if !matches!(
            member.kind(),
            "member_expression" | "optional_member_expression" | "subscript_expression"
        ) {
            return None;
        }
        let assignment = member.parent()?;
        if assignment.kind() != "assignment_expression"
            || assignment
                .child_by_field_name("left")
                .is_none_or(|left| left.id() != member.id())
        {
            return None;
        }
        let right = assignment.child_by_field_name("right")?;
        let operator = self
            .source
            .get(member.end_byte()..right.start_byte())
            .and_then(|bytes| std::str::from_utf8(bytes).ok())?;
        if operator.trim() != "=" {
            return None;
        }
        if let Some(receiver) = self.receiver_target(scope_id, object)
            && receiver.import.is_none()
            && !self.receiver_is_flow_sensitive(&receiver)
        {
            return Some(receiver);
        }
        if !matches!(
            object.kind(),
            "identifier" | "type_identifier" | "jsx_identifier"
        ) {
            return None;
        }
        let name = rightmost_identifier(object)?;
        let Resolution::Local(variable) =
            self.resolve_name(scope_id, &node_text(self.source, name), Namespace::Value)?
        else {
            return None;
        };
        if !self.immutable_bindings.contains(&variable.id) {
            return None;
        }
        let receiver = self
            .flow_assignments
            .get(&variable.id)
            .into_iter()
            .flat_map(|assignments| assignments.iter())
            .filter(|assignment| assignment.start_byte <= property.start_byte())
            .max_by_key(|assignment| assignment.start_byte)
            .map(|assignment| assignment.receiver.clone())?;
        (receiver.import.is_none()
            && !self.receiver_is_flow_sensitive(&receiver)
            && !receiver.qualified_name.is_empty())
        .then_some(receiver)
    }

    fn receiver_is_flow_sensitive(&self, receiver: &ReceiverTarget) -> bool {
        receiver.import.is_none()
            && self
                .structural_object_variables
                .contains(&receiver.qualified_name)
    }

    fn resolve_inherited_member_target(
        &self,
        receiver_qualified_name: &str,
        property_name: &str,
        argument_count: Option<u32>,
        argument_types: &[Option<String>],
    ) -> Option<Resolution> {
        let mut current = receiver_qualified_name.to_owned();
        let mut seen = HashSet::new();
        while seen.insert(current.clone()) {
            let qualified_names = [
                format!("{current}.{property_name}"),
                format!("{current}.constructor.{property_name}"),
            ];
            for qualified_name in qualified_names {
                let Some(ids) = self.declarations_by_qualified.get(&qualified_name) else {
                    continue;
                };
                let declarations = ids
                    .iter()
                    .filter_map(|id| self.declarations.get(id))
                    .filter(|declaration| {
                        argument_count.is_none_or(|count| {
                            self.declaration_call_matches(declaration, count, argument_types)
                        })
                    })
                    .collect::<Vec<_>>();
                // Class constructors often bind a method onto `this`
                // (`this.optional = this.optional.bind(this)`).  That
                // assignment publishes a duplicate property spelling, but
                // the source member still resolves to the one declared
                // method. Prefer that unique method only when all other
                // matching declarations are properties; overloads and
                // unrelated duplicates stay ambiguous.
                let methods = declarations
                    .iter()
                    .filter(|declaration| {
                        matches!(declaration.kind.as_str(), "method" | "constructor")
                    })
                    .collect::<Vec<_>>();
                if methods.len() == 1
                    && declarations
                        .iter()
                        .any(|declaration| declaration.kind == "property")
                    && declarations.iter().all(|declaration| {
                        declaration.kind == "property"
                            || matches!(declaration.kind.as_str(), "method" | "constructor")
                    })
                {
                    return methods
                        .first()
                        .map(|declaration| Resolution::Local((***declaration).clone()));
                }
                if declarations.len() == 1 {
                    return declarations
                        .first()
                        .map(|declaration| Resolution::Local((*declaration).clone()));
                }
                if argument_count.is_some() {
                    let typed_properties = ids
                        .iter()
                        .filter_map(|id| self.declarations.get(id))
                        .filter(|declaration| {
                            declaration.kind == "property"
                                && declaration.declared_type_name.is_some()
                        })
                        .collect::<Vec<_>>();
                    if typed_properties.len() == 1 {
                        return typed_properties
                            .first()
                            .map(|declaration| Resolution::Local((*declaration).clone()));
                    }
                }
            }
            let base = self.base_targets.get(&current)?;
            if base.import.is_some() {
                return None;
            }
            current = base.qualified_name.clone();
        }
        None
    }

    fn resolve_type_member_target(
        &self,
        scope_id: &str,
        module: Node<'tree>,
        property: Node<'tree>,
    ) -> Option<Resolution> {
        let module_name = node_text(self.source, module);
        let property_name = node_text(self.source, property);
        if module_name.is_empty() || property_name.is_empty() {
            return None;
        }
        let resolution = self.resolve_type_space_name(scope_id, &module_name)?;
        match resolution {
            Resolution::Local(declaration) => {
                let qualified_name = format!("{}.{property_name}", declaration.qualified_name);
                let declarations = self
                    .declarations_by_qualified
                    .get(&qualified_name)?
                    .iter()
                    .filter_map(|id| self.declarations.get(id))
                    .collect::<Vec<_>>();
                if declarations.len() == 1 {
                    declarations
                        .first()
                        .map(|candidate| Resolution::Local((*candidate).clone()))
                } else {
                    None
                }
            }
            Resolution::Import(import) => Some(Resolution::Import(ImportInfo {
                binding_id: import.binding_id,
                target: format!(
                    "{}::{property_name}",
                    import_target_without_namespace(&import.target)
                ),
                module: import.module,
                imported_name: property_name,
                namespace: Namespace::Type,
                type_only: import.type_only,
                callable_namespace: false,
            })),
        }
    }

    fn resolve_type_name_node(&self, scope_id: &str, node: Node<'tree>) -> Option<Resolution> {
        if let Some(parent) = node.parent()
            && matches!(
                parent.kind(),
                "nested_type_identifier" | "member_expression"
            )
            && let Some(module) = parent
                .child_by_field_name("module")
                .or_else(|| parent.child_by_field_name("object"))
            && parent
                .child_by_field_name("name")
                .or_else(|| parent.child_by_field_name("property"))
                .is_some_and(|property| property.id() == node.id())
        {
            return self.resolve_type_member_target(scope_id, module, node);
        }
        self.resolve_type_space_name(scope_id, &node_text(self.source, node))
            .or_else(|| {
                // TypeScript heritage expressions are value-space
                // constructor targets even though their syntax is carried by
                // a type-like node.  This is common for mixin factories such
                // as `const Parent = params?.Parent ?? Object; class D extends
                // Parent {}`.  Keep the value fallback scoped to `extends`;
                // ordinary type references remain fail-closed.
                let mut current = node.parent();
                while let Some(candidate) = current {
                    if matches!(
                        candidate.kind(),
                        "extends_clause" | "extends_type_clause" | "class_heritage"
                    ) {
                        return self.resolve_name(
                            scope_id,
                            &node_text(self.source, node),
                            Namespace::Value,
                        );
                    }
                    current = candidate.parent();
                }
                None
            })
    }

    fn resolve_type_space_name(&self, scope_id: &str, name: &str) -> Option<Resolution> {
        self.resolve_name(scope_id, name, Namespace::Type)
            .or_else(|| self.resolve_merged_type_name(scope_id, name))
            .or_else(|| {
                // JavaScript classes and imports occupy the runtime value
                // space, but `extends` is still type-like evidence for the
                // graph. Permit this narrowly for JavaScript; TypeScript
                // keeps its value/type namespaces fail-closed.
                (self.language == "javascript")
                    .then(|| self.resolve_name(scope_id, name, Namespace::Value))
                    .flatten()
            })
    }

    fn resolve_merged_type_name(&self, scope_id: &str, name: &str) -> Option<Resolution> {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            let candidates = self
                .declarations_by_scope
                .get(&scope)
                .into_iter()
                .flat_map(|ids| ids.iter())
                .filter_map(|id| self.declarations.get(id))
                .filter(|declaration| {
                    declaration.name == name
                        && declaration.namespace.accepts(Namespace::Type)
                        && self.lexically_visible_unqualified(&scope, declaration)
                        && matches!(
                            declaration.kind.as_str(),
                            "class" | "interface" | "enum" | "namespace" | "type_alias"
                        )
                })
                .collect::<Vec<_>>();
            let mut qualified_names = candidates
                .iter()
                .map(|declaration| declaration.qualified_name.as_str())
                .collect::<Vec<_>>();
            qualified_names.sort_unstable();
            qualified_names.dedup();
            if qualified_names.len() == 1 {
                // Interface/namespace declaration merging is one logical
                // type-space symbol. Prefer the interface/type declaration as
                // the representative; member lookup proceeds by qualified
                // name and therefore retains merged namespace members.
                let mut candidates = candidates;
                candidates.sort_by_key(|declaration| {
                    (
                        match declaration.kind.as_str() {
                            "interface" => 0_u8,
                            "type_alias" => 1,
                            "class" => 2,
                            "enum" => 3,
                            "namespace" => 4,
                            _ => 5,
                        },
                        declaration.id.as_str(),
                    )
                });
                return candidates
                    .first()
                    .map(|declaration| Resolution::Local((*declaration).clone()));
            }
            if !candidates.is_empty() {
                return None;
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        None
    }

    fn resolve_super_target(&self, scope_id: &str) -> Option<Resolution> {
        let class_name = self.enclosing_type(scope_id)?;
        let base = self.base_targets.get(&class_name)?.clone();
        if let Some(import) = base.import {
            return Some(Resolution::Import(import));
        }
        let declarations = self
            .declarations_by_qualified
            .get(&base.qualified_name)?
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter(|declaration| declaration.kind == "class")
            .cloned()
            .collect::<Vec<_>>();
        (declarations.len() == 1).then(|| Resolution::Local(declarations[0].clone()))
    }

    /// Narrow a local discriminated-union variable inside the positive branch
    /// of a direct property guard (`if (value.success) { value.data }`).
    /// The guard is intentionally syntax- and scope-bounded: negated,
    /// compound, reassigned, and cross-function control flow remains
    /// unresolved instead of being inferred from spelling alone.
    fn flow_narrowed_union_receiver(
        &self,
        scope_id: &str,
        object: Node<'tree>,
        declaration: &DeclarationInfo,
    ) -> Option<ReceiverTarget> {
        let variable_type = self.variable_types.get(&declaration.id)?;
        let targets = self.type_alias_union_targets.get(variable_type)?;
        let variable_name = declaration.name.as_str();
        let guard_property = self.positive_property_guard(object, variable_name)?;
        let mut matches = Vec::new();
        let mut literal_matches = Vec::new();
        for target_name in targets {
            let receiver = self.resolve_declared_type_receiver(scope_id, target_name)?;
            if receiver.import.is_some() {
                continue;
            }
            let qualified_name = format!("{}.{}", receiver.qualified_name, guard_property);
            let Some(ids) = self.declarations_by_qualified.get(&qualified_name) else {
                continue;
            };
            let literal_true = ids.iter().any(|id| {
                self.declarations
                    .get(id)
                    .and_then(|declaration| {
                        self.property_literal_values
                            .get(&declaration.qualified_name)
                    })
                    .is_some_and(|value| value == "true")
            });
            if literal_true {
                literal_matches.push(receiver.clone());
            }
            if ids.len() == 1 {
                matches.push(receiver);
            }
        }
        if let [receiver] = literal_matches.as_slice() {
            return Some(receiver.clone());
        }
        let [receiver] = matches.as_slice() else {
            return None;
        };
        Some(receiver.clone())
    }

    /// Narrow a local union by a positive literal `in` guard
    /// (`if ("run" in value) { value.run() }`). Only a unique union
    /// constituent that declares the guarded property is accepted; if two
    /// constituents expose it, the runtime test does not choose one and the
    /// receiver remains unresolved.
    fn flow_narrowed_in_receiver(
        &self,
        scope_id: &str,
        object: Node<'tree>,
        declaration: &DeclarationInfo,
    ) -> Option<ReceiverTarget> {
        let variable_type = self.variable_types.get(&declaration.id)?;
        let targets = self.type_alias_union_targets.get(variable_type)?;
        let guard_property = self.positive_in_property_guard(object, declaration.name.as_str())?;
        let mut matches = Vec::new();
        for target_name in targets {
            let receiver = self.resolve_declared_type_receiver(scope_id, target_name)?;
            if receiver.import.is_some() {
                continue;
            }
            let qualified_name = format!("{}.{}", receiver.qualified_name, guard_property);
            let Some(ids) = self.declarations_by_qualified.get(&qualified_name) else {
                continue;
            };
            if ids.len() == 1 {
                matches.push(receiver);
            }
        }
        let [receiver] = matches.as_slice() else {
            return None;
        };
        Some(receiver.clone())
    }

    /// Narrow a local union by a direct string-literal discriminant guard
    /// (`if (value.kind === "ready") { value.data }`).  This is deliberately
    /// limited to a single strict equality in the positive branch and to
    /// unique source-declared literal properties on the union members.
    fn flow_narrowed_discriminant_receiver(
        &self,
        scope_id: &str,
        object: Node<'tree>,
        declaration: &DeclarationInfo,
    ) -> Option<ReceiverTarget> {
        let variable_type = self.variable_types.get(&declaration.id)?;
        let targets = self.type_alias_union_targets.get(variable_type)?;
        let (guard_property, guard_value) =
            self.positive_discriminant_guard(object, declaration.name.as_str())?;
        let mut matches = Vec::new();
        for target_name in targets {
            let receiver = self.resolve_declared_type_receiver(scope_id, target_name)?;
            if receiver.import.is_some() {
                continue;
            }
            let qualified_name = format!("{}.{}", receiver.qualified_name, guard_property);
            let Some(ids) = self.declarations_by_qualified.get(&qualified_name) else {
                continue;
            };
            if ids.iter().any(|id| {
                self.declarations
                    .get(id)
                    .and_then(|declaration| {
                        self.property_literal_values
                            .get(&declaration.qualified_name)
                    })
                    .is_some_and(|value| value == &guard_value)
            }) {
                matches.push(receiver);
            }
        }
        let [receiver] = matches.as_slice() else {
            return None;
        };
        Some(receiver.clone())
    }

    fn positive_property_guard(&self, object: Node<'tree>, variable_name: &str) -> Option<String> {
        let mut current = object.parent();
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            let Some(ancestor) = current else {
                break;
            };
            if ancestor.kind() == "if_statement"
                && let Some(consequence) = ancestor.child_by_field_name("consequence")
                && node_is_descendant_or_same(object, consequence)
                && let Some(condition) = ancestor.child_by_field_name("condition")
            {
                let text = node_text(self.source, condition);
                let compact = text
                    .chars()
                    .filter(|character| !character.is_ascii_whitespace())
                    .collect::<String>();
                let compact = compact
                    .strip_prefix('(')
                    .and_then(|value| value.strip_suffix(')'))
                    .unwrap_or(&compact);
                let prefix = format!("{variable_name}.");
                if let Some(property) = compact.strip_prefix(&prefix)
                    && !property.is_empty()
                    && property.chars().all(|character| {
                        character == '_' || character == '$' || character.is_ascii_alphanumeric()
                    })
                {
                    return Some(property.to_owned());
                }
            }
            current = ancestor.parent();
        }
        None
    }

    fn positive_in_property_guard(
        &self,
        object: Node<'tree>,
        variable_name: &str,
    ) -> Option<String> {
        let mut current = object.parent();
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            let Some(ancestor) = current else {
                break;
            };
            if ancestor.kind() == "if_statement"
                && let Some(consequence) = ancestor.child_by_field_name("consequence")
                && node_is_descendant_or_same(object, consequence)
                && let Some(condition) = ancestor.child_by_field_name("condition")
            {
                let compact = node_text(self.source, condition)
                    .chars()
                    .filter(|character| !character.is_ascii_whitespace())
                    .collect::<String>();
                let compact = compact
                    .strip_prefix('(')
                    .and_then(|value| value.strip_suffix(')'))
                    .unwrap_or(&compact);
                let suffix = format!("in{variable_name}");
                let Some(literal) = compact.strip_suffix(&suffix) else {
                    current = ancestor.parent();
                    continue;
                };
                let property = literal
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .or_else(|| {
                        literal
                            .strip_prefix('\'')
                            .and_then(|value| value.strip_suffix('\''))
                    })
                    .unwrap_or_default();
                if !property.is_empty()
                    && property.len() <= MAX_TYPE_SHAPE_BYTES
                    && property.chars().all(|character| {
                        character == '_' || character == '$' || character.is_ascii_alphanumeric()
                    })
                {
                    return Some(property.to_owned());
                }
            }
            current = ancestor.parent();
        }
        None
    }

    fn positive_discriminant_guard(
        &self,
        object: Node<'tree>,
        variable_name: &str,
    ) -> Option<(String, String)> {
        let mut current = object.parent();
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            let Some(ancestor) = current else {
                break;
            };
            if ancestor.kind() == "if_statement"
                && let Some(consequence) = ancestor.child_by_field_name("consequence")
                && node_is_descendant_or_same(object, consequence)
                && let Some(condition) = ancestor.child_by_field_name("condition")
            {
                let compact = node_text(self.source, condition)
                    .chars()
                    .filter(|character| !character.is_ascii_whitespace())
                    .collect::<String>();
                let compact = compact
                    .strip_prefix('(')
                    .and_then(|value| value.strip_suffix(')'))
                    .unwrap_or(&compact);
                let Some((left, right)) = compact.split_once("===") else {
                    current = ancestor.parent();
                    continue;
                };
                let (member, literal) = if let Some(literal) = right
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .or_else(|| {
                        right
                            .strip_prefix('\'')
                            .and_then(|value| value.strip_suffix('\''))
                    }) {
                    (left, literal)
                } else if let Some(literal) = left
                    .strip_prefix('"')
                    .and_then(|value| value.strip_suffix('"'))
                    .or_else(|| {
                        left.strip_prefix('\'')
                            .and_then(|value| value.strip_suffix('\''))
                    })
                {
                    (right, literal)
                } else {
                    current = ancestor.parent();
                    continue;
                };
                let prefix = format!("{variable_name}.");
                let Some(property) = member.strip_prefix(&prefix) else {
                    current = ancestor.parent();
                    continue;
                };
                if property.is_empty()
                    || !property.chars().all(|character| {
                        character == '_' || character == '$' || character.is_ascii_alphanumeric()
                    })
                    || literal.is_empty()
                    || literal.len() > MAX_TYPE_SHAPE_BYTES
                {
                    current = ancestor.parent();
                    continue;
                }
                return Some((property.to_owned(), literal.to_owned()));
            }
            current = ancestor.parent();
        }
        None
    }

    fn flow_narrowed_instanceof_receiver(
        &self,
        scope_id: &str,
        object: Node<'tree>,
        declaration: &DeclarationInfo,
    ) -> Option<ReceiverTarget> {
        let mut current = object.parent();
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            let Some(ancestor) = current else {
                break;
            };
            if ancestor.kind() == "if_statement"
                && let Some(consequence) = ancestor.child_by_field_name("consequence")
                && node_is_descendant_or_same(object, consequence)
                && let Some(condition) = ancestor.child_by_field_name("condition")
            {
                let compact = node_text(self.source, condition)
                    .chars()
                    .filter(|character| !character.is_ascii_whitespace())
                    .collect::<String>();
                let compact = compact
                    .strip_prefix('(')
                    .and_then(|value| value.strip_suffix(')'))
                    .unwrap_or(&compact);
                let Some((left, right)) = compact.split_once("instanceof") else {
                    current = ancestor.parent();
                    continue;
                };
                if left != declaration.name {
                    current = ancestor.parent();
                    continue;
                }
                let right = right.trim_matches(['(', ')']);
                if right.is_empty()
                    || !right.chars().all(|character| {
                        character == '_'
                            || character == '$'
                            || character == '.'
                            || character == ':'
                            || character.is_ascii_alphanumeric()
                    })
                {
                    current = ancestor.parent();
                    continue;
                }
                if let Some(receiver) = self.resolve_declared_type_receiver(scope_id, right) {
                    return Some(receiver);
                }
            }
            current = ancestor.parent();
        }
        None
    }

    fn receiver_target(&self, scope_id: &str, object: Node<'tree>) -> Option<ReceiverTarget> {
        match object.kind() {
            "this" => self.this_receiver_target(scope_id),
            "super" => {
                let class_name = self.enclosing_type(scope_id)?;
                self.base_targets.get(&class_name).cloned()
            }
            "identifier" | "type_identifier" | "jsx_identifier" => {
                let name = node_text(self.source, object);
                if name == "this" {
                    return self.this_receiver_target(scope_id);
                }
                if name == "super" {
                    let class_name = self.enclosing_type(scope_id)?;
                    return self.base_targets.get(&class_name).cloned();
                }
                match self.resolve_name(scope_id, &name, Namespace::Value) {
                    Some(Resolution::Local(declaration)) => {
                        if matches!(
                            declaration.kind.as_str(),
                            "class" | "interface" | "enum" | "namespace" | "function"
                        ) {
                            return Some(ReceiverTarget {
                                qualified_name: declaration.qualified_name.clone(),
                                import: None,
                                scope_id: self.member_scope_for_declaration(&declaration),
                                type_arguments: None,
                            });
                        }

                        if let Some(qualified_name) = self.prototype_sources.get(&declaration.id) {
                            return Some(ReceiverTarget {
                                qualified_name: qualified_name.clone(),
                                import: None,
                                scope_id: Some(declaration.scope_id.clone()),
                                type_arguments: None,
                            });
                        }

                        // Inline structural index signatures (for example,
                        // `shape: { [key: string]: Item }`) have no nominal
                        // type declaration to resolve. The bounded index-value
                        // cache still proves the value type at this binding;
                        // preserve the binding identity so a later dynamic
                        // subscript can resolve only that source-proven value.
                        if self
                            .index_value_types
                            .contains_key(&declaration.qualified_name)
                        {
                            return Some(ReceiverTarget {
                                qualified_name: declaration.qualified_name.clone(),
                                import: None,
                                scope_id: Some(declaration.scope_id.clone()),
                                type_arguments: None,
                            });
                        }

                        if let Some(qualified_name) =
                            self.variable_inline_type_receivers.get(&declaration.id)
                        {
                            return Some(ReceiverTarget {
                                qualified_name: qualified_name.clone(),
                                import: None,
                                scope_id: Some(declaration.scope_id.clone()),
                                type_arguments: None,
                            });
                        }

                        if let Some(receiver) =
                            self.flow_narrowed_discriminant_receiver(scope_id, object, &declaration)
                        {
                            return Some(receiver);
                        }

                        if let Some(receiver) =
                            self.flow_narrowed_union_receiver(scope_id, object, &declaration)
                        {
                            return Some(receiver);
                        }

                        if let Some(receiver) =
                            self.flow_narrowed_in_receiver(scope_id, object, &declaration)
                        {
                            return Some(receiver);
                        }

                        if let Some(receiver) =
                            self.flow_narrowed_instanceof_receiver(scope_id, object, &declaration)
                        {
                            return Some(receiver);
                        }

                        if let Some(receiver) =
                            self.flow_receiver_at(&declaration.id, object.start_byte())
                        {
                            return Some(receiver);
                        }
                        if self.flow_assignment_barrier_before(&declaration.id, object.start_byte())
                        {
                            if let Some(receiver) = self.stable_nominal_flow_receiver_at(
                                &declaration.id,
                                object.start_byte(),
                            ) {
                                return Some(receiver);
                            }
                            return None;
                        }

                        // Preserve the import proof carried by a direct
                        // annotation (`value: ImportedType`).  The
                        // `variable_types` cache intentionally stores only
                        // the nominal qualified spelling for older
                        // inference paths; recovering the source annotation
                        // here keeps imported receivers attached to their
                        // binding identity so cross-file member evidence can
                        // resolve against the imported type's declarations.
                        if let Some(type_name) = declaration.declared_type_name.as_deref()
                            && let Some(receiver) =
                                self.resolve_declared_type_receiver(scope_id, type_name)
                        {
                            return Some(receiver);
                        }

                        if let Some(qualified_name) = self.variable_types.get(&declaration.id) {
                            let qualified_name = self
                                .instance_receiver_qualified_name(qualified_name)
                                .unwrap_or_else(|| qualified_name.clone());
                            let receiver_scope = self.scope_for_qualified_nominal(&qualified_name);
                            return Some(ReceiverTarget {
                                qualified_name,
                                import: None,
                                scope_id: receiver_scope,
                                type_arguments: None,
                            });
                        }

                        if let Some(source_id) = self.variable_object_sources.get(&declaration.id)
                            && let Some(source) = self.declarations.get(source_id)
                        {
                            return Some(ReceiverTarget {
                                qualified_name: source.qualified_name.clone(),
                                import: None,
                                scope_id: self.member_scope_for_declaration(source),
                                type_arguments: None,
                            });
                        }

                        if declaration.kind == "variable"
                            && self
                                .structural_object_variables
                                .contains(&declaration.qualified_name)
                        {
                            return Some(ReceiverTarget {
                                qualified_name: declaration.qualified_name,
                                import: None,
                                scope_id: Some(declaration.scope_id),
                                type_arguments: None,
                            });
                        }
                        None
                    }
                    Some(Resolution::Import(import)) => Some(ReceiverTarget {
                        qualified_name: import_target_without_namespace(&import.target),
                        import: Some(import),
                        scope_id: None,
                        type_arguments: None,
                    }),
                    None => None,
                }
            }
            "new_expression" => {
                let constructor = object
                    .child_by_field_name("constructor")
                    .or_else(|| first_named_child(object))?;
                if constructor.kind() == "this" || node_text(self.source, constructor) == "this" {
                    return self.this_receiver_target(scope_id);
                }
                let target = rightmost_identifier(constructor)?;
                match self.resolve_name(
                    scope_id,
                    &node_text(self.source, target),
                    Namespace::Both,
                )? {
                    Resolution::Local(declaration)
                        if matches!(
                            declaration.kind.as_str(),
                            "class" | "interface" | "enum" | "namespace"
                        ) =>
                    {
                        Some(ReceiverTarget {
                            qualified_name: declaration.qualified_name.clone(),
                            import: None,
                            scope_id: self.member_scope_for_declaration(&declaration),
                            type_arguments: None,
                        })
                    }
                    Resolution::Local(declaration) if declaration.kind == "function" => {
                        Some(ReceiverTarget {
                            qualified_name: format!("{}.prototype", declaration.qualified_name),
                            import: None,
                            // Prototype assignments are normally published in
                            // the constructor's binding scope, not inside its
                            // function body. Leave the scope broad enough for
                            // the exact qualified-name proof below.
                            scope_id: Some(declaration.scope_id.clone()),
                            type_arguments: None,
                        })
                    }
                    Resolution::Import(import) => Some(ReceiverTarget {
                        qualified_name: import_target_without_namespace(&import.target),
                        import: Some(import),
                        scope_id: None,
                        type_arguments: None,
                    }),
                    _ => None,
                }
            }
            "call_expression" | "optional_call_expression" => {
                self.call_return_receiver(scope_id, object)
            }
            "as_expression" | "type_assertion" => {
                let type_node = object
                    .child_by_field_name("type")
                    .or_else(|| object.child_by_field_name("type_annotation"))
                    .or_else(|| {
                        let mut cursor = object.walk();
                        (object.kind() == "as_expression")
                            .then(|| object.named_children(&mut cursor).last())
                            .flatten()
                    })
                    .or_else(|| first_named_child(object));
                let type_name = type_node
                    .map(|node| normalize_type_text(self.source, node))
                    .unwrap_or_default();
                if !type_name.is_empty() {
                    if let Some(receiver) =
                        self.resolve_declared_type_receiver(scope_id, &type_name)
                    {
                        return Some(receiver);
                    }
                    // `as any`/`as unknown` deliberately erase the
                    // receiver type. Do not recover the pre-assertion
                    // expression and invent a member target. `as const` is
                    // retained only for an object literal, where the
                    // structural declaration remains source-local.
                    if matches!(type_name.as_str(), "any" | "unknown" | "never") {
                        return None;
                    }
                }
                let value = object
                    .child_by_field_name("expression")
                    .or_else(|| object.child_by_field_name("value"))
                    .or_else(|| first_named_child(object))?;
                if type_name == "const" && unwrap_expression_node(value).kind() == "object" {
                    return self.receiver_target(scope_id, value);
                }
                None
            }
            "satisfies_expression" => {
                let value = object
                    .child_by_field_name("expression")
                    .or_else(|| object.child_by_field_name("value"))
                    .or_else(|| first_named_child(object))?;
                self.receiver_target(scope_id, value)
            }
            "parenthesized_expression" | "non_null_expression" => {
                let value = object
                    .child_by_field_name("expression")
                    .or_else(|| first_named_child(object))?;
                self.receiver_target(scope_id, value)
            }
            "member_expression"
            | "optional_member_expression"
            | "subscript_expression"
            | "jsx_member_expression"
            | "jsx_namespace_name" => {
                let property = member_property_node(object)?;
                let nested =
                    self.receiver_target(scope_id, object.child_by_field_name("object")?)?;
                if object.kind() == "subscript_expression"
                    && let Some(indexed) = self.index_value_receiver(
                        scope_id,
                        &nested,
                        member_property_name(self.source, property).as_deref(),
                    )
                {
                    return Some(indexed);
                }
                if object.kind() == "subscript_expression" && nested.import.is_some() {
                    let receiver = typescript_receiver_with_type_arguments(
                        &nested.qualified_name,
                        nested.type_arguments.as_deref(),
                    );
                    let suffix = member_property_name(self.source, property)
                        .filter(|index| {
                            !index.is_empty()
                                && index.chars().all(|character| character.is_ascii_digit())
                        })
                        .map_or_else(|| "[]".to_owned(), |index| format!("[{index}]"));
                    let qualified_name = format!("{receiver}{suffix}");
                    return Some(ReceiverTarget {
                        qualified_name,
                        import: nested.import,
                        scope_id: nested.scope_id,
                        type_arguments: None,
                    });
                }
                let property_name = member_property_name(self.source, property)?;
                // A namespace import (`import * as api`) exposes members with
                // `module::member` identity.  A named/class import denotes a
                // nominal receiver, whose instance member is
                // `module::Type.member`; retaining this distinction prevents
                // imported class members from collapsing onto the package
                // namespace.
                let namespace_import = nested
                    .import
                    .as_ref()
                    .is_some_and(|import| import.namespace == Namespace::Module);
                let qualified_name = typescript_member_qualified_name(
                    &nested.qualified_name,
                    nested.type_arguments.as_deref(),
                    &property_name,
                    namespace_import,
                    nested.import.is_some(),
                );
                let receiver = ReceiverTarget {
                    qualified_name,
                    import: nested.import,
                    scope_id: nested.scope_id,
                    type_arguments: nested.type_arguments,
                };
                Some(
                    self.typed_member_receiver(scope_id, receiver.clone())
                        .unwrap_or(receiver),
                )
            }
            _ => None,
        }
    }

    fn call_return_receiver(&self, scope_id: &str, call: Node<'tree>) -> Option<ReceiverTarget> {
        let function = call
            .child_by_field_name("function")
            .or_else(|| first_named_child(call))?;
        let callee_receiver = if matches!(
            function.kind(),
            "member_expression" | "optional_member_expression" | "subscript_expression"
        ) {
            function
                .child_by_field_name("object")
                .and_then(|object| self.receiver_target(scope_id, object))
        } else {
            None
        };
        let resolution = if matches!(
            function.kind(),
            "member_expression" | "optional_member_expression" | "subscript_expression"
        ) {
            let property = member_property_node(function)?;
            self.resolve_member_target(
                scope_id,
                function.child_by_field_name("object"),
                property,
                None,
                &[],
            )
        } else {
            let target = rightmost_identifier(function)?;
            self.resolve_name(scope_id, &node_text(self.source, target), Namespace::Value)
        }?;
        if let Resolution::Import(import) = &resolution {
            // Carry a bounded, source-proven call-result marker across the
            // file boundary.  The resolver can use the imported callable's
            // published signature to recover its nominal return receiver;
            // unknown argument shapes deliberately remain unresolved.
            let explicit_type_arguments = function
                .child_by_field_name("type_arguments")
                .or_else(|| call.child_by_field_name("type_arguments"))
                .or_else(|| first_named_child_kind(function, "type_arguments"))
                .and_then(|type_arguments| {
                    let text = node_text(self.source, type_arguments);
                    let inner = text.strip_prefix('<')?.strip_suffix('>')?;
                    split_top_level_arguments(inner).map(|arguments| {
                        self.canonical_type_arguments(
                            scope_id,
                            &arguments
                                .into_iter()
                                .map(str::trim)
                                .map(ToOwned::to_owned)
                                .collect::<Vec<_>>(),
                        )
                    })
                })
                .unwrap_or_default();
            let argument_types = self.call_argument_types(call, scope_id);
            if explicit_type_arguments.is_empty() && argument_types.iter().any(Option::is_none) {
                return None;
            }
            let argument_types = argument_types
                .into_iter()
                .map(|argument| argument.unwrap_or_else(|| "__unknown".to_owned()))
                .collect::<Vec<_>>();
            let marker_arguments = if argument_types.is_empty() {
                "__none".to_owned()
            } else {
                argument_types.join(",")
            };
            let marker_types = if explicit_type_arguments.is_empty() {
                String::new()
            } else {
                format!("#types<{}>", explicit_type_arguments.join(","))
            };
            if marker_arguments.len() + marker_types.len() > MAX_TYPE_SHAPE_BYTES {
                return None;
            }
            let receiver = import_target_without_namespace(&import.target);
            let qualified_name = format!("{receiver}#call<{marker_arguments}>{marker_types}");
            if qualified_name.len() > MAX_TYPE_SHAPE_BYTES {
                return None;
            }
            return Some(ReceiverTarget {
                qualified_name,
                import: Some(import.clone()),
                scope_id: None,
                type_arguments: None,
            });
        }
        let Resolution::Local(declaration) = resolution else {
            return None;
        };
        let return_type = self
            .call_return_type_name(scope_id, call, &declaration)
            .or_else(|| self.variable_alias_return_type(&declaration, scope_id))?;
        let return_type = return_type.trim();
        if return_type == "this" {
            return callee_receiver.or_else(|| self.this_receiver_target(scope_id));
        }
        self.resolve_declared_type_receiver(scope_id, return_type)
            .or_else(|| self.resolve_declared_type_receiver(&declaration.scope_id, return_type))
    }

    /// Infer a bounded generic callable return shape from source-visible call
    /// arguments. This intentionally handles only direct type-parameter
    /// positions and recursively matching array/generic containers; contextual
    /// inference, overload sets, conditional types, and structural
    /// assignability remain unresolved rather than guessing a receiver.
    fn call_return_type_name(
        &self,
        scope_id: &str,
        call: Node<'tree>,
        declaration: &DeclarationInfo,
    ) -> Option<String> {
        let return_type = declaration.return_type_name.clone()?;
        let Some(order) = self
            .generic_parameter_order_by_declaration
            .get(&declaration.id)
        else {
            return Some(return_type);
        };
        if order.is_empty() {
            return Some(return_type);
        }
        let function = call
            .child_by_field_name("function")
            .or_else(|| first_named_child(call))?;
        let explicit = function
            .child_by_field_name("type_arguments")
            .or_else(|| first_named_child_kind(function, "type_arguments"))
            .and_then(|type_arguments| {
                let text = node_text(self.source, type_arguments);
                let inner = text.strip_prefix('<')?.strip_suffix('>')?;
                split_top_level_arguments(inner).map(|arguments| {
                    self.canonical_type_arguments(
                        scope_id,
                        &arguments
                            .into_iter()
                            .map(str::trim)
                            .map(ToOwned::to_owned)
                            .collect::<Vec<_>>(),
                    )
                })
            })
            .unwrap_or_default();
        let mut arguments = order.clone();
        for (index, argument) in explicit.into_iter().enumerate() {
            if let Some(slot) = arguments.get_mut(index) {
                slot.clone_from(&argument);
            }
        }
        let call_arguments = self.call_argument_types(call, scope_id);
        if let Some(parameter_types) = declaration.parameter_types.as_ref() {
            for (parameter, argument) in parameter_types.iter().zip(call_arguments.iter()) {
                let Some(argument) = argument.as_deref() else {
                    continue;
                };
                if !infer_candidate_type_arguments(parameter, argument, order, &mut arguments) {
                    return None;
                }
            }
        }
        let substituted = substitute_candidate_type_parameters(&return_type, order, &arguments, 0);
        (substituted != return_type || !candidate_type_mentions_parameter(&return_type, order))
            .then_some(substituted)
    }

    fn variable_alias_return_type(
        &self,
        declaration: &DeclarationInfo,
        scope_id: &str,
    ) -> Option<String> {
        let alias = self.variable_alias_values.get(&declaration.id)?;
        let (base, property) = alias.rsplit_once('.')?;
        let receiver = self.resolve_name(scope_id, base, Namespace::Value)?;
        let qualified_name = format!("{}.{}", receiver.qualified_target()?, property);
        let ids = self.declarations_by_qualified.get(&qualified_name)?;
        let return_types = ids
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter_map(|declaration| declaration.return_type_name.clone())
            .collect::<Vec<_>>();
        let [return_type] = return_types.as_slice() else {
            return None;
        };
        Some(return_type.clone())
    }

    /// Follow one source-annotated member value to its nominal type. This is
    /// the bounded bridge that makes chains such as `ctx.common.issues` and
    /// `this._def.description` precise without attempting whole-program flow
    /// or structural inference. Generic type parameters are replaced only by
    /// their direct source-visible constraint; ambiguous or unconstrained
    /// members remain on the original receiver and are resolved normally (or
    /// left unresolved).
    fn typed_member_receiver(
        &self,
        scope_id: &str,
        receiver: ReceiverTarget,
    ) -> Option<ReceiverTarget> {
        if receiver.import.is_some() {
            return None;
        }
        let declaration = self
            .declarations_by_qualified
            .get(&receiver.qualified_name)
            .into_iter()
            .flat_map(|ids| ids.iter())
            .filter_map(|id| self.declarations.get(id))
            .filter(|declaration| declaration.declared_type_name.is_some())
            .collect::<Vec<_>>();
        let declaration = match declaration.as_slice() {
            [declaration] => (*declaration).clone(),
            [] => {
                let (base, property) = receiver.qualified_name.rsplit_once('.')?;
                match self.resolve_inherited_member_target(base, property, None, &[])? {
                    Resolution::Local(declaration) => declaration,
                    Resolution::Import(_) => return None,
                }
            }
            _ => return None,
        };
        let type_name = declaration.declared_type_name.as_deref()?;
        let type_name = receiver
            .qualified_name
            .rsplit_once('.')
            .and_then(|(base, _)| self.base_type_bindings.get(base))
            .and_then(|bindings| bindings.get(type_name))
            .cloned()
            .or_else(|| self.receiver_type_argument(&receiver, type_name))
            .unwrap_or_else(|| type_name.to_owned());
        self.resolve_declared_type_receiver(scope_id, &type_name)
            .or_else(|| self.resolve_declared_type_receiver(&declaration.scope_id, &type_name))
    }

    /// Substitute one member's generic type parameter from the concrete type
    /// arguments carried by its receiver. This only accepts a unique local
    /// nominal declaration with a bounded parameter order; overloaded or
    /// merged declarations remain unresolved.
    fn receiver_type_argument(&self, receiver: &ReceiverTarget, type_name: &str) -> Option<String> {
        let arguments = receiver.type_arguments.as_ref()?;
        let (base, _) = receiver.qualified_name.rsplit_once('.')?;
        let ids = self.declarations_by_qualified.get(base)?;
        let candidates = ids
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter_map(|declaration| {
                self.generic_parameter_order_by_declaration
                    .get(&declaration.id)
                    .map(|order| (declaration, order))
            })
            .collect::<Vec<_>>();
        let [(_declaration, order)] = candidates.as_slice() else {
            return None;
        };
        let substituted = substitute_candidate_type_parameters(type_name, order, arguments, 0);
        (substituted != type_name
            && !substituted.is_empty()
            && substituted.len() <= MAX_TYPE_SHAPE_BYTES)
            .then_some(substituted)
    }

    fn member_value_receiver(
        &self,
        scope_id: &str,
        receiver: &ReceiverTarget,
        declaration: &DeclarationInfo,
    ) -> Option<ReceiverTarget> {
        let type_name = declaration.declared_type_name.as_deref()?;
        let type_name = receiver
            .qualified_name
            .rsplit_once('.')
            .and_then(|(base, _)| self.base_type_bindings.get(base))
            .or_else(|| self.base_type_bindings.get(&receiver.qualified_name))
            .and_then(|bindings| bindings.get(type_name))
            .cloned()
            .or_else(|| self.receiver_type_argument(receiver, type_name))
            .unwrap_or_else(|| type_name.to_owned());
        self.resolve_declared_type_receiver(scope_id, &type_name)
            .or_else(|| self.resolve_declared_type_receiver(&declaration.scope_id, &type_name))
    }

    fn index_value_receiver(
        &self,
        scope_id: &str,
        receiver: &ReceiverTarget,
        index: Option<&str>,
    ) -> Option<ReceiverTarget> {
        if receiver.import.is_some() {
            return None;
        }
        if let Some(value_type) = typescript_structural_index_value(&receiver.qualified_name) {
            return self.resolve_declared_type_receiver(scope_id, value_type);
        }
        let type_name = self
            .index_value_types
            .get(&receiver.qualified_name)
            .cloned()
            .or_else(|| indexed_sequence_element_type_name(&receiver.qualified_name, index))?;
        self.resolve_declared_type_receiver(scope_id, &type_name)
    }

    /// Select a conditional-type branch only when the source proves one
    /// nominal constituent satisfies the `extends` check.  TypeScript
    /// distributes conditional types over unions; choosing either branch for
    /// a union, `any`, or an unresolved structural check would silently invent
    /// a receiver, so those cases remain unresolved.
    fn conditional_type_branch(
        &self,
        scope_id: &str,
        check: &str,
        expected: &str,
        when_true: &str,
        when_false: &str,
    ) -> Option<String> {
        let check = check.trim();
        let expected = expected.trim();
        if check.is_empty()
            || expected.is_empty()
            || when_true.trim().is_empty()
            || when_false.trim().is_empty()
            || matches!(check, "any" | "unknown" | "never")
            || conditional_type_parts(check).is_some()
        {
            return None;
        }
        let members = split_top_level_union(check)?;
        let [check] = members.as_slice() else {
            return None;
        };
        let check = check.trim();
        let matches_expected = if expected == "unknown" {
            let receiver = self.resolve_declared_type_receiver(scope_id, check)?;
            receiver.import.is_none()
        } else if expected == "object" {
            let receiver = self.resolve_declared_type_receiver(scope_id, check)?;
            receiver.import.is_none() && is_object_like_type(check)
        } else {
            if conditional_type_parts(expected).is_some() {
                return None;
            }
            let check_receiver = self.resolve_declared_type_receiver(scope_id, check)?;
            let expected_receiver = self.resolve_declared_type_receiver(scope_id, expected)?;
            if check_receiver.import.is_some() || expected_receiver.import.is_some() {
                return None;
            }
            type_names_compatible(
                &check_receiver.qualified_name,
                &expected_receiver.qualified_name,
            ) || self.source_type_assignable(
                scope_id,
                &expected_receiver.qualified_name,
                &check_receiver.qualified_name,
            )
        };
        Some(
            if matches_expected {
                when_true.trim()
            } else {
                when_false.trim()
            }
            .to_owned(),
        )
    }

    fn resolve_declared_type_receiver(
        &self,
        scope_id: &str,
        type_name: &str,
    ) -> Option<ReceiverTarget> {
        let mut current = type_name.trim().to_owned();
        let mut seen = HashSet::new();
        for _ in 0..=MAX_TYPE_SHAPE_DEPTH {
            let normalized = current.trim();
            if normalized.is_empty()
                || normalized.len() > MAX_TYPE_SHAPE_BYTES
                || !seen.insert(normalized.to_owned())
            {
                return None;
            }
            if let Some(value_type) = typescript_inline_index_value_type(normalized) {
                // Validate the value type through the same source-backed
                // resolver used by nominal index signatures before retaining
                // the structural marker. This prevents primitive, external,
                // or ambiguous value shapes from becoming member targets.
                self.resolve_declared_type_receiver(scope_id, &value_type)?;
                return Some(ReceiverTarget {
                    qualified_name: typescript_structural_index_receiver(&value_type),
                    import: None,
                    scope_id: None,
                    type_arguments: None,
                });
            }
            if indexed_sequence_element_type_name(normalized, None).is_some()
                || tuple_type_element_count(normalized).is_some()
            {
                return Some(ReceiverTarget {
                    qualified_name: normalized.to_owned(),
                    import: None,
                    scope_id: None,
                    type_arguments: None,
                });
            }
            if let Some((base, property)) = indexed_type_parts(normalized) {
                let base_receiver = self.resolve_declared_type_receiver(scope_id, base)?;
                // A compiler-selected indexed access can be projected from a
                // local nominal declaration when the key is a literal and
                // exactly one source member owns it. Imported and computed
                // projections require project-wide checker evidence and stay
                // unresolved rather than inheriting a terminal-name match.
                if base_receiver.import.is_some() {
                    return None;
                }
                let qualified_name = format!("{}.{}", base_receiver.qualified_name, property);
                let ids = self.declarations_by_qualified.get(&qualified_name)?;
                let declarations = ids
                    .iter()
                    .filter_map(|id| self.declarations.get(id))
                    .collect::<Vec<_>>();
                let [declaration] = declarations.as_slice() else {
                    return None;
                };
                if declaration.declared_type_name.is_some() {
                    return self.member_value_receiver(scope_id, &base_receiver, declaration);
                }
                return Some(ReceiverTarget {
                    qualified_name: declaration.qualified_name.clone(),
                    import: None,
                    scope_id: self.member_scope_for_declaration(declaration),
                    type_arguments: None,
                });
            }
            if let Some((utility, base, arguments)) = utility_type_parts(normalized)
                && matches!(utility, "Pick" | "Omit")
            {
                let base_receiver = self.resolve_declared_type_receiver(scope_id, base)?;
                let keys = arguments
                    .get(1)
                    .and_then(|keys| self.utility_key_names(scope_id, &base_receiver, keys))?;
                if keys.len() > MAX_INLINE_OBJECT_PROPERTIES
                    || keys
                        .iter()
                        .any(|key| key.contains('|') || key.contains(','))
                    || base.len() > MAX_TYPE_SHAPE_BYTES
                    || utility_type_parts(base).is_some()
                {
                    return None;
                }
                // Keep the first utility projection local and source-backed.
                // Imported projected members require a project-wide property
                // inventory and remain unresolved until that evidence exists.
                if base_receiver.import.is_some() {
                    return None;
                }
                return Some(ReceiverTarget {
                    qualified_name: encode_utility_projection(
                        utility,
                        &base_receiver.qualified_name,
                        &keys,
                    ),
                    import: None,
                    scope_id: base_receiver.scope_id,
                    type_arguments: base_receiver.type_arguments,
                });
            }
            if let Some((check, expected, when_true, when_false)) =
                conditional_type_parts(normalized)
            {
                let branch =
                    self.conditional_type_branch(scope_id, check, expected, when_true, when_false)?;
                current.clear();
                current.push_str(&branch);
                continue;
            }
            if let Some(unwrapped) = candidate_utility_receiver_type(normalized) {
                current = unwrapped;
                continue;
            }
            let (lookup_name, type_arguments) = generic_type_parts(normalized)
                .map(|(base, arguments)| (base.to_owned(), Some(arguments)))
                .unwrap_or_else(|| (normalized.to_owned(), None));
            let type_arguments =
                type_arguments.map(|arguments| self.canonical_type_arguments(scope_id, &arguments));
            let resolution = self
                .resolve_name(scope_id, &lookup_name, Namespace::Both)
                .or_else(|| self.resolve_qualified_type_name(&lookup_name))?;
            match resolution {
                Resolution::Import(import) => {
                    return Some(ReceiverTarget {
                        qualified_name: import_target_without_namespace(&import.target),
                        import: Some(import),
                        scope_id: None,
                        type_arguments,
                    });
                }
                Resolution::Local(declaration) if declaration.kind == "type_alias" => {
                    if let Some(alias_target) = declaration.declared_type_name.as_deref() {
                        let alias_target = self.substitute_type_alias_target(
                            &declaration,
                            alias_target,
                            type_arguments.as_deref().unwrap_or(&[]),
                        );
                        current.clear();
                        current.push_str(&alias_target);
                    } else {
                        return Some(ReceiverTarget {
                            qualified_name: declaration.qualified_name.clone(),
                            import: None,
                            scope_id: self.member_scope_for_declaration(&declaration),
                            type_arguments,
                        });
                    }
                }
                Resolution::Local(declaration)
                    if matches!(
                        declaration.kind.as_str(),
                        "class" | "interface" | "enum" | "namespace"
                    ) =>
                {
                    return Some(ReceiverTarget {
                        qualified_name: declaration.qualified_name.clone(),
                        import: None,
                        scope_id: self.member_scope_for_declaration(&declaration),
                        type_arguments,
                    });
                }
                Resolution::Local(declaration) if declaration.kind == "type_parameter" => {
                    let owner = self.scope_owners.get(&declaration.scope_id)?;
                    let constraint = self
                        .generic_parameters_by_declaration
                        .get(owner)
                        .and_then(|parameters| parameters.get(&declaration.name))
                        .and_then(|constraint| constraint.as_deref())?;
                    current.clear();
                    current.push_str(constraint);
                }
                _ => return None,
            }
        }
        None
    }

    /// Resolve the bounded key set accepted by a local `Pick`/`Omit`
    /// projection. Literal keys are source syntax; `keyof Base` is accepted
    /// only when it names the same unique local nominal base, so `Pick<Base,
    /// keyof Base>` remains an identity projection without pretending to
    /// evaluate arbitrary structural or imported key spaces.
    fn utility_key_names(
        &self,
        scope_id: &str,
        base_receiver: &ReceiverTarget,
        keys: &str,
    ) -> Option<HashSet<String>> {
        if let Some(keys) = type_literal_names(keys) {
            return Some(keys);
        }
        let key_base = keyof_type_base(keys)?;
        let key_receiver = self.resolve_declared_type_receiver(scope_id, key_base)?;
        if key_receiver.import.is_some()
            || base_receiver.import.is_some()
            || key_receiver.qualified_name != base_receiver.qualified_name
        {
            return None;
        }
        self.source_type_property_names(&base_receiver.qualified_name)
    }

    /// Resolve a canonical module-qualified nominal name produced while
    /// substituting an explicit generic argument (for example
    /// `generic-mapped-alias.Item`).  Ordinary lexical lookup intentionally
    /// accepts only source spellings; this narrow fallback keeps qualified
    /// identities from being mistaken for unqualified bindings while still
    /// allowing a same-file generic mapped alias to recover its concrete
    /// member receiver.
    fn resolve_qualified_type_name(&self, name: &str) -> Option<Resolution> {
        let ids = self.declarations_by_qualified.get(name)?;
        let declarations = ids
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter(|declaration| {
                declaration.namespace.accepts(Namespace::Type)
                    && matches!(
                        declaration.kind.as_str(),
                        "class" | "interface" | "enum" | "namespace" | "type_alias"
                    )
            })
            .cloned()
            .collect::<Vec<_>>();
        let [declaration] = declarations.as_slice() else {
            return None;
        };
        Some(Resolution::Local(declaration.clone()))
    }

    fn substitute_type_alias_target(
        &self,
        declaration: &DeclarationInfo,
        target: &str,
        arguments: &[String],
    ) -> String {
        let Some(parameters) = self
            .generic_parameter_order_by_declaration
            .get(&declaration.id)
        else {
            return target.to_owned();
        };
        substitute_candidate_type_parameters(target, parameters, arguments, 0)
    }

    /// Canonicalize direct generic arguments when the source proves an
    /// import or local nominal declaration. Keeping the module-qualified
    /// identity here prevents a same-spelled `Item` from another module from
    /// being substituted into an imported `Box<Item>` chain. Unresolved or
    /// structural arguments remain textual and are intentionally not used by
    /// the cross-file member resolver.
    fn canonical_type_arguments(&self, scope_id: &str, arguments: &[String]) -> Vec<String> {
        arguments
            .iter()
            .map(|argument| self.canonical_type_argument(scope_id, argument, 0))
            .collect()
    }

    fn canonicalize_declaration_signature(
        &self,
        scope_id: &str,
        kind: &str,
        signature: &str,
    ) -> String {
        let has_index = signature.contains("|index=") || signature.starts_with("index=");
        if (kind != "type_alias" && !has_index) || (!signature.contains('=') && !has_index) {
            return signature.to_owned();
        }
        let mut canonical = signature.to_owned();
        if let Some((prefix, target)) = canonical.split_once('=') {
            let target = target
                .split_once("|index=")
                .map_or(target, |(target, _)| target);
            let target = self.canonical_type_argument(scope_id, target, 0);
            canonical = format!("{prefix}={target}");
        }
        if let Some((prefix, value)) = canonical.split_once("|index=") {
            let value = self.canonical_type_argument(scope_id, value, 0);
            canonical = format!("{prefix}|index={value}");
        }
        canonical
    }

    fn canonical_type_argument(&self, scope_id: &str, argument: &str, depth: u32) -> String {
        let argument = argument.trim();
        if argument.is_empty()
            || argument.len() > MAX_TYPE_SHAPE_BYTES
            || depth > MAX_TYPE_SHAPE_DEPTH
        {
            return argument.to_owned();
        }
        if let Some((base, nested_arguments)) = generic_type_parts(argument) {
            let base = self.canonical_type_name(scope_id, base);
            let nested_arguments = nested_arguments
                .iter()
                .map(|nested| self.canonical_type_argument(scope_id, nested, depth + 1))
                .collect::<Vec<_>>();
            let canonical = format!("{base}<{}>", nested_arguments.join(","));
            if canonical.len() <= MAX_TYPE_SHAPE_BYTES {
                return canonical;
            }
        }
        self.canonical_type_name(scope_id, argument)
    }

    fn canonical_type_name(&self, scope_id: &str, type_name: &str) -> String {
        self.resolve_name(scope_id, strip_type_arguments(type_name), Namespace::Both)
            .and_then(|resolution| resolution.qualified_target())
            .filter(|qualified| !qualified.is_empty() && qualified.len() <= MAX_TYPE_SHAPE_BYTES)
            .unwrap_or_else(|| type_name.to_owned())
    }

    /// Return the child scope owned by a nominal declaration when one exists.
    /// Class/interface/namespace members are published in that child scope,
    /// while ordinary lexical declarations are published in their binding
    /// scope.  Keeping this distinction lets member resolution separate
    /// duplicate structural objects that share a qualified spelling in
    /// different blocks.
    fn member_scope_for_declaration(&self, declaration: &DeclarationInfo) -> Option<String> {
        if matches!(
            declaration.kind.as_str(),
            "class" | "interface" | "enum" | "namespace" | "type_alias"
        ) {
            self.scope_for_owner_declaration(&declaration.id)
                .or_else(|| Some(declaration.scope_id.clone()))
        } else if matches!(
            declaration.kind.as_str(),
            "function" | "method" | "constructor"
        ) {
            self.scope_for_owner_declaration(&declaration.id)
                .or_else(|| Some(declaration.scope_id.clone()))
        } else {
            Some(declaration.scope_id.clone())
        }
    }

    fn scope_for_owner_declaration(&self, declaration_id: &str) -> Option<String> {
        self.scope_owners
            .iter()
            .filter(|(_, owner)| owner.as_str() == declaration_id)
            .map(|(scope, _)| scope.clone())
            .min()
    }

    fn scope_for_qualified_nominal(&self, qualified_name: &str) -> Option<String> {
        let declarations = self
            .declarations_by_qualified
            .get(qualified_name)?
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter(|declaration| {
                matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "namespace" | "type_alias"
                )
            })
            .collect::<Vec<_>>();
        (declarations.len() == 1).then(|| self.member_scope_for_declaration(declarations[0]))?
    }

    fn instance_receiver_qualified_name(&self, qualified_name: &str) -> Option<String> {
        let declarations = self
            .declarations_by_qualified
            .get(qualified_name)?
            .iter()
            .filter_map(|id| self.declarations.get(id))
            .filter(|declaration| declaration.kind == "function")
            .collect::<Vec<_>>();
        (declarations.len() == 1).then(|| format!("{qualified_name}.prototype"))
    }

    fn scope_is_descendant_or_same(&self, scope_id: &str, ancestor: &str) -> bool {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            if scope == ancestor {
                return true;
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        false
    }

    fn scope_distance(&self, scope_id: &str, ancestor: &str) -> Option<usize> {
        let mut current = Some(scope_id.to_owned());
        let mut distance = 0_usize;
        while let Some(scope) = current {
            if scope == ancestor {
                return Some(distance);
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
            distance = distance.saturating_add(1);
        }
        None
    }

    fn this_receiver_target(&self, scope_id: &str) -> Option<ReceiverTarget> {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            if let Some(owner) = self.scope_owners.get(&scope)
                && let Some(declaration) = self.declarations.get(owner)
                && matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "namespace"
                )
            {
                return Some(ReceiverTarget {
                    qualified_name: declaration.qualified_name.clone(),
                    import: None,
                    scope_id: self.member_scope_for_declaration(declaration),
                    type_arguments: None,
                });
            }
            if let Some(owner) = self.scope_owners.get(&scope)
                && let Some(declaration) = self.declarations.get(owner)
                && declaration.kind == "function"
                && self.constructor_name_hints.contains(&declaration.name)
            {
                return Some(ReceiverTarget {
                    qualified_name: declaration.qualified_name.clone(),
                    import: None,
                    scope_id: self.member_scope_for_declaration(declaration),
                    type_arguments: None,
                });
            }
            if let Some(receiver) = self.this_receivers.get(&scope) {
                return Some(ReceiverTarget {
                    qualified_name: receiver.clone(),
                    import: None,
                    scope_id: self.scope_for_qualified_nominal(receiver),
                    type_arguments: None,
                });
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        None
    }

    fn enclosing_type(&self, scope_id: &str) -> Option<String> {
        let mut current = Some(scope_id.to_owned());
        while let Some(scope) = current {
            if let Some(owner) = self.scope_owners.get(&scope)
                && let Some(declaration) = self.declarations.get(owner)
                && matches!(
                    declaration.kind.as_str(),
                    "class" | "interface" | "enum" | "namespace"
                )
            {
                return Some(declaration.qualified_name.clone());
            }
            current = self.scope_parents.get(&scope).cloned().flatten();
        }
        None
    }
}

#[derive(Clone)]
struct ImportBinding<'tree> {
    local_name: String,
    imported_name: String,
    anchor: Node<'tree>,
    type_only: bool,
}

fn import_namespace(binding: &ImportBinding<'_>) -> Namespace {
    if binding.type_only {
        if binding.imported_name == "*" {
            Namespace::Module
        } else {
            Namespace::Type
        }
    } else if binding.imported_name == "*" {
        Namespace::Module
    } else {
        // A normal named/default import may denote a class or enum (both
        // value and type) until the target module is resolved.
        Namespace::Both
    }
}

const fn symbol_namespace(namespace: Namespace) -> SymbolNamespace {
    match namespace {
        Namespace::Value => SymbolNamespace::Value,
        Namespace::Type => SymbolNamespace::Type,
        Namespace::Module => SymbolNamespace::Namespace,
        Namespace::Both => SymbolNamespace::ValueAndType,
    }
}

fn import_target_without_namespace(target: &str) -> String {
    target.strip_suffix("::*").unwrap_or(target).to_owned()
}

/// Keep generic arguments attached to the root nominal in a member path.
/// `Box<Item>.value.inspect` is intentionally a constraint spelling, not a
/// graph identity; it lets the resolver recover the concrete member type
/// without treating the terminal `inspect` spelling as authoritative.
fn typescript_member_qualified_name(
    receiver: &str,
    type_arguments: Option<&[String]>,
    property: &str,
    namespace_import: bool,
    preserve_type_arguments: bool,
) -> String {
    let receiver = if preserve_type_arguments {
        typescript_receiver_with_type_arguments(receiver, type_arguments)
    } else {
        receiver.to_owned()
    };
    if namespace_import {
        format!("{receiver}::{property}")
    } else {
        format!("{receiver}.{property}")
    }
}

fn typescript_receiver_with_type_arguments(
    receiver: &str,
    type_arguments: Option<&[String]>,
) -> String {
    let Some(type_arguments) = type_arguments.filter(|arguments| !arguments.is_empty()) else {
        return receiver.to_owned();
    };
    let arguments = type_arguments.join(",");
    if arguments.is_empty() || arguments.len() > MAX_TYPE_SHAPE_BYTES {
        return receiver.to_owned();
    }
    let (module, symbol) = typescript_split_module_qualified(receiver);
    let split = typescript_first_member_separator(symbol).unwrap_or(symbol.len());
    let root = &symbol[..split];
    if root.is_empty() || root.contains('<') {
        return receiver.to_owned();
    }
    let suffix = &symbol[split..];
    match module {
        Some(module) if !module.is_empty() => {
            format!("{module}::{root}<{arguments}>{suffix}")
        }
        _ => format!("{root}<{arguments}>{suffix}"),
    }
}

fn typescript_split_module_qualified(value: &str) -> (Option<&str>, &str) {
    let bytes = value.as_bytes();
    let mut angle_depth = 0_u32;
    let mut split = None;
    for index in 0..bytes.len().saturating_sub(1) {
        match bytes[index] {
            b'<' => angle_depth = angle_depth.saturating_add(1),
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b':' if bytes[index + 1] == b':' && angle_depth == 0 => split = Some(index),
            _ => {}
        }
    }
    split.map_or((None, value), |index| {
        (
            value.get(..index).filter(|module| !module.is_empty()),
            value.get(index.saturating_add(2)..).unwrap_or_default(),
        )
    })
}

fn typescript_first_member_separator(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut angle_depth = 0_u32;
    for index in 0..bytes.len() {
        match bytes[index] {
            b'<' => angle_depth = angle_depth.saturating_add(1),
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b'.' if angle_depth == 0 => return Some(index),
            b':' if angle_depth == 0 && bytes.get(index + 1) == Some(&b':') => {
                return Some(index);
            }
            _ => {}
        }
    }
    None
}

fn is_parameter_node(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "required_parameter"
            | "optional_parameter"
            | "rest_parameter"
            | "formal_parameter"
            | "parameter"
            | "jsx_parameter"
    )
}

fn parameter_pattern_node<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    node.child_by_field_name("pattern")
        .or_else(|| node.child_by_field_name("name"))
        .or_else(|| {
            let mut cursor = node.walk();
            node.named_children(&mut cursor).find(|child| {
                matches!(
                    child.kind(),
                    "identifier"
                        | "shorthand_property_identifier_pattern"
                        | "object_pattern"
                        | "array_pattern"
                        | "rest_pattern"
                )
            })
        })
}

fn first_named_child<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

fn node_is_descendant_or_same(node: Node<'_>, ancestor: Node<'_>) -> bool {
    let mut current = Some(node);
    for _ in 0..=MAX_TRAVERSAL_DEPTH {
        let Some(candidate) = current else {
            return false;
        };
        if candidate.id() == ancestor.id() {
            return true;
        }
        current = candidate.parent();
    }
    false
}

fn default_export_target_node<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    let exported = first_named_child(node)?;
    if exported.kind() == "identifier" {
        return Some(exported);
    }
    exported.child_by_field_name("name")
}

fn anonymous_declaration_shape(
    node: Node<'_>,
) -> Option<(&'static str, Namespace, bool, &'static str)> {
    match node.kind() {
        "function"
        | "function_declaration"
        | "function_expression"
        | "generator_function"
        | "generator_function_declaration"
        | "arrow_function" => Some(("function", Namespace::Value, true, "function")),
        "class" | "class_declaration" | "abstract_class_declaration" => {
            Some(("class", Namespace::Both, true, "class"))
        }
        _ => None,
    }
}

fn declaration_shape<'tree>(
    node: Node<'tree>,
) -> Option<(&'static str, Node<'tree>, Namespace, bool, &'static str)> {
    let (kind, namespace, creates_scope, scope_kind) = match node.kind() {
        "function_declaration" | "function_signature" | "generator_function_declaration" => {
            ("function", Namespace::Value, true, "function")
        }
        "method_definition" | "method_signature" | "abstract_method_signature" => {
            ("method", Namespace::Value, true, "function")
        }
        "class_declaration" | "abstract_class_declaration" | "class" => {
            ("class", Namespace::Both, true, "class")
        }
        "interface_declaration" => ("interface", Namespace::Type, true, "interface"),
        "type_alias_declaration" => ("type_alias", Namespace::Type, true, "type"),
        "enum_declaration" => ("enum", Namespace::Both, true, "enum"),
        "internal_module" | "module" | "namespace_export" => {
            ("namespace", Namespace::Module, true, "namespace")
        }
        "public_field_definition"
        | "property_signature"
        | "field_definition"
        | "pair"
        | "enum_assignment"
        | "shorthand_property_identifier" => ("property", Namespace::Value, false, "property"),
        "type_parameter" => ("type_parameter", Namespace::Type, false, "type"),
        _ => return None,
    };
    let name = node
        .child_by_field_name("name")
        .or_else(|| {
            matches!(node.kind(), "pair" | "enum_assignment")
                .then(|| node.child_by_field_name("key"))
                .flatten()
        })
        .or_else(|| (node.kind() == "shorthand_property_identifier").then_some(node))
        .or_else(|| {
            matches!(node.kind(), "namespace_export").then(|| first_identifier_node(node))?
        })?;
    Some((kind, name, namespace, creates_scope, scope_kind))
}

fn commonjs_export_name<'tree>(left: Node<'tree>, source: &[u8]) -> Option<(String, Node<'tree>)> {
    let normalized = node_text(source, left).replace(char::is_whitespace, "");
    if normalized == "module.exports" {
        return Some(("default".to_owned(), left));
    }
    let property = member_property_node(left)?;
    let object = left.child_by_field_name("object")?;
    let object_text = node_text(source, object).replace(char::is_whitespace, "");
    if !matches!(object_text.as_str(), "module.exports" | "exports") {
        return None;
    }
    let name = member_property_name(source, property)?;
    (!name.is_empty() && name.len() <= MAX_TYPE_SHAPE_BYTES).then_some((name, property))
}

fn commonjs_object_property<'tree>(
    node: Node<'tree>,
) -> Option<(Node<'tree>, Option<Node<'tree>>)> {
    match node.kind() {
        "pair" => Some((
            node.child_by_field_name("key")?,
            node.child_by_field_name("value"),
        )),
        "method_definition" => Some((node.child_by_field_name("name")?, None)),
        "shorthand_property_identifier" => Some((node, Some(node))),
        _ => None,
    }
}

fn is_anonymous_signature_scope(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "call_signature" | "construct_signature" | "function_type"
    )
}

fn lexical_scope_kind(node: Node<'_>) -> Option<&'static str> {
    match node.kind() {
        "statement_block" => Some("block"),
        "switch_statement" => Some("switch"),
        "catch_clause" => Some("catch"),
        "for_statement" | "for_in_statement" | "for_of_statement" => Some("loop"),
        _ => None,
    }
}

fn return_value_node<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    node.child_by_field_name("value")
        .or_else(|| node.child_by_field_name("argument"))
        .or_else(|| first_named_child(node))
}

/// Strip expression-only wrappers that preserve the runtime object identity.
/// TypeScript's `as const`, `satisfies`, parenthesized expressions, and type
/// assertions do not change which source object supplies a member. Keep this
/// helper shallow and bounded so malformed or adversarial trees cannot turn a
/// structural receiver into an unbounded walk.
fn unwrap_expression_node<'tree>(mut node: Node<'tree>) -> Node<'tree> {
    for _ in 0..MAX_TYPE_SHAPE_DEPTH {
        if !matches!(
            node.kind(),
            "as_expression"
                | "satisfies_expression"
                | "parenthesized_expression"
                | "type_assertion"
                | "non_null_expression"
        ) {
            break;
        }
        let Some(inner) = node
            .child_by_field_name("expression")
            .or_else(|| first_named_child(node))
        else {
            break;
        };
        if inner.id() == node.id() {
            break;
        }
        node = inner;
    }
    node
}

fn is_var_binding_node(node: Node<'_>) -> bool {
    let mut current = Some(node);
    while let Some(candidate) = current {
        match candidate.kind() {
            "variable_declaration" => return true,
            "lexical_declaration" => return false,
            "program" | "statement_block" | "switch_statement" | "catch_clause" => {
                return false;
            }
            _ => current = candidate.parent(),
        }
    }
    false
}

fn is_callable_node(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "arrow_function"
            | "function_expression"
            | "generator_function"
            | "function"
            | "function_declaration"
            | "function_signature"
            | "method_definition"
            | "method_signature"
            | "abstract_method_signature"
            | "function_type"
            | "constructor_type"
    )
}

fn callable_value_node(node: Node<'_>) -> Option<Node<'_>> {
    if is_callable_node(node) {
        return Some(node);
    }
    if let Some(value) = node
        .child_by_field_name("value")
        .filter(|value| is_callable_node(*value))
    {
        return Some(value);
    }
    if let Some(type_node) = node
        .child_by_field_name("type")
        .or_else(|| node.child_by_field_name("annotation"))
    {
        if is_callable_node(type_node) {
            return Some(type_node);
        }
        if let Some(callable) =
            first_named_child(type_node).filter(|child| is_callable_node(*child))
        {
            return Some(callable);
        }
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| is_callable_node(*child))
}

fn callable_returns_this(node: Node<'_>) -> bool {
    let Some(body) = node
        .child_by_field_name("body")
        .or_else(|| first_named_child_kind(node, "statement_block"))
    else {
        return false;
    };
    let mut pending = vec![(body, 0_usize)];
    while let Some((current, depth)) = pending.pop() {
        if depth > MAX_TRAVERSAL_DEPTH {
            continue;
        }
        if current.kind() == "return_statement"
            && return_value_node(current)
                .is_some_and(|value| unwrap_expression_node(value).kind() == "this")
        {
            return true;
        }
        if depth > 0 && is_callable_node(current) {
            continue;
        }
        let mut cursor = current.walk();
        for child in current.named_children(&mut cursor) {
            pending.push((child, depth.saturating_add(1)));
        }
    }
    false
}

fn callable_return_constructor_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let body = node
        .child_by_field_name("body")
        .or_else(|| first_named_child_kind(node, "statement_block"))?;
    let mut pending = vec![(body, 0_usize)];
    while let Some((current, depth)) = pending.pop() {
        if depth > MAX_TRAVERSAL_DEPTH {
            continue;
        }
        if current.kind() == "return_statement"
            && let Some(value) = return_value_node(current).map(unwrap_expression_node)
            && value.kind() == "new_expression"
            && let Some(constructor) = value
                .child_by_field_name("constructor")
                .or_else(|| first_named_child(value))
            && let Some(name) = rightmost_identifier(constructor)
        {
            let name = node_text(source, name);
            if !name.is_empty() && name.len() <= MAX_TYPE_SHAPE_BYTES {
                return Some(name);
            }
        }
        if depth > 0 && is_callable_node(current) {
            continue;
        }
        let mut cursor = current.walk();
        for child in current.named_children(&mut cursor) {
            pending.push((child, depth.saturating_add(1)));
        }
    }
    None
}

fn declaration_is_callable_shape(node: Node<'_>, kind: &str, source: &[u8]) -> bool {
    if matches!(kind, "function" | "method" | "constructor" | "class") {
        return true;
    }
    if kind == "property"
        && node.child_by_field_name("value").is_some_and(|value| {
            let value = unwrap_expression_node(value);
            matches!(
                value.kind(),
                "member_expression" | "optional_member_expression" | "subscript_expression"
            ) && member_property_node(value).is_some()
        })
    {
        // A property alias initialized from another source member is a
        // callable-value candidate only when it is actually used as a call;
        // keeping the shape here lets call-site arity resolution retain the
        // alias property as the exact target without guessing its signature.
        return true;
    }
    // A variable initialized from a call is a source-grounded callable-value
    // candidate.  TypeScript/JavaScript libraries commonly expose callable
    // factories (`const schema = type(...)`, React state setters, benchmark
    // wrappers) without a local function declaration or explicit annotation.
    // Restrict this to call initializers; object literals and `new` values are
    // not treated as callable merely because they can have members.
    if matches!(kind, "variable" | "parameter")
        && node.child_by_field_name("value").is_some_and(|value| {
            matches!(value.kind(), "call_expression" | "optional_call_expression")
        })
    {
        return true;
    }
    let type_node = node
        .child_by_field_name("type")
        .or_else(|| node.child_by_field_name("annotation"))
        .or_else(|| {
            node.child_by_field_name("name")
                .and_then(|name| name.child_by_field_name("type"))
        });
    if let Some(type_node) = type_node {
        let type_text = node_text(source, type_node);
        if type_text.contains("typeof")
            || type_text.contains("Constructor")
            || type_text.contains("SchemaClass")
            || contains_node_kind(type_node, "function_type")
            || contains_node_kind(type_node, "constructor_type")
            || contains_node_kind(type_node, "function_signature")
            || contains_node_kind(type_node, "construct_signature")
        {
            return true;
        }
    }
    callable_value_node(node).is_some_and(|value| {
        is_callable_node(value) || node_text(source, value).contains(".constructor")
    })
}

fn contains_node_kind(node: Node<'_>, kind: &str) -> bool {
    if node.kind() == kind {
        return true;
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| contains_node_kind(child, kind))
}

fn inline_object_type_node<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    let root = if node.kind() == "type_annotation" {
        first_named_child(node)?
    } else {
        node
    };
    let mut pending = vec![(root, 0_usize)];
    let mut found = None;
    while let Some((current, depth)) = pending.pop() {
        if depth > MAX_TYPE_SHAPE_DEPTH as usize {
            return None;
        }
        if current.kind() == "object_type" {
            if found.is_some() {
                return None;
            }
            found = Some(current);
            continue;
        }
        let mut cursor = current.walk();
        for child in current.named_children(&mut cursor) {
            pending.push((child, depth.saturating_add(1)));
        }
    }
    found
}

fn callable_parameter_arity(node: Node<'_>, source: &[u8]) -> Option<(u32, Option<u32>)> {
    if matches!(
        node.kind(),
        "class_declaration" | "abstract_class_declaration" | "class"
    ) {
        return class_constructor_parameter_arity(node, source);
    }
    if !matches!(
        node.kind(),
        "function_declaration"
            | "function_signature"
            | "generator_function_declaration"
            | "method_definition"
            | "method_signature"
            | "abstract_method_signature"
            | "function"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "function_type"
            | "constructor_type"
    ) {
        return None;
    }
    let parameters = node
        .child_by_field_name("parameters")
        .or_else(|| first_named_child_kind(node, "formal_parameters"));
    let Some(parameters) = parameters else {
        return node.child_by_field_name("parameter").map(|_| (1, Some(1)));
    };
    let mut minimum = 0_u32;
    let mut maximum = 0_u32;
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        if parameter.kind().contains("comment") {
            continue;
        }
        maximum = maximum.saturating_add(1);
        if parameter.kind().contains("rest")
            || node_text(source, parameter).trim_start().starts_with("...")
        {
            return Some((minimum, None));
        }
        let optional = matches!(
            parameter.kind(),
            "assignment_pattern"
                | "object_assignment_pattern"
                | "optional_parameter"
                | "optional_parameter_declaration"
        ) || parameter.child_by_field_name("value").is_some()
            || parameter.child_by_field_name("initializer").is_some();
        if !optional {
            minimum = minimum.saturating_add(1);
        }
    }
    Some((minimum, Some(maximum)))
}

fn callable_parameter_types(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
    if !matches!(
        node.kind(),
        "function_declaration"
            | "function_signature"
            | "generator_function_declaration"
            | "method_definition"
            | "method_signature"
            | "abstract_method_signature"
            | "function"
            | "function_expression"
            | "generator_function"
            | "arrow_function"
            | "function_type"
            | "constructor_type"
    ) {
        return None;
    }
    let parameters = node
        .child_by_field_name("parameters")
        .or_else(|| first_named_child_kind(node, "formal_parameters"));
    let Some(parameters) = parameters else {
        let parameter = node.child_by_field_name("parameter")?;
        let type_node = parameter
            .child_by_field_name("type")
            .or_else(|| first_named_child_kind(parameter, "type_annotation"))?;
        let normalized = normalize_type_text(source, type_node);
        return (!normalized.is_empty()).then_some(vec![normalized]);
    };
    let mut types = Vec::new();
    let mut cursor = parameters.walk();
    for parameter in parameters.named_children(&mut cursor) {
        if parameter.kind().contains("rest")
            || node_text(source, parameter).trim_start().starts_with("...")
            || matches!(
                parameter.kind(),
                "optional_parameter" | "rest_parameter" | "optional_parameter_declaration"
            )
            || parameter.child_by_field_name("value").is_some()
            || parameter.child_by_field_name("initializer").is_some()
        {
            return None;
        }
        let type_node = parameter
            .child_by_field_name("type")
            .or_else(|| first_named_child_kind(parameter, "type_annotation"))?;
        let normalized = normalize_type_text(source, type_node);
        if normalized.is_empty() {
            return None;
        }
        types.push(normalized);
    }
    Some(types)
}

fn callable_type_parameters(
    node: Node<'_>,
    source: &[u8],
) -> Option<HashMap<String, Option<String>>> {
    let type_parameters = node
        .child_by_field_name("type_parameters")
        .or_else(|| first_named_child_kind(node, "type_parameters"))?;
    let mut parameters = HashMap::new();
    let mut cursor = type_parameters.walk();
    for parameter in type_parameters.named_children(&mut cursor) {
        if parameter.kind() != "type_parameter" {
            continue;
        }
        let Some(name_node) = parameter
            .child_by_field_name("name")
            .or_else(|| first_named_child_kind(parameter, "type_identifier"))
        else {
            continue;
        };
        let name = node_text(source, name_node);
        if name.is_empty() || name.len() > MAX_TYPE_SHAPE_BYTES {
            continue;
        }
        let constraint = parameter
            .child_by_field_name("constraint")
            .or_else(|| first_named_child_kind(parameter, "constraint"))
            .map(|constraint| normalize_type_text(source, constraint))
            .map(|constraint| {
                constraint
                    .strip_prefix("extends")
                    .unwrap_or(&constraint)
                    .to_owned()
            })
            .filter(|constraint| !constraint.is_empty());
        parameters.insert(name, constraint);
        if parameters.len() >= MAX_INLINE_OBJECT_PROPERTIES {
            break;
        }
    }
    (!parameters.is_empty()).then_some(parameters)
}

fn callable_type_parameter_order(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
    let type_parameters = node
        .child_by_field_name("type_parameters")
        .or_else(|| first_named_child_kind(node, "type_parameters"))?;
    let mut order = Vec::new();
    let mut cursor = type_parameters.walk();
    for parameter in type_parameters.named_children(&mut cursor) {
        if parameter.kind() != "type_parameter" {
            continue;
        }
        let Some(name_node) = parameter
            .child_by_field_name("name")
            .or_else(|| first_named_child_kind(parameter, "type_identifier"))
        else {
            continue;
        };
        let name = node_text(source, name_node);
        if !name.is_empty() && name.len() <= MAX_TYPE_SHAPE_BYTES {
            order.push(name);
        }
        if order.len() >= MAX_INLINE_OBJECT_PROPERTIES {
            break;
        }
    }
    (!order.is_empty()).then_some(order)
}

fn callable_return_type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let callable = if is_callable_node(node) {
        node
    } else {
        node.child_by_field_name("value")
            .filter(|value| is_callable_node(*value))
            .or_else(|| {
                // A property signature can be callable without a runtime
                // value, for example `getter: () => T`.  Treat only
                // source-declared property/field shapes as callable here;
                // recursively searching arbitrary declarations would make a
                // containing class appear to return the first nested method's
                // type.
                matches!(
                    node.kind(),
                    "property_signature"
                        | "public_field_definition"
                        | "field_definition"
                        | "property_declaration"
                        | "property"
                )
                .then(|| callable_value_node(node))
                .flatten()
            })?
    };
    let return_type = callable
        .child_by_field_name("return_type")
        .or_else(|| callable.child_by_field_name("type"))?;
    let return_node = if return_type.kind() == "type_annotation" {
        first_named_child(return_type).unwrap_or(return_type)
    } else {
        return_type
    };
    if return_node.kind() == "generic_type" {
        let normalized = normalize_type_text(source, return_node);
        if !normalized.is_empty() {
            return Some(normalized);
        }
    }
    if matches!(return_node.kind(), "array_type" | "tuple_type") {
        let normalized = normalize_type_text(source, return_node);
        if !normalized.is_empty() && normalized.len() <= MAX_TYPE_SHAPE_BYTES {
            return Some(normalized);
        }
    }
    if matches!(
        return_node.kind(),
        "identifier" | "type_identifier" | "predefined_type"
    ) {
        let name = node_text(source, return_node);
        if !name.is_empty() && name.len() <= MAX_TYPE_SHAPE_BYTES {
            return Some(name);
        }
    }
    let mut names = Vec::new();
    collect_type_name_nodes(return_type, &mut names);
    names
        .first()
        .map(|name| node_text(source, *name))
        .filter(|name| !name.is_empty())
}

fn normalize_type_text(source: &[u8], node: Node<'_>) -> String {
    let mut normalized = String::new();
    for character in node_text(source, node)
        .trim()
        .trim_start_matches(':')
        .chars()
    {
        if character.is_ascii_whitespace() {
            // Keep one separator after the `keyof` keyword.  The compact
            // signature format otherwise turns `keyof T` into `keyofT`,
            // which is indistinguishable from an ordinary identifier and
            // prevents bounded generic substitution from recognizing the
            // key-space expression.
            if normalized.ends_with("keyof") && !normalized.ends_with("keyof ") {
                normalized.push(' ');
            }
        } else {
            normalized.push(character);
        }
    }
    normalized
}

/// Return the small amount of declaration shape needed to follow a proven
/// TypeScript member value across files. This is deliberately not a full
/// source signature: a generic owner publishes its parameter order and a
/// property publishes its direct nominal type. The resolver can therefore
/// substitute `T` in `Box<T> { item: T }` only when the use site carries an
/// explicit `Box<Concrete>` argument.
fn typescript_declaration_signature(node: Node<'_>, kind: &str, source: &[u8]) -> Option<String> {
    if kind == "property"
        && let Some(type_name) = direct_type_reference_name(node, source)
    {
        return Some(type_name);
    }
    if kind == "variable"
        && let Some(type_name) = direct_type_reference_name(node, source)
    {
        let signature = format!("|type:{type_name}");
        if signature.len() <= MAX_TYPE_SHAPE_BYTES {
            return Some(signature);
        }
    }
    let callable_value = callable_value_node(node);
    let generic_parameter_order = callable_type_parameter_order(node, source)
        .or_else(|| callable_value.and_then(|value| callable_type_parameter_order(value, source)));
    let generic_prefix = generic_parameter_order
        .filter(|parameters| !parameters.is_empty())
        .map(|parameters| format!("<{}>", parameters.join(",")))
        .unwrap_or_default();
    if matches!(kind, "function" | "method" | "property") {
        let parameter_types = callable_parameter_types(node, source)
            .or_else(|| callable_value.and_then(|value| callable_parameter_types(value, source)));
        let return_type = callable_return_type_name(node, source);
        if parameter_types.is_some() || return_type.is_some() {
            let mut signature = generic_prefix.clone();
            if let Some(parameter_types) = parameter_types {
                signature.push_str("|params:");
                signature.push_str(&parameter_types.join(","));
            }
            if let Some(return_type) = return_type {
                signature.push_str("|return:");
                signature.push_str(&return_type);
            }
            if signature.len() <= MAX_TYPE_SHAPE_BYTES
                && (signature.contains("|params:") || signature.contains("|return:"))
            {
                return Some(signature);
            }
        }
    }
    if kind == "type_alias"
        && let Some(target) = direct_type_reference_name(node, source)
    {
        let signature = format!("{generic_prefix}={target}");
        if signature.len() <= MAX_TYPE_SHAPE_BYTES {
            return Some(signature);
        }
    }
    if matches!(kind, "interface" | "type_alias")
        && let Some(index_value) = index_value_type(node, source)
    {
        let signature = if generic_prefix.is_empty() {
            format!("index={index_value}")
        } else {
            format!("{generic_prefix}|index={index_value}")
        };
        if signature.len() <= MAX_TYPE_SHAPE_BYTES {
            return Some(signature);
        }
    }
    if matches!(kind, "class" | "interface" | "type_alias")
        && !generic_prefix.is_empty()
        && generic_prefix.len() <= MAX_TYPE_SHAPE_BYTES
    {
        return Some(generic_prefix);
    }
    None
}

fn direct_type_reference_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() == "type_alias_declaration"
        && let Some(mapped_source) = mapped_type_source_name(node, source)
    {
        return Some(mapped_source);
    }
    let annotation = node
        .child_by_field_name("type")
        .or_else(|| first_named_child_kind(node, "type_annotation"))
        .or_else(|| {
            (node.kind() == "type_alias_declaration")
                .then(|| {
                    node.child_by_field_name("value")
                        .or_else(|| first_named_child(node))
                })
                .flatten()
        })?;
    let type_node = if annotation.kind() == "type_annotation" {
        first_named_child(annotation)?
    } else {
        annotation
    };
    let normalized_type_text = node_text(source, type_node);
    if node.kind() != "type_alias_declaration"
        && type_node.kind() == "object_type"
        && contains_node_kind(type_node, "index_signature")
        && normalized_type_text.len() <= MAX_TYPE_SHAPE_BYTES
    {
        return Some(normalize_type_text(source, type_node));
    }
    if indexed_type_parts(&normalized_type_text).is_some()
        || keyof_type_base(&normalized_type_text).is_some()
    {
        if normalized_type_text.len() <= MAX_TYPE_SHAPE_BYTES {
            return Some(normalized_type_text);
        }
        return None;
    }
    if matches!(type_node.kind(), "array_type" | "tuple_type") {
        let normalized = normalize_type_text(source, type_node);
        if !normalized.is_empty() && normalized.len() <= MAX_TYPE_SHAPE_BYTES {
            return Some(normalized);
        }
    }
    if matches!(
        type_node.kind(),
        "conditional_type" | "indexed_access_type" | "keyof_type"
    ) {
        let normalized = node_text(source, type_node);
        if !normalized.is_empty()
            && normalized.len() <= MAX_TYPE_SHAPE_BYTES
            && (type_node.kind() != "conditional_type"
                || conditional_type_parts(&normalized).is_some())
        {
            return Some(normalized);
        }
    }
    let generic_text = (type_node.kind() == "generic_type")
        .then(|| normalize_type_text(source, type_node))
        .filter(|text| !text.is_empty());
    let type_node = match type_node.kind() {
        "generic_type" | "parenthesized_type" => first_named_child(type_node)?,
        _ => type_node,
    };
    if type_node.kind() == "union_type" {
        return union_property_type_name(node, type_node, source);
    }
    if let Some(generic_text) = generic_text {
        return Some(generic_text);
    }
    let type_name = match type_node.kind() {
        "identifier" | "type_identifier" | "predefined_type" => node_text(source, type_node),
        "nested_type_identifier" | "member_expression" | "qualified_name" => type_node
            .child_by_field_name("name")
            .or_else(|| type_node.child_by_field_name("property"))
            .map(|name| node_text(source, name))?,
        _ => return None,
    };
    (!type_name.is_empty()).then_some(type_name)
}

/// Return a source-preserved element type for a bounded array-like shape.
/// This is intentionally limited to postfix arrays and the standard generic
/// array containers; arbitrary structural/indexed expressions remain
/// unresolved rather than being treated as arrays by spelling alone.
fn array_element_type_name(type_name: &str) -> Option<String> {
    let type_name = type_name.trim();
    if let Some(element) = type_name.strip_suffix("[]") {
        let element = element.trim();
        return (!element.is_empty() && element.len() <= MAX_TYPE_SHAPE_BYTES)
            .then(|| element.to_owned());
    }
    let (base, arguments) = generic_type_parts(type_name)?;
    if !matches!(base, "Array" | "ReadonlyArray") || arguments.len() != 1 {
        return None;
    }
    Some(arguments[0].clone())
}

fn tuple_type_elements(type_name: &str) -> Option<Vec<&str>> {
    let type_name = type_name.trim();
    if !type_name.starts_with('[') || !type_name.ends_with(']') {
        return None;
    }
    let inner = type_name.get(1..type_name.len().saturating_sub(1))?.trim();
    if inner.is_empty() {
        return None;
    }
    split_top_level_arguments(inner)
}

fn tuple_type_element_count(type_name: &str) -> Option<usize> {
    tuple_type_elements(type_name).map(|elements| elements.len())
}

fn tuple_element_type_name(type_name: &str, index: &str) -> Option<String> {
    let index = index.parse::<usize>().ok()?;
    let element = tuple_type_elements(type_name)?.get(index)?.trim();
    if element.is_empty() || element.starts_with("...") || element.ends_with('?') {
        return None;
    }
    (element.len() <= MAX_TYPE_SHAPE_BYTES).then(|| element.to_owned())
}

fn indexed_sequence_element_type_name(type_name: &str, index: Option<&str>) -> Option<String> {
    if let Some(element) = array_element_type_name(type_name) {
        return Some(element);
    }
    index.and_then(|index| tuple_element_type_name(type_name, index))
}

const TYPESCRIPT_STRUCTURAL_INDEX_PREFIX: &str = "__compass_structural_index__";

fn typescript_structural_index_receiver(value_type: &str) -> String {
    format!("{TYPESCRIPT_STRUCTURAL_INDEX_PREFIX}{value_type}")
}

fn typescript_structural_index_value(value: &str) -> Option<&str> {
    let value = value.trim();
    value
        .strip_prefix(TYPESCRIPT_STRUCTURAL_INDEX_PREFIX)
        .filter(|value_type| !value_type.is_empty() && value_type.len() <= MAX_TYPE_SHAPE_BYTES)
}

/// Extract one bounded string/number index-signature value from an inline
/// object type such as `{[key:string]:Item}`. Mapped keys, multiple index
/// signatures, and malformed/oversized shapes stay unresolved rather than
/// widening a dynamic subscript to an arbitrary member.
fn typescript_inline_index_value_type(value: &str) -> Option<String> {
    let value = value.trim();
    if !value.starts_with('{') || !value.ends_with('}') || value.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    let open = value.find('[')?;
    let close = open.saturating_add(value.get(open..)?.find(']')?);
    let key = value.get(open.saturating_add(1)..close)?.trim();
    if key.is_empty() || key.contains(" in ") || key.starts_with("in ") {
        return None;
    }
    let remainder = value.get(close.saturating_add(1)..)?.trim_start();
    let colon = remainder.find(':')?;
    let mut tail = remainder.get(colon.saturating_add(1)..)?.trim();
    if tail.ends_with('}') {
        tail = tail.get(..tail.len().saturating_sub(1))?.trim_end();
    }
    if tail.is_empty() || tail.contains('[') {
        return None;
    }
    let mut depth = 0_u32;
    let mut end = tail.len();
    for (index, character) in tail.char_indices() {
        match character {
            '<' | '{' | '(' => depth = depth.saturating_add(1),
            '>' | '}' | ')' => depth = depth.saturating_sub(1),
            ';' | ',' if depth == 0 => {
                end = index;
                break;
            }
            _ => {}
        }
    }
    let value_type = tail.get(..end)?.trim();
    (!value_type.is_empty() && value_type.len() <= MAX_TYPE_SHAPE_BYTES)
        .then(|| value_type.to_owned())
}

fn infer_candidate_type_arguments(
    parameter: &str,
    argument: &str,
    parameters: &[String],
    arguments: &mut [String],
) -> bool {
    let parameter = parameter.trim();
    let argument = argument.trim();
    if parameter.is_empty() || argument.is_empty() {
        return true;
    }
    if let Some(index) = parameters.iter().position(|name| name == parameter) {
        let Some(slot) = arguments.get_mut(index) else {
            return true;
        };
        if slot
            == parameters
                .get(index)
                .map(String::as_str)
                .unwrap_or_default()
        {
            slot.clone_from(&argument.to_owned());
            return true;
        }
        return slot == argument;
    }
    if let (Some(parameter_element), Some(argument_element)) =
        (parameter.strip_suffix("[]"), argument.strip_suffix("[]"))
    {
        return infer_candidate_type_arguments(
            parameter_element,
            argument_element,
            parameters,
            arguments,
        );
    }
    let Some((parameter_base, parameter_arguments)) = generic_type_parts(parameter) else {
        return true;
    };
    let Some((argument_base, argument_arguments)) = generic_type_parts(argument) else {
        return true;
    };
    if parameter_base != argument_base || parameter_arguments.len() != argument_arguments.len() {
        return true;
    }
    for (parameter_argument, argument_argument) in
        parameter_arguments.iter().zip(argument_arguments.iter())
    {
        if !infer_candidate_type_arguments(
            parameter_argument,
            argument_argument,
            parameters,
            arguments,
        ) {
            return false;
        }
    }
    true
}

fn candidate_type_mentions_parameter(type_name: &str, parameters: &[String]) -> bool {
    type_name
        .split(|character: char| {
            !(character == '_' || character == '$' || character.is_ascii_alphanumeric())
        })
        .filter(|token| !token.is_empty())
        .any(|token| parameters.iter().any(|parameter| parameter == token))
}

const UTILITY_PROJECTION_PREFIX: &str = "__compass_utility_projection__";

fn encode_utility_projection(utility: &str, base: &str, keys: &HashSet<String>) -> String {
    let mut keys = keys.iter().cloned().collect::<Vec<_>>();
    keys.sort_unstable();
    format!(
        "{UTILITY_PROJECTION_PREFIX}{utility}|{base}|{}",
        keys.join(",")
    )
}

fn decode_utility_projection(type_name: &str) -> Option<(String, String, BTreeSet<String>)> {
    let encoded = type_name.strip_prefix(UTILITY_PROJECTION_PREFIX)?;
    let mut parts = encoded.splitn(3, '|');
    let utility = parts.next()?.to_owned();
    let base = parts.next()?.to_owned();
    let keys = parts
        .next()?
        .split(',')
        .filter(|key| !key.is_empty())
        .map(ToOwned::to_owned)
        .collect::<BTreeSet<_>>();
    (matches!(utility.as_str(), "Pick" | "Omit") && !base.is_empty() && !keys.is_empty())
        .then_some((utility, base, keys))
}

fn conditional_type_parts(type_name: &str) -> Option<(&str, &str, &str, &str)> {
    let type_name = type_name.trim();
    let question = top_level_delimiter(type_name, '?')?;
    let condition = type_name.get(..question)?.trim();
    let remainder = type_name.get(question.saturating_add(1)..)?.trim();
    let colon = top_level_delimiter(remainder, ':')?;
    let when_true = remainder.get(..colon)?.trim();
    let when_false = remainder.get(colon.saturating_add(1)..)?.trim();
    if condition.is_empty()
        || when_true.is_empty()
        || when_false.is_empty()
        || type_name.len() > MAX_TYPE_SHAPE_BYTES
    {
        return None;
    }
    let extends = condition.find("extends")?;
    let check = condition.get(..extends)?.trim();
    let expected = condition
        .get(extends.saturating_add("extends".len())..)?
        .trim();
    (!check.is_empty() && !expected.is_empty()).then_some((check, expected, when_true, when_false))
}

fn candidate_utility_receiver_type(type_name: &str) -> Option<String> {
    candidate_utility_receiver_type_at_depth(type_name, 0)
}

fn candidate_utility_receiver_type_at_depth(type_name: &str, depth: u32) -> Option<String> {
    if depth > MAX_TYPE_SHAPE_DEPTH {
        return None;
    }
    let (base, arguments) = generic_type_parts(type_name)?;
    if matches!(base, "Exclude" | "Extract") {
        if arguments.len() != 2 {
            return None;
        }
        let members =
            split_top_level_union(&arguments[0]).unwrap_or_else(|| vec![arguments[0].as_str()]);
        let filters =
            split_top_level_union(&arguments[1]).unwrap_or_else(|| vec![arguments[1].as_str()]);
        let selected = members
            .into_iter()
            .map(str::trim)
            .filter(|member| {
                let matches_filter = filters.iter().any(|filter| {
                    let filter = filter.trim();
                    filter == "unknown" || filter == "any" || type_names_compatible(member, filter)
                });
                (base == "Exclude" && !matches_filter) || (base == "Extract" && matches_filter)
            })
            .filter(|member| !is_non_nominal_union_member(member))
            .collect::<Vec<_>>();
        let [selected] = selected.as_slice() else {
            return None;
        };
        return Some((*selected).to_owned());
    }
    if arguments.len() != 1 {
        return None;
    }
    let argument = arguments[0].trim();
    match base {
        "NonNullable" => {
            let members = split_top_level_union(argument)?;
            let nominal = members
                .into_iter()
                .map(str::trim)
                .filter(|member| !is_non_nominal_union_member(member))
                .collect::<Vec<_>>();
            let [nominal] = nominal.as_slice() else {
                return None;
            };
            Some((*nominal).to_owned())
        }
        "Awaited" => {
            if let Some((promise, nested)) = generic_type_parts(argument)
                && matches!(promise, "Promise" | "PromiseLike")
                && nested.len() == 1
            {
                return candidate_utility_receiver_type_at_depth(
                    &format!("Awaited<{}>", nested[0]),
                    depth + 1,
                )
                .or_else(|| Some(nested[0].clone()));
            }
            Some(argument.to_owned())
        }
        "Partial" | "Required" | "Readonly" => Some(argument.to_owned()),
        _ => None,
    }
}

fn split_top_level_union(input: &str) -> Option<Vec<&str>> {
    let mut members = Vec::new();
    let mut start = 0_usize;
    let mut depth = 0_u32;
    for (index, character) in input.char_indices() {
        match character {
            '<' | '{' | '(' | '[' => depth = depth.checked_add(1)?,
            '>' | '}' | ')' | ']' => depth = depth.checked_sub(1)?,
            '|' if depth == 0 => {
                let member = input.get(start..index)?.trim();
                if member.is_empty() {
                    return None;
                }
                members.push(member);
                start = index.saturating_add(character.len_utf8());
            }
            _ => {}
        }
        if members.len() >= MAX_INLINE_OBJECT_PROPERTIES {
            return None;
        }
    }
    let member = input.get(start..)?.trim();
    if member.is_empty() {
        return None;
    }
    members.push(member);
    Some(members)
}

fn mapped_type_source_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "type_alias_declaration" {
        return None;
    }
    let value = node
        .child_by_field_name("value")
        .or_else(|| first_named_child(node))?;
    if value.kind() != "object_type" {
        return None;
    }
    let mut mapped = None;
    let mut cursor = value.walk();
    for child in value.named_children(&mut cursor) {
        if child.kind() != "index_signature"
            || !contains_node_kind(child, "mapped_type_clause")
            || mapped.replace(child).is_some()
        {
            // A mapped alias is accepted only when its object shape consists
            // of one direct homomorphic index signature. Nested mapped
            // members or additional properties would require structural
            // assignability and must remain unresolved here.
            return None;
        }
    }
    let mapped = mapped?;
    let compact = node_text(source, mapped)
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    if compact.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    let key_start = compact.find('[')?.saturating_add(1);
    let in_offset = compact.get(key_start..)?.find("inkeyof")?;
    let key = compact.get(key_start..key_start.saturating_add(in_offset))?;
    if key.is_empty()
        || !key.chars().all(|character| {
            character == '_' || character == '$' || character.is_ascii_alphanumeric()
        })
    {
        return None;
    }
    let source_start = key_start
        .saturating_add(in_offset)
        .saturating_add("inkeyof".len());
    let source_end = compact
        .get(source_start..)
        .and_then(|value| value.find(']').map(|offset| source_start + offset))?;
    let source_name = compact.get(source_start..source_end)?.trim();
    if source_name.is_empty() || source_name.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    let colon = compact.get(source_end.saturating_add(1)..)?.find(':')?;
    let value_start = source_end
        .saturating_add(1)
        .saturating_add(colon)
        .saturating_add(1);
    let value = compact
        .get(value_start..)
        .and_then(|value| value.strip_suffix('}'))
        .unwrap_or_else(|| compact.get(value_start..).unwrap_or_default())
        .trim_start_matches("readonly")
        .trim();
    let expected = format!("{}[{}]", source_name, key);
    (value == expected).then(|| source_name.to_owned())
}

fn substitute_candidate_type_parameters(
    type_name: &str,
    parameters: &[String],
    arguments: &[String],
    depth: u32,
) -> String {
    let type_name = type_name.trim();
    if type_name.is_empty()
        || type_name.len() > MAX_TYPE_SHAPE_BYTES
        || depth > MAX_TYPE_SHAPE_DEPTH
    {
        return type_name.to_owned();
    }
    if let Some(index) = parameters
        .iter()
        .position(|parameter| parameter == type_name)
        && let Some(argument) = arguments.get(index)
    {
        return argument.clone();
    }
    if let Some(key_base) = keyof_type_base(type_name) {
        let substituted =
            substitute_candidate_type_parameters(key_base, parameters, arguments, depth + 1);
        let substituted = format!("keyof {substituted}");
        return if substituted.len() <= MAX_TYPE_SHAPE_BYTES {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if indexed_type_parts(type_name).is_some()
        && let Some(open) = type_name.rfind('[')
    {
        let base = type_name.get(..open).unwrap_or_default();
        let substituted_base =
            substitute_candidate_type_parameters(base, parameters, arguments, depth + 1);
        let suffix = type_name.get(open..).unwrap_or_default();
        let substituted = format!("{substituted_base}{suffix}");
        return if substituted.len() <= MAX_TYPE_SHAPE_BYTES {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if let Some(element) = type_name.strip_suffix("[]") {
        let substituted =
            substitute_candidate_type_parameters(element, parameters, arguments, depth + 1);
        let substituted = format!("{substituted}[]");
        return if substituted.len() <= MAX_TYPE_SHAPE_BYTES {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if let Some(elements) = tuple_type_elements(type_name) {
        let substituted = elements
            .iter()
            .map(|element| {
                substitute_candidate_type_parameters(element, parameters, arguments, depth + 1)
            })
            .collect::<Vec<_>>();
        let substituted = format!("[{}]", substituted.join(","));
        return if substituted.len() <= MAX_TYPE_SHAPE_BYTES {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if let Some((check, expected, when_true, when_false)) = conditional_type_parts(type_name) {
        let substituted = format!(
            "{} extends {} ? {} : {}",
            substitute_candidate_type_parameters(check, parameters, arguments, depth + 1),
            substitute_candidate_type_parameters(expected, parameters, arguments, depth + 1),
            substitute_candidate_type_parameters(when_true, parameters, arguments, depth + 1),
            substitute_candidate_type_parameters(when_false, parameters, arguments, depth + 1),
        );
        return if substituted.len() <= MAX_TYPE_SHAPE_BYTES {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    if let Some(members) = split_top_level_union(type_name)
        && members.len() > 1
    {
        let substituted = members
            .iter()
            .map(|member| {
                substitute_candidate_type_parameters(member, parameters, arguments, depth + 1)
            })
            .collect::<Vec<_>>()
            .join("|");
        return if substituted.len() <= MAX_TYPE_SHAPE_BYTES {
            substituted
        } else {
            type_name.to_owned()
        };
    }
    let Some((base, nested)) = generic_type_parts(type_name) else {
        return type_name.to_owned();
    };
    let nested = nested
        .iter()
        .map(|argument| {
            substitute_candidate_type_parameters(argument, parameters, arguments, depth + 1)
        })
        .collect::<Vec<_>>();
    let substituted = format!("{base}<{}>", nested.join(","));
    if substituted.len() <= MAX_TYPE_SHAPE_BYTES {
        substituted
    } else {
        type_name.to_owned()
    }
}

/// Return the one nominal type preserved by an optional property union such
/// as `Callback | undefined`.  This is intentionally limited to declaration
/// nodes that publish a property: a union on a variable or type alias does
/// not prove a single receiver and must remain unresolved.  Primitive,
/// literal, and top-like union members are ignored; multiple nominal members
/// stay ambiguous.
fn union_property_type_name(node: Node<'_>, union_node: Node<'_>, source: &[u8]) -> Option<String> {
    if !matches!(
        node.kind(),
        "property_signature"
            | "public_field_definition"
            | "field_definition"
            | "property_declaration"
            | "property"
    ) {
        return None;
    }
    let text = node_text(source, union_node);
    if text.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    let mut names = Vec::new();
    let mut start = 0_usize;
    let mut angle = 0_u32;
    let mut bracket = 0_u32;
    let mut paren = 0_u32;
    let mut brace = 0_u32;
    for (index, character) in text.char_indices() {
        match character {
            '<' => angle = angle.saturating_add(1),
            '>' => angle = angle.saturating_sub(1),
            '[' => bracket = bracket.saturating_add(1),
            ']' => bracket = bracket.saturating_sub(1),
            '(' => paren = paren.saturating_add(1),
            ')' => paren = paren.saturating_sub(1),
            '{' => brace = brace.saturating_add(1),
            '}' => brace = brace.saturating_sub(1),
            '|' if angle == 0 && bracket == 0 && paren == 0 && brace == 0 => {
                if let Some(name) = union_target_name(text[start..index].trim())
                    && !is_non_nominal_union_member(&name)
                {
                    names.push(name);
                }
                start = index.saturating_add(character.len_utf8());
            }
            _ => {}
        }
        if names.len() > 1 {
            return None;
        }
    }
    if let Some(name) = union_target_name(text[start..].trim())
        && !is_non_nominal_union_member(&name)
    {
        names.push(name);
    }
    (names.len() == 1).then(|| names.remove(0))
}

fn is_non_nominal_union_member(name: &str) -> bool {
    matches!(
        name,
        "any"
            | "unknown"
            | "never"
            | "void"
            | "undefined"
            | "null"
            | "string"
            | "number"
            | "boolean"
            | "bigint"
            | "symbol"
            | "object"
            | "true"
            | "false"
    )
}

fn variable_binding_is_immutable(node: Node<'_>, source: &[u8]) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    if parent.kind() != "lexical_declaration" {
        return false;
    }
    node_text(source, parent)
        .split_whitespace()
        .next()
        .is_some_and(|keyword| keyword == "const")
}

fn object_literal_has_spread(node: Node<'_>) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .any(|child| child.kind() == "spread_element")
}

fn type_alias_union_targets(node: Node<'_>, source: &[u8]) -> Option<Vec<String>> {
    let text = node_text(source, node);
    if text.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    let (_, rhs) = text.split_once('=')?;
    let mut targets = Vec::new();
    let mut start = 0_usize;
    let mut angle = 0_u32;
    let mut bracket = 0_u32;
    let mut paren = 0_u32;
    let mut brace = 0_u32;
    for (index, character) in rhs.char_indices() {
        match character {
            '<' => angle = angle.saturating_add(1),
            '>' => angle = angle.saturating_sub(1),
            '[' => bracket = bracket.saturating_add(1),
            ']' => bracket = bracket.saturating_sub(1),
            '(' => paren = paren.saturating_add(1),
            ')' => paren = paren.saturating_sub(1),
            '{' => brace = brace.saturating_add(1),
            '}' => brace = brace.saturating_sub(1),
            '|' if angle == 0 && bracket == 0 && paren == 0 && brace == 0 => {
                let part = rhs[start..index].trim();
                if let Some(target) = union_target_name(part) {
                    targets.push(target);
                }
                start = index.saturating_add(character.len_utf8());
            }
            _ => {}
        }
        if targets.len() >= MAX_INLINE_OBJECT_PROPERTIES {
            return None;
        }
    }
    if targets.is_empty() {
        return None;
    }
    if let Some(target) = union_target_name(rhs[start..].trim()) {
        targets.push(target);
    }
    (targets.len() >= 2).then_some(targets)
}

fn property_literal_value(node: Node<'_>, source: &[u8]) -> Option<String> {
    let text = node_text(source, node);
    let (_, value) = text.split_once(':')?;
    let value = value
        .trim()
        .trim_end_matches(';')
        .trim_end_matches(',')
        .trim();
    let quoted = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        });
    if let Some(quoted) =
        quoted.filter(|value| !value.is_empty() && value.len() <= MAX_TYPE_SHAPE_BYTES)
    {
        return Some(quoted.to_owned());
    }
    matches!(value, "true" | "false").then(|| value.to_owned())
}

fn index_value_type(node: Node<'_>, source: &[u8]) -> Option<String> {
    let container = node
        .child_by_field_name("value")
        .or_else(|| node.child_by_field_name("body"))
        .or_else(|| node.child_by_field_name("type"))
        .or_else(|| node.child_by_field_name("type_annotation"))
        .or_else(|| (node.kind() == "object_type").then_some(node))?;
    let container = if container.kind() == "type_annotation" {
        first_named_child(container)?
    } else {
        container
    };
    let mut cursor = container.walk();
    let index_signature = container
        .named_children(&mut cursor)
        .find(|child| child.kind() == "index_signature")?;
    if contains_node_kind(index_signature, "mapped_type_clause") {
        return None;
    }
    let text = node_text(source, index_signature);
    if text.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    let close = text.find(']')?;
    let remainder = text.get(close + 1..)?;
    let colon = remainder.find(':')?;
    let value = remainder.get(colon + 1..)?.trim();
    let value = value
        .split([';', '}'])
        .next()
        .unwrap_or(value)
        .trim()
        .trim_start_matches("readonly ")
        .trim();
    if value.is_empty() || value.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    Some(value.to_owned())
}

fn union_target_name(part: &str) -> Option<String> {
    let part = part.trim().trim_end_matches(';').trim();
    if part.is_empty() || part.starts_with('{') || part.contains("=>") {
        return None;
    }
    let part = part
        .strip_prefix("readonly ")
        .or_else(|| part.strip_prefix("keyof "))
        .unwrap_or(part)
        .trim();
    let part = strip_type_arguments(part).trim();
    let part = part.strip_prefix("typeof ").unwrap_or(part).trim();
    if part.is_empty()
        || part.len() > MAX_TYPE_SHAPE_BYTES
        || !part.chars().all(|character| {
            character == '_'
                || character == '$'
                || character == '.'
                || character == ':'
                || character.is_ascii_alphanumeric()
        })
    {
        return None;
    }
    Some(part.to_owned())
}

fn indexed_type_parts(type_name: &str) -> Option<(&str, String)> {
    let type_name = type_name.trim();
    if !type_name.ends_with(']') {
        return None;
    }
    let open = type_name.rfind('[')?;
    let base = type_name.get(..open)?.trim();
    let raw_property = type_name.get(open + 1..type_name.len() - 1)?.trim();
    let property = raw_property
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            raw_property
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .or_else(|| {
            (!raw_property.is_empty()
                && raw_property
                    .chars()
                    .all(|character| character.is_ascii_digit()))
            .then_some(raw_property)
        })?;
    if base.is_empty() || property.is_empty() || property.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    Some((base, property.to_owned()))
}

fn keyof_type_base(type_name: &str) -> Option<&str> {
    let rest = type_name.trim().strip_prefix("keyof")?;
    rest.chars()
        .next()
        .filter(|character| character.is_whitespace())?;
    let base = rest.trim();
    (!base.is_empty()).then_some(base)
}

fn strip_type_arguments(type_name: &str) -> &str {
    let mut depth = 0_u32;
    for (index, character) in type_name.char_indices() {
        match character {
            '<' => depth = depth.saturating_add(1),
            '>' if depth > 0 => depth = depth.saturating_sub(1),
            _ => {}
        }
        if character == '<' && depth == 1 {
            return type_name[..index].trim();
        }
    }
    type_name.trim()
}

fn generic_type_parts(type_name: &str) -> Option<(&str, Vec<String>)> {
    let type_name = type_name.trim();
    let open = type_name.find('<')?;
    if !type_name.ends_with('>') || open == 0 {
        return None;
    }
    let base = type_name[..open].trim();
    let arguments_text = &type_name[open + 1..type_name.len() - 1];
    if base.is_empty() || arguments_text.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    let mut arguments = Vec::new();
    let mut start = 0_usize;
    let mut depth = 0_u32;
    for (index, character) in arguments_text.char_indices() {
        match character {
            '<' | '[' | '(' => depth = depth.saturating_add(1),
            '>' | ']' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let argument = arguments_text[start..index].trim();
                if argument.is_empty() || argument.len() > MAX_TYPE_SHAPE_BYTES {
                    return None;
                }
                arguments.push(argument.to_owned());
                start = index.saturating_add(character.len_utf8());
            }
            _ => {}
        }
        if arguments.len() >= MAX_INLINE_OBJECT_PROPERTIES {
            return None;
        }
    }
    let argument = arguments_text[start..].trim();
    if argument.is_empty() || argument.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    arguments.push(argument.to_owned());
    Some((base, arguments))
}

fn parameter_type_matches(parameter: &str, argument: &str) -> bool {
    parameter_type_matches_depth(parameter, argument, 0)
}

fn parameter_type_matches_depth(parameter: &str, argument: &str, depth: u32) -> bool {
    if depth > MAX_TYPE_SHAPE_DEPTH {
        return false;
    }
    let parameter = parameter.trim();
    let argument = argument.trim();
    if parameter == argument {
        return true;
    }
    if parameter == "any" || parameter == "unknown" {
        return true;
    }
    if parameter.starts_with("typeof") {
        return true;
    }
    if parameter.contains("=>") || parameter.contains("function") {
        return argument == "function" || argument == "any";
    }
    if let Some(base_type) = utility_base_type(parameter)
        && (argument == "object" || type_names_compatible(base_type, argument))
    {
        // `Pick<T, K>`, `Omit<T, K>`, and the other standard utility wrappers
        // preserve the source object's nominal identity for receiver/call
        // selection. Match only the first type argument; an unrelated base
        // type must remain unresolved rather than being widened structurally.
        return true;
    }
    if let Some(union) = parameter.strip_suffix("|") {
        return parameter_type_matches_depth(union, argument, depth.saturating_add(1));
    }
    if parameter.contains('|') {
        return parameter
            .split('|')
            .any(|member| parameter_type_matches_depth(member, argument, depth.saturating_add(1)));
    }
    // A source object literal is structurally assignable to an annotated
    // object/interface/utility type even when the local extractor cannot
    // prove every property at the call site. Preserve the declaration
    // target, but never widen primitive or callable signatures this way.
    if argument == "object" && is_object_like_type(parameter) {
        return true;
    }
    if parameter.ends_with("[]") {
        let element = parameter.trim_end_matches("[]").trim();
        if argument == "array" || type_names_compatible(element, argument) {
            return true;
        }
    }
    // Parameter annotations are often unqualified (`SourceLine[]`) while a
    // source-proven argument carries its module-qualified declaration name.
    // Compare the leaf only when at least one side is unqualified; two
    // independently qualified names remain distinct and fail closed.
    if type_names_compatible(parameter, argument) {
        return true;
    }
    if parameter.starts_with('"') || parameter.starts_with('\'') || parameter.starts_with('`') {
        return argument == "string";
    }
    match (parameter, argument) {
        ("string", "string")
        | ("number", "number")
        | ("boolean", "boolean")
        | ("null", "null")
        | ("undefined", "undefined")
        | ("object", "object")
        | ("Function", "function")
        | ("function", "function")
        | ("Array", "array")
        | ("unknown", _) => true,
        _ => parameter == argument,
    }
}

fn is_object_like_type(type_name: &str) -> bool {
    let type_name = type_name.trim();
    if type_name.starts_with('{')
        || type_name.starts_with("Pick<")
        || type_name.starts_with("Omit<")
        || type_name.starts_with("Partial<")
        || type_name.starts_with("Required<")
        || type_name.starts_with("Readonly<")
        || type_name.starts_with("Record<")
    {
        return true;
    }
    !matches!(
        type_name,
        "string"
            | "number"
            | "boolean"
            | "bigint"
            | "symbol"
            | "null"
            | "undefined"
            | "void"
            | "never"
            | "unknown"
            | "any"
            | "object"
            | "Function"
            | "function"
            | "Array"
    ) && !type_name.contains("=>")
}

fn utility_base_type(type_name: &str) -> Option<&str> {
    utility_type_parts(type_name).map(|(_, base, _)| base)
}

fn utility_type_parts(type_name: &str) -> Option<(&str, &str, Vec<&str>)> {
    let type_name = type_name.trim();
    if type_name.len() > MAX_TYPE_SHAPE_BYTES {
        return None;
    }
    let (utility, prefix) = [
        ("Pick", "Pick<"),
        ("Omit", "Omit<"),
        ("Exclude", "Exclude<"),
        ("Extract", "Extract<"),
        ("Partial", "Partial<"),
        ("Required", "Required<"),
        ("Readonly", "Readonly<"),
        ("Record", "Record<"),
    ]
    .into_iter()
    .find(|(_, prefix)| type_name.starts_with(*prefix))?;
    let inner = type_name.strip_prefix(prefix)?.strip_suffix('>')?;
    let arguments = split_top_level_arguments(inner)?;
    (arguments.len() >= 2).then_some((utility, arguments[0], arguments))
}

fn split_top_level_arguments(input: &str) -> Option<Vec<&str>> {
    let mut arguments = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    for (index, character) in input.char_indices() {
        match character {
            '<' | '{' | '(' | '[' => {
                depth = depth.checked_add(1)?;
                if depth > MAX_TYPE_SHAPE_DEPTH {
                    return None;
                }
            }
            '>' | '}' | ')' | ']' => {
                depth = depth.checked_sub(1)?;
            }
            ',' if depth == 0 => {
                arguments.push(input[start..index].trim());
                start = index.saturating_add(character.len_utf8());
                if arguments.len() >= MAX_INLINE_OBJECT_PROPERTIES {
                    return None;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    arguments.push(input[start..].trim());
    arguments
        .iter()
        .all(|argument| !argument.is_empty())
        .then_some(arguments)
}

fn type_literal_names(keys: &str) -> Option<HashSet<String>> {
    let mut names = HashSet::new();
    for key in keys.split('|').map(str::trim) {
        let key = key
            .strip_prefix('"')
            .and_then(|key| key.strip_suffix('"'))
            .or_else(|| {
                key.strip_prefix('\'')
                    .and_then(|key| key.strip_suffix('\''))
            })
            .or_else(|| key.strip_prefix('`').and_then(|key| key.strip_suffix('`')))?;
        if key.is_empty() || key.len() > MAX_TYPE_SHAPE_BYTES {
            return None;
        }
        names.insert(key.to_owned());
    }
    (!names.is_empty()).then_some(names)
}

fn inline_object_required_property_names(type_name: &str) -> Option<HashSet<String>> {
    let mut type_name = type_name.trim();
    while let Some(stripped) = type_name.strip_suffix("[]") {
        type_name = stripped.trim();
    }
    if type_name.len() > MAX_TYPE_SHAPE_BYTES
        || !type_name.starts_with('{')
        || !type_name.ends_with('}')
    {
        return None;
    }
    let inner = &type_name[1..type_name.len().saturating_sub(1)];
    let mut properties = HashSet::new();
    for member in split_top_level_members(inner)? {
        let member = member.trim();
        if member.is_empty() || member.starts_with("...") || member.starts_with('[') {
            continue;
        }
        let Some(colon) = top_level_delimiter(member, ':') else {
            continue;
        };
        let mut name = member[..colon].trim();
        if name.starts_with("readonly ") {
            name = name.trim_start_matches("readonly ").trim();
        }
        if name.ends_with('?') {
            continue;
        }
        let name = name
            .strip_prefix('"')
            .and_then(|name| name.strip_suffix('"'))
            .or_else(|| {
                name.strip_prefix('\'')
                    .and_then(|name| name.strip_suffix('\''))
            })
            .or_else(|| {
                name.strip_prefix('`')
                    .and_then(|name| name.strip_suffix('`'))
            })
            .unwrap_or(name)
            .trim();
        if !name.is_empty() {
            properties.insert(name.to_owned());
        }
        if properties.len() >= MAX_INLINE_OBJECT_PROPERTIES {
            break;
        }
    }
    Some(properties)
}

fn split_top_level_members(input: &str) -> Option<Vec<&str>> {
    let mut members = Vec::new();
    let mut start = 0;
    let mut depth = 0_u32;
    for (index, character) in input.char_indices() {
        match character {
            '<' | '{' | '(' | '[' => {
                depth = depth.checked_add(1)?;
                if depth > MAX_TYPE_SHAPE_DEPTH {
                    return None;
                }
            }
            '>' | '}' | ')' | ']' => depth = depth.checked_sub(1)?,
            ';' | ',' if depth == 0 => {
                members.push(input[start..index].trim());
                start = index.saturating_add(character.len_utf8());
                if members.len() >= MAX_INLINE_OBJECT_PROPERTIES {
                    return None;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return None;
    }
    members.push(input[start..].trim());
    Some(members)
}

fn top_level_delimiter(input: &str, delimiter: char) -> Option<usize> {
    let mut depth = 0_u32;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if let Some(quoted) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == quoted {
                quote = None;
            }
            continue;
        }
        if matches!(character, '\'' | '"' | '`') {
            quote = Some(character);
            continue;
        }
        match character {
            '<' | '{' | '(' | '[' => depth = depth.checked_add(1)?,
            '>' | '}' | ')' | ']' => depth = depth.checked_sub(1)?,
            character if character == delimiter && depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

fn type_names_compatible(left: &str, right: &str) -> bool {
    // Generic arguments affect assignability, but they are not needed to
    // choose a unique source declaration when the surrounding evidence has
    // already proven the nominal base. Erase only the bounded outer argument
    // list (`Result<T>` -> `Result`) before comparing qualified leaves; this
    // recovers calls through imported/local generic aliases without widening
    // two independently qualified base names.
    let left = strip_type_arguments(left.trim())
        .trim_end_matches("[]")
        .trim();
    let right = strip_type_arguments(right.trim())
        .trim_end_matches("[]")
        .trim();
    if left == right || left.is_empty() || right.is_empty() {
        return left == right;
    }
    let left_qualified = left.contains('.') || left.contains("::");
    let right_qualified = right.contains('.') || right.contains("::");
    if left_qualified && right_qualified {
        return false;
    }
    let left_leaf = left.rsplit(['.', ':']).next().unwrap_or(left);
    let right_leaf = right.rsplit(['.', ':']).next().unwrap_or(right);
    left_leaf == right_leaf
}

/// Infer the only arity that is source-proven for a class construction. A
/// class without an explicit constructor accepts zero arguments by default;
/// one fixed constructor gives its exact arity. Optional, rest, overloaded,
/// or otherwise incomplete constructor declarations deliberately return
/// `None` so call resolution stays unresolved instead of guessing.
fn class_constructor_parameter_arity(node: Node<'_>, source: &[u8]) -> Option<(u32, Option<u32>)> {
    let body = node
        .child_by_field_name("body")
        .or_else(|| first_named_child_kind(node, "class_body"))?;
    let mut constructor_count = 0_usize;
    let mut result = None;
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if !matches!(child.kind(), "method_definition" | "method_signature") {
            continue;
        }
        let Some(name) = child.child_by_field_name("name") else {
            continue;
        };
        if node_text(source, name) != "constructor" {
            continue;
        }
        constructor_count = constructor_count.saturating_add(1);
        let arity = callable_parameter_arity(child, source);
        if constructor_count == 1 {
            result = arity;
        } else if result != arity {
            return None;
        }
    }
    if constructor_count == 0 {
        Some((0, Some(0)))
    } else {
        result
    }
}

fn class_has_explicit_constructor(node: Node<'_>, source: &[u8]) -> bool {
    let Some(body) = node
        .child_by_field_name("body")
        .or_else(|| first_named_child_kind(node, "class_body"))
    else {
        return false;
    };
    let mut cursor = body.walk();
    body.named_children(&mut cursor).any(|child| {
        matches!(child.kind(), "method_definition" | "method_signature")
            && child
                .child_by_field_name("name")
                .is_some_and(|name| node_text(source, name) == "constructor")
    })
}

fn call_argument_count(node: Node<'_>) -> Option<u32> {
    let arguments = node
        .child_by_field_name("arguments")
        .or_else(|| first_named_child_kind(node, "arguments"))?;
    let mut cursor = arguments.walk();
    let count = arguments
        .named_children(&mut cursor)
        .filter(|argument| !argument.kind().contains("comment"))
        .count();
    u32::try_from(count).ok()
}

fn collect_pattern_names<'tree>(
    node: Node<'tree>,
    source: &[u8],
    output: &mut Vec<(String, Node<'tree>)>,
) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" | "this" => {
            let name = node_text(source, node);
            if !name.is_empty() {
                output.push((name, node));
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                collect_pattern_names(child, source, output);
            }
        }
    }
}

fn direct_require_call<'tree>(node: Node<'tree>, source: &[u8]) -> Option<Node<'tree>> {
    let node = unwrap_expression_node(node);
    (node.kind() == "call_expression"
        && node
            .child_by_field_name("function")
            .is_some_and(|function| node_text(source, function) == "require"))
    .then_some(node)
}

fn static_require_property_name(source: &[u8], node: Node<'_>) -> Option<String> {
    let node = unwrap_expression_node(node);
    let name = match node.kind() {
        "identifier" | "property_identifier" | "private_property_identifier" | "number" => {
            node_text(source, node)
        }
        "string" | "string_fragment" => string_literal(source, node),
        _ => return None,
    };
    (!name.is_empty() && name.len() <= MAX_TYPE_SHAPE_BYTES).then_some(name)
}

fn direct_require_binding_identifier<'tree>(
    node: Node<'tree>,
    source: &[u8],
) -> Option<(String, Node<'tree>)> {
    let node = unwrap_expression_node(node);
    if matches!(
        node.kind(),
        "identifier" | "shorthand_property_identifier_pattern"
    ) {
        let name = node_text(source, node);
        return (!name.is_empty()).then_some((name, node));
    }
    if node.kind() == "assignment_pattern"
        && let Some(left) = node.child_by_field_name("left")
    {
        return direct_require_binding_identifier(left, source);
    }
    None
}

fn collect_import_bindings<'tree>(
    source: &[u8],
    clause: Node<'tree>,
    reexport: bool,
    statement_type_only: bool,
    output: &mut Vec<ImportBinding<'tree>>,
) {
    if reexport && clause.kind() == "export_clause" {
        let mut specifiers = Vec::new();
        collect_nodes_of_kind(clause, "export_specifier", &mut specifiers);
        for specifier in specifiers {
            let identifiers = direct_identifier_children(specifier, source);
            let Some(imported) = identifiers.first().copied() else {
                continue;
            };
            let local = identifiers.get(1).copied().unwrap_or(imported);
            output.push(ImportBinding {
                local_name: node_text(source, local),
                imported_name: node_text(source, imported),
                anchor: local,
                type_only: statement_type_only
                    || node_text(source, specifier)
                        .trim_start()
                        .starts_with("type "),
            });
        }
        return;
    }
    let mut cursor = clause.walk();
    for child in clause.named_children(&mut cursor) {
        match child.kind() {
            "identifier" if !reexport => output.push(ImportBinding {
                local_name: node_text(source, child),
                imported_name: "default".to_owned(),
                anchor: child,
                type_only: statement_type_only,
            }),
            "namespace_import" => {
                if let Some(name) = first_identifier_node(child) {
                    output.push(ImportBinding {
                        local_name: node_text(source, name),
                        imported_name: "*".to_owned(),
                        anchor: name,
                        type_only: statement_type_only,
                    });
                }
            }
            "named_imports" | "export_clause" => {
                let mut specifiers = Vec::new();
                collect_nodes_of_kind(
                    child,
                    if reexport {
                        "export_specifier"
                    } else {
                        "import_specifier"
                    },
                    &mut specifiers,
                );
                for specifier in specifiers {
                    let identifiers = direct_identifier_children(specifier, source);
                    let Some(imported) = identifiers.first().copied() else {
                        continue;
                    };
                    let local = identifiers.get(1).copied().unwrap_or(imported);
                    output.push(ImportBinding {
                        local_name: node_text(source, local),
                        imported_name: node_text(source, imported),
                        anchor: local,
                        type_only: statement_type_only
                            || node_text(source, specifier)
                                .trim_start()
                                .starts_with("type "),
                    });
                }
            }
            _ => {}
        }
    }
}

fn collect_type_name_nodes<'tree>(node: Node<'tree>, output: &mut Vec<Node<'tree>>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if matches!(child.kind(), "nested_type_identifier" | "member_expression") {
            let property = child
                .child_by_field_name("name")
                .or_else(|| child.child_by_field_name("property"));
            if let Some(property) = property {
                output.push(property);
            } else {
                output.push(child);
            }
        } else if matches!(child.kind(), "type_identifier" | "identifier") {
            output.push(child);
        } else if child.kind() != "type_arguments" {
            collect_type_name_nodes(child, output);
        }
    }
}

fn direct_identifier_children<'tree>(node: Node<'tree>, source: &[u8]) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| {
            matches!(
                child.kind(),
                "identifier" | "type_identifier" | "property_identifier"
            )
        })
        .filter(|child| !node_text(source, *child).is_empty())
        .collect()
}

fn collect_nodes_of_kind<'tree>(node: Node<'tree>, kind: &str, output: &mut Vec<Node<'tree>>) {
    if node.kind() == kind {
        output.push(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_nodes_of_kind(child, kind, output);
    }
}

fn first_named_child_kind<'tree>(node: Node<'tree>, kind: &str) -> Option<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn first_identifier_node<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "property_identifier" | "private_property_identifier"
    ) {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find_map(first_identifier_node)
}

fn rightmost_identifier<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    if matches!(
        node.kind(),
        "identifier" | "type_identifier" | "property_identifier" | "private_property_identifier"
    ) {
        return Some(node);
    }
    if matches!(
        node.kind(),
        "member_expression" | "optional_member_expression"
    ) {
        return node
            .child_by_field_name("property")
            .and_then(rightmost_identifier);
    }
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .last()
        .and_then(rightmost_identifier)
}

fn member_property_name(source: &[u8], property: Node<'_>) -> Option<String> {
    let computed = is_computed_member_property(property);
    if computed
        && !matches!(
            property.kind(),
            "string" | "string_fragment" | "template_string" | "number"
        )
    {
        return None;
    }
    let name = if matches!(
        property.kind(),
        "string" | "string_fragment" | "template_string"
    ) {
        string_literal(source, property)
    } else {
        node_text(source, property)
    };
    (!name.is_empty()).then_some(name)
}

fn is_computed_member_property(property: Node<'_>) -> bool {
    property.parent().is_some_and(|parent| {
        let mut cursor = parent.walk();
        parent
            .children(&mut cursor)
            .any(|child| matches!(child.kind(), "[" | "]"))
    })
}

fn member_property_node<'tree>(node: Node<'tree>) -> Option<Node<'tree>> {
    node.child_by_field_name("property")
        .or_else(|| node.child_by_field_name("index"))
}

fn known_builtin_static_member(receiver: &str, property: &str) -> bool {
    match receiver {
        "Array" => matches!(property, "from" | "isArray" | "of"),
        "Boolean" => matches!(property, "prototype"),
        "Date" => matches!(property, "now" | "parse" | "UTC"),
        "JSON" => matches!(property, "parse" | "stringify"),
        "Math" => matches!(
            property,
            "abs" | "ceil" | "floor" | "max" | "min" | "pow" | "random" | "round" | "trunc"
        ),
        "Number" => matches!(
            property,
            "isFinite" | "isInteger" | "isNaN" | "isSafeInteger" | "parseFloat" | "parseInt"
        ),
        "Object" => matches!(
            property,
            "assign"
                | "create"
                | "defineProperty"
                | "entries"
                | "freeze"
                | "fromEntries"
                | "getOwnPropertyDescriptor"
                | "getOwnPropertyNames"
                | "hasOwn"
                | "keys"
                | "values"
        ),
        "Promise" => matches!(
            property,
            "all" | "allSettled" | "any" | "race" | "reject" | "resolve"
        ),
        "Reflect" => matches!(
            property,
            "apply" | "construct" | "defineProperty" | "get" | "has" | "ownKeys" | "set"
        ),
        "RegExp" => matches!(property, "escape"),
        "String" => matches!(property, "fromCharCode" | "fromCodePoint" | "raw"),
        "console" => matches!(
            property,
            "assert"
                | "debug"
                | "dir"
                | "error"
                | "info"
                | "log"
                | "table"
                | "time"
                | "timeEnd"
                | "trace"
                | "warn"
        ),
        "ArrayBuffer" => matches!(property, "isView"),
        _ => false,
    }
}

fn known_builtin_instance_member(receiver: &str, property: &str) -> bool {
    match receiver {
        "Array" => matches!(
            property,
            "at" | "concat"
                | "entries"
                | "every"
                | "fill"
                | "filter"
                | "find"
                | "findIndex"
                | "findLast"
                | "findLastIndex"
                | "flat"
                | "flatMap"
                | "forEach"
                | "includes"
                | "indexOf"
                | "join"
                | "keys"
                | "lastIndexOf"
                | "map"
                | "pop"
                | "push"
                | "reduce"
                | "reduceRight"
                | "reverse"
                | "shift"
                | "slice"
                | "some"
                | "sort"
                | "splice"
                | "toReversed"
                | "toSorted"
                | "toSpliced"
                | "unshift"
                | "values"
        ),
        "Boolean" => matches!(property, "toString" | "valueOf"),
        "Date" => matches!(
            property,
            "getDate"
                | "getDay"
                | "getFullYear"
                | "getHours"
                | "getMilliseconds"
                | "getMinutes"
                | "getMonth"
                | "getSeconds"
                | "getTime"
                | "getTimezoneOffset"
                | "getUTCDate"
                | "getUTCDay"
                | "getUTCFullYear"
                | "getUTCHours"
                | "getUTCMilliseconds"
                | "getUTCMinutes"
                | "getUTCMonth"
                | "getUTCSeconds"
                | "setDate"
                | "setFullYear"
                | "setHours"
                | "setMilliseconds"
                | "setMinutes"
                | "setMonth"
                | "setSeconds"
                | "setTime"
                | "setUTCDate"
                | "setUTCFullYear"
                | "setUTCHours"
                | "setUTCMilliseconds"
                | "setUTCMinutes"
                | "setUTCMonth"
                | "setUTCSeconds"
                | "toDateString"
                | "toISOString"
                | "toJSON"
                | "toLocaleDateString"
                | "toLocaleString"
                | "toLocaleTimeString"
                | "toString"
                | "toTimeString"
                | "toUTCString"
                | "valueOf"
        ),
        "Number" => matches!(
            property,
            "toExponential" | "toFixed" | "toLocaleString" | "toPrecision" | "toString" | "valueOf"
        ),
        "String" => matches!(
            property,
            "at" | "charAt"
                | "charCodeAt"
                | "codePointAt"
                | "concat"
                | "endsWith"
                | "includes"
                | "indexOf"
                | "lastIndexOf"
                | "match"
                | "matchAll"
                | "normalize"
                | "padEnd"
                | "padStart"
                | "repeat"
                | "replace"
                | "replaceAll"
                | "search"
                | "slice"
                | "split"
                | "startsWith"
                | "substring"
                | "substr"
                | "toLocaleLowerCase"
                | "toLocaleUpperCase"
                | "toLowerCase"
                | "toString"
                | "toUpperCase"
                | "trim"
                | "trimEnd"
                | "trimStart"
                | "valueOf"
        ),
        "RegExp" => matches!(property, "compile" | "exec" | "test" | "toString"),
        "ArrayBuffer" => matches!(
            property,
            "byteLength" | "detached" | "maxByteLength" | "resizable" | "resize" | "slice"
        ),
        "DataView" => matches!(
            property,
            "getBigInt64"
                | "getBigUint64"
                | "getFloat32"
                | "getFloat64"
                | "getInt16"
                | "getInt32"
                | "getInt8"
                | "getUint16"
                | "getUint32"
                | "getUint8"
                | "setBigInt64"
                | "setBigUint64"
                | "setFloat32"
                | "setFloat64"
                | "setInt16"
                | "setInt32"
                | "setInt8"
                | "setUint16"
                | "setUint32"
                | "setUint8"
        ),
        "Map" => matches!(
            property,
            "delete" | "entries" | "forEach" | "get" | "has" | "keys" | "set" | "values"
        ),
        "Set" => matches!(
            property,
            "add" | "delete" | "entries" | "forEach" | "has" | "keys" | "values"
        ),
        "WeakMap" => matches!(property, "delete" | "get" | "has" | "set"),
        "WeakSet" => matches!(property, "add" | "delete" | "has"),
        "Promise" => matches!(property, "catch" | "finally" | "then"),
        _ => false,
    }
}

fn is_typescript_utility_type(name: &str) -> bool {
    matches!(
        name,
        "ArrayLike"
            | "Awaited"
            | "Capitalize"
            | "ConstructorParameters"
            | "ThisType"
            | "Exclude"
            | "Extract"
            | "InstanceType"
            | "Lowercase"
            | "NoInfer"
            | "NonNullable"
            | "Omit"
            | "OmitThisParameter"
            | "Parameters"
            | "Partial"
            | "Pick"
            | "Readonly"
            | "ReadonlyArray"
            | "Record"
            | "Required"
            | "ReturnType"
            | "ThisParameterType"
            | "Uncapitalize"
            | "Uppercase"
            | "Iterable"
            | "IterableIterator"
            | "Iterator"
            | "IteratorResult"
            | "AsyncIterable"
            | "AsyncIterableIterator"
            | "AsyncIterator"
            | "AsyncIteratorResult"
            | "PromiseLike"
            | "PropertyKey"
            | "ArrayConstructor"
            | "Function"
            | "ObjectConstructor"
            | "StringConstructor"
            | "NumberConstructor"
            | "BooleanConstructor"
            | "RegExpConstructor"
            | "DateConstructor"
            | "ErrorConstructor"
    )
}

fn standard_library_type_target(name: &str) -> Option<(String, String)> {
    if matches!(
        name,
        "AggregateError"
            | "ArrayConstructor"
            | "ArrayLike"
            | "ArrayBufferView"
            | "AsyncGenerator"
            | "AsyncGeneratorFunction"
            | "AsyncIterator"
            | "AsyncIteratorResult"
            | "DataView"
            | "ErrorOptions"
            | "Float32Array"
            | "Float64Array"
            | "Generator"
            | "GeneratorFunction"
            | "Int8Array"
            | "Int16Array"
            | "Int32Array"
            | "Iterator"
            | "IteratorResult"
            | "MapConstructor"
            | "PromiseLike"
            | "RegExpExecArray"
            | "RegExpMatchArray"
            | "RegExpSplitMatchArray"
            | "ReadonlyMap"
            | "ReadonlySet"
            | "SetConstructor"
            | "SharedArrayBuffer"
            | "Uint8Array"
            | "Uint8ClampedArray"
            | "Uint16Array"
            | "Uint32Array"
            | "WeakKeyTypes"
            | "WeakMapConstructor"
            | "WeakSetConstructor"
    ) {
        return Some((
            format!("typescript.lib::{name}"),
            "typescript.lib".to_owned(),
        ));
    }
    if matches!(
        name,
        "AbortController"
            | "AbortSignal"
            | "Animation"
            | "Blob"
            | "CSSStyleDeclaration"
            | "CanvasRenderingContext2D"
            | "CustomEvent"
            | "Document"
            | "Element"
            | "Event"
            | "EventTarget"
            | "File"
            | "FileList"
            | "FormData"
            | "Headers"
            | "HTMLElement"
            | "HTMLDivElement"
            | "HTMLInputElement"
            | "HTMLTextAreaElement"
            | "ImageData"
            | "MessageEvent"
            | "MouseEvent"
            | "Node"
            | "ReadableStream"
            | "Request"
            | "Response"
            | "Storage"
            | "TextDecoder"
            | "TextEncoder"
            | "URL"
            | "URLSearchParams"
            | "WebSocket"
            | "Window"
            | "Worker"
    ) {
        return Some((format!("dom.lib::{name}"), "lib.dom".to_owned()));
    }
    if name == "Buffer" {
        return Some(("node.global::Buffer".to_owned(), "@types/node".to_owned()));
    }
    None
}

fn is_type_reference_node(node: Node<'_>) -> bool {
    if node.kind() == "type_identifier" || node.kind() == "nested_type_identifier" {
        return true;
    }
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(
            parent.kind(),
            "type_annotation"
                | "return_type"
                | "type_arguments"
                | "type_parameters"
                | "generic_type"
                | "predefined_type"
                | "infer_type"
                | "type_alias_declaration"
                | "interface_declaration"
        ) {
            return true;
        }
        if matches!(
            parent.kind(),
            "expression_statement"
                | "statement_block"
                | "program"
                | "arguments"
                | "formal_parameters"
        ) {
            break;
        }
        current = parent.parent();
    }
    false
}

fn module_name(path: &Path, source_file: &str) -> String {
    let name = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            Path::new(source_file)
                .file_stem()
                .and_then(|value| value.to_str())
        })
        .unwrap_or("module");
    name.to_owned()
}

fn dialect_for_path(path: &Path, fallback: &str) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| {
            matches!(
                extension.as_str(),
                "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts"
            )
        })
        .unwrap_or_else(|| fallback.to_owned())
}

fn stable_graph_id(source_file: &str, kind: &str, name: &str, start: usize) -> String {
    let start = start.to_string();
    make_id(&[source_file, kind, name, &start])
}

fn fact_key(role: SemanticRole, node: Node<'_>, context: Option<&str>) -> String {
    format!(
        "{}:{}:{}:{}",
        role_name(role),
        node.start_byte(),
        node.end_byte(),
        context.unwrap_or_default()
    )
}

fn role_name(role: SemanticRole) -> &'static str {
    match role {
        SemanticRole::Import => "import",
        SemanticRole::Reexport => "reexport",
        SemanticRole::Alias => "alias",
        SemanticRole::Call => "call",
        SemanticRole::CallableReference => "callable_reference",
        SemanticRole::Construction => "construction",
        SemanticRole::Decorator => "decorator",
        SemanticRole::Annotation => "annotation",
        SemanticRole::BaseType => "base_type",
        SemanticRole::TypeReference => "type_reference",
        SemanticRole::MemberAccess => "member_access",
        SemanticRole::Ownership => "ownership",
        SemanticRole::Receiver => "receiver",
        SemanticRole::Embedding => "embedding",
        SemanticRole::TraitBound => "trait_bound",
        SemanticRole::MacroInvocation => "macro_invocation",
    }
}

fn string_literal(source: &[u8], node: Node<'_>) -> String {
    node_text(source, node)
        .trim()
        .trim_matches(['\'', '"', '`'])
        .to_owned()
}

fn declaration_name_text(source: &[u8], node: Node<'_>) -> String {
    if node.kind() == "string" {
        let literal = string_literal(source, node);
        if literal.is_empty() {
            // An empty string key is valid JavaScript but cannot become an
            // empty graph identity. Preserve its quoted source spelling as a
            // deterministic, non-empty declaration name; computed access
            // remains unresolved unless a non-empty proof exists.
            node_text(source, node)
        } else {
            literal
        }
    } else {
        node_text(source, node)
    }
}

fn node_text(source: &[u8], node: Node<'_>) -> String {
    source
        .get(node.start_byte()..node.end_byte())
        .map_or_else(String::new, |bytes| {
            String::from_utf8_lossy(bytes).into_owned()
        })
}
