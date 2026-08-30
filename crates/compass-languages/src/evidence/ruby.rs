//! Conservative universal evidence for Ruby.
//!
//! Ruby's syntax is intentionally separated from Ruby's runtime.  This module
//! records source-grounded declarations, lexical scopes, literal bindings,
//! mixins, hierarchy facts, and calls.  Dynamic evaluation, load paths, and
//! receiver dispatch are retained as diagnostics rather than guessed edges.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use tree_sitter::Node;

use super::build::{EvidenceBuilder, range_for_node};
use super::model::{
    BindingKind, CandidateRelation, EvidenceRange, HierarchyConstraint, ReceiverDispatchStrategy,
    ResolutionConstraint, SemanticEvidenceBatch, SemanticRole, SymbolNamespace,
};
use super::validate::{EvidenceError, EvidenceErrorCode, EvidenceLimits};
use crate::make_id;

// Keep the recursive AST visitor well below the default Rayon worker stack.
// Rails contains generated/nested DSL expressions that can otherwise make a
// 512-frame visitor overflow the native worker stack before the bounded
// evidence limit is reached.  Deep subtrees are reported as partial evidence
// rather than risking a process abort.
const MAX_TRAVERSAL_DEPTH: usize = 32;
const MAX_LITERAL_BYTES: usize = 4 * 1024;

/// Ruby remains in qualification while the independent corpus audit is
/// complete; the production registry intentionally exposes the same pipeline
/// used by qualification tooling so the two paths cannot drift.
use crate::evidence_pipeline::RUBY_EVIDENCE_PIPELINE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MethodSpace {
    Instance,
    Singleton,
}

impl MethodSpace {
    const fn separator(self) -> &'static str {
        match self {
            Self::Instance => "#",
            Self::Singleton => ".",
        }
    }

    const fn context(self) -> &'static str {
        match self {
            Self::Instance => "instance",
            Self::Singleton => "singleton",
        }
    }
}

#[derive(Clone, Debug)]
struct ScopeFrame {
    scope_id: String,
    owner_declaration_id: String,
    owner_qualified_name: String,
    lexical_prefix: String,
    receiver_qualified_name: Option<String>,
    receiver_scope_id: Option<String>,
    method_space: Option<MethodSpace>,
    method_name: Option<String>,
    local_bindings: HashSet<String>,
    local_receivers: HashMap<String, String>,
}

#[derive(Clone, Debug, Default)]
struct TypeInfo {
    declaration_id: Option<String>,
    scope_id: Option<String>,
}

#[derive(Clone, Debug)]
struct MethodOwner {
    declaration_id: String,
    scope_id: String,
    qualified_name: String,
}

struct RubyState<'source> {
    source_file: &'source str,
    source: &'source [u8],
    builder: EvidenceBuilder,
    frames: Vec<ScopeFrame>,
    types: BTreeMap<String, TypeInfo>,
    emitted_diagnostics: HashSet<String>,
}

pub(crate) fn emit_tree_evidence(
    path: &Path,
    source_file: &str,
    source: &[u8],
    root: Node<'_>,
) -> Result<SemanticEvidenceBatch, EvidenceError> {
    let mut state = RubyState::new(path, source_file, source, root)?;
    state.extract(root)?;
    state.builder.finish()
}

impl<'source> RubyState<'source> {
    fn new(
        path: &'source Path,
        source_file: &'source str,
        source: &'source [u8],
        root: Node<'_>,
    ) -> Result<Self, EvidenceError> {
        let mut builder = EvidenceBuilder::new_with_dialect(
            &RUBY_EVIDENCE_PIPELINE,
            "compass.languages.ruby.universal",
            source_file,
            EvidenceLimits::default(),
            Some("ruby"),
        );
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(source_file);
        let file_graph_id = stable_graph_id("file", source_file);
        let file_range = range_for_node(source_file, root);
        let file_declaration_id = builder.declare_with_namespace(
            "file",
            &file_graph_id,
            file_name,
            source_file,
            Some(source_file),
            None,
            Some(SymbolNamespace::Namespace),
            file_range.clone(),
        )?;
        let root_scope_id =
            builder.open_scope("module", Some(&file_declaration_id), None, file_range)?;
        let root_frame = ScopeFrame {
            scope_id: root_scope_id.clone(),
            owner_declaration_id: file_declaration_id.clone(),
            owner_qualified_name: source_file.to_owned(),
            lexical_prefix: String::new(),
            receiver_qualified_name: None,
            receiver_scope_id: None,
            method_space: None,
            method_name: None,
            local_bindings: HashSet::new(),
            local_receivers: HashMap::new(),
        };
        Ok(Self {
            source_file,
            source,
            builder,
            frames: vec![root_frame],
            types: BTreeMap::new(),
            emitted_diagnostics: HashSet::new(),
        })
    }

    fn extract(&mut self, root: Node<'_>) -> Result<(), EvidenceError> {
        if root.has_error() {
            self.diagnose_once(
                "partial_parser_recovery",
                Some(range_for_node(self.source_file, root)),
                "parser recovered from malformed Ruby source; only trusted facts are emitted",
            )?;
        }
        self.index_types(root, String::new(), 0);
        self.walk(root, 0)
    }

    fn index_types(&mut self, node: Node<'_>, prefix: String, depth: usize) {
        if depth > MAX_TRAVERSAL_DEPTH {
            return;
        }
        if matches!(node.kind(), "class" | "module") {
            let Some(name_node) = node.child_by_field_name("name") else {
                return;
            };
            let Some(raw_name) = self.text(name_node) else {
                return;
            };
            let qualified = qualify(&prefix, &raw_name);
            self.types.entry(qualified.clone()).or_default();
            if let Some(body) = node.child_by_field_name("body") {
                self.index_types(body, qualified, depth.saturating_add(1));
            }
            return;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            self.index_types(child, prefix.clone(), depth.saturating_add(1));
        }
    }

    fn walk(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        if depth > MAX_TRAVERSAL_DEPTH {
            self.diagnose_once(
                "traversal_limit",
                Some(range_for_node(self.source_file, node)),
                "Ruby syntax traversal depth exceeded the bounded candidate limit",
            )?;
            return Ok(());
        }
        if self.overlaps_error(node) {
            return Ok(());
        }
        match node.kind() {
            "class" => return self.walk_type(node, false, depth),
            "module" => return self.walk_type(node, true, depth),
            "method" => {
                let space = self.current().method_space.unwrap_or(MethodSpace::Instance);
                return self.walk_method(node, space, depth);
            }
            "singleton_method" => return self.walk_singleton_method(node, depth),
            "singleton_class" => return self.walk_singleton_class(node, depth),
            "call" => self.emit_call(node)?,
            "super" => self.emit_super(node)?,
            "alias" => self.emit_alias(node)?,
            "assignment" => self.emit_assignment(node)?,
            "identifier" => self.emit_bare_call(node)?,
            "block" | "do_block" | "lambda" => return self.walk_block(node, depth),
            _ => {}
        }
        self.walk_named_children(node, depth)
    }

    fn walk_named_children(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor).filter(Node::is_named) {
            self.walk(child, depth.saturating_add(1))?;
        }
        Ok(())
    }

    fn walk_type(
        &mut self,
        node: Node<'_>,
        is_module: bool,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        let Some(name_node) = node.child_by_field_name("name") else {
            self.diagnose_once(
                "missing_type_name",
                Some(range_for_node(self.source_file, node)),
                "Ruby class/module declaration has no trusted name",
            )?;
            return Ok(());
        };
        let Some(raw_name) = self.text(name_node) else {
            return Ok(());
        };
        let current_prefix = self.current().lexical_prefix.clone();
        let qualified_name = qualify(&current_prefix, &raw_name);
        let kind = if is_module { "trait" } else { "class" };
        let name = last_component(&raw_name);
        let graph_node_id = stable_graph_id(kind, &qualified_name);
        let parent_owner_id = self.current().owner_declaration_id.clone();
        let parent_scope_id = self.current().scope_id.clone();
        let declaration_id = self.builder.declare_with_namespace(
            kind,
            &graph_node_id,
            &name,
            &qualified_name,
            package_of(&qualified_name),
            Some(&parent_scope_id),
            Some(if is_module {
                SymbolNamespace::Namespace
            } else {
                SymbolNamespace::Value
            }),
            range_for_node(self.source_file, name_node),
        )?;
        self.emit_contains(&parent_owner_id, &declaration_id, &name, kind)?;
        let body_range = node.child_by_field_name("body").map_or_else(
            || range_for_node(self.source_file, node),
            |body| range_for_node(self.source_file, body),
        );
        let scope_id = self.builder.open_scope(
            if is_module { "trait" } else { "class" },
            Some(&declaration_id),
            Some(&parent_scope_id),
            body_range,
        )?;
        let type_info = self.types.entry(qualified_name.clone()).or_default();
        if type_info.declaration_id.is_none() {
            type_info.declaration_id = Some(declaration_id.clone());
            type_info.scope_id = Some(scope_id.clone());
        }
        let frame = ScopeFrame {
            scope_id: scope_id.clone(),
            owner_declaration_id: declaration_id.clone(),
            owner_qualified_name: qualified_name.clone(),
            lexical_prefix: qualified_name.clone(),
            receiver_qualified_name: Some(qualified_name.clone()),
            receiver_scope_id: Some(scope_id.clone()),
            method_space: None,
            method_name: None,
            local_bindings: HashSet::new(),
            local_receivers: HashMap::new(),
        };
        self.frames.push(frame);
        if let Some(superclass) = node.child_by_field_name("superclass") {
            self.emit_hierarchy(node, superclass, &declaration_id, &qualified_name)?;
        }
        if let Some(body) = node.child_by_field_name("body") {
            self.walk_named_children(body, depth.saturating_add(1))?;
        }
        self.frames.pop();
        Ok(())
    }

    fn walk_method(
        &mut self,
        node: Node<'_>,
        space: MethodSpace,
        depth: usize,
    ) -> Result<(), EvidenceError> {
        self.walk_method_owned(node, space, depth, None)
    }

    fn walk_method_owned(
        &mut self,
        node: Node<'_>,
        space: MethodSpace,
        depth: usize,
        owner: Option<MethodOwner>,
    ) -> Result<(), EvidenceError> {
        let Some(name_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let Some(name) = self.text(name_node) else {
            return Ok(());
        };
        let receiver_name = owner.as_ref().map_or_else(
            || {
                self.current()
                    .receiver_qualified_name
                    .clone()
                    .unwrap_or_else(|| self.source_file.to_owned())
            },
            |owner| owner.qualified_name.clone(),
        );
        let qualified_name = format!("{receiver_name}{}{name}", space.separator());
        let graph_node_id = stable_graph_id("method", &qualified_name);
        let parent_scope_id = owner.as_ref().map_or_else(
            || self.current().scope_id.clone(),
            |owner| owner.scope_id.clone(),
        );
        let declaration_id = self.builder.declare_with_namespace(
            "method",
            &graph_node_id,
            &name,
            &qualified_name,
            Some(&receiver_name),
            Some(&parent_scope_id),
            Some(SymbolNamespace::Value),
            range_for_node(self.source_file, name_node),
        )?;
        let parent_owner = owner.as_ref().map_or_else(
            || self.current().owner_declaration_id.clone(),
            |owner| owner.declaration_id.clone(),
        );
        self.emit_contains(&parent_owner, &declaration_id, &name, "method")?;
        let method_scope_range = node.child_by_field_name("body").map_or_else(
            || range_for_node(self.source_file, node),
            |body| range_for_node(self.source_file, body),
        );
        let method_scope_id = self.builder.open_scope(
            "method",
            Some(&declaration_id),
            Some(&parent_scope_id),
            method_scope_range,
        )?;
        let frame = ScopeFrame {
            scope_id: method_scope_id,
            owner_declaration_id: declaration_id.clone(),
            owner_qualified_name: format!("{receiver_name}{}{name}", space.separator()),
            lexical_prefix: receiver_name.clone(),
            receiver_qualified_name: Some(receiver_name),
            receiver_scope_id: owner
                .as_ref()
                .map(|owner| owner.scope_id.clone())
                .or_else(|| self.current().receiver_scope_id.clone()),
            method_space: Some(space),
            method_name: Some(name.to_owned()),
            local_bindings: HashSet::new(),
            local_receivers: HashMap::new(),
        };
        self.frames.push(frame);
        self.emit_parameters(node)?;
        if let Some(body) = node.child_by_field_name("body") {
            self.walk_named_children(body, depth.saturating_add(1))?;
        }
        self.frames.pop();
        Ok(())
    }

    fn walk_singleton_method(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        let Some(object) = node.child_by_field_name("object") else {
            return self.walk_method(node, MethodSpace::Singleton, depth);
        };
        let Some(object_name) = self.text(object) else {
            return Ok(());
        };
        if object_name == "self" {
            if self.current().receiver_qualified_name.is_none() {
                self.diagnose_once(
                    "singleton_owner_unresolved",
                    Some(range_for_node(self.source_file, object)),
                    "Ruby singleton method has no source-visible self owner",
                )?;
                return Ok(());
            }
            return self.walk_method(node, MethodSpace::Singleton, depth);
        }
        if !is_constant_path(&object_name) {
            self.diagnose_once(
                "singleton_owner_unresolved",
                Some(range_for_node(self.source_file, object)),
                "Ruby singleton method owner is not a source-visible constant",
            )?;
            return Ok(());
        }
        let qualified = self.resolve_constant_name(&object_name);
        let Some(type_info) = self.types.get(&qualified).cloned() else {
            self.diagnose_once(
                "singleton_owner_unresolved",
                Some(range_for_node(self.source_file, object)),
                "Ruby singleton method owner is not an indexed source declaration",
            )?;
            return Ok(());
        };
        let Some(declaration_id) = type_info.declaration_id else {
            self.diagnose_once(
                "singleton_owner_unresolved",
                Some(range_for_node(self.source_file, object)),
                "Ruby singleton method owner declaration is not source-grounded",
            )?;
            return Ok(());
        };
        let Some(scope_id) = type_info.scope_id else {
            self.diagnose_once(
                "singleton_owner_unresolved",
                Some(range_for_node(self.source_file, object)),
                "Ruby singleton method owner scope is not source-grounded",
            )?;
            return Ok(());
        };
        self.walk_method_owned(
            node,
            MethodSpace::Singleton,
            depth,
            Some(MethodOwner {
                declaration_id,
                scope_id,
                qualified_name: qualified,
            }),
        )
    }

    fn walk_singleton_class(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        let parent = self.current().clone();
        let Some(receiver) = parent.receiver_qualified_name.clone() else {
            return self.walk_named_children(node, depth.saturating_add(1));
        };
        let scope_id = self.builder.open_scope(
            "singleton_class",
            Some(&parent.owner_declaration_id),
            Some(&parent.scope_id),
            range_for_node(self.source_file, node),
        )?;
        self.frames.push(ScopeFrame {
            scope_id,
            owner_declaration_id: parent.owner_declaration_id,
            owner_qualified_name: parent.owner_qualified_name,
            lexical_prefix: parent.lexical_prefix,
            receiver_qualified_name: Some(receiver),
            receiver_scope_id: parent.receiver_scope_id,
            method_space: Some(MethodSpace::Singleton),
            method_name: parent.method_name,
            local_bindings: parent.local_bindings,
            local_receivers: parent.local_receivers,
        });
        self.walk_named_children(node, depth.saturating_add(1))?;
        self.frames.pop();
        Ok(())
    }

    fn walk_block(&mut self, node: Node<'_>, depth: usize) -> Result<(), EvidenceError> {
        let parent = self.current().clone();
        let scope_id = self.builder.open_scope(
            "block",
            Some(&parent.owner_declaration_id),
            Some(&parent.scope_id),
            range_for_node(self.source_file, node),
        )?;
        self.frames.push(ScopeFrame { scope_id, ..parent });
        self.walk_named_children(node, depth.saturating_add(1))?;
        self.frames.pop();
        Ok(())
    }

    fn emit_parameters(&mut self, method: Node<'_>) -> Result<(), EvidenceError> {
        let Some(parameters) = method.child_by_field_name("parameters") else {
            return Ok(());
        };
        let owner = self.current().owner_qualified_name.clone();
        let scope_id = self.current().scope_id.clone();
        let mut cursor = parameters.walk();
        for parameter in parameters.children(&mut cursor).filter(Node::is_named) {
            let Some(name_node) = parameter
                .child_by_field_name("name")
                .or_else(|| (parameter.kind() == "identifier").then_some(parameter))
            else {
                continue;
            };
            let Some(name) = self.text(name_node) else {
                continue;
            };
            if name.is_empty() || name == "_" {
                continue;
            }
            if let Some(frame) = self.frames.last_mut() {
                frame.local_bindings.insert(name.clone());
            }
            let qualified_name = format!("{owner}.{name}");
            let graph_node_id = stable_graph_id("parameter", &qualified_name);
            let declaration_id = self.builder.declare_with_namespace(
                "parameter",
                &graph_node_id,
                &name,
                &qualified_name,
                Some(&owner),
                Some(&scope_id),
                Some(SymbolNamespace::Value),
                range_for_node(self.source_file, name_node),
            )?;
            self.emit_contains(
                &self.current().owner_declaration_id.clone(),
                &declaration_id,
                &name,
                "parameter",
            )?;
        }
        Ok(())
    }

    fn emit_hierarchy(
        &mut self,
        owner_node: Node<'_>,
        superclass: Node<'_>,
        owner_declaration_id: &str,
        owner_qualified_name: &str,
    ) -> Result<(), EvidenceError> {
        let Some(raw) = self
            .text_node_child(superclass)
            .or_else(|| self.text(superclass))
        else {
            return Ok(());
        };
        let qualified = self.resolve_constant_name(&raw);
        let scope_id = self.current().scope_id.clone();
        let Some(occurrence) = self
            .builder
            .occur_with_context(
                SemanticRole::BaseType,
                owner_declaration_id,
                &raw,
                Some(&qualified),
                Some(&scope_id),
                Some("superclass"),
                range_for_node(self.source_file, superclass),
            )
            .ok()
        else {
            return Ok(());
        };
        let constraints = ResolutionConstraint {
            exact_language: Some("ruby".to_owned()),
            qualified_name: Some(qualified.clone()),
            allowed_target_kinds: vec!["class".to_owned(), "trait".to_owned()],
            hierarchy: Some(HierarchyConstraint::DirectBase {
                base_set_complete: true,
            }),
            ..ResolutionConstraint::default()
        };
        let mut constraints = constraints;
        constraints.scope_id = Some(scope_id);
        self.builder.relate(
            CandidateRelation::Extends,
            owner_declaration_id,
            Some(&occurrence),
            None,
            last_component(&raw).as_str(),
            constraints,
        )?;
        let _ = owner_node;
        let _ = owner_qualified_name;
        Ok(())
    }

    fn emit_call(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(method_node) = node.child_by_field_name("method") else {
            return Ok(());
        };
        let Some(mut method_name) = self.text(method_node) else {
            return Ok(());
        };
        // Tree-sitter represents a receiver setter (`Config.value = x`) as a
        // call whose method token is `value`, with the assignment operator
        // outside that token.  Preserve Ruby's setter identity in the
        // candidate while keeping the exact UTF-8 anchor on the method name.
        let is_setter = node
            .parent()
            .filter(|parent| parent.kind() == "assignment")
            .and_then(|parent| parent.child_by_field_name("left"))
            .is_some_and(|left| left.id() == node.id());
        if is_setter {
            method_name.push('=');
        }
        if method_name.is_empty() || method_name.len() > MAX_LITERAL_BYTES {
            return Ok(());
        }
        let implicit_receiver = node
            .child_by_field_name("receiver")
            .and_then(|receiver| self.text(receiver))
            .is_none_or(|receiver| receiver == "self");
        if matches!(
            method_name.as_str(),
            "send" | "public_send" | "method_missing" | "eval" | "class_eval" | "module_eval"
        ) {
            self.diagnose_once(
                "dynamic_dispatch_unresolved",
                Some(range_for_node(self.source_file, node)),
                "dynamic Ruby dispatch is intentionally unresolved",
            )?;
            return Ok(());
        }
        if implicit_receiver && matches!(method_name.as_str(), "include" | "prepend" | "extend") {
            return self.emit_mixin(node, &method_name);
        }
        if implicit_receiver && matches!(method_name.as_str(), "require" | "require_relative") {
            return self.emit_require(node, &method_name);
        }
        if implicit_receiver && method_name == "autoload" {
            return self.emit_autoload(node);
        }
        if implicit_receiver
            && matches!(
                method_name.as_str(),
                "attr_reader" | "attr_writer" | "attr_accessor"
            )
        {
            return self.emit_attributes(node, &method_name);
        }
        if implicit_receiver && method_name == "alias_method" {
            return self.emit_alias_method(node);
        }
        if implicit_receiver && method_name == "define_method" {
            return self.emit_define_method(node);
        }
        let source_owner = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let receiver_node = node.child_by_field_name("receiver");
        let receiver_text = receiver_node.and_then(|receiver| self.text(receiver));
        let receiver_qualified = receiver_node
            .and_then(|receiver| self.receiver_type(receiver))
            .or_else(|| {
                self.current()
                    .method_space
                    .and(self.current().receiver_qualified_name.clone())
            });
        let argument_count = node
            .child_by_field_name("arguments")
            .map(count_arguments)
            .or_else(|| (node.child_by_field_name("arguments").is_none()).then_some(0));
        let call_space = self.call_method_space(receiver_text.as_deref());
        let occurrence_role = SemanticRole::Call;
        let qualifier = receiver_text.as_deref();
        let context = call_space.map(MethodSpace::context);
        let occurrence = self.builder.occur_with_context(
            occurrence_role,
            &source_owner,
            &method_name,
            qualifier,
            Some(&scope_id),
            context,
            range_for_node(self.source_file, method_node),
        )?;
        let is_constructor = method_name == "new"
            && receiver_qualified.is_some()
            && (receiver_node.is_some() || call_space == Some(MethodSpace::Singleton));
        let relation = if is_constructor {
            CandidateRelation::Constructs
        } else {
            CandidateRelation::Calls
        };
        let target_qualified = receiver_qualified.as_ref().map(|receiver| {
            let space = if receiver_text.as_deref() == Some("self") {
                call_space.unwrap_or(MethodSpace::Singleton)
            } else {
                MethodSpace::Instance
            };
            if is_constructor {
                receiver.clone()
            } else {
                format!("{receiver}{}{method_name}", space.separator())
            }
        });
        let hierarchy = (!is_constructor)
            .then_some(receiver_qualified.as_ref())
            .flatten()
            .map(|receiver| HierarchyConstraint::ReceiverDispatch {
                receiver_qualified_name: receiver.clone(),
                strategy: ReceiverDispatchStrategy::C3FromReceiver,
            });
        let qualified_name = hierarchy.is_none().then_some(target_qualified).flatten();
        let constraints = ResolutionConstraint {
            exact_language: Some("ruby".to_owned()),
            qualified_name,
            argument_count,
            allowed_target_kinds: if is_constructor {
                vec!["class".to_owned(), "trait".to_owned()]
            } else {
                vec!["method".to_owned(), "function".to_owned()]
            },
            hierarchy,
            ..ResolutionConstraint::default()
        };
        self.builder.relate(
            relation,
            &source_owner,
            Some(&occurrence),
            None,
            &method_name,
            constraints,
        )?;
        if let Some(left) = node.parent().filter(|parent| parent.kind() == "assignment")
            && let Some(receiver) = receiver_qualified
            && method_name == "new"
            && let Some(name) = left
                .child_by_field_name("left")
                .and_then(|left| self.text(left))
            && let Some(frame) = self.frames.last_mut()
        {
            frame.local_receivers.insert(name, receiver);
        }
        Ok(())
    }

    fn emit_bare_call(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(method_name) = self.text(node) else {
            return Ok(());
        };
        if method_name.is_empty()
            || method_name.len() > MAX_LITERAL_BYTES
            || self.current().method_space.is_none()
            || self.lookup_local_receiver(&method_name).is_some()
            || self.current().local_bindings.contains(&method_name)
            || node.parent().is_some_and(|parent| {
                parent.kind() == "call"
                    && parent
                        .child_by_field_name("method")
                        .is_some_and(|method| method.id() == node.id())
            })
        {
            return Ok(());
        }
        let owner_id = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let Some(receiver) = self.current().receiver_qualified_name.clone() else {
            return Ok(());
        };
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Call,
            &owner_id,
            &method_name,
            None,
            Some(&scope_id),
            Some(
                self.current()
                    .method_space
                    .map_or("instance", MethodSpace::context),
            ),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            CandidateRelation::Calls,
            &owner_id,
            Some(&occurrence),
            None,
            &method_name,
            ResolutionConstraint {
                exact_language: Some("ruby".to_owned()),
                allowed_target_kinds: vec!["method".to_owned(), "function".to_owned()],
                hierarchy: Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name: receiver,
                    strategy: ReceiverDispatchStrategy::C3FromReceiver,
                }),
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_super(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(method_space) = self.current().method_space else {
            return Ok(());
        };
        let Some(receiver) = self.current().receiver_qualified_name.clone() else {
            return Ok(());
        };
        let method_name = self
            .current()
            .method_name
            .as_deref()
            .map(str::to_owned)
            .unwrap_or_default();
        if method_name.is_empty() {
            return Ok(());
        }
        let owner_id = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Call,
            &owner_id,
            "super",
            Some(&receiver),
            Some(&scope_id),
            Some(method_space.context()),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            CandidateRelation::Calls,
            &owner_id,
            Some(&occurrence),
            None,
            &method_name,
            ResolutionConstraint {
                exact_language: Some("ruby".to_owned()),
                allowed_target_kinds: vec!["method".to_owned()],
                hierarchy: Some(HierarchyConstraint::ReceiverDispatch {
                    receiver_qualified_name: receiver,
                    strategy: ReceiverDispatchStrategy::C3AfterReceiver,
                }),
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_mixin(&mut self, node: Node<'_>, operation: &str) -> Result<(), EvidenceError> {
        let Some(argument) = first_argument(node) else {
            self.diagnose_once(
                "dynamic_mixin_unresolved",
                Some(range_for_node(self.source_file, node)),
                "Ruby mixin target is not a single source-visible constant",
            )?;
            return Ok(());
        };
        let Some(raw) = self.text(argument) else {
            return Ok(());
        };
        if !is_constant_path(&raw) {
            self.diagnose_once(
                "dynamic_mixin_unresolved",
                Some(range_for_node(self.source_file, argument)),
                "Ruby mixin target is dynamic or not a constant path",
            )?;
            return Ok(());
        }
        let qualified = self.resolve_constant_name(&raw);
        let owner_id = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let occurrence = self.builder.occur_with_context(
            SemanticRole::TraitBound,
            &owner_id,
            &raw,
            Some(&qualified),
            Some(&scope_id),
            Some(operation),
            range_for_node(self.source_file, argument),
        )?;
        self.builder.relate(
            CandidateRelation::UsesTrait,
            &owner_id,
            Some(&occurrence),
            None,
            last_component(&raw).as_str(),
            ResolutionConstraint {
                exact_language: Some("ruby".to_owned()),
                qualified_name: Some(qualified),
                scope_id: Some(scope_id),
                allowed_target_kinds: vec!["trait".to_owned()],
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_require(&mut self, node: Node<'_>, method_name: &str) -> Result<(), EvidenceError> {
        let Some(argument) = first_argument(node) else {
            self.diagnose_once(
                "dynamic_require_unresolved",
                Some(range_for_node(self.source_file, node)),
                "dynamic Ruby require target is intentionally unresolved",
            )?;
            return Ok(());
        };
        let Some(raw) = literal_string(argument, self.source) else {
            self.diagnose_once(
                "dynamic_require_unresolved",
                Some(range_for_node(self.source_file, argument)),
                "Ruby require target is not a literal string",
            )?;
            return Ok(());
        };
        if raw.len() > MAX_LITERAL_BYTES {
            return Err(EvidenceError::new(
                EvidenceErrorCode::ResourceLimit,
                "Ruby require literal exceeds the bounded evidence size",
            ));
        }
        let owner_id = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Import,
            &owner_id,
            &raw,
            None,
            Some(&scope_id),
            Some(method_name),
            range_for_node(self.source_file, argument),
        )?;
        let binding = self.builder.bind(
            BindingKind::Import,
            &raw,
            &raw,
            None,
            Some(&scope_id),
            range_for_node(self.source_file, argument),
        )?;
        self.builder.relate(
            CandidateRelation::Imports,
            &owner_id,
            Some(&occurrence),
            Some(&binding),
            &raw,
            ResolutionConstraint {
                exact_language: Some("ruby".to_owned()),
                qualified_name: Some(raw.clone()),
                allowed_target_kinds: vec![
                    "file".to_owned(),
                    "module".to_owned(),
                    "trait".to_owned(),
                ],
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_autoload(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return Ok(());
        };
        let mut cursor = arguments.walk();
        let mut values = arguments.children(&mut cursor).filter(Node::is_named);
        let Some(constant) = values.next() else {
            return Ok(());
        };
        let Some(path) = values
            .next()
            .and_then(|node| literal_string(node, self.source))
        else {
            self.diagnose_once(
                "dynamic_autoload_unresolved",
                Some(range_for_node(self.source_file, node)),
                "Ruby autoload path is not a literal string",
            )?;
            return Ok(());
        };
        let Some(constant_name) = self.text(constant) else {
            return Ok(());
        };
        let qualified = self.resolve_constant_name(&constant_name);
        let owner_id = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Import,
            &owner_id,
            &path,
            Some(&qualified),
            Some(&scope_id),
            Some("autoload"),
            range_for_node(self.source_file, node),
        )?;
        let binding = self.builder.bind(
            BindingKind::Import,
            &constant_name,
            &path,
            None,
            Some(&scope_id),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            CandidateRelation::Imports,
            &owner_id,
            Some(&occurrence),
            Some(&binding),
            &constant_name,
            ResolutionConstraint {
                exact_language: Some("ruby".to_owned()),
                qualified_name: Some(path),
                allowed_target_kinds: vec![
                    "file".to_owned(),
                    "module".to_owned(),
                    "trait".to_owned(),
                ],
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_alias(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(alias_node) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let Some(target_node) = node.child_by_field_name("alias") else {
            return Ok(());
        };
        let Some(alias) = self.text(alias_node).map(|value| strip_symbol(&value)) else {
            return Ok(());
        };
        let Some(target) = self.text(target_node).map(|value| strip_symbol(&value)) else {
            return Ok(());
        };
        let owner = self.current().receiver_qualified_name.clone();
        let target_qualified = owner.as_ref().map(|owner| format!("{owner}#{target}"));
        let owner_id = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Alias,
            &owner_id,
            &alias,
            Some(&target),
            Some(&scope_id),
            Some("alias"),
            range_for_node(self.source_file, node),
        )?;
        let binding = self.builder.bind(
            BindingKind::LocalAlias,
            &alias,
            target_qualified.as_deref().unwrap_or(&target),
            None,
            Some(&scope_id),
            range_for_node(self.source_file, alias_node),
        )?;
        self.builder.relate(
            CandidateRelation::References,
            &owner_id,
            Some(&occurrence),
            Some(&binding),
            &target,
            ResolutionConstraint {
                exact_language: Some("ruby".to_owned()),
                qualified_name: target_qualified,
                allowed_target_kinds: vec!["method".to_owned()],
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_alias_method(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return Ok(());
        };
        let mut cursor = arguments.walk();
        let mut values = arguments.children(&mut cursor).filter(Node::is_named);
        let Some(alias) = values
            .next()
            .and_then(|value| literal_string(value, self.source))
        else {
            self.diagnose_once(
                "dynamic_alias_unresolved",
                Some(range_for_node(self.source_file, node)),
                "Ruby alias_method name is not a literal symbol or string",
            )?;
            return Ok(());
        };
        let Some(target) = values
            .next()
            .and_then(|value| literal_string(value, self.source))
        else {
            self.diagnose_once(
                "dynamic_alias_unresolved",
                Some(range_for_node(self.source_file, node)),
                "Ruby alias_method target is not a literal symbol or string",
            )?;
            return Ok(());
        };
        self.emit_method_alias(node, &alias, &target)
    }

    fn emit_method_alias(
        &mut self,
        node: Node<'_>,
        alias: &str,
        target: &str,
    ) -> Result<(), EvidenceError> {
        let Some(owner) = self.current().receiver_qualified_name.clone() else {
            self.diagnose_once(
                "dynamic_alias_unresolved",
                Some(range_for_node(self.source_file, node)),
                "Ruby method alias has no source-visible owner",
            )?;
            return Ok(());
        };
        let owner_id = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let space = self.current().method_space.unwrap_or(MethodSpace::Instance);
        let target_qualified = format!("{owner}{}{target}", space.separator());
        let alias_qualified = format!("{owner}{}{alias}", space.separator());
        let alias_id = self.builder.declare_with_namespace(
            "method",
            &stable_graph_id("method", &alias_qualified),
            alias,
            &alias_qualified,
            Some(&owner),
            Some(&scope_id),
            Some(SymbolNamespace::Value),
            range_for_node(self.source_file, node),
        )?;
        self.emit_contains(&owner_id, &alias_id, alias, "method")?;
        let occurrence = self.builder.occur_with_context(
            SemanticRole::Alias,
            &alias_id,
            target,
            Some(&target_qualified),
            Some(&scope_id),
            Some("alias_method"),
            range_for_node(self.source_file, node),
        )?;
        self.builder.relate(
            CandidateRelation::References,
            &alias_id,
            Some(&occurrence),
            None,
            target,
            ResolutionConstraint {
                exact_language: Some("ruby".to_owned()),
                qualified_name: Some(target_qualified),
                allowed_target_kinds: vec!["method".to_owned()],
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn emit_define_method(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(argument) = first_argument(node) else {
            return Ok(());
        };
        let Some(name) = literal_string(argument, self.source) else {
            self.diagnose_once(
                "dynamic_define_method_unresolved",
                Some(range_for_node(self.source_file, node)),
                "Ruby define_method name is not a literal symbol or string",
            )?;
            return Ok(());
        };
        let Some(owner) = self.current().receiver_qualified_name.clone() else {
            self.diagnose_once(
                "dynamic_define_method_unresolved",
                Some(range_for_node(self.source_file, node)),
                "Ruby define_method has no source-visible owner",
            )?;
            return Ok(());
        };
        let owner_id = self.current().owner_declaration_id.clone();
        let scope_id = self.current().scope_id.clone();
        let qualified_name = format!("{owner}#{name}");
        let declaration_id = self.builder.declare_with_namespace(
            "method",
            &stable_graph_id("method", &qualified_name),
            &name,
            &qualified_name,
            Some(&owner),
            Some(&scope_id),
            Some(SymbolNamespace::Value),
            range_for_node(self.source_file, argument),
        )?;
        self.emit_contains(&owner_id, &declaration_id, &name, "method")?;
        Ok(())
    }

    fn emit_attributes(&mut self, node: Node<'_>, method_name: &str) -> Result<(), EvidenceError> {
        let Some(owner_type) = self.current().receiver_qualified_name.clone() else {
            return Ok(());
        };
        let Some(arguments) = node.child_by_field_name("arguments") else {
            return Ok(());
        };
        let receiver_scope_id = self.current().receiver_scope_id.clone();
        let method_separator = self
            .current()
            .method_space
            .filter(|space| *space == MethodSpace::Singleton)
            .map_or("#", |_| ".");
        let mut cursor = arguments.walk();
        for argument in arguments.children(&mut cursor).filter(Node::is_named) {
            let Some(raw) = self.text(argument) else {
                continue;
            };
            let name = raw.trim_start_matches(':').trim_matches(['"', '\'']);
            if name.is_empty() || name.len() > MAX_LITERAL_BYTES {
                continue;
            }
            let field_name = if method_name == "attr_writer" {
                format!("{name}=")
            } else {
                name.to_owned()
            };
            let qualified_name = format!("{owner_type}.{field_name}");
            let graph_node_id = stable_graph_id("field", &qualified_name);
            let declaration_id = self.builder.declare_with_namespace(
                "field",
                &graph_node_id,
                &field_name,
                &qualified_name,
                Some(&owner_type),
                receiver_scope_id.as_deref(),
                Some(SymbolNamespace::Value),
                range_for_node(self.source_file, argument),
            )?;
            self.emit_contains(
                &self.current().owner_declaration_id.clone(),
                &declaration_id,
                &field_name,
                "field",
            )?;
            let generated_methods = match method_name {
                "attr_reader" => vec![name.to_owned()],
                "attr_writer" => vec![format!("{name}=")],
                "attr_accessor" => vec![name.to_owned(), format!("{name}=")],
                _ => Vec::new(),
            };
            for generated_name in generated_methods {
                let qualified_method = format!("{owner_type}{method_separator}{generated_name}");
                let method_id = self.builder.declare_with_namespace(
                    "method",
                    &stable_graph_id("method", &qualified_method),
                    &generated_name,
                    &qualified_method,
                    Some(&owner_type),
                    receiver_scope_id.as_deref(),
                    Some(SymbolNamespace::Value),
                    range_for_node(self.source_file, argument),
                )?;
                self.emit_contains(
                    &self.current().owner_declaration_id.clone(),
                    &method_id,
                    &generated_name,
                    "method",
                )?;
            }
        }
        Ok(())
    }

    fn emit_assignment(&mut self, node: Node<'_>) -> Result<(), EvidenceError> {
        let Some(left) = node.child_by_field_name("left") else {
            return Ok(());
        };
        let Some(name) = self.text(left) else {
            return Ok(());
        };
        let Some(right) = node.child_by_field_name("right") else {
            return Ok(());
        };
        if let Some(receiver) = self.constructor_receiver(right)
            && is_local_identifier(&name)
            && let Some(frame) = self.frames.last_mut()
        {
            frame.local_receivers.insert(name.clone(), receiver);
        }
        if is_local_identifier(&name)
            && let Some(frame) = self.frames.last_mut()
        {
            frame.local_bindings.insert(name.clone());
        }
        if name.starts_with('@') {
            if let Some(owner) = self.current().receiver_qualified_name.clone() {
                let receiver_scope_id = self.current().receiver_scope_id.clone();
                let qualified_name = format!("{owner}.{name}");
                let graph_node_id = stable_graph_id("field", &qualified_name);
                let declaration_id = self.builder.declare_with_namespace(
                    "field",
                    &graph_node_id,
                    &name,
                    &qualified_name,
                    Some(&owner),
                    receiver_scope_id.as_deref(),
                    Some(SymbolNamespace::Value),
                    range_for_node(self.source_file, left),
                )?;
                self.emit_contains(
                    &self.current().owner_declaration_id.clone(),
                    &declaration_id,
                    &name,
                    "field",
                )?;
            }
        } else if is_constant_path(&name) {
            let qualified_name = qualify(&self.current().lexical_prefix, &name);
            let scope_id = self.current().scope_id.clone();
            let graph_node_id = stable_graph_id("constant", &qualified_name);
            let declaration_id = self.builder.declare_with_namespace(
                "constant",
                &graph_node_id,
                &last_component(&name),
                &qualified_name,
                package_of(&qualified_name),
                Some(&scope_id),
                Some(SymbolNamespace::Namespace),
                range_for_node(self.source_file, left),
            )?;
            self.emit_contains(
                &self.current().owner_declaration_id.clone(),
                &declaration_id,
                &last_component(&name),
                "constant",
            )?;
        }
        Ok(())
    }

    fn constructor_receiver(&self, node: Node<'_>) -> Option<String> {
        if node.kind() != "call"
            || node
                .child_by_field_name("method")
                .and_then(|node| self.text(node))
                .as_deref()
                != Some("new")
        {
            return None;
        }
        node.child_by_field_name("receiver")
            .and_then(|receiver| self.receiver_type(receiver))
    }

    fn emit_contains(
        &mut self,
        owner_id: &str,
        target_id: &str,
        name: &str,
        kind: &str,
    ) -> Result<(), EvidenceError> {
        self.builder
            .relate(
                CandidateRelation::Contains,
                owner_id,
                None,
                None,
                name,
                ResolutionConstraint {
                    exact_target_declaration_id: Some(target_id.to_owned()),
                    exact_language: Some("ruby".to_owned()),
                    allowed_target_kinds: vec![kind.to_owned()],
                    ..ResolutionConstraint::default()
                },
            )
            .map(|_| ())
    }

    fn receiver_type(&self, node: Node<'_>) -> Option<String> {
        let text = self.text(node)?;
        if text == "self" {
            return self.current().receiver_qualified_name.clone();
        }
        if is_constant_path(&text) {
            return Some(self.resolve_constant_name(&text));
        }
        self.lookup_local_receiver(&text)
    }

    fn call_method_space(&self, receiver: Option<&str>) -> Option<MethodSpace> {
        if receiver.is_some_and(is_constant_path) {
            return Some(MethodSpace::Singleton);
        }
        if receiver == Some("self") && self.current().method_space == Some(MethodSpace::Singleton) {
            return Some(MethodSpace::Singleton);
        }
        self.current().method_space
    }

    fn lookup_local_receiver(&self, name: &str) -> Option<String> {
        self.frames
            .iter()
            .rev()
            .find_map(|frame| frame.local_receivers.get(name).cloned())
    }

    fn resolve_constant_name(&self, raw: &str) -> String {
        let raw = raw.trim();
        if raw.starts_with("::") {
            return raw.trim_start_matches("::").to_owned();
        }
        let mut prefix = self.current().lexical_prefix.clone();
        loop {
            let candidate = qualify(&prefix, raw);
            if self.types.contains_key(&candidate) {
                return candidate;
            }
            let Some(parent) = prefix
                .rsplit_once("::")
                .map(|(parent, _)| parent.to_owned())
            else {
                break;
            };
            prefix = parent;
        }
        let fallback_prefix = self.current().lexical_prefix.rsplit_once("::").map_or_else(
            || self.current().lexical_prefix.clone(),
            |(parent, _)| parent.to_owned(),
        );
        qualify(&fallback_prefix, raw)
    }

    fn text_node_child(&self, node: Node<'_>) -> Option<String> {
        let mut cursor = node.walk();
        node.children(&mut cursor)
            .find(Node::is_named)
            .and_then(|child| self.text(child))
    }

    fn text(&self, node: Node<'_>) -> Option<String> {
        self.source
            .get(node.start_byte()..node.end_byte())
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .map(str::to_owned)
    }

    fn current(&self) -> &ScopeFrame {
        self.frames.last().unwrap_or(&self.frames[0])
    }

    fn overlaps_error(&self, node: Node<'_>) -> bool {
        // Tree-sitter can wrap an otherwise useful recovered prefix in one
        // root ERROR node.  Walk that node's children so valid calls and
        // declarations before the malformed token still produce evidence;
        // only zero-width missing nodes are inherently untrusted.
        node.is_missing()
    }

    fn diagnose_once(
        &mut self,
        code: &str,
        range: Option<EvidenceRange>,
        message: &str,
    ) -> Result<(), EvidenceError> {
        if self.emitted_diagnostics.insert(code.to_owned()) {
            self.builder.diagnose(code, None, range, message)?;
        }
        Ok(())
    }
}

fn stable_graph_id(kind: &str, value: &str) -> String {
    let encoded = value
        .replace("::", "_scope_")
        .replace('#', "_instance_")
        .replace('.', "_singleton_");
    make_id(&["ruby", kind, &encoded])
}

fn qualify(prefix: &str, raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with("::") || prefix.is_empty() || raw.contains("::") {
        raw.trim_start_matches("::").to_owned()
    } else {
        format!("{prefix}::{raw}")
    }
}

fn package_of(qualified: &str) -> Option<&str> {
    qualified.rsplit_once("::").map(|(package, _)| package)
}

fn last_component(value: &str) -> String {
    value
        .rsplit("::")
        .next()
        .unwrap_or(value)
        .rsplit(['#', '.'])
        .next()
        .unwrap_or(value)
        .to_owned()
}

fn is_constant_path(value: &str) -> bool {
    let value = value.trim().trim_start_matches("::");
    !value.is_empty()
        && value.split("::").all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(char::is_uppercase)
                && chars.all(|character| character.is_alphanumeric() || character == '_')
        })
}

fn is_local_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character.is_lowercase() || character == '_')
        && chars.all(|character| character.is_alphanumeric() || character == '_')
}

fn first_argument(node: Node<'_>) -> Option<Node<'_>> {
    let arguments = node.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    arguments.children(&mut cursor).find(Node::is_named)
}

fn count_arguments(arguments: Node<'_>) -> u32 {
    let mut cursor = arguments.walk();
    let count = arguments
        .children(&mut cursor)
        .filter(Node::is_named)
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn literal_string(node: Node<'_>, source: &[u8]) -> Option<String> {
    if !matches!(
        node.kind(),
        "string" | "string_content" | "symbol" | "simple_symbol"
    ) {
        return None;
    }
    let value = source.get(node.start_byte()..node.end_byte())?;
    let value = std::str::from_utf8(value).ok()?.trim();
    let value = value
        .strip_prefix(":")
        .unwrap_or(value)
        .trim_matches(['"', '\'']);
    (!value.is_empty()).then(|| value.to_owned())
}

fn strip_symbol(value: &str) -> String {
    value
        .trim()
        .strip_prefix(':')
        .unwrap_or(value.trim())
        .trim_matches(['"', '\''])
        .to_owned()
}
