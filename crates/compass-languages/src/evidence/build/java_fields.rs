//! Java field evidence uses AST lexical ranges, not the method-wide call type map.
use super::*;

const DEPTH: usize = 64;
const RECEIVER_DEPTH: usize = 16;

#[derive(Clone)]
struct Local {
    start: usize,
    end: usize,
    nominal: Option<String>,
}

#[derive(Clone)]
struct Field {
    qualified: String,
    nominal: Option<String>,
    is_static: bool,
}

#[derive(Default)]
pub(super) struct JavaFieldIndex {
    locals: HashMap<usize, HashMap<String, Vec<Local>>>,
    local_types: HashMap<usize, HashMap<String, usize>>,
    fields: HashMap<String, HashMap<String, Vec<Field>>>,
    named_types: HashSet<String>,
    bindings: usize,
    static_scopes: HashSet<usize>,
}

enum Value<'a> {
    Local(Option<&'a str>),
    Field(&'a Field),
    Unknown,
    Absent,
}

fn ancestors(node: Node<'_>) -> impl Iterator<Item = Node<'_>> {
    std::iter::successors(Some(node), |node| node.parent()).take(DEPTH)
}

fn is_static(node: Node<'_>, source: &[u8]) -> bool {
    if !matches!(
        node.kind(),
        "static_initializer"
            | "field_declaration"
            | "constant_declaration"
            | "method_declaration"
            | "constructor_declaration"
    ) && java_container_kind(node.kind()).is_none()
    {
        return false;
    }
    node.kind() == "static_initializer" || {
        let mut cursor = node.walk();
        node.named_children(&mut cursor)
            .find(|child| child.kind() == "modifiers")
            .is_some_and(|modifiers| {
                let mut cursor = modifiers.walk();
                modifiers.children(&mut cursor).any(|child| {
                    child.kind() == "static" && child.utf8_text(source).ok() == Some("static")
                })
            })
    }
}

fn anonymous_body(node: Node<'_>) -> bool {
    node.kind() == "class_body"
        && node.parent().is_some_and(|parent| {
            matches!(
                parent.kind(),
                "object_creation_expression" | "enum_constant"
            )
        })
}

fn first_named(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .find(|child| !child.is_extra())
}

impl DirectEvidenceState<'_> {
    pub(super) fn index_java_field_values(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if node.parent().is_none() {
            self.java_fields.named_types = self
                .java_containers
                .values()
                .map(|context| context.qualified_name.clone())
                .collect();
        }
        if is_static(node, self.source) {
            ensure_capacity(
                "Java field lexical bindings",
                self.java_fields.bindings,
                self.builder.limits.bindings,
            )?;
            self.java_fields.bindings += 1;
            self.java_fields.static_scopes.insert(node.id());
        }
        let active = self
            .declarations
            .get(&node.id())
            .cloned()
            .unwrap_or_else(|| owner.clone());
        if java_container_kind(node.kind()).is_some()
            && !self.java_containers.contains_key(&node.id())
            && let (Some(parent), Some(name)) = (node.parent(), node.child_by_field_name("name"))
            && matches!(parent.kind(), "block" | "constructor_body" | "switch_block")
        {
            ensure_capacity(
                "Java field lexical bindings",
                self.java_fields.bindings,
                self.builder.limits.bindings,
            )?;
            self.java_fields.bindings += 1;
            let spelling = self.text(name);
            self.java_fields
                .local_types
                .entry(parent.id())
                .or_default()
                .entry(spelling)
                .and_modify(|start| *start = (*start).min(node.start_byte()))
                .or_insert(node.start_byte());
        }
        match node.kind() {
            "variable_declarator" => {
                if let (Some(parent), Some(name)) =
                    (node.parent(), node.child_by_field_name("name"))
                {
                    if matches!(parent.kind(), "field_declaration" | "constant_declaration") {
                        if let Some(context) = self.declarations.get(&node.id()) {
                            let nominal = parent
                                .child_by_field_name("type")
                                .and_then(|ty| self.java_field_type(&active, ty, node, 0));
                            let spelling = self.text(name);
                            self.java_fields
                                .fields
                                .entry(
                                    context
                                        .enclosing_type_qualified_name
                                        .clone()
                                        .unwrap_or_default(),
                                )
                                .or_default()
                                .entry(spelling)
                                .or_default()
                                .push(Field {
                                    qualified: context.qualified_name.clone(),
                                    nominal,
                                    is_static: parent.kind() == "constant_declaration"
                                        || self.java_fields.static_scopes.contains(&parent.id()),
                                });
                        }
                    } else if parent.kind() == "local_variable_declaration"
                        && let Some(scope) = ancestors(parent).skip(1).find(|ancestor| {
                            matches!(
                                ancestor.kind(),
                                "block" | "constructor_body" | "for_statement" | "switch_block"
                            )
                        })
                    {
                        let nominal = parent
                            .child_by_field_name("type")
                            .and_then(|ty| self.java_field_type(&active, ty, node, 0));
                        self.java_field_local(
                            scope,
                            name,
                            name.start_byte(),
                            scope.end_byte(),
                            nominal,
                        )?;
                    }
                }
            }
            "formal_parameter" | "spread_parameter" | "catch_formal_parameter" => {
                if let Some(name) = java_parameter_declarator(node).child_by_field_name("name")
                    && let Some(scope) = ancestors(node).skip(1).find(|ancestor| {
                        matches!(
                            ancestor.kind(),
                            "method_declaration"
                                | "constructor_declaration"
                                | "lambda_expression"
                                | "catch_clause"
                        )
                    })
                {
                    let nominal = java_parameter_type_node(node)
                        .or_else(|| first_named(node).filter(|child| child.kind() == "catch_type"))
                        .and_then(|ty| self.java_field_type(&active, ty, node, 0));
                    let body = scope.child_by_field_name("body").unwrap_or(scope);
                    self.java_field_local(
                        scope,
                        name,
                        body.start_byte(),
                        body.end_byte(),
                        nominal,
                    )?;
                }
            }
            "enhanced_for_statement" | "resource" => {
                if let Some(name) = node.child_by_field_name("name") {
                    let scope = if node.kind() == "resource" {
                        ancestors(node)
                            .find(|ancestor| ancestor.kind() == "try_with_resources_statement")
                    } else {
                        Some(node)
                    };
                    if let Some(scope) = scope
                        && let Some(body) = scope.child_by_field_name("body")
                    {
                        let nominal = node
                            .child_by_field_name("type")
                            .and_then(|ty| self.java_field_type(&active, ty, node, 0));
                        let start = if node.kind() == "resource" {
                            name.start_byte()
                        } else {
                            body.start_byte()
                        };
                        self.java_field_local(scope, name, start, body.end_byte(), nominal)?;
                    }
                }
            }
            "lambda_expression" => {
                if let (Some(parameters), Some(body)) = (
                    node.child_by_field_name("parameters"),
                    node.child_by_field_name("body"),
                ) {
                    if parameters.kind() == "identifier" {
                        self.java_field_local(
                            node,
                            parameters,
                            body.start_byte(),
                            body.end_byte(),
                            None,
                        )?;
                    } else if parameters.kind() == "inferred_parameters" {
                        let mut cursor = parameters.walk();
                        for name in parameters
                            .named_children(&mut cursor)
                            .filter(|n| n.kind() == "identifier")
                        {
                            self.java_field_local(
                                node,
                                name,
                                body.start_byte(),
                                body.end_byte(),
                                None,
                            )?;
                        }
                    }
                }
            }
            "instanceof_expression" => self.index_java_field_pattern(node, &active)?,
            kind if kind.ends_with("_pattern") => {
                let mut names = Vec::new();
                collect_nodes(node, "identifier", &mut names);
                for name in names {
                    self.java_field_unknown_pattern(node, name)?;
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            self.index_java_field_values(child, &active)?;
        }
        Ok(())
    }

    fn java_field_local(
        &mut self,
        scope: Node<'_>,
        name: Node<'_>,
        start: usize,
        end: usize,
        nominal: Option<String>,
    ) -> Result<(), EvidenceError> {
        ensure_capacity(
            "Java field lexical bindings",
            self.java_fields.bindings,
            self.builder.limits.bindings,
        )?;
        self.java_fields.bindings += 1;
        let spelling = self.text(name);
        let bindings = self
            .java_fields
            .locals
            .entry(scope.id())
            .or_default()
            .entry(spelling)
            .or_default();
        // Valid ordinary Java scopes have one binding per name. A crowded
        // malformed/flow-sensitive scope becomes one unknown shadow barrier,
        // so lookup never scans an unbounded version list or selects a winner.
        if bindings.len() >= 2 {
            bindings.clear();
            bindings.push(Local {
                start: 0,
                end: usize::MAX,
                nominal: None,
            });
        }
        bindings.push(Local {
            start,
            end,
            nominal,
        });
        Ok(())
    }

    fn java_field_type(
        &self,
        owner: &DeclarationContext,
        ty: Node<'_>,
        declarator: Node<'_>,
        depth: usize,
    ) -> Option<String> {
        if depth >= RECEIVER_DEPTH || self.overlaps_parser_error(ty) {
            return None;
        }
        let text = self.text(ty);
        if text.contains('|') {
            return None;
        }
        let mut raw = java_normalize_type(&text);
        if raw.is_empty() || raw == "var" || raw.contains('|') {
            return None;
        }
        if let Some(dimensions) = declarator.child_by_field_name("dimensions") {
            raw.push_str(&java_dimensions_suffix(dimensions));
        }
        if declarator.kind() == "spread_parameter" {
            raw.push_str("[]");
        }
        let base = raw.trim_end_matches("[]");
        let suffix = &raw[base.len()..];
        let head = base.split('.').next()?;
        if java_primitive_type(base) {
            return None;
        }
        // Type parameters shadow same-named classes. A single explicit bound
        // can prove field ownership; intersections and recursive bounds cannot.
        for scope in ancestors(ty) {
            if let Some(parameters) = scope.child_by_field_name("type_parameters") {
                let mut cursor = parameters.walk();
                let parameters = parameters
                    .named_children(&mut cursor)
                    .take(DEPTH + 1)
                    .collect::<Vec<_>>();
                if parameters.len() > DEPTH {
                    return None;
                }
                for parameter in parameters {
                    if first_named(parameter).is_some_and(|name| self.text(name) == head) {
                        if head != base {
                            return None;
                        }
                        let mut cursor = parameter.walk();
                        let bound = parameter
                            .named_children(&mut cursor)
                            .find(|n| n.kind() == "type_bound")?;
                        let mut cursor = bound.walk();
                        let mut bounds =
                            bound.named_children(&mut cursor).filter(|n| !n.is_extra());
                        let first = bounds.next()?;
                        if bounds.next().is_some() {
                            return None;
                        }
                        return self
                            .java_field_type(owner, first, first, depth + 1)
                            .map(|target| format!("{target}{suffix}"));
                    }
                }
            }
        }
        if ancestors(ty)
            .last()
            .is_some_and(|node| node.parent().is_some())
            || self.java_field_local_type(ty, head)
        {
            return None;
        }
        // A visible source type shadows imports and package prefixes. Once
        // selected, a missing nested type cannot fall back to that package.
        if let Some(local) = self.local_target_for(owner, head) {
            let target = format!("{local}{}", base[head.len()..].replace('.', "::"));
            return self
                .java_fields
                .named_types
                .contains(&target)
                .then(|| format!("{target}{suffix}"));
        }
        if self.visible_import_binding_is_ambiguous(owner, head) {
            return None;
        }
        self.java_qualified_type(owner, base, ty.start_byte())
            .map(|target| format!("{target}{suffix}"))
    }

    fn java_field_local_type(&self, node: Node<'_>, name: &str) -> bool {
        ancestors(node).any(|scope| {
            self.java_fields
                .local_types
                .get(&scope.id())
                .and_then(|names| names.get(name))
                .is_some_and(|start| *start <= node.start_byte())
        })
    }

    fn java_field_value(&self, node: Node<'_>, name: &str) -> Value<'_> {
        let mut instance = true;
        for scope in ancestors(node) {
            if let Some(bindings) = self
                .java_fields
                .locals
                .get(&scope.id())
                .and_then(|names| names.get(name))
            {
                let mut matching = bindings.iter().filter(|binding| {
                    binding.start <= node.start_byte() && node.start_byte() < binding.end
                });
                if let Some(binding) = matching.next() {
                    return if matching.next().is_some() {
                        Value::Unknown
                    } else {
                        Value::Local(binding.nominal.as_deref())
                    };
                }
            }
            if java_container_kind(scope.kind()).is_none()
                && self.java_fields.static_scopes.contains(&scope.id())
            {
                instance = false;
            }
            if anonymous_body(scope) {
                return Value::Unknown;
            }
            if java_container_kind(scope.kind()).is_some() {
                let Some(context) = self.java_containers.get(&scope.id()) else {
                    return Value::Unknown;
                };
                if let Some(fields) = self
                    .java_fields
                    .fields
                    .get(&context.qualified_name)
                    .and_then(|fields| fields.get(name))
                {
                    return match fields.as_slice() {
                        [field] if instance || field.is_static => Value::Field(field),
                        _ => Value::Unknown,
                    };
                }
                if self.java_fields.static_scopes.contains(&scope.id()) {
                    instance = false;
                }
                // An inherited member can hide an enclosing field. Cross-file
                // hierarchy selection belongs to the resolver, never this map.
                if scope.child_by_field_name("superclass").is_some()
                    || scope.child_by_field_name("interfaces").is_some()
                    || scope.kind() == "enum_declaration"
                    || scope.kind() == "record_declaration"
                {
                    return Value::Unknown;
                }
            }
        }
        if ancestors(node)
            .last()
            .is_some_and(|node| node.parent().is_none())
        {
            Value::Absent
        } else {
            Value::Unknown
        }
    }

    fn java_field_this(&self, node: Node<'_>, named: Option<&str>) -> Option<String> {
        for scope in ancestors(node) {
            if java_container_kind(scope.kind()).is_none()
                && self.java_fields.static_scopes.contains(&scope.id())
            {
                return None;
            }
            if named.is_none() && anonymous_body(scope) {
                return None;
            }
            if java_container_kind(scope.kind()).is_some() {
                let context = self.java_containers.get(&scope.id())?;
                if named.is_none_or(|name| context.name == name) {
                    return Some(context.qualified_name.clone());
                }
                if self.java_fields.static_scopes.contains(&scope.id()) {
                    return None;
                }
            }
        }
        None
    }

    fn java_field_receiver(
        &self,
        owner: &DeclarationContext,
        node: Node<'_>,
        depth: usize,
    ) -> Option<String> {
        if depth >= RECEIVER_DEPTH || self.overlaps_parser_error(node) {
            return None;
        }
        match node.kind() {
            "this" => self.java_field_this(node, None),
            "identifier" => match self.java_field_value(node, &self.text(node)) {
                Value::Local(target) => target.map(str::to_owned),
                Value::Field(field) => field.nominal.clone(),
                Value::Unknown => None,
                Value::Absent => {
                    let name = self.text(node);
                    if self.java_field_local_type(node, &name)
                        || self.visible_import_binding_is_ambiguous(owner, &name)
                    {
                        return None;
                    }
                    self.local_target_for(owner, &name).cloned().or_else(|| {
                        self.imported_target_for_occurrence(owner, &name, node.start_byte(), true)
                            .cloned()
                    })
                }
            },
            "parenthesized_expression" => {
                self.java_field_receiver(owner, first_named(node)?, depth + 1)
            }
            "cast_expression" => {
                self.java_field_type(owner, node.child_by_field_name("type")?, node, 0)
            }
            "array_access" => self
                .java_field_receiver(owner, node.child_by_field_name("array")?, depth + 1)?
                .strip_suffix("[]")
                .map(str::to_owned),
            "object_creation_expression" => {
                let mut cursor = node.walk();
                if node
                    .named_children(&mut cursor)
                    .any(|n| n.kind() == "class_body")
                {
                    return None;
                }
                let mut cursor = node.walk();
                if node
                    .children(&mut cursor)
                    .find(|n| !n.is_extra())
                    .is_none_or(|n| n.kind() != "new")
                {
                    return None;
                }
                self.java_field_type(owner, node.child_by_field_name("type")?, node, 0)
            }
            "field_access" => {
                let object = node.child_by_field_name("object")?;
                let field = node.child_by_field_name("field")?;
                if field.kind() == "this" {
                    return self.java_field_this(node, Some(&self.text(object)));
                }
                let receiver = self.java_field_receiver(owner, object, depth + 1)?;
                let fields = self
                    .java_fields
                    .fields
                    .get(&receiver)?
                    .get(&self.text(field))?;
                match fields.as_slice() {
                    [field] => field.nominal.clone(),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub(super) fn add_java_field_access(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        if self.overlaps_parser_error(node) {
            return Ok(());
        }
        let owner_known = ancestors(node)
            .find(|scope| {
                matches!(
                    scope.kind(),
                    "method_declaration" | "constructor_declaration"
                ) || java_container_kind(scope.kind()).is_some()
                    || anonymous_body(*scope)
            })
            .is_some_and(|scope| {
                if java_container_kind(scope.kind()).is_some() {
                    self.java_containers
                        .get(&scope.id())
                        .is_some_and(|context| {
                            owner.enclosing_type_qualified_name.as_deref()
                                == Some(context.qualified_name.as_str())
                        })
                } else {
                    self.declarations
                        .get(&scope.id())
                        .is_some_and(|context| context.fact_id == owner.fact_id)
                }
            });
        let (field, qualifier, qualified_name) = if node.kind() == "field_access" {
            let (Some(object), Some(field)) = (
                node.child_by_field_name("object"),
                node.child_by_field_name("field"),
            ) else {
                return Ok(());
            };
            if field.kind() != "identifier" {
                return Ok(());
            }
            let target = owner_known
                .then(|| self.java_field_receiver(owner, object, 0))
                .flatten()
                .filter(|target| !target.ends_with("[]"))
                .map(|target| format!("{target}::{}", self.text(field)));
            (field, self.text(object), target)
        } else {
            if !owner_known || !java_value_identifier(node) {
                return Ok(());
            }
            let Value::Field(field) = self.java_field_value(node, &self.text(node)) else {
                return Ok(());
            };
            let qualifier = if field.is_static {
                field
                    .qualified
                    .rsplit_once("::")
                    .map(|(owner, _)| owner)
                    .unwrap_or_default()
                    .to_owned()
            } else {
                "this".to_owned()
            };
            (node, qualifier, Some(field.qualified.clone()))
        };
        let spelling = self.text(field);
        let occurrence = self.builder.occur_with_context(
            SemanticRole::MemberAccess,
            &owner.fact_id,
            &spelling,
            Some(&qualifier),
            Some(&owner.scope_id),
            Some("member"),
            range_for_node(self.source_file, field),
        )?;
        self.builder.relate(
            CandidateRelation::AccessesMember,
            &owner.fact_id,
            Some(&occurrence),
            None,
            &spelling,
            ResolutionConstraint {
                exact_language: Some(self.language.to_owned()),
                module_or_package: Some(self.module_or_package.clone()),
                scope_id: Some(owner.scope_id.clone()),
                qualified_name,
                allowed_target_kinds: vec!["field".to_owned()],
                allow_external: false,
                ..ResolutionConstraint::default()
            },
        )?;
        Ok(())
    }

    fn index_java_field_pattern(
        &mut self,
        node: Node<'_>,
        owner: &DeclarationContext,
    ) -> Result<(), EvidenceError> {
        let Some(name) = node.child_by_field_name("name") else {
            return Ok(());
        };
        let mut condition = node;
        let mut negative = false;
        for _ in 0..DEPTH {
            let Some(parent) = condition.parent() else {
                break;
            };
            if parent.kind() == "parenthesized_expression" {
                condition = parent;
            } else if parent.kind() == "unary_expression"
                && self.text(parent).trim_start().starts_with('!')
            {
                negative = !negative;
                condition = parent;
            } else {
                break;
            }
        }
        if let Some(statement) = condition
            .parent()
            .filter(|parent| parent.kind() == "if_statement")
            && statement
                .child_by_field_name("condition")
                .is_some_and(|n| n.id() == condition.id())
        {
            let nominal = node
                .child_by_field_name("right")
                .and_then(|ty| self.java_field_type(owner, ty, ty, 0));
            let selected = if negative {
                "alternative"
            } else {
                "consequence"
            };
            if let Some(branch) = statement.child_by_field_name(selected) {
                self.java_field_local(
                    branch,
                    name,
                    branch.start_byte(),
                    branch.end_byte(),
                    nominal.clone(),
                )?;
            }
            // A simple abrupt negative guard proves the pattern on the
            // remainder of its enclosing block. Complex flow stays unknown.
            let rejected = if negative {
                "consequence"
            } else {
                "alternative"
            };
            if let Some(rejected) = statement.child_by_field_name(rejected)
                && let Some(block) = statement.parent().filter(|n| n.kind() == "block")
            {
                let after_type = java_abrupt_statement(rejected).then_some(nominal).flatten();
                self.java_field_local(
                    block,
                    name,
                    statement.end_byte(),
                    block.end_byte(),
                    after_type,
                )?;
            }
            return Ok(());
        }
        // Unsupported flow must not turn a pattern-bound value into an outer
        // field. Conservatively mask the name for its entire callable.
        self.java_field_unknown_pattern(node, name)
    }

    fn java_field_unknown_pattern(
        &mut self,
        node: Node<'_>,
        name: Node<'_>,
    ) -> Result<(), EvidenceError> {
        let scope = ancestors(node)
            .find(|n| {
                matches!(
                    n.kind(),
                    "method_declaration" | "constructor_declaration" | "lambda_expression"
                )
            })
            .or_else(|| ancestors(node).find(|n| java_container_kind(n.kind()).is_some()));
        if let Some(scope) = scope {
            self.java_field_local(scope, name, scope.start_byte(), scope.end_byte(), None)?;
        }
        Ok(())
    }
}

fn java_abrupt_statement(node: Node<'_>) -> bool {
    if matches!(node.kind(), "return_statement" | "throw_statement") {
        return true;
    }
    if node.kind() != "block" {
        return false;
    }
    let mut cursor = node.walk();
    let mut statements = node.named_children(&mut cursor).filter(|n| !n.is_extra());
    let first = statements.next();
    statements.next().is_none()
        && first.is_some_and(|n| matches!(n.kind(), "return_statement" | "throw_statement"))
}

fn java_value_identifier(node: Node<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    let in_field = |name| {
        parent
            .child_by_field_name(name)
            .is_some_and(|n| n.id() == node.id())
    };
    match parent.kind() {
        "method_invocation" | "field_access" => in_field("object"),
        "variable_declarator" | "resource" => in_field("value"),
        "lambda_expression" => in_field("body"),
        "enhanced_for_statement" => in_field("value"),
        "cast_expression" => in_field("value"),
        "instanceof_expression" => in_field("left"),
        "argument_list"
        | "return_statement"
        | "throw_statement"
        | "yield_statement"
        | "expression_statement"
        | "binary_expression"
        | "unary_expression"
        | "update_expression"
        | "assignment_expression"
        | "parenthesized_expression"
        | "ternary_expression"
        | "array_access"
        | "array_initializer"
        | "assert_statement" => true,
        _ => false,
    }
}
