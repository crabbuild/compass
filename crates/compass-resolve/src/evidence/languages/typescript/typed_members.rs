//! Definition-bound TypeScript receiver transitions. Typed arguments never
//! fall back to the legacy display spellings in a member-path constraint.

use super::*;
use compass_languages::{SymbolNamespace, TypeScriptTypeArgument};
use std::sync::Arc;

mod exports;

const MAX_TYPED_STEPS: usize = 16_384;
const MAX_TYPED_DEPTH: usize = 16;
// Wire argument-node counts exclude the owning nominal itself.
const MAX_TYPED_VALUE_NODES: usize = compass_languages::MAX_TYPESCRIPT_TYPE_ARGUMENT_NODES + 1;

#[derive(Clone, Debug, Eq, PartialEq)]
enum TypeValue {
    Unknown,
    /// Resolve an actual only when a transition needs it. A missing import
    /// must not hide a direct member whose identity is independent of it.
    Proof(Arc<TypeScriptTypeArgument>),
    Nominal {
        slot: DeclarationSlot,
        arguments: Vec<Self>,
    },
    Array(Box<Self>),
    Tuple(Vec<Self>),
    Projection {
        base: Box<Self>,
        keys: BTreeSet<String>,
        omit: bool,
    },
}

type Environment = BTreeMap<String, TypeValue>;
type TypedResult<T> = Result<T, TypeFailure>;

#[derive(Clone, Copy)]
enum TypeFailure {
    Unknown,
    Ambiguous(usize),
    Limit {
        resource: &'static str,
        limit: usize,
    },
}

impl TypeFailure {
    fn decision(self) -> ResolutionDecision {
        match self {
            Self::Unknown => ResolutionDecision::Unresolved,
            Self::Ambiguous(candidate_count) => ResolutionDecision::Ambiguous { candidate_count },
            Self::Limit { resource, limit } => {
                ResolutionDecision::ResourceLimit { resource, limit }
            }
        }
    }
}

struct TypeWork {
    remaining: usize,
    limit: usize,
}
impl TypeWork {
    fn new(limit: usize) -> Self {
        Self {
            remaining: limit,
            limit,
        }
    }
    fn charge(&mut self, count: usize) -> TypedResult<()> {
        if count > self.remaining {
            return Err(TypeFailure::Limit {
                resource: "typescript_type_steps",
                limit: self.limit,
            });
        }
        self.remaining -= count;
        Ok(())
    }
    fn enter(&mut self, depth: usize) -> TypedResult<()> {
        if depth > MAX_TYPED_DEPTH {
            return Err(TypeFailure::Limit {
                resource: "typescript_type_depth",
                limit: MAX_TYPED_DEPTH,
            });
        }
        self.charge(1)
    }
    fn copy(&mut self, value: &TypeValue) -> TypedResult<TypeValue> {
        self.charge(value.nodes())?;
        Ok(value.clone())
    }
}
impl TypeValue {
    fn nodes(&self) -> usize {
        1 + match self {
            Self::Unknown => 0,
            Self::Proof(proof) => proof_nodes(proof).saturating_sub(1),
            Self::Nominal { arguments, .. } | Self::Tuple(arguments) => {
                arguments.iter().map(Self::nodes).sum()
            }
            Self::Array(value) | Self::Projection { base: value, .. } => value.nodes(),
        }
    }
}
fn proof_nodes(proof: &TypeScriptTypeArgument) -> usize {
    1 + match proof {
        TypeScriptTypeArgument::Unknown => 0,
        TypeScriptTypeArgument::Source { arguments, .. }
        | TypeScriptTypeArgument::Import { arguments, .. } => {
            arguments.iter().map(proof_nodes).sum()
        }
    }
}
fn check_value_nodes(nodes: usize) -> TypedResult<()> {
    if nodes > MAX_TYPED_VALUE_NODES {
        Err(TypeFailure::Limit {
            resource: "typescript_type_value_nodes",
            limit: MAX_TYPED_VALUE_NODES,
        })
    } else {
        Ok(())
    }
}
fn only<T>(mut values: Vec<T>) -> TypedResult<T> {
    match values.len() {
        0 => Err(TypeFailure::Unknown),
        1 => values.pop().ok_or(TypeFailure::Unknown),
        count => Err(TypeFailure::Ambiguous(count)),
    }
}

impl ResolutionDb<'_> {
    pub(super) fn resolve_typescript_typed_members(
        &self,
        path: &TypeScriptMemberPath,
        candidate: &RelationshipCandidate,
    ) -> ResolutionDecision {
        self.resolve_typescript_typed_members_with_work(path, candidate, MAX_TYPED_STEPS)
    }

    fn resolve_typescript_typed_members_with_work(
        &self,
        path: &TypeScriptMemberPath,
        candidate: &RelationshipCandidate,
        steps: usize,
    ) -> ResolutionDecision {
        let result = (|| {
            let mut work = TypeWork::new(steps);
            let proof = candidate
                .constraints
                .typescript_type_arguments
                .as_ref()
                .ok_or(TypeFailure::Unknown)?;
            work.charge(1)?;
            let binding = self
                .facts
                .bindings
                .get(&proof.owner_binding_id)
                .ok_or(TypeFailure::Unknown)?;
            let (_, exported) = binding
                .qualified_target
                .rsplit_once("::")
                .ok_or(TypeFailure::Unknown)?;
            if exported != path.root_export && exported != "*" {
                return Err(TypeFailure::Unknown);
            }
            if path.members.is_empty() {
                return Err(TypeFailure::Unknown);
            }
            if path.members.len() > MAX_TYPED_DEPTH {
                return Err(TypeFailure::Limit {
                    resource: "typescript_member_depth",
                    limit: MAX_TYPED_DEPTH,
                });
            }
            let roots = self.typed_binding_export_slots(
                &binding.id,
                &path.root_export,
                candidate,
                &mut work,
            )?;
            let root = only(roots.into_iter().collect())?;
            let arguments = self.typed_wire_arguments(&proof.arguments, candidate, &mut work, 0)?;
            let mut receiver = self.typed_nominal(root, arguments)?;
            if path.call_result && path.call_member_index.is_none() {
                // Generic callable arguments require their own call-site proof.
                // A nominal receiver proof cannot authorize string inference.
                return Err(TypeFailure::Unknown);
            }
            if path.indexed {
                receiver = self.typed_index(receiver, None, candidate, &mut work, 0)?;
            }
            for (index, raw_member) in path.members.iter().enumerate() {
                work.charge(1)?;
                receiver =
                    self.typed_expand(receiver, candidate, &mut work, 0, &mut BTreeSet::new())?;
                let (member, selector) =
                    typescript_indexed_member_segment(raw_member).ok_or(TypeFailure::Unknown)?;
                if let TypeValue::Projection { base, keys, omit } = receiver {
                    if keys.contains(&member) == omit {
                        return Err(TypeFailure::Unknown);
                    }
                    receiver = *base;
                    receiver =
                        self.typed_expand(receiver, candidate, &mut work, 0, &mut BTreeSet::new())?;
                }
                let TypeValue::Nominal { slot, arguments } = receiver else {
                    return Err(TypeFailure::Unknown);
                };
                let members = self.typed_member_slots(slot, &member)?;
                let final_member = index + 1 == path.members.len();
                if final_member {
                    let targets = members
                        .into_iter()
                        .filter(|slot| self.typescript_declaration_allowed_slot(*slot, candidate))
                        .collect::<Vec<_>>();
                    return self
                        .unique_typescript_decision(
                            Some(&targets),
                            candidate,
                            ResolutionRule::MemberBinding,
                        )
                        .ok_or(TypeFailure::Unknown);
                }
                let member_slot = only(members)?;
                let declaration = self.declaration(member_slot).ok_or(TypeFailure::Unknown)?;
                let mut environment = self.typed_environment(slot, &arguments, &mut work)?;
                let signature = declaration
                    .signature
                    .as_deref()
                    .ok_or(TypeFailure::Unknown)?;
                let type_name = if path.call_result && path.call_member_index == Some(index) {
                    // Method parameters shadow the enclosing class parameters
                    // by their exact IDs, including unknown method actuals.
                    let names = typescript_generic_parameter_names(signature);
                    let unknown = vec![TypeValue::Unknown; names.len()];
                    environment.extend(self.typed_environment(member_slot, &unknown, &mut work)?);
                    typescript_callable_return_type(signature).ok_or(TypeFailure::Unknown)?
                } else if declaration.kind == "property" {
                    typescript_value_type(signature).unwrap_or(signature)
                } else {
                    return Err(TypeFailure::Unknown);
                };
                receiver = self.typed_expression(
                    member_slot,
                    type_name,
                    &environment,
                    candidate,
                    &mut work,
                    0,
                )?;
                if let Some(selector) = selector {
                    receiver =
                        self.typed_index(receiver, Some(&selector), candidate, &mut work, 0)?;
                }
            }
            Err(TypeFailure::Unknown)
        })();
        match result {
            Ok(decision) => decision,
            Err(failure) => failure.decision(),
        }
    }

    fn check_typed_candidates(&self, count: usize) -> TypedResult<()> {
        if count > self.budget.candidates_per_lookup() {
            Err(TypeFailure::Limit {
                resource: "typescript_type_candidates",
                limit: self.budget.candidates_per_lookup(),
            })
        } else {
            Ok(())
        }
    }

    fn typed_wire_arguments(
        &self,
        arguments: &[TypeScriptTypeArgument],
        _candidate: &RelationshipCandidate,
        work: &mut TypeWork,
        depth: usize,
    ) -> TypedResult<Vec<TypeValue>> {
        if arguments.is_empty() {
            return Ok(Vec::new());
        }
        work.enter(depth)?;
        let mut values = Vec::with_capacity(arguments.len());
        let mut nodes = 1;
        for argument in arguments {
            work.charge(1)?;
            let count = proof_nodes(argument);
            work.charge(count)?;
            check_value_nodes(nodes + count)?;
            let value = TypeValue::Proof(Arc::new(argument.clone()));
            nodes += value.nodes();
            check_value_nodes(nodes)?;
            values.push(value);
        }
        Ok(values)
    }

    fn typed_nominal(
        &self,
        slot: DeclarationSlot,
        arguments: Vec<TypeValue>,
    ) -> TypedResult<TypeValue> {
        let declaration = self.declaration(slot).ok_or(TypeFailure::Unknown)?;
        if !matches!(
            declaration.kind.as_str(),
            "class" | "interface" | "enum" | "type_alias" | "namespace"
        ) {
            return Err(TypeFailure::Unknown);
        }
        let names = typescript_generic_parameter_names(
            declaration.signature.as_deref().unwrap_or_default(),
        );
        if arguments.len() != names.len() {
            return Err(TypeFailure::Unknown);
        }
        check_value_nodes(1 + arguments.iter().map(TypeValue::nodes).sum::<usize>())?;
        Ok(TypeValue::Nominal { slot, arguments })
    }

    fn typed_environment(
        &self,
        owner: DeclarationSlot,
        arguments: &[TypeValue],
        work: &mut TypeWork,
    ) -> TypedResult<Environment> {
        let declaration = self.declaration(owner).ok_or(TypeFailure::Unknown)?;
        let names = typescript_generic_parameter_names(
            declaration.signature.as_deref().unwrap_or_default(),
        );
        if names.len() != arguments.len() {
            return Err(TypeFailure::Unknown);
        }
        if names.is_empty() {
            return Ok(Environment::new());
        }
        let scope = self
            .indexes
            .typescript
            .definition_scopes
            .get(&owner)
            .and_then(Option::as_deref)
            .ok_or(TypeFailure::Unknown)?;
        let mut environment = Environment::new();
        for (name, argument) in names.iter().zip(arguments) {
            work.charge(1)?;
            if let Some(indexed) = self.indexes.names.by_scope_name.get(&(
                declaration.language.clone(),
                scope.to_owned(),
                name.clone(),
            )) {
                self.check_typed_candidates(indexed.len())?;
            }
            let parameters = self
                .indexes
                .names
                .by_scope_name
                .get(&(declaration.language.clone(), scope.to_owned(), name.clone()))
                .into_iter()
                .flatten()
                .copied()
                .filter(|slot| {
                    self.declaration(*slot).is_some_and(|parameter| {
                        parameter.kind == "type_parameter"
                            && parameter.namespace == Some(SymbolNamespace::Type)
                            && parameter.scope_id.as_deref() == Some(scope)
                    })
                })
                .collect::<Vec<_>>();
            self.check_typed_candidates(parameters.len())?;
            let parameter = only(parameters)?;
            let id = self.declaration_id(parameter).ok_or(TypeFailure::Unknown)?;
            if environment
                .insert(id.to_owned(), work.copy(argument)?)
                .is_some()
            {
                return Err(TypeFailure::Unknown);
            }
        }
        Ok(environment)
    }

    fn typed_member_slots(
        &self,
        owner: DeclarationSlot,
        member: &str,
    ) -> TypedResult<Vec<DeclarationSlot>> {
        let declaration = self.declaration(owner).ok_or(TypeFailure::Unknown)?;
        let scope = self
            .indexes
            .typescript
            .definition_scopes
            .get(&owner)
            .and_then(Option::as_deref)
            .ok_or(TypeFailure::Unknown)?;
        let indexed = self
            .indexes
            .members
            .members_by_owner
            .get(&(
                declaration.language.clone(),
                declaration.qualified_name.clone(),
                member.to_owned(),
            ))
            .ok_or(TypeFailure::Unknown)?;
        self.check_typed_candidates(indexed.len())?;
        Ok(indexed
            .iter()
            .copied()
            .filter(|slot| {
                self.declaration(*slot).is_some_and(|member| {
                    member.range.source_file == declaration.range.source_file
                        && member.scope_id.as_deref().is_some_and(|member_scope| {
                            member_scope == scope
                                || (declaration.kind == "type_alias"
                                    && self.facts.scopes.get(member_scope).is_some_and(
                                        |member_scope| {
                                            member_scope.kind == "object_type"
                                                && member_scope.owner_declaration_id.as_deref()
                                                    == Some(declaration.id.as_str())
                                                && member_scope.parent_scope_id.as_deref()
                                                    == Some(scope)
                                        },
                                    ))
                        })
                })
            })
            .collect())
    }

    fn typed_import_target(
        &self,
        id: &str,
        path: &[String],
        candidate: &RelationshipCandidate,
        work: &mut TypeWork,
    ) -> TypedResult<DeclarationSlot> {
        work.charge(1)?;
        let binding = self.facts.bindings.get(id).ok_or(TypeFailure::Unknown)?;
        if !matches!(
            binding.namespace,
            Some(
                SymbolNamespace::Type | SymbolNamespace::ValueAndType | SymbolNamespace::Namespace
            )
        ) {
            return Err(TypeFailure::Unknown);
        }
        let (_module, exported) = binding
            .qualified_target
            .rsplit_once("::")
            .ok_or(TypeFailure::Unknown)?;
        let (root_export, remaining_path) = if exported == "*" {
            (
                path.first().ok_or(TypeFailure::Unknown)?.as_str(),
                &path[1..],
            )
        } else {
            (exported, path)
        };
        let roots = self.typed_binding_export_slots(id, root_export, candidate, work)?;
        let mut root = only(roots.into_iter().collect())?;
        for member in remaining_path {
            work.charge(1)?;
            root = only(self.typed_member_slots(root, member)?)?;
        }
        Ok(root)
    }

    fn typed_expand(
        &self,
        value: TypeValue,
        candidate: &RelationshipCandidate,
        work: &mut TypeWork,
        depth: usize,
        visiting: &mut BTreeSet<DeclarationSlot>,
    ) -> TypedResult<TypeValue> {
        work.enter(depth)?;
        if let TypeValue::Proof(proof) = value {
            let (slot, arguments) = match proof.as_ref() {
                TypeScriptTypeArgument::Unknown => return Err(TypeFailure::Unknown),
                TypeScriptTypeArgument::Source {
                    declaration_id,
                    arguments,
                } => (
                    declaration_slot(&self.facts.declaration_ids, declaration_id)
                        .ok_or(TypeFailure::Unknown)?,
                    arguments,
                ),
                TypeScriptTypeArgument::Import {
                    binding_id,
                    member_path,
                    arguments,
                } => (
                    self.typed_import_target(binding_id, member_path, candidate, work)?,
                    arguments,
                ),
            };
            let arguments = self.typed_wire_arguments(arguments, candidate, work, depth + 1)?;
            let nominal = self.typed_nominal(slot, arguments)?;
            return self.typed_expand(nominal, candidate, work, depth + 1, visiting);
        }
        let TypeValue::Nominal {
            slot,
            ref arguments,
        } = value
        else {
            return Ok(value);
        };
        let declaration = self.declaration(slot).ok_or(TypeFailure::Unknown)?;
        if declaration.kind != "type_alias" {
            return Ok(value);
        }
        let Some(target) = declaration
            .signature
            .as_deref()
            .and_then(typescript_type_alias_target)
        else {
            return Ok(value);
        };
        if !visiting.insert(slot) {
            return Err(TypeFailure::Unknown);
        }
        let environment = self.typed_environment(slot, arguments, work)?;
        let expanded =
            self.typed_expression(slot, target, &environment, candidate, work, depth + 1)?;
        let expanded = self.typed_expand(expanded, candidate, work, depth + 1, visiting);
        visiting.remove(&slot);
        expanded
    }

    fn typed_index(
        &self,
        value: TypeValue,
        selector: Option<&str>,
        candidate: &RelationshipCandidate,
        work: &mut TypeWork,
        depth: usize,
    ) -> TypedResult<TypeValue> {
        work.enter(depth)?;
        let value = self.typed_expand(value, candidate, work, depth, &mut BTreeSet::new())?;
        match value {
            TypeValue::Array(element) => Ok(*element),
            TypeValue::Tuple(elements) => {
                let index = selector
                    .and_then(|index| index.parse::<usize>().ok())
                    .ok_or(TypeFailure::Unknown)?;
                elements.into_iter().nth(index).ok_or(TypeFailure::Unknown)
            }
            TypeValue::Nominal { slot, arguments } => {
                let signature = self
                    .declaration(slot)
                    .and_then(|declaration| declaration.signature.as_deref())
                    .ok_or(TypeFailure::Unknown)?;
                let value = typescript_index_value_type(signature).ok_or(TypeFailure::Unknown)?;
                let environment = self.typed_environment(slot, &arguments, work)?;
                self.typed_expression(slot, value, &environment, candidate, work, depth + 1)
            }
            _ => Err(TypeFailure::Unknown),
        }
    }

    fn typed_type_name_is_shadowed(
        &self,
        definition: DeclarationSlot,
        name: &str,
        work: &mut TypeWork,
    ) -> TypedResult<bool> {
        let declaration = self.declaration(definition).ok_or(TypeFailure::Unknown)?;
        let mut scope = self
            .indexes
            .typescript
            .definition_scopes
            .get(&definition)
            .and_then(Option::as_deref)
            .or(declaration.scope_id.as_deref());
        let mut visited = BTreeSet::new();
        while let Some(scope_id) = scope {
            work.charge(1)?;
            if !visited.insert(scope_id) {
                return Err(TypeFailure::Unknown);
            }
            let key = (
                declaration.language.clone(),
                scope_id.to_owned(),
                name.to_owned(),
            );
            if let Some(locals) = self.indexes.names.by_scope_name.get(&key) {
                self.check_typed_candidates(locals.len())?;
                if locals.iter().any(|slot| {
                    self.declaration(*slot).is_some_and(|declaration| {
                        matches!(
                            declaration.kind.as_str(),
                            "class"
                                | "interface"
                                | "enum"
                                | "type_alias"
                                | "namespace"
                                | "type_parameter"
                        ) && matches!(
                            declaration.namespace,
                            Some(
                                SymbolNamespace::Type
                                    | SymbolNamespace::ValueAndType
                                    | SymbolNamespace::Namespace
                            )
                        )
                    })
                }) {
                    return Ok(true);
                }
            }
            if let Some(bindings) = self.indexes.typescript.type_bindings_by_scope.get(&key) {
                self.check_typed_candidates(bindings.len())?;
                if bindings.iter().any(|id| {
                    self.facts.bindings.get(id).is_some_and(|binding| {
                        matches!(
                            binding.namespace,
                            Some(
                                SymbolNamespace::Type
                                    | SymbolNamespace::ValueAndType
                                    | SymbolNamespace::Namespace
                            )
                        )
                    })
                }) {
                    return Ok(true);
                }
            }
            scope = self
                .facts
                .scopes
                .get(scope_id)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        Ok(false)
    }

    fn typed_awaited_type(
        &self,
        definition: DeclarationSlot,
        expression: &str,
        environment: &Environment,
        candidate: &RelationshipCandidate,
        work: &mut TypeWork,
        depth: usize,
    ) -> TypedResult<TypeValue> {
        work.enter(depth)?;
        if let Some((wrapper, arguments)) = typescript_generic_type_parts(expression)
            && matches!(wrapper, "Promise" | "PromiseLike")
        {
            // Every spelling belongs to the field/alias definition scope.
            // An intrinsic outer Promise cannot certify a nested namesake.
            // Custom thenable semantics are unsupported here, so do not
            // erase a shadowed nominal or infer its Awaited result.
            if arguments.len() != 1
                || self.typed_type_name_is_shadowed(definition, wrapper, work)?
            {
                return Err(TypeFailure::Unknown);
            }
            return self.typed_awaited_type(
                definition,
                &arguments[0],
                environment,
                candidate,
                work,
                depth + 1,
            );
        }
        let terminal =
            self.typed_expression(definition, expression, environment, candidate, work, depth)?;
        let terminal =
            self.typed_expand(terminal, candidate, work, depth + 1, &mut BTreeSet::new())?;
        let TypeValue::Nominal { slot, .. } = &terminal else {
            return Err(TypeFailure::Unknown);
        };
        self.typed_non_thenable_nominal(*slot, work)?;
        Ok(terminal)
    }

    fn typed_non_thenable_nominal(
        &self,
        slot: DeclarationSlot,
        work: &mut TypeWork,
    ) -> TypedResult<()> {
        work.charge(1)?;
        let declaration = self.declaration(slot).ok_or(TypeFailure::Unknown)?;
        if declaration.language != "typescript"
            || !matches!(declaration.kind.as_str(), "class" | "interface")
            || declaration.member_inventory_complete != Some(true)
            || !declaration.direct_bases_complete
            // Absence requires complete inventories from every TypeScript
            // input, including sources that could augment an exported type.
            || !self.indexes.typescript.merge_inventory_complete
        {
            return Err(TypeFailure::Unknown);
        }
        let scope = declaration
            .scope_id
            .as_deref()
            .ok_or(TypeFailure::Unknown)?;
        let names = self
            .indexes
            .names
            .by_scope_name
            .get(&(
                declaration.language.clone(),
                scope.to_owned(),
                declaration.name.clone(),
            ))
            .ok_or(TypeFailure::Unknown)?;
        self.check_typed_candidates(names.len())?;
        let mut type_names = 0_usize;
        for local in names {
            work.charge(1)?;
            if self.declaration(*local).is_some_and(|fact| {
                matches!(
                    fact.namespace,
                    Some(
                        SymbolNamespace::Type
                            | SymbolNamespace::ValueAndType
                            | SymbolNamespace::Namespace
                    )
                ) && fact.kind != "export"
            }) {
                type_names += 1;
            }
        }
        if type_names != 1 || !names.contains(&slot) {
            return Err(TypeFailure::Unknown);
        }
        let key = (
            declaration.language.clone(),
            declaration.qualified_name.clone(),
        );
        if let Some(bases) = self.indexes.hierarchy.direct_bases.get(&key) {
            self.check_typed_candidates(bases.links.len())?;
            work.charge(bases.links.len())?;
            if !bases.complete || !bases.links.is_empty() {
                return Err(TypeFailure::Unknown);
            }
        }
        if let Some(members) = self.indexes.members.members_by_owner.get(&(
            declaration.language.clone(),
            declaration.qualified_name.clone(),
            "then".to_owned(),
        )) {
            self.check_typed_candidates(members.len())?;
            work.charge(members.len())?;
            if !members.is_empty() {
                // A custom thenable's result requires its own bounded
                // callback/type proof, not the wrapper's nominal identity.
                return Err(TypeFailure::Unknown);
            }
        }
        Ok(())
    }

    fn typed_expression(
        &self,
        definition: DeclarationSlot,
        expression: &str,
        environment: &Environment,
        candidate: &RelationshipCandidate,
        work: &mut TypeWork,
        depth: usize,
    ) -> TypedResult<TypeValue> {
        work.enter(depth)?;
        let expression = expression.trim();
        if expression.is_empty() || typescript_non_nominal_union_member(expression) {
            return Err(TypeFailure::Unknown);
        }
        if expression.len() > 1024 {
            return Err(TypeFailure::Limit {
                resource: "typescript_type_expression_bytes",
                limit: 1024,
            });
        }
        if let Some(element) = typescript_array_element_type(expression)
            && (expression.ends_with("[]")
                || !self.typed_type_name_is_shadowed(
                    definition,
                    expression.split('<').next().unwrap_or_default(),
                    work,
                )?)
        {
            let element = self.typed_expression(
                definition,
                &element,
                environment,
                candidate,
                work,
                depth + 1,
            )?;
            check_value_nodes(1 + element.nodes())?;
            return Ok(TypeValue::Array(Box::new(element)));
        }
        if let Some(elements) = typescript_tuple_elements(expression) {
            let mut values = Vec::with_capacity(elements.len());
            let mut nodes = 1;
            for element in elements {
                let value = match self.typed_expression(
                    definition,
                    element,
                    environment,
                    candidate,
                    work,
                    depth + 1,
                ) {
                    Ok(value) => value,
                    Err(TypeFailure::Unknown) => TypeValue::Unknown,
                    Err(error) => return Err(error),
                };
                nodes += value.nodes();
                check_value_nodes(nodes)?;
                values.push(value);
            }
            return Ok(TypeValue::Tuple(values));
        }
        if let Some((base, property)) = typescript_literal_indexed_type(expression) {
            let base =
                self.typed_expression(definition, base, environment, candidate, work, depth + 1)?;
            let base = self.typed_expand(base, candidate, work, depth + 1, &mut BTreeSet::new())?;
            let TypeValue::Nominal { slot, arguments } = base else {
                return Err(TypeFailure::Unknown);
            };
            let member = only(self.typed_member_slots(slot, &property)?)?;
            let member_fact = self.declaration(member).ok_or(TypeFailure::Unknown)?;
            let signature = member_fact
                .signature
                .as_deref()
                .ok_or(TypeFailure::Unknown)?;
            if member_fact.kind != "property" {
                return Err(TypeFailure::Unknown);
            }
            let environment = self.typed_environment(slot, &arguments, work)?;
            return self.typed_expression(
                member,
                typescript_value_type(signature).unwrap_or(signature),
                &environment,
                candidate,
                work,
                depth + 1,
            );
        }
        if let Some((utility, arguments)) = typescript_generic_type_parts(expression)
            && matches!(utility, "Pick" | "Omit")
            && arguments.len() == 2
            && !self.typed_type_name_is_shadowed(definition, utility, work)?
        {
            let base = self.typed_expression(
                definition,
                &arguments[0],
                environment,
                candidate,
                work,
                depth + 1,
            )?;
            if typescript_keyof_type_base(&arguments[1]) == Some(arguments[0].as_str()) {
                return if utility == "Pick" {
                    Ok(base)
                } else {
                    Err(TypeFailure::Unknown)
                };
            }
            let keys = typescript_literal_key_names(&arguments[1]).ok_or(TypeFailure::Unknown)?;
            check_value_nodes(1 + base.nodes())?;
            return Ok(TypeValue::Projection {
                base: Box::new(base),
                keys,
                omit: utility == "Omit",
            });
        }
        if let Some((utility, arguments)) = typescript_generic_type_parts(expression)
            && arguments.len() == 1
            && matches!(
                utility,
                "Awaited" | "NonNullable" | "Partial" | "Required" | "Readonly"
            )
            && !self.typed_type_name_is_shadowed(definition, utility, work)?
        {
            if utility == "Awaited" {
                return self.typed_awaited_type(
                    definition,
                    &arguments[0],
                    environment,
                    candidate,
                    work,
                    depth + 1,
                );
            }
            if let Some(inner) = typescript_utility_receiver_type(expression) {
                return self.typed_expression(
                    definition,
                    &inner,
                    environment,
                    candidate,
                    work,
                    depth + 1,
                );
            }
        }
        let (base, arguments) =
            typescript_generic_type_parts(expression).unwrap_or((expression, Vec::new()));
        let mut actuals = Vec::with_capacity(arguments.len());
        let mut nodes = 1;
        for argument in arguments {
            let value = match self.typed_expression(
                definition,
                &argument,
                environment,
                candidate,
                work,
                depth + 1,
            ) {
                Ok(value) => value,
                Err(TypeFailure::Unknown) => TypeValue::Unknown,
                Err(error) => return Err(error),
            };
            nodes += value.nodes();
            check_value_nodes(nodes)?;
            actuals.push(value);
        }
        let declaration = self.declaration(definition).ok_or(TypeFailure::Unknown)?;
        let mut scope = self
            .indexes
            .typescript
            .definition_scopes
            .get(&definition)
            .and_then(Option::as_deref)
            .or(declaration.scope_id.as_deref());
        let parts = base.split('.').collect::<Vec<_>>();
        if parts.len() > MAX_TYPED_DEPTH {
            return Err(TypeFailure::Limit {
                resource: "typescript_type_path_depth",
                limit: MAX_TYPED_DEPTH,
            });
        }
        if parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .chars()
                    .all(|character| character.is_alphanumeric() || matches!(character, '_' | '$'))
        }) {
            return Err(TypeFailure::Unknown);
        }
        let mut visited = BTreeSet::new();
        while let Some(scope_id) = scope {
            work.charge(1)?;
            if !visited.insert(scope_id) {
                return Err(TypeFailure::Unknown);
            }
            let key = (
                declaration.language.clone(),
                scope_id.to_owned(),
                parts[0].to_owned(),
            );
            // These indexes retain limit+1 as an overflow witness. Inspect
            // that witness before namespace/kind filtering; discarded suffixes
            // cannot be treated as a complete lexical inventory.
            if let Some(locals) = self.indexes.names.by_scope_name.get(&key) {
                self.check_typed_candidates(locals.len())?;
            }
            if let Some(bindings) = self.indexes.typescript.type_bindings_by_scope.get(&key) {
                self.check_typed_candidates(bindings.len())?;
            }
            let locals = self
                .indexes
                .names
                .by_scope_name
                .get(&key)
                .into_iter()
                .flatten()
                .copied()
                .filter(|slot| {
                    self.declaration(*slot).is_some_and(|declaration| {
                        matches!(
                            declaration.kind.as_str(),
                            "class"
                                | "interface"
                                | "enum"
                                | "type_alias"
                                | "namespace"
                                | "type_parameter"
                        ) && matches!(
                            declaration.namespace,
                            Some(
                                SymbolNamespace::Type
                                    | SymbolNamespace::ValueAndType
                                    | SymbolNamespace::Namespace
                            )
                        )
                    })
                })
                .collect::<Vec<_>>();
            self.check_typed_candidates(locals.len())?;
            let bindings = self
                .indexes
                .typescript
                .type_bindings_by_scope
                .get(&key)
                .into_iter()
                .flatten()
                .filter(|id| {
                    self.facts.bindings.get(id).is_some_and(|binding| {
                        matches!(
                            binding.namespace,
                            Some(
                                SymbolNamespace::Type
                                    | SymbolNamespace::ValueAndType
                                    | SymbolNamespace::Namespace
                            )
                        )
                    })
                })
                .collect::<Vec<_>>();
            self.check_typed_candidates(bindings.len())?;
            let count = locals.len() + bindings.len();
            if count > 1 {
                return Err(TypeFailure::Ambiguous(count));
            }
            if let Some(slot) = locals.first().copied() {
                let local = self.declaration(slot).ok_or(TypeFailure::Unknown)?;
                if local.kind == "type_parameter" {
                    if parts.len() != 1 || !actuals.is_empty() {
                        return Err(TypeFailure::Unknown);
                    }
                    return environment
                        .get(&local.id)
                        .map_or(Ok(TypeValue::Unknown), |value| work.copy(value));
                }
                let mut target = slot;
                for member in &parts[1..] {
                    work.charge(1)?;
                    target = only(self.typed_member_slots(target, member)?)?;
                }
                return self.typed_nominal(target, actuals);
            }
            if let Some(binding) = bindings.first() {
                let path = parts[1..]
                    .iter()
                    .map(|part| (*part).to_owned())
                    .collect::<Vec<_>>();
                let target = self.typed_import_target(binding, &path, candidate, work)?;
                return self.typed_nominal(target, actuals);
            }
            scope = self
                .facts
                .scopes
                .get(scope_id)
                .and_then(|scope| scope.parent_scope_id.as_deref());
        }
        Err(TypeFailure::Unknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_languages::Engine;

    fn fixture_index(
        provider: &str,
        consumer: &str,
    ) -> Result<
        (
            tempfile::TempDir,
            Vec<SemanticEvidenceBatch>,
            UniversalResolutionIndex,
            String,
        ),
        Box<dyn std::error::Error>,
    > {
        fixture_files(&[
            ("provider.ts".to_owned(), provider.to_owned()),
            ("main.ts".to_owned(), consumer.to_owned()),
        ])
    }

    #[allow(clippy::type_complexity)]
    fn fixture_files(
        files: &[(String, String)],
    ) -> Result<
        (
            tempfile::TempDir,
            Vec<SemanticEvidenceBatch>,
            UniversalResolutionIndex,
            String,
        ),
        Box<dyn std::error::Error>,
    > {
        let root = tempfile::tempdir()?;
        let mut batches = Vec::new();
        for (file, source) in files {
            let path = root.path().join(file);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, source)?;
            batches.push(Engine::default().extract_source_universal_evidence(
                &path,
                file,
                source.as_bytes(),
            )?);
        }
        let lookups = batches
            .iter()
            .flat_map(|batch| &batch.candidates)
            .filter(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "lookup"
            })
            .collect::<Vec<_>>();
        assert_eq!(
            lookups.len(),
            1,
            "fixture must name one intended typed lookup"
        );
        let candidate = lookups[0];
        assert!(candidate.constraints.typescript_type_arguments.is_some());
        let occurrence = batches
            .iter()
            .flat_map(|batch| &batch.occurrences)
            .find(|occurrence| Some(occurrence.id.as_str()) == candidate.occurrence_id.as_deref())
            .ok_or("missing lookup occurrence")?;
        assert_eq!(occurrence.range.source_file, "main.ts");
        assert!(occurrence.range.start_byte < occurrence.range.end_byte);
        let caller = batches
            .iter()
            .flat_map(|batch| &batch.declarations)
            .find(|declaration| declaration.id == candidate.source_declaration_id)
            .ok_or("missing lookup caller")?;
        assert_eq!(caller.range.source_file, "main.ts");
        let id = candidate.id.clone();
        let index = UniversalResolutionIndex::new_with_project_inventory_owned(
            batches.clone(),
            &[],
            &[],
            root.path(),
            UniversalResolutionLimits::default(),
        )?;
        Ok((root, batches, index, id))
    }

    fn fixture_load_proof(
        batches: &[SemanticEvidenceBatch],
        kind: compass_languages::TypeScriptMergeDependencyKind,
        specifier: &str,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let dependency = batches
            .iter()
            .flat_map(|batch| &batch.typescript_merge_dependencies)
            .find(|dependency| dependency.kind == kind && dependency.specifier == specifier)
            .ok_or("missing observed source load")?;
        let proof = TypeScriptSourceLoadProof {
            version: 1,
            dependency: dependency.clone(),
        };
        let value = serde_json::to_value(&proof)?;
        assert_eq!(
            serde_json::from_value::<TypeScriptSourceLoadProof>(value.clone())?,
            proof
        );
        Ok(value)
    }

    #[test]
    fn typed_export_depth_overflow_cannot_publish_a_short_branch_winner()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut files = vec![
            ("provider.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
            ("other.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
            ("barrel.ts".to_owned(), "export * from './provider'; export * from './chain0';".to_owned()),
            ("main.ts".to_owned(), "import type { Holder } from './barrel'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()),
        ];
        for depth in 0..70 {
            let next = if depth == 69 {
                "other".to_owned()
            } else {
                format!("chain{}", depth + 1)
            };
            files.push((
                format!("chain{depth}.ts"),
                format!("export * from './{next}';"),
            ));
        }
        let (_root, _batches, index, id) = fixture_files(&files)?;
        assert!(
            matches!(index.resolve(&id), ResolutionDecision::ResourceLimit { .. }),
            "{:?}",
            index.resolve(&id)
        );
        Ok(())
    }

    #[test]
    fn typed_export_depth_boundary_is_inclusive_without_partial_targets()
    -> Result<(), Box<dyn std::error::Error>> {
        for depth in [64, 65] {
            let mut files = vec![
                ("provider.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
                ("barrel.ts".to_owned(), "export * from './chain0';".to_owned()),
                ("main.ts".to_owned(), "import type { Holder } from './barrel'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()),
            ];
            for hop in 0..depth - 1 {
                let next = if hop == depth - 2 {
                    "provider".to_owned()
                } else {
                    format!("chain{}", hop + 1)
                };
                files.push((
                    format!("chain{hop}.ts"),
                    format!("export * from './{next}';"),
                ));
            }
            let (_root, batches, index, id) = fixture_files(&files)?;
            let decision = index.resolve(&id);
            if depth == 64 {
                let target = batches
                    .iter()
                    .flat_map(|batch| &batch.declarations)
                    .find(|declaration| declaration.qualified_name == "main.Good.lookup")
                    .ok_or("missing expected lookup")?;
                assert!(
                    matches!(&decision, ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == &target.id),
                    "{decision:?}"
                );
            } else {
                assert!(
                    matches!(
                        decision,
                        ResolutionDecision::ResourceLimit {
                            resource: "typescript_export_depth",
                            limit: 64
                        }
                    ),
                    "{decision:?}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn typed_export_missing_branch_cannot_prove_a_unique_target()
    -> Result<(), Box<dyn std::error::Error>> {
        let files = vec![
            ("provider.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
            ("barrel.ts".to_owned(), "export * from './provider'; export * from './unavailable';".to_owned()),
            ("main.ts".to_owned(), "import type { Holder } from './barrel'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()),
        ];
        let (_root, _batches, index, id) = fixture_files(&files)?;
        assert!(
            matches!(index.resolve(&id), ResolutionDecision::Unresolved),
            "{:?}",
            index.resolve(&id)
        );
        Ok(())
    }

    #[test]
    fn typed_export_explicit_declaration_shadows_an_unavailable_star()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_root, _batches, index, id) = fixture_index(
            "export interface Holder<T> { field: T } export * from './unavailable';",
            "import type { Holder } from './provider'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }",
        )?;
        assert!(
            matches!(index.resolve(&id), ResolutionDecision::Resolved { .. }),
            "{:?}",
            index.resolve(&id)
        );
        Ok(())
    }

    #[test]
    fn typed_export_explicit_reexport_shadows_an_unavailable_star()
    -> Result<(), Box<dyn std::error::Error>> {
        let files = vec![
            ("provider.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
            ("barrel.ts".to_owned(), "export { Holder } from './provider'; export * from './unavailable';".to_owned()),
            ("main.ts".to_owned(), "import type { Holder } from './barrel'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()),
        ];
        let (_root, _batches, index, id) = fixture_files(&files)?;
        assert!(
            matches!(index.resolve(&id), ResolutionDecision::Resolved { .. }),
            "{:?}",
            index.resolve(&id)
        );
        Ok(())
    }

    #[test]
    fn typed_export_diamond_walk_shares_the_receiver_work_budget()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut files = vec![
            ("provider.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
            ("barrel.ts".to_owned(), "export * from './left0'; export * from './right0';".to_owned()),
            ("main.ts".to_owned(), "import type { Holder } from './barrel'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()),
        ];
        for depth in 0..8 {
            let source = if depth == 7 {
                "export * from './provider';".to_owned()
            } else {
                format!(
                    "export * from './left{}'; export * from './right{}';",
                    depth + 1,
                    depth + 1
                )
            };
            files.push((format!("left{depth}.ts"), source.clone()));
            files.push((format!("right{depth}.ts"), source));
        }
        let (_root, _batches, index, id) = fixture_files(&files)?;
        assert!(matches!(
            index.resolve(&id),
            ResolutionDecision::Resolved { .. }
        ));
        let run = |index: &UniversalResolutionIndex,
                   id: &str,
                   limit|
         -> Result<ResolutionDecision, Box<dyn std::error::Error>> {
            let indexes = index.indexes.lock().map_err(|_| "poisoned test index")?;
            let db = ResolutionDb::new(index, &indexes);
            let candidate = index.facts.candidates.get(id).ok_or("missing candidate")?;
            let path = typescript_member_path(
                candidate
                    .constraints
                    .qualified_name
                    .as_deref()
                    .ok_or("missing path")?,
                "lookup",
            )
            .ok_or("invalid path")?;
            Ok(db.resolve_typescript_typed_members_with_work(&path, &candidate, limit))
        };
        let minimum = |index: &UniversalResolutionIndex,
                       id: &str|
         -> Result<usize, Box<dyn std::error::Error>> {
            let expected = index.resolve(id);
            assert!(matches!(expected, ResolutionDecision::Resolved { .. }));
            let mut low = 0;
            let mut high = MAX_TYPED_STEPS;
            assert_eq!(run(index, id, high)?, expected);
            while high - low > 1 {
                let middle = low + (high - low) / 2;
                match run(index, id, middle)? {
                    ResolutionDecision::ResourceLimit {
                        resource: "typescript_type_steps",
                        ..
                    } => low = middle,
                    decision => {
                        assert_eq!(decision, expected);
                        high = middle;
                    }
                }
            }
            assert!(
                matches!(run(index, id, low)?, ResolutionDecision::ResourceLimit { resource: "typescript_type_steps", limit } if limit == low)
            );
            assert_eq!(run(index, id, high)?, expected);
            assert_eq!(run(index, id, high + 1)?, expected);
            Ok(high)
        };
        let (_direct_root, _direct_batches, direct, direct_id) = fixture_index(
            "export interface Holder<T> { field: T }",
            "import type { Holder } from './provider'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }",
        )?;
        let direct_steps = minimum(&direct, &direct_id)?;
        let diamond_steps = minimum(&index, &id)?;
        // Both queries have the same receiver transitions and one provider.
        // A walk over extra supplied export obligations must spend additional
        // units on the SAME meter before those receiver transitions run.
        assert!(
            diamond_steps > direct_steps,
            "direct={direct_steps}, diamond={diamond_steps}"
        );
        assert_eq!(
            run(&direct, &direct_id, diamond_steps - 1)?,
            direct.resolve(&direct_id)
        );

        Ok(())
    }

    #[test]
    fn typed_export_obligations_preserve_public_visibility_and_known_empty_sources()
    -> Result<(), Box<dyn std::error::Error>> {
        let cases: &[(&[(&str, &str)], bool)] = &[
            (
                &[
                    (
                        "provider.ts",
                        "export default class Container<T> { field: T } export interface Holder<T> { other: T }",
                    ),
                    (
                        "barrel.ts",
                        "export { default as Holder } from './provider';",
                    ),
                ],
                true,
            ),
            (
                &[
                    ("provider.ts", "interface Holder<T> { field: T } export {};"),
                    ("barrel.ts", "export * from './provider';"),
                ],
                false,
            ),
            (
                &[
                    (
                        "provider.ts",
                        "export namespace Nested { export interface Holder<T> { field: T } }",
                    ),
                    ("barrel.ts", "export * from './provider';"),
                ],
                false,
            ),
            (
                &[
                    ("provider.ts", "export interface Holder<T> { field: T }"),
                    (
                        "barrel.ts",
                        "export { Holder } from './unavailable'; export * from './provider';",
                    ),
                ],
                false,
            ),
            (
                &[
                    ("provider.ts", "export interface Holder<T> { field: T }"),
                    ("empty.ts", ""),
                    (
                        "barrel.ts",
                        "export * from './provider'; export * from './empty';",
                    ),
                ],
                true,
            ),
            (
                &[
                    ("provider.ts", "export interface Holder<T> { field: T }"),
                    (
                        "cycle.ts",
                        "export * from './barrel'; export * from './provider';",
                    ),
                    ("barrel.ts", "export * from './cycle';"),
                ],
                true,
            ),
            (
                &[
                    ("cycle.ts", "export * from './barrel';"),
                    ("barrel.ts", "export * from './cycle';"),
                ],
                false,
            ),
            (
                &[
                    ("provider.ts", "export interface Holder<T> { field: T }"),
                    ("barrel.ts", "export { Holder } from 'provider';"),
                ],
                false,
            ),
        ];
        for (sources, expected) in cases {
            let mut files = sources
                .iter()
                .map(|(file, source)| (file.to_string(), source.to_string()))
                .collect::<Vec<_>>();
            files.push(("main.ts".to_owned(), "import type { Holder } from './barrel'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()));
            let (root, batches, index, id) = fixture_files(&files)?;
            let decision = index.resolve(&id);
            if *expected {
                let target = batches
                    .iter()
                    .flat_map(|batch| &batch.declarations)
                    .find(|declaration| declaration.qualified_name == "main.Good.lookup")
                    .ok_or("missing expected lookup")?;
                assert!(
                    matches!(&decision, ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == &target.id),
                    "{sources:?}: {decision:?}"
                );
            } else {
                assert!(
                    matches!(decision, ResolutionDecision::Unresolved),
                    "{sources:?}: {decision:?}"
                );
            }
            let reversed = UniversalResolutionIndex::new_with_inventory(
                &batches.into_iter().rev().collect::<Vec<_>>(),
                &[],
                root.path(),
                UniversalResolutionLimits::default(),
            )?;
            assert_eq!(decision, reversed.resolve(&id));
        }
        Ok(())
    }

    #[test]
    fn typed_export_unretained_syntax_cannot_become_a_star_winner()
    -> Result<(), Box<dyn std::error::Error>> {
        for barrel in [
            "export { Missing as Holder }; export * from './provider';",
            "export * from './provider'; export { Missing as Holder };",
            "export * from './provider'; export { Broken as",
            "export * from './provider'; export = missing;",
            "export { default as Holder } from './provider';",
            "export { Missing as 'Holder' }; export * from './provider';",
            "export * from './provider'; export * from './recovered';",
            "export * from './provider'; export * from './unsupported';",
        ] {
            let files = [
                ("recovered.ts".to_owned(), "export { Broken as".to_owned()),
                ("unsupported.ts".to_owned(), "export = missing;".to_owned()),
                ("provider.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
                ("barrel.ts".to_owned(), barrel.to_owned()),
                ("main.ts".to_owned(), "import type { Holder } from './barrel'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()),
            ];
            let (root, batches, index, id) = fixture_files(&files)?;
            let decision = index.resolve(&id);
            assert!(
                matches!(decision, ResolutionDecision::Unresolved),
                "{barrel}: {decision:?}"
            );
            let reversed = batches
                .into_iter()
                .rev()
                .map(|mut batch| {
                    batch.bindings.reverse();
                    batch.declarations.reverse();
                    batch
                })
                .collect::<Vec<_>>();
            let index = UniversalResolutionIndex::new_with_inventory(
                &reversed,
                &[],
                root.path(),
                UniversalResolutionLimits::default(),
            )?;
            assert_eq!(decision, index.resolve(&id), "reordered {barrel}");
        }
        Ok(())
    }

    #[test]
    fn typed_export_legacy_or_incomplete_source_cannot_certify_empty_star()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, original, _index, id) = fixture_files(&[
            ("provider.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
            ("empty.ts".to_owned(), "".to_owned()),
            ("barrel.ts".to_owned(), "export * from './provider'; export * from './empty';".to_owned()),
            ("main.ts".to_owned(), "import type { Holder } from './barrel'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()),
        ])?;
        for file in ["empty.ts", "provider.ts", "barrel.ts"] {
            for absent in [true, false] {
                let mut batches = original.clone();
                let source = batches
                    .iter_mut()
                    .flat_map(|batch| &mut batch.declarations)
                    .find(|fact| fact.scope_id.is_none() && fact.range.source_file == file)
                    .ok_or("missing source")?;
                if absent {
                    source.ecmascript_export_inventory = None;
                } else {
                    source
                        .ecmascript_export_inventory
                        .as_mut()
                        .ok_or("missing census")?
                        .complete = false;
                }
                for reversed in [false, true] {
                    if reversed {
                        batches.reverse();
                    }
                    let index = UniversalResolutionIndex::new_with_inventory(
                        &batches,
                        &[],
                        root.path(),
                        UniversalResolutionLimits::default(),
                    )?;
                    assert!(
                        matches!(index.resolve(&id), ResolutionDecision::Unresolved),
                        "{file}, absent={absent}, reversed={reversed}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn typed_export_owning_targets_keep_exact_file_identity_and_sticky_unknowns()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, batches, _index, id) = fixture_files(&[
            ("provider.ts".to_owned(), "export {};".to_owned()),
            ("provider/index.ts".to_owned(), "export interface Holder<T> { field: T }".to_owned()),
            ("main.ts".to_owned(), "import type { Holder } from './provider'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }".to_owned()),
        ])?;
        let edge = |target: &str| EdgeRecord {
            source: "main".to_owned(),
            target: target.to_owned(),
            attributes: Map::from_iter([
                (
                    "relation".to_owned(),
                    Value::String("imports_from".to_owned()),
                ),
                (
                    "source_file".to_owned(),
                    Value::String("main.ts".to_owned()),
                ),
                ("target_file".to_owned(), Value::String(target.to_owned())),
                ("module".to_owned(), Value::String("./provider".to_owned())),
            ]),
        };
        for (targets, expected) in [
            (vec!["provider.ts"], false),
            (vec!["provider/index.ts"], true),
            (vec!["provider.ts", "provider/index.ts"], false),
            (vec!["provider/index.ts", "unavailable.ts"], false),
        ] {
            let edges = targets
                .iter()
                .map(|target| edge(target))
                .collect::<Vec<_>>();
            let run =
                |edges: &[EdgeRecord]| -> Result<ResolutionDecision, Box<dyn std::error::Error>> {
                    let index = UniversalResolutionIndex::new_with_project_inventory_owned(
                        batches.clone(),
                        &[],
                        edges,
                        root.path(),
                        UniversalResolutionLimits::default(),
                    )?;
                    Ok(index.resolve(&id))
                };
            let decision = run(&edges)?;
            if expected {
                let target = batches
                    .iter()
                    .flat_map(|batch| &batch.declarations)
                    .find(|declaration| declaration.qualified_name == "main.Good.lookup")
                    .ok_or("missing expected lookup")?;
                assert!(
                    matches!(&decision, ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == &target.id),
                    "{targets:?}: {decision:?}"
                );
            } else {
                assert!(
                    matches!(decision, ResolutionDecision::Unresolved),
                    "{targets:?}: {decision:?}"
                );
            }
            assert_eq!(decision, run(&edges.into_iter().rev().collect::<Vec<_>>())?);
        }
        Ok(())
    }

    #[test]
    fn typed_work_accepts_the_exact_boundary_without_partial_results()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_root, _batches, index, id) = fixture_index(
            "export interface Holder<T> { field: T }",
            "import type { Holder } from './provider'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }",
        )?;
        let indexes = index.indexes.lock().map_err(|_| "poisoned test index")?;
        let db = ResolutionDb::new(&index, &indexes);
        let candidate = index.facts.candidates.get(&id).ok_or("missing candidate")?;
        let path = typescript_member_path(
            candidate
                .constraints
                .qualified_name
                .as_deref()
                .ok_or("missing member path")?,
            "lookup",
        )
        .ok_or("invalid member path")?;
        let run = |limit| db.resolve_typescript_typed_members_with_work(&path, &candidate, limit);
        let expected = run(MAX_TYPED_STEPS);
        assert!(matches!(expected, ResolutionDecision::Resolved { .. }));
        let mut low = 0;
        let mut high = MAX_TYPED_STEPS;
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            match run(middle) {
                ResolutionDecision::ResourceLimit {
                    resource: "typescript_type_steps",
                    ..
                } => low = middle,
                decision => {
                    assert_eq!(decision, expected);
                    high = middle;
                }
            }
        }
        assert!(
            matches!(run(low), ResolutionDecision::ResourceLimit { resource: "typescript_type_steps", limit } if limit == low)
        );
        assert_eq!(run(high), expected);
        assert_eq!(run(high + 1), expected);
        Ok(())
    }

    #[test]
    fn legacy_true_merge_certificate_without_census_version_is_unknown()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, batches, _, id) = fixture_index(
            "export interface Holder<T> { field: Awaited<T>; direct(): void }",
            "import type {Holder} from './provider'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); value.direct(); }",
        )?;
        let direct = batches
            .iter()
            .flat_map(|batch| &batch.candidates)
            .find(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "direct"
            })
            .ok_or("missing direct call")?
            .id
            .clone();
        for pipeline_version in [4, 5] {
            let mut legacy = batches.clone();
            assert_eq!(legacy[0].typescript_merge_inventory_complete, Some(true));
            legacy[0].typescript_merge_inventory_version = None;
            legacy[0].pipeline.version = pipeline_version;
            // A producer identity alone cannot stand in for this capability.
            let index = UniversalResolutionIndex::new_with_project_inventory_owned(
                legacy,
                &[],
                &[],
                root.path(),
                UniversalResolutionLimits::default(),
            )?;
            assert_eq!(index.resolve(&id), ResolutionDecision::Unresolved);
            assert!(matches!(
                index.resolve(&direct),
                ResolutionDecision::Resolved { .. }
            ));
        }
        Ok(())
    }

    #[test]
    fn awaited_identity_requires_explicit_complete_terminal_inventory()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, batches, index, id) = fixture_index(
            "export interface Holder<T> { field: Awaited<Promise<T>> }",
            "import type { Holder } from './provider'; class Good { lookup() {} } function read(value: Holder<Good>) { value.field.lookup(); }",
        )?;
        let good = batches
            .iter()
            .flat_map(|batch| &batch.declarations)
            .find(|fact| fact.qualified_name == "main.Good")
            .ok_or("missing terminal class")?;
        assert_eq!(good.member_inventory_complete, Some(true));
        assert!(good.direct_bases_complete);
        let target = batches
            .iter()
            .flat_map(|batch| &batch.declarations)
            .find(|fact| fact.qualified_name == "main.Good.lookup")
            .ok_or("missing actual member")?;
        assert!(
            matches!(index.resolve(&id), ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == target.id)
        );
        for case in 0..5 {
            let mut incomplete = batches.clone();
            let terminal = incomplete
                .iter_mut()
                .flat_map(|batch| &mut batch.declarations)
                .find(|fact| fact.id == good.id)
                .ok_or("missing retained class")?;
            match case {
                0 => terminal.member_inventory_complete = None,
                1 => terminal.member_inventory_complete = Some(false),
                2 => terminal.direct_bases_complete = false,
                _ => {}
            }
            if case >= 3 {
                incomplete[0].typescript_merge_inventory_complete =
                    if case == 3 { None } else { Some(false) };
            }
            let incomplete = UniversalResolutionIndex::new_with_project_inventory_owned(
                incomplete,
                &[],
                &[],
                root.path(),
                UniversalResolutionLimits::default(),
            )?;
            assert_eq!(incomplete.resolve(&id), ResolutionDecision::Unresolved);
        }
        let missing = NodeRecord {
            id: "missing-typescript-file".to_owned(),
            attributes: Map::from_iter([
                (
                    "source_file".to_owned(),
                    Value::String("missing.ts".to_owned()),
                ),
                (
                    "language".to_owned(),
                    Value::String("typescript".to_owned()),
                ),
            ]),
        };
        let partial = UniversalResolutionIndex::new_with_project_inventory_owned(
            batches.clone(),
            &[missing],
            &[],
            root.path(),
            UniversalResolutionLimits::default(),
        )?;
        assert_eq!(partial.resolve(&id), ResolutionDecision::Unresolved);
        Ok(())
    }

    fn assert_missing_augmentation_coverage_is_unknown(
        prefix: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let provider = format!(
            "{prefix}\nexport class Good {{ lookup() {{}} }} export interface Holder<T> {{ field: Awaited<T> }}"
        );
        let (_root, _batches, index, id) = fixture_index(
            &provider,
            "import type { Holder, Good } from './provider'; function read(value: Holder<Good>) { value.field.lookup(); }",
        )?;
        assert_eq!(index.resolve(&id), ResolutionDecision::Unresolved);
        Ok(())
    }

    #[test]
    fn awaited_missing_side_effect_dependency_prevents_absence_proof()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_missing_augmentation_coverage_is_unknown("import './augment';")
    }

    #[test]
    fn awaited_missing_reference_path_prevents_absence_proof()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_missing_augmentation_coverage_is_unknown(
            r#"/// <reference path="./augment.d.ts" />"#,
        )
    }

    #[test]
    fn awaited_missing_reference_types_prevents_absence_proof()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_missing_augmentation_coverage_is_unknown(r#"/// <reference types="augment" />"#)
    }

    #[test]
    fn awaited_load_extension_must_not_use_an_unrelated_module_stem()
    -> Result<(), Box<dyn std::error::Error>> {
        for prefix in ["import './provider.mjs';", "import './provider.cjs';"] {
            assert_missing_augmentation_coverage_is_unknown(prefix)?;
        }
        Ok(())
    }

    #[test]
    fn awaited_invalid_project_load_target_must_not_fall_back_to_lexical_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, mut batches, _, id) = fixture_index(
            "import './augment'; export class Good { lookup() {} } export interface Holder<T> { field: Awaited<T> }",
            "import type {Holder, Good} from './provider'; function read(value: Holder<Good>) { value.field.lookup(); }",
        )?;
        let source = b"export {};";
        let path = root.path().join("augment.ts");
        std::fs::write(&path, source)?;
        batches.push(Engine::default().extract_source_universal_evidence(
            &path,
            "augment.ts",
            source,
        )?);
        let edges = [EdgeRecord {
            source: "provider".to_owned(),
            target: "outside".to_owned(),
            attributes: Map::from_iter([
                (
                    "relation".to_owned(),
                    Value::String("imports_from".to_owned()),
                ),
                (
                    "source_file".to_owned(),
                    Value::String("provider.ts".to_owned()),
                ),
                (
                    "target_file".to_owned(),
                    Value::String("../outside.d.ts".to_owned()),
                ),
                ("module".to_owned(), Value::String("./augment".to_owned())),
            ]),
        }];
        let index = UniversalResolutionIndex::new_with_project_inventory_owned(
            batches,
            &[],
            &edges,
            root.path(),
            UniversalResolutionLimits::default(),
        )?;
        assert_eq!(index.resolve(&id), ResolutionDecision::Unresolved);
        Ok(())
    }

    #[test]
    fn awaited_missing_literal_quote_is_not_a_similarly_named_supplied_source()
    -> Result<(), Box<dyn std::error::Error>> {
        for prefix in [
            r#"import './provider"';"#,
            r#"export * from "./provider'";"#,
            r#"export const loaded = import('./provider`');"#,
        ] {
            assert_missing_augmentation_coverage_is_unknown(prefix)?;
        }
        Ok(())
    }

    #[test]
    fn awaited_known_and_unknown_project_targets_do_not_certify_absence()
    -> Result<(), Box<dyn std::error::Error>> {
        let (root, mut batches, _, id) = fixture_index(
            "/// <reference types=\"augment\" />\nexport class Good { lookup() {} } export interface Holder<T> { field: Awaited<T>; direct(): void }",
            "import type {Holder, Good} from './provider'; function read(value: Holder<Good>) { value.field.lookup(); value.direct(); }",
        )?;
        let path = root.path().join("augment.d.ts");
        let source = b"export {};";
        std::fs::write(&path, source)?;
        batches.push(Engine::default().extract_source_universal_evidence(
            &path,
            "augment.d.ts",
            source,
        )?);
        let mut known = EdgeRecord {
            source: "provider".to_owned(),
            target: "known".to_owned(),
            attributes: Map::from_iter([
                (
                    "relation".to_owned(),
                    Value::String("imports_from".to_owned()),
                ),
                (
                    "source_file".to_owned(),
                    Value::String("provider.ts".to_owned()),
                ),
                (
                    "target_file".to_owned(),
                    Value::String("augment.d.ts".to_owned()),
                ),
                ("module".to_owned(), Value::String("augment".to_owned())),
            ]),
        };
        known.attributes.insert(
            TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE.to_owned(),
            fixture_load_proof(
                &batches,
                compass_languages::TypeScriptMergeDependencyKind::ReferenceTypes,
                "augment",
            )?,
        );
        let complete = UniversalResolutionIndex::new_with_project_inventory_owned(
            batches.clone(),
            &[],
            std::slice::from_ref(&known),
            root.path(),
            UniversalResolutionLimits::default(),
        )?;
        assert!(matches!(
            complete.resolve(&id),
            ResolutionDecision::Resolved { .. }
        ));
        let direct = batches
            .iter()
            .flat_map(|batch| &batch.candidates)
            .find(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "direct"
            })
            .ok_or("missing independent direct member")?
            .id
            .clone();
        let one = UniversalResolutionLimits {
            candidates_per_lookup: 1,
            ..UniversalResolutionLimits::default()
        };
        UniversalResolutionIndex::new_with_project_inventory_owned(
            batches.clone(),
            &[],
            std::slice::from_ref(&known),
            root.path(),
            one,
        )?;
        assert!(
            matches!(UniversalResolutionIndex::new_with_project_inventory_owned(batches.clone(), &[], &[known.clone(), known.clone()], root.path(), one), Err(reason) if reason.contains("source-load alternatives"))
        );
        for case in 0..3 {
            let mut unknown = known.clone();
            unknown.target = "unavailable".to_owned();
            if case == 1 {
                unknown.attributes.insert(
                    "target_file".to_owned(),
                    Value::String("../outside.d.ts".to_owned()),
                );
            } else {
                unknown.attributes.remove("target_file");
            }
            if case == 2 {
                unknown.attributes.insert(
                    "context".to_owned(),
                    Value::String("other-condition".to_owned()),
                );
            }
            for edges in [[known.clone(), unknown.clone()], [unknown, known.clone()]] {
                let index = UniversalResolutionIndex::new_with_project_inventory_owned(
                    batches.clone(),
                    &[],
                    &edges,
                    root.path(),
                    UniversalResolutionLimits::default(),
                )?;
                assert_eq!(
                    index.resolve(&id),
                    ResolutionDecision::Unresolved,
                    "case {case}"
                );
                assert!(matches!(
                    index.resolve(&direct),
                    ResolutionDecision::Resolved { .. }
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn awaited_same_spelling_load_kinds_need_their_own_source_proofs()
    -> Result<(), Box<dyn std::error::Error>> {
        use compass_languages::TypeScriptMergeDependencyKind as Kind;
        let (root, mut batches, _, id) = fixture_index(
            "/// <reference types=\"augment\" />\nimport 'augment'; export class Good { lookup() {} } export interface Holder<T> { field: Awaited<T>; direct(): void }",
            "import type {Holder, Good} from './provider'; function read(value: Holder<Good>) { value.field.lookup(); value.direct(); }",
        )?;
        let source = b"export {};";
        for file in ["module.ts", "types.d.ts"] {
            let path = root.path().join(file);
            std::fs::write(&path, source)?;
            batches.push(Engine::default().extract_source_universal_evidence(&path, file, source)?);
        }
        let edge = |kind: Kind, target: &str| -> Result<EdgeRecord, Box<dyn std::error::Error>> {
            let dependency = batches
                .iter()
                .flat_map(|batch| &batch.typescript_merge_dependencies)
                .find(|dependency| dependency.kind == kind && dependency.specifier == "augment")
                .ok_or("missing typed load")?;
            Ok(EdgeRecord {
                source: "provider".to_owned(),
                target: target.to_owned(),
                attributes: Map::from_iter([
                    (
                        "relation".to_owned(),
                        Value::String("imports_from".to_owned()),
                    ),
                    (
                        "source_file".to_owned(),
                        Value::String("provider.ts".to_owned()),
                    ),
                    ("target_file".to_owned(), Value::String(target.to_owned())),
                    ("module".to_owned(), Value::String("augment".to_owned())),
                    (
                        "typescript_source_load".to_owned(),
                        serde_json::to_value(TypeScriptSourceLoadProof {
                            version: 1,
                            dependency: dependency.clone(),
                        })?,
                    ),
                ]),
            })
        };
        let module = edge(Kind::Module, "module.ts")?;
        let types = edge(Kind::ReferenceTypes, "types.d.ts")?;
        let direct = batches
            .iter()
            .flat_map(|batch| &batch.candidates)
            .find(|candidate| {
                candidate.relation == CandidateRelation::Calls
                    && candidate.target_spelling == "direct"
            })
            .ok_or("missing direct member")?
            .id
            .clone();
        for (mut edges, expected) in [
            (vec![module.clone()], false),
            (vec![types.clone()], false),
            (vec![module.clone(), types.clone()], true),
        ] {
            for reverse in [false, true] {
                let mut inputs = batches.clone();
                if reverse {
                    inputs.reverse();
                    edges.reverse();
                }
                let index = UniversalResolutionIndex::new_with_project_inventory_owned(
                    inputs,
                    &[],
                    &edges,
                    root.path(),
                    UniversalResolutionLimits::default(),
                )?;
                assert_eq!(
                    matches!(index.resolve(&id), ResolutionDecision::Resolved { .. }),
                    expected,
                    "edge count {}; reverse={reverse}",
                    edges.len()
                );
                assert!(matches!(
                    index.resolve(&direct),
                    ResolutionDecision::Resolved { .. }
                ));
            }
        }
        for kind in [Kind::Module, Kind::ReferenceTypes] {
            let mut edges = vec![module.clone(), types.clone()];
            let unknown = edges
                .iter_mut()
                .find(|edge| {
                    edge.attributes["typescript_source_load"]["dependency"]["kind"]
                        == serde_json::to_value(kind).unwrap_or(Value::Null)
                })
                .ok_or("missing target mode")?;
            unknown.attributes.remove("target_file");
            let index = UniversalResolutionIndex::new_with_project_inventory_owned(
                batches.clone(),
                &[],
                &edges,
                root.path(),
                UniversalResolutionLimits::default(),
            )?;
            assert_eq!(index.resolve(&id), ResolutionDecision::Unresolved);
        }
        Ok(())
    }

    #[test]
    fn awaited_observed_loads_require_covered_non_augmenting_sources()
    -> Result<(), Box<dyn std::error::Error>> {
        for (prefix, file, project_mapping) in [
            ("import './augment';", "augment.ts", false),
            (
                "import type { Marker } from './augment';",
                "augment.ts",
                false,
            ),
            ("export * from './augment';", "augment.ts", false),
            (
                r#"/// <reference path="./augment.d.ts" />"#,
                "augment.d.ts",
                false,
            ),
            (r#"/// <reference types="augment" />"#, "augment.d.ts", true),
        ] {
            let provider = format!(
                "{prefix}\nexport class Good {{ lookup() {{}} }} export interface Holder<T> {{ field: Awaited<T>; direct(): void }}"
            );
            let (root, batches, _, id) = fixture_index(
                &provider,
                "import type {Holder, Good} from './provider'; function read(value: Holder<Good>) { value.field.lookup(); value.direct(); }",
            )?;
            let mut edges = if project_mapping {
                vec![EdgeRecord {
                    source: "provider".to_owned(),
                    target: "augmentation".to_owned(),
                    attributes: Map::from_iter([
                        (
                            "relation".to_owned(),
                            Value::String("imports_from".to_owned()),
                        ),
                        (
                            "source_file".to_owned(),
                            Value::String("provider.ts".to_owned()),
                        ),
                        ("target_file".to_owned(), Value::String(file.to_owned())),
                        ("module".to_owned(), Value::String("augment".to_owned())),
                    ]),
                }]
            } else {
                Vec::new()
            };
            if project_mapping {
                edges[0].attributes.insert(
                    TYPESCRIPT_SOURCE_LOAD_PROOF_ATTRIBUTE.to_owned(),
                    fixture_load_proof(
                        &batches,
                        compass_languages::TypeScriptMergeDependencyKind::ReferenceTypes,
                        "augment",
                    )?,
                );
            }
            for (loaded, resolves) in [
                ("export interface Marker {}", true),
                (
                    "import './provider'; declare module './provider' { interface Good { then(done: (value: string) => void): void } }",
                    false,
                ),
                ("import './missing'; export interface Marker {}", false),
            ] {
                let path = root.path().join(file);
                std::fs::write(&path, loaded)?;
                let extra = Engine::default().extract_source_universal_evidence(
                    &path,
                    file,
                    loaded.as_bytes(),
                )?;
                let mut inputs = batches.clone();
                inputs.push(extra);
                for reverse in [false, true] {
                    if reverse {
                        inputs.reverse();
                    }
                    let index = UniversalResolutionIndex::new_with_project_inventory_owned(
                        inputs.clone(),
                        &[],
                        &edges,
                        root.path(),
                        UniversalResolutionLimits::default(),
                    )?;
                    let decision = index.resolve(&id);
                    assert_eq!(
                        matches!(decision, ResolutionDecision::Resolved { .. }),
                        resolves,
                        "{prefix}; {loaded}; reverse={reverse}: {decision:?}"
                    );
                    let direct = inputs
                        .iter()
                        .flat_map(|batch| &batch.candidates)
                        .find(|candidate| {
                            candidate.relation == CandidateRelation::Calls
                                && candidate.target_spelling == "direct"
                        })
                        .ok_or("missing independent direct call")?;
                    assert!(matches!(
                        index.resolve(&direct.id),
                        ResolutionDecision::Resolved { .. }
                    ));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn typed_depth_limits_publish_explicit_collection_degradation()
    -> Result<(), Box<dyn std::error::Error>> {
        let expression = format!("value.{}lookup()", "next.".repeat(17));
        let consumer = format!(
            "import type {{ Holder }} from './provider'; class Good {{}} function read(value: Holder<Good>) {{ {expression}; }}"
        );
        let (root, batches, index, id) = fixture_index(
            "export interface Holder<T> { next: Holder<T>; lookup(): void }",
            &consumer,
        )?;
        assert!(matches!(
            index.resolve(&id),
            ResolutionDecision::ResourceLimit {
                resource: "typescript_member_depth",
                limit: 16
            }
        ));
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        // The extraction also retains member-read observations along this
        // chain. The public report must preserve every explicit limit, not
        // just the final lookup call selected by fixture_index.
        let limited_ids = index
            .candidate_ids()
            .into_iter()
            .filter(|candidate_id| {
                matches!(
                    index.resolve(candidate_id),
                    ResolutionDecision::ResourceLimit { .. }
                )
            })
            .collect::<Vec<_>>();
        assert!(limited_ids.contains(&id.as_str()));
        let public_report = index.materialize(&mut nodes, &mut edges);
        assert_eq!(public_report.omitted_candidates, limited_ids.len());
        assert!(
            !edges.iter().any(|edge| edge.string("relation") == "calls"
                && edge.string("source_file") == "main.ts")
        );
        nodes.clear();
        edges.clear();
        let report = crate::evidence::partition::materialize_bounded_owned(
            batches,
            &[],
            root.path(),
            UniversalResolutionLimits::default(),
            crate::ResolutionAdmission::Low,
            false,
            (&mut nodes, &mut edges),
        );
        assert!(report.degraded && report.omitted_candidates > 0);
        assert_eq!(report.failed_partitions, 0);
        assert!(
            report
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("TypeScript receiver analysis"))
        );
        assert!(
            !edges.iter().any(|edge| edge.string("relation") == "calls"
                && edge.string("source_file") == "main.ts")
        );
        Ok(())
    }

    #[test]
    fn generic_object_and_indexed_aliases_keep_exact_owned_parameters()
    -> Result<(), Box<dyn std::error::Error>> {
        for (provider, expression, access) in [
            (
                "export type Holder<T> = { field: T };",
                "Holder<Good>",
                "value.field.lookup()",
            ),
            (
                "export type Holder<T> = T['nested'];",
                "Holder<Good>",
                "value.lookup()",
            ),
        ] {
            let root = tempfile::tempdir()?;
            let consumer = format!(
                "import type {{ Holder }} from './provider'; class Leaf {{ lookup() {{}} }} class Good {{ nested: Leaf; lookup() {{}} }} export function read(value: {expression}) {{ return {access}; }}"
            );
            let mut batches = Vec::new();
            for (file, source) in [("provider.ts", provider), ("main.ts", consumer.as_str())] {
                let path = root.path().join(file);
                std::fs::write(&path, source)?;
                batches.push(Engine::default().extract_source_universal_evidence(
                    &path,
                    file,
                    source.as_bytes(),
                )?);
            }
            let id = batches
                .iter()
                .flat_map(|batch| &batch.candidates)
                .find(|candidate| {
                    candidate.relation == CandidateRelation::Calls
                        && candidate.target_spelling == "lookup"
                })
                .ok_or("missing lookup candidate")?
                .id
                .clone();
            let index = UniversalResolutionIndex::new_with_project_inventory_owned(
                batches.clone(),
                &[],
                &[],
                root.path(),
                UniversalResolutionLimits::default(),
            )?;
            let decision = index.resolve(&id);
            let expected_name = if access.contains("field") {
                "main.Good.lookup"
            } else {
                "main.Leaf.lookup"
            };
            let expected = index
                .facts
                .declarations
                .values()
                .find(|declaration| declaration.qualified_name == expected_name)
                .ok_or("missing expected method")?;
            assert!(
                matches!(&decision, ResolutionDecision::Resolved { declaration_id, .. } if declaration_id == &expected.id),
                "{provider}: {decision:?}; declarations/scopes: {:#?}",
                batches
                    .iter()
                    .map(|batch| (&batch.declarations, &batch.scopes))
                    .collect::<Vec<_>>()
            );
        }
        Ok(())
    }
}
